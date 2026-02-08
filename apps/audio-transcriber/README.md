
# Audio Transcriber Service

A high-performance, production-ready audio transcription service built around OpenAI's Whisper model, designed to handle real-time audio streaming with proper CPU-bound workload management.

## Architecture Philosophy

This service is architected around a fundamental constraint: **Whisper is a blocking, CPU-intensive, single-threaded operation**. Everything in this codebase respects that reality instead of fighting it.

### Core Design Principles

#### 1. **Async Until the Model, Blocking at the Model**

```
┌─────────────────────────────────────────┐
│  ASYNC LAYER (Non-blocking I/O)        │
├─────────────────────────────────────────┤
│  • NATS message reception              │
│  • Audio chunk decoding                │
│  • Sample rate conversion              │
│  • Buffer accumulation                 │
│  • Silence/noise detection             │
└──────────────┬──────────────────────────┘
               │
               ▼
      ┌────────────────┐
      │ Bounded Queue  │  (Backpressure)
      │   (4 jobs)     │
      └────────┬───────┘
               │
               ▼
┌─────────────────────────────────────────┐
│  BLOCKING LAYER (CPU-bound)             │
├─────────────────────────────────────────┤
│  • Single Whisper worker thread        │
│  • One model instance per stream       │
│  • Sequential transcription            │
│  • Cannot be cancelled mid-job         │
└──────────────┬──────────────────────────┘
               │
               ▼
      ┌────────────────┐
      │ Async Publish  │  (NATS)
      └────────────────┘
```

**Why this matters:**
- The async layer ensures audio ingestion never blocks waiting for transcription
- The queue provides natural backpressure when CPU is saturated
- The blocking worker respects Whisper's FFI constraints
- Horizontal scaling is clean: more workers = more streams, no contention

#### 2. **The Model is a Serializing Owner**

One Whisper model instance cannot be safely called concurrently. This service embraces this by:

- **One worker per stream**: Each audio stream gets its own dedicated worker thread
- **Sequential processing**: Jobs are processed FIFO, maintaining temporal order
- **No cross-thread access**: Each worker owns its model instance exclusively
- **State integrity**: Whisper's internal state remains consistent

**Alternative rejected approaches:**
- ❌ Multiple threads sharing one model → FFI safety violations
- ❌ Multiple models per stream → transcript merging nightmares, ordering issues
- ❌ Async-wrapping blocking calls → hides the problem, doesn't solve it

#### 3. **Backpressure is a Feature, Not a Bug**

When the queue fills up (CPU saturation), the service **intentionally drops audio chunks**:

```rust
match queue_tx.try_send(job) {
    Ok(_) => { /* enqueued */ },
    Err(Full(job)) => {
        metrics.jobs_dropped.add(1);
        warn!("Queue full - dropping audio");
        // This is CORRECT behavior under load
    }
}
```

**Why this is correct:**
- Better to drop audio than lie about timeliness
- Bounded queue prevents unbounded memory growth
- Drops are observable via metrics (not silent failures)
- Large queues hide saturation instead of exposing it

**Queue capacity is scientifically derived:**
```
CAPACITY = floor(max_acceptable_latency / (audio_duration × RTF))

Example:
- Max latency: 20s (product requirement)
- Audio duration: 5s per job
- Whisper RTF: 0.8 (on this CPU)
- Result: floor(20 / (5 × 0.8)) = 5 jobs

DO NOT increase arbitrarily - larger queues = higher latency
```

## System Components

### 1. Audio Ingestion (`audio/`)
**Responsibilities:**
- Receive PCM audio chunks from NATS
- Decode from bytes to f32 samples
- Normalize sample rates (48kHz → 16kHz for Whisper)
- Accumulate into fixed-duration buffers
- Detect silence/heartbeat timeouts

**Key characteristic:** Fully async, never blocks

### 2. Transcription Queue (`worker/queue.rs`)
**Responsibilities:**
- Bounded MPSC channel (capacity: 4)
- Sequence numbering for ordering
- Job metadata (created_at, audio_duration, etc.)

**Design decisions:**
- Small capacity (4) forces early backpressure
- `try_send` not `send().await` - non-blocking producer
- Jobs are immutable once enqueued

### 3. Whisper Worker (`worker/whisper.rs`)
**Responsibilities:**
- Blocking loop on dedicated thread
- Whisper model invocation (FFI)
- Result publishing back to async runtime

**Critical constraints:**
- Cannot be cancelled mid-transcription
- One worker per service instance
- Abandons work on shutdown (OS cleanup)

### 4. State & Observability
**Metrics tracked:**
- Queue depth, latency, drops
- Whisper processing time (RTF)
- End-to-end latency
- Worker busy/idle state

**Gauges use atomic backing:**
```rust
metrics.gauges.audio_buffer_size.store(size, Ordering::Relaxed);
```
No locks, fully async-safe.

## Configuration

### Environment Variables
```bash
# OpenTelemetry
OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4317

# NATS
NATS_URL=nats://localhost:4222

# Whisper
WHISPER_MODEL_PATH=/models/ggml-base.bin
WHISPER_THREADS=4

# Audio
TARGET_SAMPLE_RATE=16000
BUFFER_DURATION_SECS=5
```

### Queue Capacity Tuning

The queue capacity is a **product decision**, not a technical one:

```
TRANSCRIPTION_QUEUE_CAPACITY = floor(L_max / (D × RTF))

Where:
- L_max = maximum acceptable end-to-end latency (seconds)
- D = audio duration per job (buffer_duration_secs)
- RTF = Whisper real-time factor (measure in production)
```

**Example calculations:**

| Use Case | L_max | D (secs) | RTF | Capacity |
|----------|-------|----------|-----|----------|
| Live captions | 10s | 5s | 0.8 | 2 |
| Near-live | 20s | 5s | 0.8 | 5 |
| Post-processing | 60s | 10s | 1.0 | 6 |

**To measure RTF in production:**
```
RTF = processing_latency_ms / (audio_duration_secs × 1000)
```

Monitor `transcription_processing_latency` histogram P50/P95/P99.

## Operational Characteristics

### Shutdown Behavior

**Graceful shutdown sequence:**
1. SIGTERM/SIGINT received
2. Cancellation token signals async tasks
3. Audio ingestion stops accepting new chunks
4. Queue sender dropped (no new jobs)
5. Worker completes current job (if any)
6. Process exits after grace period (200ms)

**If worker is mid-transcription during shutdown:**
- Thread is abandoned (cannot be cancelled)
- OS cleans up thread on process exit
- This is **safe and correct** for containers

**Logged on exit:**
```
jobs_enqueued: 142
jobs_dropped: 3
queue_depth: 0
```

### Performance Characteristics

**Throughput (single worker):**
```
Max throughput = 1 / (D × RTF)

Example (D=5s, RTF=0.8):
Max = 1 / 4s = 0.25 jobs/sec = 15 jobs/min
```

**Latency breakdown:**
```
End-to-end latency = queue_latency + processing_latency + publish_latency

Typical distribution:
- queue_latency: 0-20s (depends on queue depth)
- processing_latency: 3-6s (Whisper execution)
- publish_latency: <100ms (NATS)
```

**CPU utilization:**
- Worker busy = 100% CPU on worker thread
- Worker idle = minimal CPU
- Async tasks = negligible CPU (I/O bound)

### Scaling Strategy

**Horizontal scaling (recommended):**
```
┌─────────────┐     ┌─────────────┐     ┌─────────────┐
│ Stream A    │     │ Stream B    │     │ Stream C    │
│ Transcriber │     │ Transcriber │     │ Transcriber │
│ (1 worker)  │     │ (1 worker)  │     │ (1 worker)  │
└─────────────┘     └─────────────┘     └─────────────┘
      ▲                   ▲                   ▲
      └───────────────────┴───────────────────┘
                    NATS (routing)
```

Each instance:
- Subscribes to specific stream (via subject routing)
- Owns one Whisper model
- Independent queue and metrics
- No inter-instance coordination

**When to scale:**
- `transcription_jobs_dropped` counter increasing
- `transcription_queue_latency` P95 > 50% of L_max
- `worker_busy` gauge always = 1

## Metrics & Observability

### Key Metrics

**Queue health:**
```
transcriber.queue.depth          # Current jobs waiting
transcriber.jobs.enqueued        # Total jobs accepted
transcriber.jobs.dropped         # Total jobs dropped (backpressure)
transcriber.queue.latency_ms     # Time waiting in queue
```

**Transcription performance:**
```
transcriber.processing.latency_ms     # Whisper execution time
transcriber.end_to_end.latency_ms     # Total pipeline latency
transcriber.worker.busy               # 0 = idle, 1 = transcribing
```

**Audio ingestion:**
```
transcriber.chunks.received      # Raw audio chunks from NATS
transcriber.chunks.dropped       # Decode/processing failures
transcriber.buffer.size          # Current buffer fill level
```

### Alert Thresholds (Recommended)

```yaml
# Queue saturation
- alert: TranscriptionQueueSaturated
  expr: transcriber_queue_depth >= transcriber_queue_capacity * 0.8
  for: 2m

# High drop rate
- alert: TranscriptionJobsDropping
  expr: rate(transcriber_jobs_dropped[5m]) > 0.1
  for: 1m

# High latency
- alert: TranscriptionLatencyHigh
  expr: histogram_quantile(0.95, transcriber_end_to_end_latency_ms) > 25000
  for: 5m

# Worker starvation
- alert: TranscriptionWorkerStarved
  expr: transcriber_worker_busy == 0 AND transcriber_queue_depth > 0
  for: 30s
```

## Future Improvements

### 1. Pre-Transcription Filtering (High Priority)

**Problem:** Silent or noisy audio chunks waste CPU on Whisper invocations that produce no useful output.

**Solution:** Add VAD (Voice Activity Detection) before queueing:

```rust
// In audio processor, before creating TranscriptionJob:
if is_silence(&audio_buffer) || is_noise(&audio_buffer) {
    metrics.chunks_filtered.add(1);
    debug!("Filtered silent/noisy chunk");
    return Ok(()); // Don't enqueue
}
```

**Implementation options:**
- **webrtc-vad**: Fast, lightweight, good for real-time
- **silero-vad**: ML-based, more accurate but slower
- **Energy threshold**: Cheapest, least accurate

**Expected impact:**
- 30-50% reduction in wasted Whisper calls
- Lower latency (fewer jobs in queue)
- Same transcription quality (only filtering non-speech)

### 2. Multi-Stream Support (Medium Priority)

**Current:** One service instance = one audio stream

**Enhancement:** Support multiple concurrent streams with stream-aware routing:

```rust
struct StreamWorker {
    stream_id: String,
    queue: TranscriptionQueue,
    worker: WhisperWorker,
    model: Arc<WhisperContext>,
}

struct MultiStreamTranscriber {
    workers: HashMap<String, StreamWorker>,
    // Dynamic worker creation/cleanup
}
```

**Benefits:**
- Better resource utilization (one deployment, N streams)
- Stream isolation (one slow stream doesn't block others)
- Dynamic scaling (add/remove workers as streams start/stop)

**Challenges:**
- Memory: N models in memory (consider lazy loading)
- Fairness: Prevent one stream from starving others
- Monitoring: Per-stream metrics cardinality

### 3. Adaptive Queue Sizing (Low Priority)

**Current:** Static queue capacity based on worst-case latency

**Enhancement:** Dynamically adjust queue size based on measured RTF:

```rust
// Measure RTF over sliding window
let measured_rtf = processing_latency / audio_duration;

// Adjust capacity to maintain target latency
let optimal_capacity = (target_latency / (buffer_duration * measured_rtf)).floor();

// Update queue (requires new queue implementation)
```

**Benefits:**
- Better CPU utilization on fast hardware
- Consistent latency across different deployment environments
- Automatic adaptation to model changes

**Challenges:**
- Queue resizing is non-trivial (need draining strategy)
- Risk of oscillation (needs smoothing)
- Complexity vs. benefit tradeoff

### 4. Whisper Model Hot-Swapping (Low Priority)

**Use case:** Upgrade model without service restart

**Approach:**
```rust
// Load new model
let new_model = load_model(&new_model_path)?;

// Drain queue
while let Some(job) = queue.try_recv() {
    process_with_old_model(job);
}

// Swap atomically
Arc::make_mut(&worker.model).swap(new_model);
```

**Challenges:**
- Memory spike (two models loaded briefly)
- Queue draining could take minutes
- Complexity: probably not worth it (restart is fine)

### 5. Persistent Queue for Durability (Medium Priority)

**Current:** In-memory queue - jobs lost on crash

**Enhancement:** Disk-backed queue for at-least-once processing:

```rust
// Use something like:
// - sled (embedded key-value store)
// - SQLite (simple, reliable)
// - Custom memory-mapped file

struct DurableQueue {
    mem_queue: VecDeque<TranscriptionJob>,
    wal: WriteAheadLog, // Persist to disk
}
```

**Benefits:**
- No lost audio on crashes
- Can survive restarts mid-job
- Replay capability for debugging

**Tradeoffs:**
- Slower enqueue (disk I/O)
- More complex (need compaction, recovery)
- May not be needed (idempotent source = fine to re-process)

### 6. GPU Acceleration (High Impact, High Effort)

**Current:** CPU-only Whisper (via whisper.cpp)

**Enhancement:** GPU-accelerated inference:

**Options:**
- **faster-whisper** (CTranslate2): 4x speedup on GPU
- **whisper.cpp** with cuBLAS: 3x speedup
- **TensorRT**: 5-8x speedup, complex setup

**Architecture changes needed:**
```rust
// GPU worker pool (multiple workers, one GPU)
struct GpuWorkerPool {
    workers: Vec<GpuWorker>,
    queue: Arc<TranscriptionQueue>,
    // Round-robin or work-stealing
}
```

**Expected impact:**
- RTF: 0.8 → 0.15 (5x faster)
- Queue capacity: Can be reduced proportionally
- Latency: Dramatic reduction
- Cost: GPU instance required

**Considerations:**
- Batch size tuning for GPU efficiency
- Memory management (VRAM limits)
- Fallback to CPU if GPU unavailable

### 7. Streaming Transcription (Low Priority)

**Current:** Fixed buffer size (e.g., 5s chunks)

**Enhancement:** Continuous streaming with sliding window:

```rust
// Instead of discrete jobs:
// - Continuous audio feed to model
// - Emit words/segments as they're decoded
// - Lower latency (no buffer accumulation)
```

**Challenges:**
- Whisper's architecture expects full audio context
- Would need custom Whisper integration
- Significantly more complex
- Probably requires model changes (not just wrapper code)

## Development

### Prerequisites
```bash
# Rust toolchain
rustup default stable

# Whisper model
wget https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin

# NATS server (for local testing)
docker run -p 4222:4222 nats:latest
```

### Running Locally
```bash
# Set environment
export WHISPER_MODEL_PATH=./ggml-base.bin
export NATS_URL=nats://localhost:4222

# Run
cargo run --release
```

### Testing
```bash
# Unit tests
cargo test

# Integration test (requires NATS)
cargo test --test integration -- --ignored

# Benchmarks
cargo bench
```

### Profiling

**CPU profiling:**
```bash
# Install flamegraph
cargo install flamegraph

# Profile
sudo flamegraph -- ./target/release/audio-transcriber

# View flamegraph.svg
```

**Memory profiling:**
```bash
# Install heaptrack
sudo apt install heaptrack

# Profile
heaptrack ./target/release/audio-transcriber

# Analyze
heaptrack_gui heaptrack.*.gz
```

## Deployment

### Container Image
```dockerfile
FROM rust:1.75-slim AS builder
WORKDIR /app
COPY . .
RUN cargo build --release

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates
COPY --from=builder /app/target/release/audio-transcriber /usr/local/bin/
COPY models/ /models/

ENV WHISPER_MODEL_PATH=/models/ggml-base.bin
ENV NATS_URL=nats://nats:4222

CMD ["audio-transcriber"]
```

### Kubernetes Deployment
```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: audio-transcriber
spec:
  replicas: 3  # Horizontal scaling
  selector:
    matchLabels:
      app: audio-transcriber
  template:
    metadata:
      labels:
        app: audio-transcriber
    spec:
      containers:
      - name: transcriber
        image: audio-transcriber:latest
        resources:
          requests:
            cpu: "2"      # Whisper needs CPU
            memory: "2Gi"
          limits:
            cpu: "4"
            memory: "4Gi"
        env:
        - name: WHISPER_MODEL_PATH
          value: "/models/ggml-base.bin"
        - name: NATS_URL
          value: "nats://nats.default.svc:4222"
        - name: OTEL_EXPORTER_OTLP_ENDPOINT
          value: "http://otel-collector:4317"
```

## Design Rationale Summary

**This service is designed around constraints, not wishes:**

1. **Whisper is blocking** → We isolate it in a dedicated thread
2. **Whisper is CPU-intensive** → We limit concurrency with a bounded queue
3. **Whisper cannot be cancelled** → We abandon on shutdown (OS cleanup)
4. **Audio is continuous** → We use async ingestion to prevent blocking
5. **CPU is finite** → We drop audio when saturated (backpressure)

**What makes this architecture correct:**
- ✅ Respects FFI safety constraints
- ✅ Makes saturation observable (metrics, not silent drops)
- ✅ Scales horizontally (more instances, not bigger instances)
- ✅ Fails explicitly (drops with logs) not implicitly (timeouts, deadlocks)
- ✅ Simple mental model (producer-queue-consumer, not async spaghetti)

**This is production-grade because:**
- Every line of code has a documented reason
- Every constant has a derivation
- Every drop is logged
- Every latency is measured
- Shutdown is explicit, not hopeful

---

## License

MIT

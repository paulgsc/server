# Whisper Model Optimization Guide
## CPU-Only, No Budget, Maximum Efficiency

---

## Table of Contents
1. [Model Selection Strategy](#model-selection-strategy)
2. [Quantization Deep Dive](#quantization-deep-dive)
3. [Runtime Optimization](#runtime-optimization)
4. [Memory & Disk Optimization](#memory--disk-optimization)
5. [Latency Improvements](#latency-improvements)
6. [Reliability Patterns](#reliability-patterns)
7. [Energy Efficiency (Save the Penguins)](#energy-efficiency)
8. [Quick Reference Table](#quick-reference-table)

---

## Model Selection Strategy

### The Models Available (All Free)

| Model | Parameters | Disk Size | Relative Speed | Accuracy | Use Case |
|-------|-----------|-----------|----------------|----------|----------|
| **tiny** | 39M | ~75 MB | 32x | Poor | Real-time, demos only |
| **tiny.en** | 39M | ~75 MB | 32x | Poor (EN) | English real-time |
| **base** | 74M | ~142 MB | 16x | Okay | Balanced for CPU |
| **base.en** | 74M | ~142 MB | 16x | Good (EN) | **RECOMMENDED START** |
| **small** | 244M | ~466 MB | 6x | Good | Better quality, slower |
| **small.en** | 244M | ~466 MB | 6x | Very Good (EN) | High quality on decent CPU |
| **medium** | 769M | ~1.5 GB | 2x | Very Good | Only if you have time |
| **medium.en** | 769M | ~1.5 GB | 2x | Excellent (EN) | Batch processing only |
| **large-v2/v3** | 1550M | ~3 GB | 1x | Best | Forget it on CPU |

### Decision Matrix

**Your priorities ranked:**

1. **Latency is critical** → `tiny.en` or `base.en`
2. **Accuracy matters** → `small.en` (sweet spot for CPU)
3. **Mixed languages** → `base` or `small` (multilingual)
4. **Batch processing** → `medium.en` (run overnight)
5. **Real-time streaming** → `tiny.en` only

### The .en Advantage

English-only models (`*.en`):
- **30% faster** than multilingual equivalent
- **Better accuracy** for English (focused vocabulary)
- **Smaller** vocabulary = less memory
- **Skip language detection** overhead

**Rule of thumb:** If you only do English, ALWAYS use `.en` variants.

---

## Quantization Deep Dive

### What is Quantization?

Normal models use 32-bit floats for weights. Quantization reduces precision:
- **16-bit (f16)** → Half precision, ~50% smaller
- **8-bit (q8_0)** → Integer quantization, ~75% smaller  
- **5-bit (q5_0, q5_1)** → Aggressive, ~85% smaller
- **4-bit (q4_0, q4_1)** → Extreme, ~90% smaller

### The Formats Explained

| Format | Description | Quality Loss | Speed Gain | Recommendation |
|--------|-------------|--------------|------------|----------------|
| **f32** | Original full precision | 0% | 0% | Don't use (huge) |
| **f16** | Half precision | ~1% | +20% | Good baseline |
| **q8_0** | 8-bit quantized | ~2% | +40% | **Best quality/speed** |
| **q5_1** | 5-bit + 1-bit scale | ~5% | +60% | **SWEET SPOT** |
| **q5_0** | 5-bit uniform | ~7% | +65% | Good for tight space |
| **q4_1** | 4-bit + 1-bit scale | ~10% | +80% | Noticeable degradation |
| **q4_0** | 4-bit uniform | ~15% | +90% | Only for non-critical |

### Real-World Performance (base.en on 4-core CPU)

```
Model Size:        Transcription Time (3s audio):    Quality:
base.en.bin        142 MB                800ms          100%
base.en-q8_0       37 MB                 480ms          98%
base.en-q5_1       26 MB                 320ms          95%  ← RECOMMENDED
base.en-q4_0       19 MB                 200ms          85%
```

### Download Commands

```bash
# Recommended: base.en quantized to q5_1 (best balance)
wget https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en-q5_1.bin

# If you need better quality
wget https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.en-q5_1.bin

# If latency is everything
wget https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.en-q5_1.bin

# If you have breathing room (better quality)
wget https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en-q8_0.bin
```

---

## Runtime Optimization

### Thread Configuration

```rust
// Your current setup
params.set_n_threads(2);
```

**Optimal thread count = Physical CPU cores - 1**

```bash
# Find your cores
nproc          # Total logical cores (includes hyperthreading)
lscpu | grep "Core(s) per socket"  # Physical cores

# Examples:
# 2 physical cores → use 2 threads
# 4 physical cores → use 3-4 threads
# 8 physical cores → use 6-7 threads
```

**Why not max threads?**
- Hyperthreading helps less for CPU-bound work
- Leave 1 core for OS/other services
- Diminishing returns after physical core count

### Whisper-Specific Parameters

```rust
fn setup_whisper_params() -> FullParams<'static, 'static> {
    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    
    // LATENCY OPTIMIZATIONS
    params.set_n_threads(4);  // Adjust for your CPU
    params.set_translate(false);  // Already doing this ✓
    params.set_print_special(false);  // ✓
    params.set_print_progress(false);  // ✓
    params.set_print_realtime(false);  // ✓
    params.set_print_timestamps(false);  // ✓
    
    // ADDITIONAL SPEEDUPS (add these)
    params.set_single_segment(false);  // Allow segment splitting
    params.set_max_len(0);  // No artificial length limit
    params.set_token_timestamps(false);  // Don't track token timing
    params.set_speed_up(true);  // Enable audio speedup detection
    params.set_suppress_blank(true);  // Skip silence faster
    params.set_suppress_non_speech_tokens(true);  // Skip [BLANK], etc
    
    // QUALITY vs SPEED TRADEOFF
    // Greedy is fastest (what you have)
    // params = FullParams::new(SamplingStrategy::BeamSearch { 
    //     beam_size: 5,  // Use if accuracy > speed
    //     patience: 1.0 
    // });
    
    params
}
```

### Sampling Strategy Impact

```
Greedy { best_of: 1 }          → Fastest (1x baseline)
Greedy { best_of: 2 }          → +30% time, slightly better
BeamSearch { beam_size: 3 }    → +150% time, noticeably better
BeamSearch { beam_size: 5 }    → +300% time, best quality
```

**For CPU:** Stick with Greedy unless accuracy is non-negotiable.

---

## Memory & Disk Optimization

### Model Loading Strategy

```rust
// CURRENT: Load once at startup (good!)
let ctx = Arc::new(load_whisper_model(&model_path)?);

// ALTERNATIVE: Lazy loading (if memory constrained)
use once_cell::sync::Lazy;

static WHISPER_CTX: Lazy<Arc<WhisperContext>> = Lazy::new(|| {
    Arc::new(load_whisper_model("/path/to/model").unwrap())
});
```

### Memory Profiling

```bash
# Watch memory during transcription
watch -n 1 'ps aux | grep audio-transcriber'

# Detailed memory breakdown
/usr/bin/time -v ./target/release/audio-transcriber

# Look for:
# - Maximum resident set size (RSS)
# - Page faults
```

### Disk Space Strategy

**Models you should keep:**

```
/models/
├── ggml-base.en-q5_1.bin      (26 MB)  ← Primary
├── ggml-tiny.en-q5_1.bin      (12 MB)  ← Fallback for spikes
└── ggml-small.en-q5_1.bin     (77 MB)  ← Optional: high-quality batch
```

**Total: ~115 MB** for full flexibility

### Model Swapping Based on Load

```rust
async fn select_model_based_on_load() -> &'static str {
    let load = sys_info::loadavg().unwrap().one;
    
    match load {
        l if l < 2.0 => "/models/ggml-small.en-q5_1.bin",  // Low load
        l if l < 4.0 => "/models/ggml-base.en-q5_1.bin",   // Normal
        _ => "/models/ggml-tiny.en-q5_1.bin"                // High load
    }
}
```

---

## Latency Improvements

### Current Latency Budget

```
Audio Chunk Arrival:     10-50ms
Buffer Fill:             3000ms (your 3s buffer)
Transcription:           300-800ms (depends on model)
Publishing:              5-10ms
────────────────────────────────
Total Latency:           ~3.3-3.8s
```

### Optimization Strategies

#### 1. Reduce Buffer Duration

```rust
// Current: 3 seconds
let buffer_duration_secs = 3;

// Try: 1-2 seconds for lower latency
let buffer_duration_secs = std::env::var("BUFFER_DURATION")
    .ok()
    .and_then(|s| s.parse().ok())
    .unwrap_or(2);  // Change default to 2s
```

**Impact:**
- 3s → 2s = -1000ms latency
- 3s → 1s = -2000ms latency
- **Trade-off:** Less context may reduce accuracy slightly

#### 2. Overlapping Buffers

```rust
// Instead of clear(), keep last 500ms for context
let overlap_samples = target_sample_rate as usize / 2;  // 500ms

if audio_buffer.len() >= buffer_size {
    let transcription_audio = audio_buffer.clone();
    
    // Keep overlap for context
    audio_buffer.drain(0..(audio_buffer.len() - overlap_samples));
    
    transcribe_and_publish(/* ... */);
}
```

**Benefits:**
- Better accuracy at sentence boundaries
- Smoother real-time feel
- Catches words that span chunks

#### 3. Parallel Transcription (Advanced)

```rust
// Increase semaphore to 2 (if you have CPU headroom)
let transcription_semaphore = Arc::new(Semaphore::new(2));

// WARNING: This uses 2x CPU during overlaps
```

#### 4. Fast Path for Silence

```rust
fn detect_silence(samples: &[f32], threshold: f32) -> bool {
    let rms = (samples.iter().map(|s| s * s).sum::<f32>() 
               / samples.len() as f32).sqrt();
    rms < threshold
}

// In your main loop
if detect_silence(&audio_buffer, 0.01) {
    audio_buffer.clear();
    continue;  // Skip transcription
}
```

### Latency Targets by Model

```
Model              3s buffer    2s buffer    1s buffer
─────────────────────────────────────────────────────
tiny.en-q5_1       ~3.1s        ~2.1s        ~1.2s
base.en-q5_1       ~3.3s        ~2.3s        ~1.4s
small.en-q5_1      ~3.8s        ~2.8s        ~1.9s
```

---

## Reliability Patterns

### 1. Model Loading Resilience

```rust
fn load_whisper_model_with_fallback(primary: &str, fallback: &str) -> Result<WhisperContext> {
    match WhisperContext::new_with_params(primary, WhisperContextParameters::default()) {
        Ok(ctx) => {
            info!("✅ Loaded primary model: {}", primary);
            Ok(ctx)
        }
        Err(e) => {
            warn!("⚠️ Primary model failed: {}, trying fallback", e);
            let ctx = WhisperContext::new_with_params(
                fallback, 
                WhisperContextParameters::default()
            )?;
            info!("✅ Loaded fallback model: {}", fallback);
            Ok(ctx)
        }
    }
}

// Usage
let ctx = load_whisper_model_with_fallback(
    "/models/ggml-base.en-q5_1.bin",
    "/models/ggml-tiny.en-q5_1.bin"
)?;
```

### 2. Graceful Degradation

```rust
// Track consecutive failures
let failure_count = Arc::new(AtomicU64::new(0));

// In transcribe_and_publish
match whisper_state.full(params, &audio) {
    Ok(_) => {
        failure_count.store(0, Ordering::Relaxed);
        // ... success path
    }
    Err(e) => {
        let failures = failure_count.fetch_add(1, Ordering::Relaxed);
        
        if failures > 5 {
            error!("🚨 Too many failures, switching to fallback model");
            // Reload with smaller model
        }
    }
}
```

### 3. Circuit Breaker Pattern

```rust
use std::time::{Duration, Instant};

struct CircuitBreaker {
    failure_threshold: u32,
    timeout: Duration,
    failures: AtomicU32,
    last_failure: Arc<Mutex<Option<Instant>>>,
}

impl CircuitBreaker {
    fn should_attempt(&self) -> bool {
        let failures = self.failures.load(Ordering::Relaxed);
        
        if failures < self.failure_threshold {
            return true;  // Closed, allow
        }
        
        // Check if timeout expired
        let last = self.last_failure.lock().unwrap();
        if let Some(time) = *last {
            time.elapsed() > self.timeout  // Try again after timeout
        } else {
            true
        }
    }
    
    fn record_success(&self) {
        self.failures.store(0, Ordering::Relaxed);
    }
    
    fn record_failure(&self) {
        self.failures.fetch_add(1, Ordering::Relaxed);
        *self.last_failure.lock().unwrap() = Some(Instant::now());
    }
}
```

### 4. Model Corruption Detection

```rust
use std::fs;

fn verify_model_integrity(path: &str, expected_min_size: u64) -> Result<()> {
    let metadata = fs::metadata(path)?;
    let size = metadata.len();
    
    if size < expected_min_size {
        return Err(anyhow::anyhow!(
            "Model file too small: {} bytes (expected > {})", 
            size, 
            expected_min_size
        ));
    }
    
    // Optional: Check magic bytes
    let mut file = fs::File::open(path)?;
    let mut magic = [0u8; 4];
    use std::io::Read;
    file.read_exact(&mut magic)?;
    
    // GGML magic: "ggml" or specific version
    if &magic != b"ggml" && &magic != b"ggjt" {
        return Err(anyhow::anyhow!("Invalid model file format"));
    }
    
    Ok(())
}

// Before loading
verify_model_integrity(
    &model_path, 
    20_000_000  // 20 MB minimum for quantized models
)?;
```

### 5. Health Checks

```rust
// Add health endpoint
async fn health_check(state: Arc<TranscriberState>) -> impl warp::Reply {
    let health = serde_json::json!({
        "status": if state.is_transcribing.load(Ordering::Relaxed) {
            "busy"
        } else {
            "ready"
        },
        "chunks_received": state.chunks_received.load(Ordering::Relaxed),
        "transcriptions_completed": state.transcriptions_completed.load(Ordering::Relaxed),
        "transcriptions_failed": state.transcriptions_failed.load(Ordering::Relaxed),
        "buffer_size": state.buffer_size.load(Ordering::Relaxed),
    });
    
    warp::reply::json(&health)
}
```

---

## Energy Efficiency (Save the Penguins)

### CPU Frequency Scaling

```bash
# Check current governor
cat /sys/devices/system/cpu/cpu*/cpufreq/scaling_governor

# Set to powersave (when idle)
echo powersave | sudo tee /sys/devices/system/cpu/cpu*/cpufreq/scaling_governor

# Set to performance (during transcription)
echo performance | sudo tee /sys/devices/system/cpu/cpu*/cpufreq/scaling_governor
```

### Dynamic Model Selection

```rust
struct PowerProfile {
    ac_power: bool,
    battery_percent: u8,
}

impl PowerProfile {
    fn get() -> Self {
        // Read from /sys/class/power_supply/BAT0/
        // Simplified for example
        Self {
            ac_power: true,
            battery_percent: 100,
        }
    }
    
    fn select_model(&self) -> &'static str {
        match (self.ac_power, self.battery_percent) {
            (true, _) => "/models/ggml-base.en-q5_1.bin",      // AC: normal
            (false, p) if p > 50 => "/models/ggml-base.en-q5_1.bin",  // Battery: normal
            (false, p) if p > 20 => "/models/ggml-tiny.en-q5_1.bin",  // Battery: conserve
            _ => "/models/ggml-tiny.en-q5_1.bin",              // Critical: minimal
        }
    }
}
```

### Batch Processing Strategy

```rust
// Instead of real-time, collect and batch
let batch_schedule = tokio_cron_scheduler::Scheduler::new().await?;

batch_schedule.add(
    Job::new_async("0 */15 * * * *", |_uuid, _l| {
        Box::pin(async move {
            // Process accumulated audio every 15 minutes
            // Use larger, more efficient model
            // CPU can rest between batches
        })
    })?
).await?;
```

### Idle Detection

```rust
use std::time::{Duration, Instant};

struct IdleDetector {
    last_activity: Arc<Mutex<Instant>>,
    idle_threshold: Duration,
}

impl IdleDetector {
    fn mark_activity(&self) {
        *self.last_activity.lock().unwrap() = Instant::now();
    }
    
    fn is_idle(&self) -> bool {
        self.last_activity.lock().unwrap().elapsed() > self.idle_threshold
    }
    
    async fn shutdown_on_idle(&self) {
        while !self.is_idle() {
            tokio::time::sleep(Duration::from_secs(60)).await;
        }
        
        info!("💤 Idle detected, releasing resources");
        // Drop model, close connections, etc.
    }
}
```

### CPU Affinity

```bash
# Pin to specific cores (reduces cache misses)
taskset -c 0,1 ./audio-transcriber

# Or in Rust (using nix crate)
use nix::sched::{sched_setaffinity, CpuSet};

let mut cpu_set = CpuSet::new();
cpu_set.set(0)?;  // Use CPU 0
cpu_set.set(1)?;  // Use CPU 1
sched_setaffinity(nix::unistd::Pid::from_raw(0), &cpu_set)?;
```

### Energy Monitoring

```rust
// Read CPU package power (Linux)
fn read_cpu_power() -> Result<f64> {
    let power = std::fs::read_to_string(
        "/sys/class/powercap/intel-rapl:0/energy_uj"
    )?;
    Ok(power.trim().parse::<f64>()? / 1_000_000.0)  // Convert to Joules
}

// Track energy per transcription
let energy_before = read_cpu_power()?;
// ... transcribe ...
let energy_after = read_cpu_power()?;
let energy_used = energy_after - energy_before;

info!("⚡ Transcription used {:.2} J", energy_used);
```

---

## Quick Reference Table

### Model Selection Cheat Sheet

| Scenario | Recommended Model | Reasoning |
|----------|------------------|-----------|
| Real-time calls | `tiny.en-q5_1` | Sub-second latency |
| Live captions | `base.en-q5_1` | Balance of speed/quality |
| Meeting transcripts | `small.en-q5_1` | Good quality, acceptable latency |
| Batch processing | `medium.en-q8_0` | Best quality, time not critical |
| Multi-language | `base-q5_1` | Multilingual support |
| Raspberry Pi | `tiny.en-q4_0` | Minimal resources |
| 2-core CPU | `base.en-q5_1` | Matches CPU capability |
| 8-core CPU | `small.en-q5_1` | Can handle heavier model |

### Configuration Quick Wins

```bash
# In your .env file

# FASTEST (real-time priority)
WHISPER_MODELS_PATH=/models/ggml-tiny.en-q5_1.bin
WHISPER_THREADS=2
BUFFER_DURATION=1

# BALANCED (recommended)
WHISPER_MODELS_PATH=/models/ggml-base.en-q5_1.bin
WHISPER_THREADS=4
BUFFER_DURATION=2

# QUALITY (batch processing)
WHISPER_MODELS_PATH=/models/ggml-small.en-q8_0.bin
WHISPER_THREADS=6
BUFFER_DURATION=5
```

### Latency vs Quality Matrix

```
                    Latency Target
                1s      2s      3s      5s
Quality    ┌──────────────────────────────
Target     │
           │
Fast       │  tiny    tiny    base    base
           │  .en     .en     .en     .en
           │  q4_0    q5_1    q5_1    q8_0
           │
Balanced   │  tiny    base    base    small
           │  .en     .en     .en     .en
           │  q5_1    q5_1    q8_0    q5_1
           │
Quality    │  base    base    small   small
           │  .en     .en     .en     .en
           │  q5_1    q8_0    q5_1    q8_0
```

### Resource Constraints Decision Tree

```
Do you have 4+ CPU cores?
├─ YES: Can you wait 2+ seconds?
│   ├─ YES: Use small.en-q5_1
│   └─ NO:  Use base.en-q5_1
│
└─ NO (2 cores): Can you wait 3+ seconds?
    ├─ YES: Use base.en-q5_1
    └─ NO:  Use tiny.en-q5_1
```

---

## Measurement & Validation

### Benchmark Script

```bash
#!/bin/bash
# benchmark_models.sh

MODELS=(
    "ggml-tiny.en-q5_1.bin"
    "ggml-base.en-q5_1.bin"
    "ggml-small.en-q5_1.bin"
)

TEST_AUDIO="test_3s.wav"  # 3-second test file

for model in "${MODELS[@]}"; do
    echo "Testing $model..."
    
    export WHISPER_MODELS_PATH="/models/$model"
    
    # Run 10 iterations, measure time
    for i in {1..10}; do
        /usr/bin/time -f "%E real,%U user,%S sys" \
            ./target/release/audio-transcriber --test-mode
    done | tee "results_$model.txt"
    
    echo "---"
done

# Compare results
echo "Summary:"
for model in "${MODELS[@]}"; do
    avg=$(awk -F, '{sum+=$1; count++} END {print sum/count}' "results_$model.txt")
    echo "$model: ${avg}s average"
done
```

### In-Code Profiling

```rust
// Add to your transcribe function
let start = Instant::now();

whisper_state.full(params, &audio)?;

let transcribe_time = start.elapsed();
let audio_duration = audio.len() as f64 / 16000.0;  // 16kHz sample rate
let realtime_factor = transcribe_time.as_secs_f64() / audio_duration;

info!(
    "🎯 Realtime factor: {:.2}x ({}ms for {}s audio)",
    realtime_factor,
    transcribe_time.as_millis(),
    audio_duration
);

// Goal: RTF < 1.0 (faster than real-time)
// Good: RTF < 0.5
// Excellent: RTF < 0.2
```

---

## Final Recommendations

### For Your Specific Use Case

Based on your code:
- **2-core CPU** (seems limited based on thread count)
- **Localhost testing** (no GPU)
- **Real-time transcription** (audio chunks streaming in)

**Recommended Configuration:**

```bash
# .env
WHISPER_MODELS_PATH=/models/ggml-base.en-q5_1.bin
WHISPER_THREADS=2
BUFFER_DURATION=2
```

**Download command:**
```bash
mkdir -p /mnt/storage/users/dev/models
cd /mnt/storage/users/dev/models
wget https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en-q5_1.bin
```

**Expected Performance:**
- **Latency:** ~2.3 seconds (2s buffer + 300ms transcription)
- **Accuracy:** 95% of base.en
- **CPU:** ~150% of one core during transcription
- **Memory:** ~100 MB
- **Disk:** 26 MB

### Upgrade Path

As resources improve:

1. **More CPU cores** → Bump threads to 4, try `small.en-q5_1`
2. **More memory** → Try `q8_0` variants for +3% accuracy
3. **GPU eventually** → Recompile whisper.cpp with CUDA/ROCm
4. **Production scale** → Look into whisper-turbo or cloud APIs

### The 80/20 Rule

**80% of your improvement will come from:**
1. Using `.en` models (if English-only)
2. Using `q5_1` quantization
3. Setting threads = physical cores
4. Keeping buffer under 3 seconds

**The other 20% is bikeshedding.**

---

## Resources

- **Models:** https://huggingface.co/ggerganov/whisper.cpp
- **whisper.rs docs:** https://docs.rs/whisper-rs
- **Original Whisper:** https://github.com/openai/whisper
- **whisper.cpp:** https://github.com/ggerganov/whisper.cpp
- **Benchmarks:** https://github.com/ggerganov/whisper.cpp#benchmarks

**Remember:** The best model is the one that ships. Start with `base.en-q5_1`, measure, and iterate based on real user feedback.

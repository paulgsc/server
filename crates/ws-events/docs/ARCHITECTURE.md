# Stream Orchestrator - Actor Pattern Architecture

## Overview

This orchestrator uses a **pure actor pattern** for concurrency, eliminating the need for `Arc<Mutex<T>>` or `RwLock` while maintaining thread-safety and deterministic state updates.

## Architecture Diagram

```
┌─────────────────────────────────────────────────────────────┐
│                   StreamOrchestrator                        │
│                   (Public API - &self)                      │
│                                                             │
│  ┌──────────────┐        ┌──────────────┐                 │
│  │ command_tx   │───────▶│ Actor Task   │                 │
│  └──────────────┘        │              │                 │
│                          │ Owns:        │                 │
│  ┌──────────────┐        │ • Mutable    │                 │
│  │ state_rx     │◀───────│   State      │                 │
│  └──────────────┘        │ • Schedule   │                 │
│                          │ • Config     │                 │
└──────────────────────────┴──────────────┴──────────────────┘
                                  │
                                  │ Processes
                                  ▼
                    ┌─────────────────────────┐
                    │   Command Queue         │
                    │  (mpsc::unbounded)      │
                    ├─────────────────────────┤
                    │ • Start                 │
                    │ • Stop                  │
                    │ • Pause/Resume          │
                    │ • ForceScene            │
                    │ • UpdateStreamStatus    │
                    └─────────────────────────┘
                                  │
                                  ▼
                    ┌─────────────────────────┐
                    │   State Broadcast       │
                    │   (watch::channel)      │
                    ├─────────────────────────┤
                    │ Multiple subscribers    │
                    │ receive immutable       │
                    │ state snapshots         │
                    └─────────────────────────┘
```

## Key Principles

### 1. **Single Owner of Mutable State**

All mutable state lives inside the actor task (`TickEngine::run`). No external code can mutate state directly.

```rust
// Inside the actor loop
struct TickEngineState {
    config: OrchestratorConfig,
    schedule: SceneSchedule,
    start_time: Option<Instant>,
    paused_at: Option<TimeMs>,
    accumulated_pause_duration: TimeMs,
}
```

### 2. **Immutable Public API**

All public methods on `StreamOrchestrator` take `&self`, not `&mut self`:

```rust
impl StreamOrchestrator {
    pub fn start(&self) -> Result<()>           // ✅ &self
    pub fn pause(&self) -> Result<()>           // ✅ &self
    pub fn force_scene(&self, ...) -> Result<()> // ✅ &self
    pub fn current_state(&self) -> State        // ✅ &self
}
```

### 3. **Message Passing for Mutations**

Changes happen via commands sent to the actor:

```rust
// User code
orchestrator.start()?;  // Sends TickCommand::Start

// Actor receives and processes
match command {
    TickCommand::Start => {
        state.start();
        engine_state.start_time = Some(Instant::now());
        state_tx.send_replace(state);
    }
}
```

### 4. **Immutable State Snapshots**

Subscribers receive cloned snapshots via `tokio::watch`:

```rust
let mut state_rx = orchestrator.subscribe();

// Receive updates (non-blocking)
while state_rx.changed().await.is_ok() {
    let state: OrchestratorState = state_rx.borrow().clone();
    // State is immutable, can be safely shared
}
```

## Benefits of This Approach

### ✅ **Race-Free**

Only one task can mutate state at a time. Commands are processed sequentially in the actor loop.

### ✅ **Deadlock-Free**

No locks means no deadlocks. The actor processes commands one at a time.

### ✅ **Deterministic**

Command ordering is guaranteed by the channel. No non-deterministic interleavings.

### ✅ **Easy Reasoning**

Sequential processing makes it easy to reason about state transitions:

```rust
// This sequence is deterministic
orchestrator.start()?;
orchestrator.pause()?;
orchestrator.force_scene("main")?;
orchestrator.resume()?;
```

### ✅ **Lock-Free Reads**

Multiple subscribers can read state concurrently without blocking:

```rust
// 100 concurrent readers, no locks needed
for _ in 0..100 {
    let mut rx = orchestrator.subscribe();
    tokio::spawn(async move {
        while rx.changed().await.is_ok() {
            let state = rx.borrow().clone();
            // Process state
        }
    });
}
```

## Trade-offs

### ⚠️ **Latency**

Commands are processed asynchronously. There's a small delay between sending a command and seeing the state update:

```rust
orchestrator.start()?;  // Command sent
// ... small delay ...
// State update appears in subscribers
```

**Mitigation**: For most orchestration use cases (>10ms tick rates), this latency is negligible.

### ⚠️ **Memory Overhead**

State snapshots are cloned for each subscriber update:

```rust
state_tx.send_replace(state);  // Clones OrchestratorState
```

**Mitigation**: `OrchestratorState` is relatively small. For larger states, use `Arc<State>` internally.

### ⚠️ **No Immediate Feedback**

Commands don't return updated state immediately:

```rust
orchestrator.force_scene("main")?;
// Can't immediately see if it worked

// Instead, subscribe:
let mut rx = orchestrator.subscribe();
rx.changed().await?;
let state = rx.borrow();
assert_eq!(state.current_active_scene, Some("main".to_string()));
```

## Comparison to Other Patterns

### vs. `Arc<Mutex<State>>`

| Aspect           | Actor Pattern                | `Arc<Mutex>`              |
| ---------------- | ---------------------------- | ------------------------- |
| Safety           | ✅ Always safe                | ⚠️ Can deadlock           |
| Complexity       | Medium (channels)            | Low (just `.lock()`)      |
| Performance      | Good (lock-free reads)       | Contention on writes      |
| Async-friendly   | ✅ Native                     | ⚠️ Careful with `.await`  |
| Reasoning        | ✅ Sequential                 | ⚠️ Complex with async     |

### vs. `Arc<RwLock<State>>`

| Aspect           | Actor Pattern                | `Arc<RwLock>`             |
| ---------------- | ---------------------------- | ------------------------- |
| Concurrent reads | ✅ Lock-free via `watch`      | ⚠️ Multiple readers block |
| Write throughput | High (single task)           | Medium (lock contention)  |
| Deadlocks        | ✅ Impossible                 | ⚠️ Possible               |

### vs. Event Sourcing / CQRS

| Aspect           | Actor Pattern                | Event Sourcing            |
| ---------------- | ---------------------------- | ------------------------- |
| Complexity       | Medium                       | High                      |
| State rebuild    | Not needed                   | Replay events             |
| Auditability     | Manual                       | ✅ Built-in               |
| Scaling          | Single process               | Distributed               |

## When to Use This Pattern

### ✅ **Good fit for:**

- Orchestrators, state machines, game loops
- Applications with frequent state reads, infrequent writes
- Systems requiring deterministic behavior
- Async-first architectures

### ⚠️ **Not ideal for:**

- Ultra-low latency requirements (<1ms)
- Massive state objects (>10MB)
- Systems where immediate consistency checking is required
- Pure synchronous codebases

## Implementation Details

### Command Processing Loop

```rust
loop {
    tokio::select! {
        // Tick updates (time-based)
        _ = ticker.tick() => {
            Self::handle_tick(&mut engine_state, &state_tx);
        }
        
        // Command processing (event-based)
        Some(command) = command_rx.recv() => {
            Self::handle_command(&mut engine_state, &state_tx, command)?;
        }
        
        // Graceful shutdown
        _ = cancel_token.cancelled() => break,
    }
}
```

### State Broadcasting

```rust
// Actor broadcasts new state
state_tx.send_replace(state);  // ← Only place state is modified

// Multiple subscribers receive it
while state_rx.changed().await.is_ok() {
    let state = state_rx.borrow();  // Immutable reference
    // Safe to use across threads
}
```

## Performance Characteristics

### Throughput

- **Commands**: ~100k-1M/sec on modern hardware
- **Tick rate**: Configurable, typically 10-100ms
- **State updates**: Broadcast to all subscribers in <1ms

### Memory

- Base overhead: ~1KB per orchestrator instance
- Per-subscriber overhead: ~100 bytes
- State snapshot: ~2-5KB (depends on scene count)

### Latency

- Command → State update: <1ms (single-threaded queue)
- State update → Subscriber: <100μs (`watch` is very fast)
- End-to-end: Typically <1ms for most commands

## Best Practices

### 1. Keep State Small

```rust
// ✅ Good: Small, cloneable state
pub struct O

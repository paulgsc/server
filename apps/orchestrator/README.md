# Orchestrator Service Architecture

## Overview

A standalone NATS-based orchestrator service that manages stream orchestrators dynamically based on client subscriptions. Built with actor-based subscriber management using the `ws-connection` crate and transport abstraction via `some-transport`.

## Architecture

```
┌─────────────────┐          NATS           ┌──────────────────────┐
│                 │  ───────────────────→   │  Orchestrator        │
│  Axum Server    │  Commands & Sub Mgmt    │  Service             │
│                 │  ←───────────────────   │  (this binary)       │
└─────────────────┘  State Updates          └──────────────────────┘
        │                                              │
        │                                              │
        │                                    ┌─────────┴─────────┐
        │                                    │ ManagedOrchestrator│
        │                                    │  per stream_id     │
        │                                    │                    │
        │                                    │ StreamOrchestrator │
        │                                    │ ConnectionStore    │
        │                                    │ - ConnectionActor  │
        │                                    │   per subscriber   │
        │                                    │ State Publisher    │
        │                                    └────────────────────┘
        ▼
   WebSocket
   Clients
```

## Key Design Principles

### 1. **Transport Abstraction**
Uses the `some-transport` crate's `Transport` and `ReceiverTrait` traits, making it transport-agnostic. Currently uses NATS, but can swap to any other transport implementation (e.g., in-memory for testing).

```rust
pub struct OrchestratorService<T, R>
where
    T: Transport<StateUpdate> + Transport<OrchestratorCommand> + ...,
    R: ReceiverTrait<OrchestratorCommand> + ...
```

### 2. **Actor-Based Subscriber Management**
Each subscriber is managed by a `ConnectionActor` from `ws-connection`, providing:
- Lock-free message passing (no `RwLock` needed)
- Built-in staleness detection and heartbeat tracking
- Automatic lifecycle management via cancellation tokens
- Concurrent state queries with `ConnectionStore::stats()`

```rust
subscribers: Arc<ConnectionStore<StreamSubscription>>
```

### 3. **Protocol Buffers for Serialization**
All messages use Protocol Buffers (via `prost`) for efficient, schema-validated serialization over NATS.

### 4. **Subscription-Driven Lifecycle**
- Orchestrators only run when clients are subscribed
- Automatic cleanup of stale subscribers (90s timeout)
- Idle orchestrators removed after 60s with no subscribers
- First subscriber triggers `StreamOrchestrator.start()`
- Last unsubscribe triggers `StreamOrchestrator.stop()`

### 5. **Per-Stream Isolation**
Each stream gets its own `ManagedOrchestrator` instance with:
- Dedicated `StreamOrchestrator` from `ws-events` crate
- `ConnectionStore` with actor per subscriber
- Independent state publisher task
- Isolated cancellation token

## Module Structure

```
bin/orchestrator_service/
├── main.rs                    # Entry point, NATS transport setup
├── service.rs                 # Main event loop & command processing
├── managed_orchestrator.rs    # Per-stream orchestrator manager with actors
└── types.rs                   # Protobuf message definitions
```

### `types.rs`
Defines Protocol Buffer messages:
- `OrchestratorCommand` - Control commands (Start, Stop, Pause, Resume, ForceScene, SkipScene, UpdateStreamStatus, Reconfigure)
- `SubscriptionCommand` - Client lifecycle (Register, Unregister, Heartbeat)
- `StateUpdate` - Stream state broadcasts (scene, progress, timing)
- `OrchestratorConfigDto` - Configuration with scenes and tick interval

### `managed_orchestrator.rs`
Wraps a `StreamOrchestrator` and manages:
- `ConnectionStore<StreamSubscription>` - Actor-based subscriber registry
- Each subscriber is a `ConnectionActor` tracking heartbeats
- State publishing task (broadcasts via transport)
- Stale subscriber cleanup using actor's built-in staleness check
- Statistics via `ConnectionStore::stats()`

### `service.rs`
Main orchestrator service that:
- Listens for commands and subscription requests via transport receivers
- Creates/destroys `ManagedOrchestrator` instances on-demand
- Coordinates orchestrator lifecycle based on subscriber count
- Spawns cleanup task for stale subscriber detection

## Actor Pattern Benefits

### Before (Lock-Based)
```rust
// Manual timestamp tracking with locks
subscribers: Arc<RwLock<DashMap<ClientId, Instant>>>

// Every operation needs locks
subscribers.write().await.insert(client_id, Instant::now());
```

### After (Actor-Based)
```rust
// Each subscriber is an actor
subscribers: Arc<ConnectionStore<StreamSubscription>>

// Lock-free message passing
handle.record_activity().await?;
handle.check_stale(timeout).await?;
```

**Key Improvements:**
- ✅ No lock contention - concurrent operations via message passing
- ✅ Built-in staleness detection - automatic timeout tracking
- ✅ Graceful shutdown - actors clean up themselves
- ✅ Rich statistics - `active`, `stale`, `unique_clients` counts
- ✅ Reusable pattern - aligns with `ws-connection` crate design

## Message Flow

### 1. Starting a Stream

```
Client → Axum → NATS (command)
  OrchestratorCommand::Start { stream_id, config }
    ↓
Service creates ManagedOrchestrator
    ↓
StreamOrchestrator initialized
ConnectionStore created
    ↓
Waits for first subscriber...
```

### 2. Client Connection

```
Client connects via WebSocket
    ↓
Axum extracts SocketAddr via ConnectInfo
    ↓
Axum → NATS (subscription)
  SubscriptionCommand::Register {
    stream_id,
    client_id,
    source_addr  // NEW: for Connection metadata
  }
    ↓
ManagedOrchestrator.add_subscriber()
  - Creates Connection(client_id, source_addr)
  - Spawns ConnectionActor
  - Subscribes to StreamSubscription event
    ↓
If first subscriber: StreamOrchestrator.start()
    ↓
State updates begin publishing via Transport
```

### 3. State Broadcasting

```
StreamOrchestrator ticks
    ↓
State changes detected (watch::channel)
    ↓
ManagedOrchestrator checks subscriber count
    ↓
If subscribers > 0:
  StateUpdate created
    ↓
  Transport.broadcast(StateUpdate)
    ↓
NATS publishes to all subscribers
    ↓
Axum receives & filters by stream_id
    ↓
Forwards to WebSocket clients
```

### 4. Heartbeat Management

```
Every 30 seconds (client-side):
  Axum → NATS
    SubscriptionCommand::Heartbeat { stream_id, client_id }
      ↓
  ManagedOrchestrator.update_heartbeat()
    ↓
  ConnectionActor.record_activity()
    ↓
  Updates last_activity timestamp

Every 30 seconds (service-side):
  Cleanup task runs
    ↓
  For each ManagedOrchestrator:
    cleanup_stale_subscribers(90s timeout)
      ↓
    ConnectionActor.check_stale(90s)
      ↓
    If stale: ConnectionActor.disconnect()
      ↓
    ConnectionStore removes actor
```

### 5. Client Disconnection

```
Client disconnects (WebSocket close)
    ↓
Axum → NATS (subscription)
  SubscriptionCommand::Unregister { stream_id, client_id }
    ↓
ManagedOrchestrator.remove_subscriber()
  - ConnectionStore.remove(key)
  - ConnectionActor shutdown via handle
    ↓
If no subscribers remaining:
  StreamOrchestrator.stop()
    ↓
After 60s idle:
  Remove ManagedOrchestrator
  Cleanup all resources
```

## Running the Service

### Prerequisites
1. NATS server running: `nats-server`
2. Rust toolchain with workspace dependencies:
   - `ws-events` (features = ["stream-orch"])
   - `ws-connection`
   - `some-transport` (features = ["nats"])

### Start Service
```bash
# Terminal 1: NATS
nats-server

# Terminal 2: Orchestrator Service
RUST_LOG=info cargo run --bin orchestrator_service

# Terminal 3: Axum Example
RUST_LOG=info cargo run --example axum_client
```

### Environment Variables
- `NATS_URL` - NATS server URL (default: `nats://localhost:4222`)
- `RUST_LOG` - Log level (e.g., `info`, `debug`, `trace`)

## Integration with Axum

See `examples/axum_client.rs` for complete example with:
- Stream creation and control endpoints
- WebSocket handler with `ConnectInfo<SocketAddr>`
- Heartbeat management (30s interval)
- Graceful connection cleanup

### Example Usage

```bash
# Create stream
curl -X POST http://localhost:3000/streams \
  -H "Content-Type: application/json" \
  -d '{
    "scenes": [
      {"name": "Intro", "duration_ms": 3000},
      {"name": "Main", "duration_ms": 10000},
      {"name": "Outro", "duration_ms": 2000}
    ],
    "tick_interval_ms": 100
  }'

# Response: {"stream_id": "uuid", "message": "Stream created successfully"}

# Connect WebSocket
wscat -c ws://localhost:3000/streams/{stream_id}/ws

# Control stream
curl -X POST http://localhost:3000/streams/{stream_id}/pause
curl -X POST http://localhost:3000/streams/{stream_id}/resume
curl -X POST http://localhost:3000/streams/{stream_id}/scene/Main
curl -X POST http://localhost:3000/streams/{stream_id}/skip
curl -X POST http://localhost:3000/streams/{stream_id}/stop

# List active streams
curl http://localhost:3000/streams
```

## Benefits of This Architecture

### Scalability
- Service can run multiple instances (NATS handles distribution)
- Horizontal scaling via NATS queue groups (future enhancement)
- Actor-based concurrency eliminates lock contention

### Decoupling
- Axum server doesn't manage orchestrator lifecycle
- Services communicate only via NATS
- Independent deployment and restart

### Resource Efficiency
- Orchestrators only run when needed (subscriber-driven)
- Automatic cleanup of idle resources
- Stale connection detection prevents resource leaks

### Type Safety
- Protocol Buffers ensure schema compatibility
- Transport abstraction enables compile-time guarantees
- Actor pattern enforces message-based communication

### Testability
- Can swap NATS for in-memory transport in tests
- Each component (service, managed orchestrator, actor) is independently testable
- Mock transport for integration tests

### Resilience
- Automatic stale client detection (90s timeout)
- Graceful shutdown via cancellation tokens
- Actor supervision and cleanup
- NATS handles reconnections automatically

## Performance Characteristics

| Operation | Complexity | Notes |
|-----------|-----------|-------|
| Add subscriber | O(1) | Spawn actor task |
| Remove subscriber | O(1) | Send shutdown message |
| Heartbeat update | O(1) | Message to actor |
| Check staleness | O(n) | Concurrent actor queries |
| Get stats | O(n) | Parallel JoinSet queries |
| State broadcast | O(1) | NATS pub (fan-out handled by server) |

**n** = number of subscribers per stream

## Monitoring & Observability

### Built-in Statistics
```rust
let stats = managed_orchestrator.get_stats().await;
println!("Total: {}", stats.total);
println!("Active: {}", stats.active);
println!("Stale: {}", stats.stale);
println!("Unique clients: {}", stats.unique_clients);
```

### Tracing Integration
Service uses `tracing` for structured logging:
```bash
RUST_LOG=orchestrator_service=debug,ws_connection=info cargo run --bin orchestrator_service
```

Key trace points:
- Client registration/unregistration
- Orchestrator start/stop
- Stale subscriber cleanup
- State publication events

## Future Enhancements

### Short-term
- [ ] Per-subscriber state filtering (scene-specific subscriptions)
- [ ] Metrics endpoint (Prometheus format)
- [ ] Health check endpoint
- [ ] Configurable timeouts (staleness, idle removal)

### Medium-term
- [ ] JetStream for command persistence/replay
- [ ] Multi-region coordination with NATS leafnodes
- [ ] Dynamic reconfiguration without restart
- [ ] Rate limiting per subscriber

### Long-term
- [ ] State snapshots for recovery (Redis/PostgreSQL)
- [ ] Advanced subscription filters (scene, progress threshold)
- [ ] Backpressure handling with actor buffers
- [ ] Distributed tracing with OpenTelemetry

## Troubleshooting

### Service won't start
```bash
# Check NATS is running
nats-server --version
ps aux | grep nats-server

# Verify NATS_URL
echo $NATS_URL

# Check port availability
lsof -i :4222
```

### Clients not receiving updates
```bash
# Check service logs
RUST_LOG=debug cargo run --bin orchestrator_service

# Verify registration succeeded
# Look for: "Added subscriber {client_id} to stream {stream_id}"

# Check stream_id filtering in client
# StateUpdate contains stream_id field

# Test NATS connectivity
nats sub ">"
```

### Orchestrators not stopping
```bash
# Check subscriber counts
# Look for: "Remaining subscribers for stream {id}: {count}"

# Verify unregister commands are sent
# Look for: "Client {id} unregistering from stream {id}"

# Check heartbeat intervals
# Client: 30s, Server timeout: 90s

# Review actor states
# Enable debug logging: RUST_LOG=ws_connection=debug
```

### Memory leaks or resource buildup
```bash
# Check for stale connections
# Look for cleanup logs: "Cleaned up {n} stale subscribers"

# Verify idle orchestrator removal
# Look for: "Removing idle orchestrator for stream {id}"

# Monitor connection counts
# Should decrease after disconnects

# Check actor task count
# ps aux | grep orchestrator_service
# Monitor with tokio-console (optional)
```

## Dependencies

```toml
[dependencies]
# Core orchestration
ws-events = { path = "../ws-events", features = ["stream-orch"] }
ws-connection = { path = "../ws-connection" }
some-transport = { path = "../some-transport", features = ["nats"] }

# NATS
async-nats = "0.37"

# Async runtime
tokio = { version = "1", features = ["full"] }
tokio-util = "0.7"

# Serialization
serde = { version = "1", features = ["derive"] }
serde_json = "1"
prost = "0.13"

# Concurrency
dashmap = "6"
futures = "0.3"
async-trait = "0.1"

# Logging
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }

# Utilities
uuid = { version = "1", features = ["v4", "serde"] }
```


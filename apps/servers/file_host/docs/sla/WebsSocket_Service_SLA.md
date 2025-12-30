# WebSocket Service SLA - Precise Definitions & Implementation Guide

## Part 1: Metric Definitions (No Ambiguity)

### 1.1 System Uptime

**Definition:** The percentage of time the HTTP server accepts TCP connections on port 3000 and the `/ws` endpoint returns a valid WebSocket upgrade response (101 Switching Protocols).

**What Counts as "Up":**
- `TcpListener::bind("0.0.0.0:3000")` is actively listening
- HTTP server responds to TCP SYN packets
- GET request to `/ws` with upgrade headers returns 101 status code
- OR returns 429/503 (service still "up", just rate-limited)

**What Counts as "Down":**
- Connection refused (ECONNREFUSED)
- Connection timeout (no TCP ACK within 10 seconds)
- HTTP 500/502/504 errors
- Process crash/panic
- Server not responding to health check pings

**Measurement Method:**
```rust
// External synthetic monitor (every 60 seconds)
async fn check_uptime() -> bool {
    match tokio::time::timeout(
        Duration::from_secs(10),
        TcpStream::connect("your-server:3000")
    ).await {
        Ok(Ok(_)) => true,  // Server is up
        _ => false          // Server is down
    }
}
```

**Calculation:**
```
Uptime % = (Total Minutes - Down Minutes) / Total Minutes × 100

Example for 99.9% monthly:
- 30 days = 43,200 minutes
- Max downtime = 43.2 minutes/month
```

**Your Implementation:**
- Already have: TCP listener on port 3000
- Need: External uptime monitor (Prometheus Blackbox Exporter or UptimeRobot)
- Need: `/health` endpoint returning 200 OK when server is healthy

---

### 1.2 WebSocket Connection Success Rate

**Definition:** Percentage of WebSocket upgrade attempts that successfully transition from HTTP to WebSocket protocol and receive an initial handshake message.

**What Counts as "Success":**
1. Client sends HTTP upgrade request to `/ws`
2. Server returns 101 Switching Protocols
3. `establish_connection()` completes without error
4. `send_initial_handshake()` sends welcome message
5. Connection added to `ConnectionStore`

**What Counts as "Failure":**
1. 429 Too Many Requests (rate limit - your `ConnectionGuard`)
2. 503 Service Unavailable (queue full)
3. 408 Request Timeout (permit acquisition timeout - your 5s timeout)
4. Network error during handshake
5. `establish_connection()` returns `Err(_)`

**Measurement Method:**
```rust
// In your websocket_handler
async fn websocket_handler(...) -> impl IntoResponse {
    metrics::WS_CONNECTION_ATTEMPTS.inc();
    
    match timeout(Duration::from_secs(5), 
                  state.connection_guard.acquire(client_id.clone())).await 
    {
        Ok(Ok(permit)) => {
            metrics::WS_CONNECTION_SUCCESS.inc();
            ws.on_upgrade(...)
        }
        Ok(Err(err)) => {
            metrics::WS_CONNECTION_REJECTED.inc();
            metrics::WS_REJECTION_REASON.with_label_values(&[err.kind.as_str()]).inc();
            (StatusCode::SERVICE_UNAVAILABLE, reason).into_response()
        }
        Err(_) => {
            metrics::WS_CONNECTION_TIMEOUT.inc();
            (StatusCode::REQUEST_TIMEOUT, "...").into_response()
        }
    }
}
```

**Calculation:**
```
Success Rate % = Successful Upgrades / Total Attempts × 100

Example:
- 10,000 attempts
- 9,950 succeed (101 response + handshake)
- 50 fail (429/503/timeout)
- Success Rate = 99.5%
```

**Your Current State:**
- ✅ Have: `ConnectionGuard` with acquire logic
- ✅ Have: Timeout on permit acquisition (5 seconds)
- ❌ Need: Metrics counters for attempts/success/failures
- ❌ Need: Prometheus histogram for upgrade duration

---

### 1.3 WebSocket Connection Stability

**Definition:** Percentage of established connections that remain active for their intended lifetime without unexpected disconnection.

**What Counts as "Stable":**
- Connection remains in `ConnectionStore` until client explicitly closes
- OR until `HeartbeatManager` detects legitimate timeout
- No premature cleanup due to server errors

**What Counts as "Unstable":**
- Connection dropped due to server panic/crash
- Forced disconnect due to backpressure/queue overflow
- Connection lost during server restart (unless graceful shutdown)
- Heartbeat timeout due to server not sending pings

**Measurement Method:**
```rust
// Track connection lifecycle
pub enum DisconnectReason {
    ClientClosed,           // Expected (stable)
    HeartbeatTimeout,       // Expected (stable)
    ServerError,            // Unexpected (unstable)
    ServerShutdown,         // Expected (stable if graceful)
    BackpressureOverflow,   // Unexpected (unstable)
    ForwardTaskPanic,       // Unexpected (unstable)
}

async fn cleanup_connection_with_stats(...) {
    let reason = determine_disconnect_reason();
    
    metrics::WS_DISCONNECTIONS_TOTAL
        .with_label_values(&[reason.as_str()])
        .inc();
    
    if reason.is_unexpected() {
        metrics::WS_UNSTABLE_DISCONNECTIONS.inc();
    }
}
```

**Calculation:**
```
Stability % = (Total Connections - Unstable Disconnects) / Total Connections × 100

Example:
- 1,000 connections established today
- 980 closed normally (client/timeout)
- 20 dropped due to server error
- Stability = 98%
```

**Your Current State:**
- ✅ Have: `cleanup_connection_with_stats()` function
- ❌ Need: Track disconnect reasons in `ConnectionCleanup` event
- ❌ Need: Distinguish expected vs unexpected disconnects

---

### 1.4 API Latency (HTTP Endpoints)

**Definition:** Time from when the server receives the first byte of an HTTP request to when the last byte of the response is sent to the TCP socket.

**What to Measure:**
```rust
// For your routes:
// - GET /sheets/...
// - GET /gdrive/...
// - GET /repos
// - POST /mood-events
// - POST /utterance
// - POST /now-playing

async fn metrics_middleware(req: Request, next: Next) -> Response {
    let start = Instant::now();
    let path = req.uri().path().to_string();
    
    let response = next.run(req).await;
    let duration = start.elapsed();
    
    metrics::HTTP_REQUEST_DURATION
        .with_label_values(&[&path, response.status().as_str()])
        .observe(duration.as_secs_f64());
    
    response
}
```

**Target:** <200ms at 95th percentile

**Breakdown:**
- p50 (median): <50ms
- p95: <200ms
- p99: <500ms
- p99.9: <2000ms

**What Counts:**
- Total end-to-end time including:
  - HTTP parsing
  - Authentication/authorization
  - Database queries
  - External API calls (Sheets/Drive/GitHub)
  - Response serialization

**Your Current State:**
- ✅ Have: `metrics_middleware` in main.rs
- ❌ Need: Implement histogram tracking in middleware
- ❌ Need: Per-endpoint latency labels

---

### 1.5 WebSocket Message Latency

**Definition:** Time from when a message event is created (e.g., `Event::NowPlaying`) to when it's written to the client's TCP socket.

**What to Measure:**
```rust
pub enum Event {
    NowPlaying {
        timestamp: Instant,  // ← Add this
        data: NowPlaying
    },
    // ... other variants
}

// In spawn_event_forwarder
async fn forward_events(...) {
    while let Ok(event) = event_rx.recv().await {
        let send_start = Instant::now();
        
        match sender.send(event.to_message()).await {
            Ok(_) => {
                let latency = send_start.elapsed();
                metrics::WS_MESSAGE_LATENCY
                    .with_label_values(&[event.event_type()])
                    .observe(latency.as_secs_f64());
            }
            Err(e) => { /* ... */ }
        }
    }
}
```

**Target:** <100ms at 95th percentile

**Breakdown:**
- p50: <10ms
- p95: <100ms
- p99: <250ms

**Components:**
- Event creation → transport.broadcast() call
- InMemTransport queue time
- Event forwarding task wakeup
- Serialization (serde_json)
- TCP send buffer time

**Your Current State:**
- ❌ Need: Add timestamps to Event variants
- ❌ Need: Track latency in `spawn_event_forwarder`
- ❌ Need: Separate metrics for broadcast vs direct send

---

### 1.6 Heartbeat Success Rate

**Definition:** Percentage of active connections that successfully respond to server ping within the configured timeout.

**What to Measure:**
```rust
// In your HeartbeatManager
async fn check_connections(&self) {
    for connection in self.store.iter() {
        metrics::HEARTBEAT_PINGS_SENT.inc();
        
        let last_ping = connection.last_ping().await;
        let timeout = self.policy.ping_timeout;
        
        if last_ping.elapsed() > timeout {
            metrics::HEARTBEAT_TIMEOUTS.inc();
            // Mark as stale
        } else {
            metrics::HEARTBEAT_SUCCESS.inc();
        }
    }
}
```

**Target:** ≥99% success rate

**Your Current State:**
- ✅ Have: `HeartbeatManager` with ping tracking
- ✅ Have: `record_ping()` method
- ❌ Need: Metrics for ping/pong success/failure
- ❌ Need: Distinguish client timeout from server issues

---

## Part 2: Concrete SLA for Your WebSocket Service

### Service-Level Agreement v1.0

**Effective Date:** [Current Date]  
**Review Cadence:** Quarterly  
**Owner:** Tech Lead  

---

#### **SLA-001: HTTP Server Availability**

**Commitment:** Server accepts TCP connections ≥99.9% per month

**Measurement:**
- External probe every 60s to `tcp://your-server:3000`
- Success = TCP handshake completes within 10s
- Downtime = 3 consecutive failed probes (180s)

**Exclusions:**
- Scheduled maintenance (with 48h notice)
- DDoS attacks (force majeure)
- Client-side network issues

**Credits:** 
- <99.9%: Incident review required
- <99%: Root cause analysis mandatory
- <95%: Postmortem with stakeholders

---

#### **SLA-002: WebSocket Upgrade Success**

**Commitment:** ≥99% of upgrade attempts succeed (when not rate-limited)

**Measurement:**
```
Success Rate = successful_upgrades / (total_attempts - rate_limited) × 100
```

**Target Breakdown:**
- Rate limit rejections (429): Not counted as failures
- Timeouts (408): Counted as failures
- Server errors (500): Counted as failures

**Alert Threshold:** <99% over 5-minute window

---

#### **SLA-003: Connection Stability**

**Commitment:** ≥99% of established connections close gracefully

**Measurement:**
- Graceful = client close, heartbeat timeout, or planned shutdown
- Ungraceful = server panic, backpressure drop, forced cleanup

**Target:** <1% ungraceful disconnections per day

---

#### **SLA-004: HTTP API Latency**

**Commitment:**
- p50 latency <50ms
- p95 latency <200ms  
- p99 latency <500ms

**Measurement:** Per-endpoint histogram over 5-minute windows

**Excluded Endpoints:**
- `/metrics` (observability overhead expected)
- External proxy errors (3rd party timeout)

---

#### **SLA-005: WebSocket Message Delivery**

**Commitment:**
- p95 latency <100ms (event creation → socket send)
- p99 latency <250ms

**Measurement:** Timestamp tracking for broadcast events

**Exclusions:**
- Client-side processing time (not measured)
- Network propagation delay (outside control)

---

#### **SLA-006: Heartbeat Health**

**Commitment:** ≥99% of active connections respond to ping

**Measurement:**
- Success = pong received within `ping_timeout`
- Tracked per connection over 1-hour windows

**Alert:** <95% success rate over 5 minutes

---

#### **SLA-007: Data Retention**

**Commitment:** 
- Ephemeral messages: Auto-expire ≤5 minutes after disconnect
- Persistent data: 30-day retention (if applicable)

**Your Current State:** 
- Ephemeral only (no durable storage)
- Cleanup happens immediately in `cleanup_connection_with_stats`

**Compliance:** ✅ Already met (immediate cleanup < 5 min)

---

#### **SLA-008: Incident Response**

**Commitment:**
- Critical (total outage): Response <15 min
- High (degraded service): Response <1 hour
- Medium (isolated issues): Response <4 hours

**Requirements:**
- On-call rotation via PagerDuty
- Runbooks for common failures
- Alert escalation after 2 missed pages

---

#### **SLA-009: Security Patching**

**Commitment:** Critical CVEs patched within 7 days

**Process:**
1. `cargo audit` in CI/CD
2. Security advisory review
3. Test in staging
4. Deploy to production

**Measurement:** Time from CVE disclosure to production deployment

---

## Part 3: Implementation Stories (Jira-Ready)

### Epic: WebSocket SLA Monitoring & Compliance

---

### **Story WS-001: Instrument Connection Success Metrics**

**Priority:** P0 (Blocker)  
**Story Points:** 3  

**Description:**  
Add Prometheus metrics to track WebSocket upgrade success/failure rates for SLA-002 compliance.

**Acceptance Criteria:**
1. Counter `ws_connection_attempts_total` incremented on every upgrade request
2. Counter `ws_connection_success_total` incremented after successful handshake
3. Counter `ws_connection_rejected_total{reason}` with labels:
   - `reason="rate_limit"` (429)
   - `reason="queue_full"` (503)
   - `reason="timeout"` (408)
4. Histogram `ws_upgrade_duration_seconds` tracks time from request to handshake
5. Grafana dashboard shows success rate over 5-min windows

**Implementation Notes:**
```rust
// In websocket_handler
lazy_static! {
    static ref WS_ATTEMPTS: IntCounter = register_int_counter!(
        "ws_connection_attempts_total",
        "Total WebSocket upgrade attempts"
    ).unwrap();
    
    static ref WS_SUCCESS: IntCounter = register_int_counter!(
        "ws_connection_success_total",
        "Successful WebSocket upgrades"
    ).unwrap();
    
    static ref WS_REJECTED: IntCounterVec = register_int_counter_vec!(
        "ws_connection_rejected_total",
        "Rejected WebSocket upgrades",
        &["reason"]
    ).unwrap();
}

// Usage:
WS_ATTEMPTS.inc();
match state.connection_guard.acquire(...).await {
    Ok(Ok(permit)) => {
        WS_SUCCESS.inc();
        // ...
    }
    Ok(Err(err)) => {
        WS_REJECTED.with_label_values(&[err.kind.as_str()]).inc();
    }
}
```

**Testing:**
- Simulate 1000 concurrent connections
- Trigger rate limits (exceed ConnectionGuard capacity)
- Verify metrics match expected counts

---

### **Story WS-002: Track Connection Stability**

**Priority:** P0  
**Story Points:** 5

**Description:**  
Distinguish between graceful and ungraceful disconnections to measure SLA-003 stability.

**Acceptance Criteria:**
1. Enum `DisconnectReason` with variants:
   ```rust
   pub enum DisconnectReason {
       ClientClosed,          // Stable
       HeartbeatTimeout,      // Stable
       GracefulShutdown,      // Stable
       ServerError(String),   // Unstable
       BackpressureOverflow,  // Unstable
       TaskPanic,             // Unstable
   }
   ```
2. Update `cleanup_connection_with_stats()` to accept reason
3. Counter `ws_disconnections_total{reason}` tracks all disconnects
4. Counter `ws_unstable_disconnections_total` counts only unstable reasons
5. Alert fires if unstable rate >1% over 5 minutes

**Implementation:**
```rust
// In cleanup_connection_with_stats
async fn cleanup_connection_with_stats(
    state: &WebSocketFsm,
    conn_key: &str,
    message_count: usize,
    forward_task: JoinHandle<Result<(), String>>,
) {
    let reason = match forward_task.await {
        Ok(Ok(())) => DisconnectReason::ClientClosed,
        Ok(Err(e)) if e.contains("heartbeat") => DisconnectReason::HeartbeatTimeout,
        Ok(Err(e)) => DisconnectReason::ServerError(e),
        Err(_) => DisconnectReason::TaskPanic,
    };
    
    metrics::WS_DISCONNECTIONS
        .with_label_values(&[reason.as_str()])
        .inc();
    
    if reason.is_unstable() {
        metrics::WS_UNSTABLE_DISCONNECTIONS.inc();
    }
    
    // Emit event with reason
    state.emit_system_event(Event::ConnectionCleanup {
        connection_id: conn_key.to_string(),
        reason: reason.to_string(),
        resources_freed: message_count,
    }).await;
}
```

---

### **Story WS-003: Implement HTTP Latency Histograms**

**Priority:** P0  
**Story Points:** 2

**Description:**  
Track per-endpoint HTTP latency for SLA-004 compliance.

**Acceptance Criteria:**
1. Histogram `http_request_duration_seconds{endpoint, status}` with buckets:
   ```rust
   vec![0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0]
   ```
2. Middleware records latency for all routes
3. Grafana dashboard shows p50/p95/p99 per endpoint
4. Alert fires if p95 >200ms for 5 consecutive minutes

**Implementation:**
```rust
// Update metrics_middleware in main.rs
pub async fn metrics_middleware(
    req: Request<Body>,
    next: Next,
) -> Response {
    let start = Instant::now();
    let path = req.uri().path().to_string();
    
    let response = next.run(req).await;
    
    let duration = start.elapsed();
    HTTP_REQUEST_DURATION
        .with_label_values(&[&path, response.status().as_str()])
        .observe(duration.as_secs_f64());
    
    response
}
```

---

### **Story WS-004: Add WebSocket Message Latency Tracking**

**Priority:** P1  
**Story Points:** 5

**Description:**  
Measure time from event creation to socket send for SLA-005.

**Acceptance Criteria:**
1. Add `created_at: Instant` field to Event variants
2. Histogram `ws_message_latency_seconds{event_type}` tracks delivery time
3. Measure in `spawn_event_forwarder` before `sender.send()`
4. Dashboard shows p95/p99 latency per event type
5. Alert if p95 >100ms sustained for 5 minutes

**Implementation:**
```rust
// Update Event enum
#[derive(Clone)]
pub enum Event {
    NowPlaying {
        created_at: Instant,
        data: NowPlaying,
    },
    // ... add to all variants
}

// In spawn_event_forwarder
async fn forward_events(mut sender, mut event_rx, state, conn_key) {
    while let Ok(event) = event_rx.recv().await {
        let latency = event.created_at().elapsed();
        
        metrics::WS_MESSAGE_LATENCY
            .with_label_values(&[event.event_type().as_str()])
            .observe(latency.as_secs_f64());
        
        if let Err(e) = sender.send(event.to_message()).await {
            // handle error
        }
    }
}
```

---

### **Story WS-005: Heartbeat Metrics & Alerting**

**Priority:** P1  
**Story Points:** 3

**Description:**  
Track heartbeat success rate for SLA-006.

**Acceptance Criteria:**
1. Counter `ws_heartbeat_pings_sent_total`
2. Counter `ws_heartbeat_pongs_received_total`
3. Counter `ws_heartbeat_timeouts_total`
4. Gauge `ws_stale_connections` (current count)
5. Alert if success rate <99% over 5 minutes

**Implementation:**
```rust
// In HeartbeatManager::check_heartbeats
async fn check_heartbeats(&self) {
    for handle in self.store.iter() {
        metrics::HEARTBEAT_PINGS_SENT.inc();
        
        let state = handle.get_state().await?;
        if state.last_ping.elapsed() > self.policy.ping_timeout {
            metrics::HEARTBEAT_TIMEOUTS.inc();
            // Mark as stale
        } else {
            metrics::HEARTBEAT_PONGS_RECEIVED.inc();
        }
    }
    
    metrics::STALE_CONNECTIONS.set(self.store.stale_count().await as f64);
}
```

---

### **Story WS-006: External Uptime Monitoring**

**Priority:** P0  
**Story Points:** 2

**Description:**  
Set up external probe for SLA-001 compliance.

**Acceptance Criteria:**
1. Prometheus Blackbox Exporter configured
2. TCP probe every 60s to `your-server:3000`
3. HTTP probe to `/health` endpoint
4. Downtime = 3 consecutive failures (180s)
5. PagerDuty alert on downtime
6. Grafana dashboard shows uptime percentage

**Configuration:**
```yaml
# blackbox.yml
modules:
  tcp_connect:
    prober: tcp
    timeout: 10s
  http_health:
    prober: http
    timeout: 10s
    http:
      preferred_ip_protocol: "ip4"
      valid_status_codes: [200]
```

---

### **Story WS-007: Graceful Shutdown Compliance**

**Priority:** P1  
**Story Points:** 3

**Description:**  
Ensure server shutdown doesn't violate SLA-003 stability.

**Acceptance Criteria:**
1. On `Ctrl+C`, wait for in-flight messages to send
2. Close all connections with `Event::ServerShutdown`
3. Timeout after 30 seconds (force close)
4. Shutdown events marked as "stable" disconnections
5. Verify no ungraceful disconnects during restart tests

**Implementation:**
```rust
// In main.rs
tokio::spawn(async move {
    tokio::signal::ctrl_c().await.ok();
    tracing::info!("Graceful shutdown initiated");
    
    // Notify all connections
    ws_bridge.broadcast_shutdown_notice().await;
    
    // Wait for connections to close (max 30s)
    for _ in 0..30 {
        if ws_bridge.get_client_count().await == 0 {
            break;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    
    signal_shutdown_token.cancel();
});
```

---

### **Story WS-008: Monthly SLA Report Generation**

**Priority:** P2  
**Story Points:** 5

**Description:**  
Automate monthly SLA compliance reports.

**Acceptance Criteria:**
1. Script queries Prometheus for:
   - Uptime % (SLA-001)
   - Connection success rate (SLA-002)
   - Stability % (SLA-003)
   - Latency percentiles (SLA-004, SLA-005)
   - Heartbeat success % (SLA-006)
2. Generate PDF/HTML report
3. Email to stakeholders on 1st of month
4. Include breach analysis if SLA violated

**Example Output:**
```
WebSocket Service SLA Report - January 2025

SLA-001 HTTP Availability: 99.95% ✅ (Target: 99.9%)
  - Total downtime: 21 minutes
  - Incidents: 1 (database connection pool exhaustion)

SLA-002 Connection Success: 99.8% ✅ (Target: 99%)
  - Total attempts: 1,245,332
  - Rejected: 2,491 (rate limit)
  - Failed: 12 (timeout)

SLA-003 Stability: 99.2% ✅ (Target: 99%)
  - Unstable disconnects: 892 (0.8%)
  - Primary cause: Client network issues

SLA-004 HTTP Latency:
  - p50: 42ms ✅, p95: 187ms ✅, p99: 412ms ✅

SLA-005 WebSocket Latency:
  - p50: 8ms ✅, p95: 89ms ✅, p99: 203ms ✅

SLA-006 Heartbeat: 99.4% ✅ (Target: 99%)
```

---

## Part 4: Tech Lead Operational Checklist

### Daily Tasks
- [ ] Review Grafana dashboard for anomalies
- [ ] Check alert queue in PagerDuty
- [ ] Verify all metrics are reporting
- [ ] Review error logs for patterns

### Weekly Tasks
- [ ] Analyze slow HTTP endpoints (>200ms p95)
- [ ] Review unstable disconnection reasons
- [ ] Check heartbeat timeout trends
- [ ] Verify backup completion (if applicable)

### Monthly Tasks
- [ ] Generate SLA compliance report
- [ ] Conduct SLA breach postmortems
- [ ] Review and update runbooks
- [ ] Test restore procedures
- [ ] Update on-call schedule

### Quarterly Tasks
- [ ] Review SLA targets with stakeholders
- [ ] Load test with 2x peak traffic
- [ ] Audit security dependencies (`cargo audit`)
- [ ] Update incident response procedures

---

## Part 5: Alert Definitions

### Critical Alerts (Page Immediately)

**Alert: WebSocketServerDown**
```yaml
- alert: WebSocketServerDown
  expr: up{job="websocket"} == 0
  for: 3m
  labels:
    severity: critical
  annotations:
    summary: "WebSocket server is down"
    description: "TCP probe failed for 3 consecutive checks (180s)"
```

**Alert: ConnectionSuccessRateLow**
```yaml
- alert: ConnectionSuccessRateLow
  expr: |
    (
      rate(ws_connection_success_total[5m]) 
      / 
      (rate(ws_connection_attempts_total[5m]) - rate(ws_connection_rejected_total{reason="rate_limit"}[5m]))
    ) < 0.99
  for: 5m
  labels:
    severity: critical
  annotations:
    summary: "WebSocket connection success rate below SLA"
```

### Warning Alerts (Slack Notification)

**Alert: HTTPLatencyHigh**
```yaml
- alert: HTTPLatencyHigh
  expr: |
    histogram_quantile(0.95, 
      rate(http_request_duration_seconds_bucket[5m])
    ) > 0.2
  for: 5m
  labels:
    severity: warning
  annotations:
    summary: "HTTP p95 latency exceeds 200ms"
```

---

## Summary

This document provides:
1. **Precise definitions** for every metric (no ambiguity)
2. **Concrete SLA** tailored to your WebSocket service
3. **8 implementation stories** with acceptance criteria
4. **Operational checklist** for tech leads
5. **Alert definitions** for automated monitoring


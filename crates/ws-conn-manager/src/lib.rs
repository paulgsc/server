//! # ConnectionGuard Crate
//!
//! This crate implements a *multi-tenant connection limiter* designed for
//! environments such as WebSocket servers where each client may open
//! multiple concurrent connections.
//!
//! ## Overview
//!
//! `ConnectionGuard` enforces two orthogonal constraints:
//!
//! 1. **Global limit** — caps total active connections across all clients
//!    (`Σ active(c) ≤ MAX_GLOBAL`).
//! 2. **Per-client limit** — caps active connections per client
//!    (`active(c) ≤ MAX_PER_CLIENT`), with bounded queueing of pending
//!    requests (`queue(c) ≤ MAX_QUEUE_PER_CLIENT`).
//!
//! Each acquired connection returns a [`ConnectionPermit`] that holds both
//! a global semaphore slot and a per-client active slot. When the permit is
//! dropped, cleanup happens asynchronously via `tokio::spawn`: the per-client
//! counter is decremented and the next queued waiter (if any) is woken.
//!
//! ## Design Goals
//!
//! - Deterministic enforcement of global and per-client invariants.
//! - Thread-safe, async-friendly, and transport-agnostic (usable for
//!   WebSocket, HTTP/2, or gRPC sessions).
//! - RAII-based resource management via automatic `Drop` cleanup.
//!
//! ## Current Known Limitations
//!
//! 1. **Fairness between clients**
//!    - Queues are FIFO *per client*, but there is no global fairness
//!      mechanism. A single client releasing frequently may monopolize
//!      available slots while others remain queued.
//!    - **Future improvement:** introduce a global fair scheduler
//!      (e.g. weighted round-robin or rotating priority) to interleave
//!      wakeups across clients.
//!
//! 2. **Global–queue coupling**
//!    - Currently a global permit is acquired *before* inspecting per-client
//!      state. Under heavy contention, clients whose queues are already
//!      full may still block global capacity briefly.
//!    - **Future improvement:** perform a fast per-client pre-check before
//!      acquiring the global semaphore, or add hierarchical admission
//!      control (client-level pre-semaphores).
//!
//! 3. **No cross-client backpressure signaling**
//!    - Clients are independently queued; there is no visibility into
//!      global saturation for adaptive retry strategies.
//!    - **Future improvement:** expose a shared metrics or notification API
//!      that allows callers to detect near-saturation and apply jittered
//!      backoff instead of immediate retry.
//!
//! 4. **Queue cleanup of canceled waiters**
//!    - When a queued task is dropped before being woken, its oneshot
//!      receiver is dropped but the sender remains in the queue until
//!      the next wakeup attempt (which will fail silently).
//!    - **Future improvement:** periodically prune stale waiters or switch
//!      to an `async_broadcast`/`Notify`-based structure that detects
//!      cancellation earlier.
//!
//! 5. **Metrics granularity**
//!    - Current counters are derived from instantaneous semaphore values
//!      and `AtomicUsize` reads, which may lag slightly under high churn.
//!    - **Future improvement:** integrate structured metrics (e.g. via
//!      `metrics` crate or Prometheus exporter) for consistent sampling.
//!
//! 6. **Async Drop via `tokio::spawn`**
//!    - The `Drop` impl spawns a new task to perform cleanup, which adds
//!      latency and relies on the tokio runtime being available.
//!    - Cleanup is fire-and-forget; there is no guarantee of immediate
//!      execution or completion before process exit.
//!    - **Future improvement:** explore explicit async cleanup methods or
//!      structured concurrency patterns that avoid spawning in Drop.
//!
//! 7. **Lock contention on per-client state**
//!    - `DashMap` entries require mutable access for queue modifications.
//!      Under heavy per-client load, this can create contention bottlenecks.
//!    - **Future improvement:** replace `VecDeque` with lock-free concurrent
//!      queue (e.g. `crossbeam::queue::SegQueue`) or per-client channels.
//!
//! 8. **Global semaphore held during queue await**
//!    - If a request must queue, it holds the global semaphore permit while
//!      waiting, which can reduce effective global capacity.
//!    - **Future improvement:** release and re-acquire global permit around
//!      queue waits, or implement two-phase admission control.
//!
//! ## API Summary
//!
//! ```rust,ignore
//! let guard = ConnectionGuard::new();
//!
//! // Acquire a connection permit (may queue if per-client limit reached)
//! match guard.acquire(client_id).await {
//!     Ok(permit) => {
//!         // Connection active; permit automatically released on drop
//!     }
//!     Err(e) => {
//!         // Either QueueFull or GlobalLimit
//!     }
//! }
//!
//! // Fast hint check before expensive operations
//! if !guard.try_acquire_permit_hint() {
//!     // Global capacity exhausted, reject early
//! }
//!
//! // Query current state
//! let global_active = guard.active_global();
//! let client_active = guard.active_per_client("client-123");
//! ```
//!
//! ## Recommended Future Work
//!
//! - Integrate global fairness and client rotation.
//! - Provide a non-blocking `try_acquire()` API for early rejection.
//! - Add tracing spans and metrics hooks for observability.
//! - Implement an optional *hierarchical semaphore* model to separate
//!   global from per-client resource pools.
//! - Expose structured diagnostics for testing invariants under load.
//! - Replace `tokio::spawn` in Drop with explicit async cleanup method.
//! - Benchmark and optimize lock-free alternatives to VecDeque + DashMap.
//!
//! ---
//! **Summary:**  
//! This crate currently provides *correct RAII-based invariant enforcement*
//! for bounded connection concurrency with automatic cleanup, but fairness,
//! fine-grained scheduling, and lock-free optimizations are deferred to
//! future iterations.

use dashmap::DashMap;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::{oneshot, OwnedSemaphorePermit, Semaphore};
use tracing::{debug, info};

/// Configuration constants
pub const MAX_GLOBAL: usize = 1000;
pub const MAX_PER_CLIENT: usize = 5;
pub const MAX_QUEUE_PER_CLIENT: usize = 10;

/// Errors for acquire failures
#[derive(Debug, thiserror::Error)]
pub enum AcquireErrorKind {
	#[error("per-client limit reached and queue full")]
	QueueFull,
	#[error("global limit reached")]
	GlobalLimit,
}

#[derive(Debug, thiserror::Error)]
#[error("failed to acquire connection permit")]
pub struct AcquireError {
	pub kind: AcquireErrorKind,
}

/// RAII permit holding both global and per-client resources
pub struct ConnectionPermit {
	_global: OwnedSemaphorePermit,
	client_id: String,
	guard: Arc<ConnectionGuardInner>,
}

impl ConnectionPermit {
	/// Explicit async cleanup (instead of spawning in Drop)
	pub fn release(self) {
		// Call internal cleanup before dropping self
		self.cleanup();
	}

	fn cleanup(&self) {
		if let Some(mut client_state) = self.guard.clients.get_mut(&self.client_id) {
			// decrement active count
			let active = client_state.active.fetch_sub(1, Ordering::SeqCst);
			tracing::info!("ConnectionPermit released for client {} (active={})", self.client_id, active - 1);

			// wake next queued connection if any
			if let Some(waiter) = client_state.queue.pop_front() {
				let _ = waiter.send(());
				debug!("Client {} dequeued into active slot", self.client_id);
			}

			// cleanup if empty
			if client_state.active.load(Ordering::SeqCst) == 0 && client_state.queue.is_empty() {
				drop(client_state); // Release before remove
				self.guard.clients.remove(&self.client_id);
				debug!("Client state cleaned up for {}", self.client_id);
			}
		}
	}
}

/// Per-client state
pub struct ClientState {
	pub active: AtomicUsize,
	pub queue: VecDeque<oneshot::Sender<()>>,
}

/// Inner shared state
pub struct ConnectionGuardInner {
	pub global: Arc<Semaphore>,
	pub clients: DashMap<String, ClientState>,
}

/// Public ConnectionGuard
#[derive(Clone)]
pub struct ConnectionGuard {
	pub inner: Arc<ConnectionGuardInner>,
}

impl ConnectionGuard {
	pub fn new() -> Self {
		Self {
			inner: Arc::new(ConnectionGuardInner {
				global: Arc::new(Semaphore::new(MAX_GLOBAL)),
				clients: DashMap::new(),
			}),
		}
	}

	pub async fn acquire(&self, client_id: String) -> Result<ConnectionPermit, AcquireError> {
		info!("Client {} attempting to acquire connection permit", client_id);

		// fast global check
		let global_permit = self.inner.global.clone().acquire_owned().await.map_err(|_| AcquireError {
			kind: AcquireErrorKind::GlobalLimit,
		})?;

		let mut client_state = self.inner.clients.entry(client_id.clone()).or_insert_with(|| ClientState {
			active: AtomicUsize::new(0),
			queue: VecDeque::new(),
		});

		let active_count = client_state.active.load(Ordering::SeqCst);

		if active_count < MAX_PER_CLIENT {
			client_state.active.fetch_add(1, Ordering::SeqCst);
			info!("Client {} acquired active slot ({}/{})", client_id, active_count + 1, MAX_PER_CLIENT);
			return Ok(ConnectionPermit {
				_global: global_permit,
				client_id,
				guard: self.inner.clone(),
			});
		}

		if client_state.queue.len() < MAX_QUEUE_PER_CLIENT {
			let (tx, rx) = oneshot::channel();
			client_state.queue.push_back(tx);
			info!(
				"Client {} queued for connection slot (queue={}/{})",
				client_id,
				client_state.queue.len(),
				MAX_QUEUE_PER_CLIENT
			);
			drop(client_state); // Release lock before awaiting

			let _ = rx.await;

			// Re-acquire lock to increment active count
			let client_state = self.inner.clients.get(&client_id).expect("client state should exist");
			client_state.active.fetch_add(1, Ordering::SeqCst);

			info!(
				"Client {} dequeued into active slot ({}/{})",
				client_id,
				client_state.active.load(Ordering::SeqCst),
				MAX_PER_CLIENT
			);
			return Ok(ConnectionPermit {
				_global: global_permit,
				client_id,
				guard: self.inner.clone(),
			});
		}

		drop(global_permit);
		info!("Client {} connection rejected: queue full", client_id);
		Err(AcquireError {
			kind: AcquireErrorKind::QueueFull,
		})
	}

	pub fn try_acquire_permit_hint(&self) -> bool {
		self.inner.global.available_permits() > 0
	}

	pub fn active_global(&self) -> usize {
		MAX_GLOBAL - self.inner.global.available_permits()
	}

	pub fn active_per_client(&self, client_id: &str) -> usize {
		self.inner.clients.get(client_id).map(|c| c.active.load(Ordering::SeqCst)).unwrap_or(0)
	}
}

impl Default for ConnectionGuard {
	fn default() -> Self {
		Self::new()
	}
}

use axum::{
	extract::{ConnectInfo, State},
	http::{HeaderMap, StatusCode},
	middleware::Next,
	response::Response,
};
use dashmap::DashMap;
use std::{
	collections::HashMap,
	net::SocketAddr,
	sync::Arc,
	time::{Duration, Instant},
};
use tokio::sync::Semaphore;
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use super::connection::core::ClientId;

// === Configuration ===

#[derive(Debug, Clone)]
pub struct ConnectionLimitConfig {
	pub max_per_client: usize,
	pub max_global: usize,
	pub acquire_timeout: Duration,
	pub enable_queuing: bool,
	pub queue_size_per_client: usize,
	pub max_queue_time: Duration,
}

impl Default for ConnectionLimitConfig {
	fn default() -> Self {
		Self {
			max_per_client: 5,
			max_global: 1000,
			acquire_timeout: Duration::from_secs(5),
			enable_queuing: true,
			queue_size_per_client: 10,
			max_queue_time: Duration::from_secs(30),
		}
	}
}

// === Connection Limiter ===

pub struct ConnectionLimiter {
	config: ConnectionLimitConfig,
	global: Arc<Semaphore>,
	clients: DashMap<String, Arc<ClientState>>,
}

impl ConnectionLimiter {
	pub fn new(config: ConnectionLimitConfig) -> Arc<Self> {
		let max_global = config.max_global;
		let limiter = Arc::new(Self {
			config,
			global: Arc::new(Semaphore::new(max_global)),
			clients: DashMap::new(),
		});

		// Start periodic cleanup task
		limiter.clone().start_cleanup_task(Duration::from_secs(60), Duration::from_secs(300));

		limiter
	}

	fn get_or_create_client(&self, client_id: &str) -> Arc<ClientState> {
		self
			.clients
			.entry(client_id.to_string())
			.or_insert_with(|| Arc::new(ClientState::new(self.config.max_per_client)))
			.clone()
	}

	pub async fn acquire(&self, client_id: &str) -> Result<ConnectionGuard, ConnectionLimitError> {
		let start = Instant::now();

		// Fast check: global capacity
		if self.global.available_permits() == 0 {
			return Err(ConnectionLimitError::GlobalLimitExceeded {
				current: self.config.max_global,
				limit: self.config.max_global,
			});
		}

		let client_state = self.get_or_create_client(client_id);

		// Check queue limits
		if self.config.enable_queuing {
			let queued = client_state.queued_count.load(std::sync::atomic::Ordering::Relaxed);
			if queued >= self.config.queue_size_per_client {
				return Err(ConnectionLimitError::QueueFull {
					client_id: client_id.to_string(),
					queue_size: queued,
					limit: self.config.queue_size_per_client,
				});
			}
			client_state.queued_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
		}

		// Acquire global permit with timeout
		let global_permit = match timeout(self.config.acquire_timeout, self.global.clone().acquire_owned()).await {
			Ok(Ok(permit)) => permit,
			Ok(Err(_)) => {
				self.decrement_queued(&client_state);
				return Err(ConnectionLimitError::GlobalSemaphoreError);
			}
			Err(_) => {
				self.decrement_queued(&client_state);
				return Err(ConnectionLimitError::AcquireTimeout {
					client_id: client_id.to_string(),
					timeout: self.config.acquire_timeout,
					elapsed: start.elapsed(),
				});
			}
		};

		// Acquire client permit
		let client_permit = match timeout(self.config.acquire_timeout.saturating_sub(start.elapsed()), client_state.semaphore.clone().acquire_owned()).await {
			Ok(Ok(permit)) => permit,
			Ok(Err(_)) => {
				self.decrement_queued(&client_state);
				drop(global_permit); // auto-release global permit
				return Err(ConnectionLimitError::ClientSemaphoreError);
			}
			Err(_) => {
				self.decrement_queued(&client_state);
				drop(global_permit);
				return Err(ConnectionLimitError::AcquireTimeout {
					client_id: client_id.to_string(),
					timeout: self.config.acquire_timeout,
					elapsed: start.elapsed(),
				});
			}
		};

		// Success: commit
		self.decrement_queued(&client_state);
		client_state.update_activity().await;

		info!(
			"Connection acquired for client {}: global={}, client={}",
			client_id,
			self.config.max_global - self.global.available_permits(),
			client_state.capacity - client_state.semaphore.available_permits()
		);

		Ok(ConnectionGuard {
			client_id: client_id.to_string(),
			client_state,
			global_permit: Arc::new(global_permit),
			client_permit: Arc::new(client_permit),
		})
	}

	fn decrement_queued(&self, client_state: &ClientState) {
		if self.config.enable_queuing {
			client_state.queued_count.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
		}
	}

	pub fn start_cleanup_task(self: Arc<Self>, interval: Duration, threshold: Duration) {
		tokio::spawn(async move {
			let mut ticker = tokio::time::interval(interval);
			loop {
				ticker.tick().await;
				self.cleanup_inactive(threshold).await;
			}
		});
	}

	pub fn start_cleanup_task_with_cancellation(self: Arc<Self>, interval: Duration, threshold: Duration, cancel_token: CancellationToken) -> tokio::task::JoinHandle<()> {
		tokio::spawn(async move {
			let mut ticker = tokio::time::interval(interval);

			loop {
				tokio::select! {
					// Check for cancellation first
					_ = cancel_token.cancelled() => {
						tracing::info!("Connection limiter cleanup task shutting down");
						break;
					}
					// Wait for next tick
					_ = ticker.tick() => {
						tokio::select! {
							// Allow cancellation during cleanup operation
							_ = cancel_token.cancelled() => {
								tracing::info!("Connection limiter cleanup interrupted during operation");
								break;
							}
							// Perform cleanup
							_ = self.cleanup_inactive(threshold) => {
								tracing::debug!("Connection limiter cleanup completed");
							}
						}
					}
				}
			}

			tracing::info!("Connection limiter cleanup task ended");
		})
	}

	async fn cleanup_inactive(&self, threshold: Duration) {
		let now = Instant::now();
		let mut to_remove = Vec::new();

		// Collect keys to remove
		for entry in self.clients.iter() {
			let (id, state) = entry.pair();
			let active = state.capacity - state.semaphore.available_permits();
			let last_activity = *state.last_activity.lock().await;
			if active == 0 && now.duration_since(last_activity) > threshold {
				to_remove.push(id.clone());
			}
		}

		// Remove inactive clients
		for id in to_remove {
			self.clients.remove(&id);
			info!("Cleaned up inactive client: {}", id);
		}
	}

	pub async fn get_stats(&self) -> ConnectionStats {
		let mut client_stats = HashMap::new();
		let mut total_queued = 0;

		for entry in self.clients.iter() {
			let (id, state) = entry.pair();
			let active = state.capacity - state.semaphore.available_permits();
			let queued = state.queued_count.load(std::sync::atomic::Ordering::Relaxed);
			total_queued += queued;
			client_stats.insert(id.clone(), ClientStats { active, queued });
		}

		ConnectionStats {
			global_active: self.config.max_global - self.global.available_permits(),
			global_limit: self.config.max_global,
			total_queued,
			client_stats,
		}
	}
}

// === Client State ===

struct ClientState {
	semaphore: Arc<Semaphore>,
	queued_count: std::sync::atomic::AtomicUsize,
	last_activity: tokio::sync::Mutex<Instant>,
	capacity: usize,
}

impl ClientState {
	fn new(max: usize) -> Self {
		Self {
			semaphore: Arc::new(Semaphore::new(max)),
			queued_count: std::sync::atomic::AtomicUsize::new(0),
			last_activity: tokio::sync::Mutex::new(Instant::now()),
			capacity: max,
		}
	}

	async fn update_activity(&self) {
		*self.last_activity.lock().await = Instant::now();
	}
}

// === RAII Connection Guard ===

#[derive(Clone)]
pub struct ConnectionGuard {
	client_id: String,
	client_state: Arc<ClientState>,
	pub global_permit: Arc<tokio::sync::OwnedSemaphorePermit>,
	pub client_permit: Arc<tokio::sync::OwnedSemaphorePermit>,
}

impl Drop for ConnectionGuard {
	fn drop(&mut self) {
		let active_client = self.client_state.capacity - self.client_state.semaphore.available_permits();
		info!("Connection released for client {}: active={}", self.client_id, active_client);
	}
}

// === Errors ===

#[derive(Debug, Clone)]
pub enum ConnectionLimitError {
	GlobalLimitExceeded { current: usize, limit: usize },
	ClientLimitExceeded { client_id: String, current: usize, limit: usize },
	QueueFull { client_id: String, queue_size: usize, limit: usize },
	AcquireTimeout { client_id: String, timeout: Duration, elapsed: Duration },
	GlobalSemaphoreError,
	ClientSemaphoreError,
}

impl std::fmt::Display for ConnectionLimitError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::GlobalLimitExceeded { current, limit } => {
				write!(f, "Global limit exceeded: {}/{}", current, limit)
			}
			Self::ClientLimitExceeded { client_id, current, limit } => {
				write!(f, "Client {} limit exceeded: {}/{}", client_id, current, limit)
			}
			Self::QueueFull { client_id, queue_size, limit } => {
				write!(f, "Queue full for {}: {}/{}", client_id, queue_size, limit)
			}
			Self::AcquireTimeout { client_id, timeout, elapsed } => {
				write!(f, "Timeout acquiring connection for {}: {:?} elapsed, {:?} timeout", client_id, elapsed, timeout)
			}
			Self::GlobalSemaphoreError => write!(f, "Global semaphore error"),
			Self::ClientSemaphoreError => write!(f, "Client semaphore error"),
		}
	}
}

impl std::error::Error for ConnectionLimitError {}

// === Stats ===

#[derive(Debug, Clone)]
pub struct ClientStats {
	pub active: usize,
	pub queued: usize,
}

#[derive(Debug, Clone)]
pub struct ConnectionStats {
	pub global_active: usize,
	pub global_limit: usize,
	pub total_queued: usize,
	pub client_stats: HashMap<String, ClientStats>,
}

// === Middleware ===

pub async fn connection_limit_middleware(
	State(limiter): State<Arc<ConnectionLimiter>>,
	ConnectInfo(addr): ConnectInfo<SocketAddr>,
	headers: HeaderMap,
	mut req: axum::extract::Request,
	next: Next,
) -> Result<Response, (StatusCode, &'static str)> {
	// Only apply to WebSocket upgrade requests
	if !is_websocket_upgrade(&headers) {
		return Ok(next.run(req).await);
	}

	// Extract client ID
	let client_id = ClientId::from_request(&headers, &addr).to_string();

	match limiter.acquire(&client_id).await {
		Ok(guard) => {
			req.extensions_mut().insert(guard);
			Ok(next.run(req).await)
		}
		Err(e) => {
			warn!("Connection rejected: {}", e);
			let (status, msg) = error_to_response(&e);
			Err((status, msg))
		}
	}
}

fn is_websocket_upgrade(headers: &HeaderMap) -> bool {
	headers
		.get("upgrade")
		.and_then(|v| v.to_str().ok())
		.map(|s| s.eq_ignore_ascii_case("websocket"))
		.unwrap_or(false)
}

fn error_to_response(error: &ConnectionLimitError) -> (StatusCode, &'static str) {
	match error {
		ConnectionLimitError::GlobalLimitExceeded { .. } => (StatusCode::SERVICE_UNAVAILABLE, "Server at capacity"),
		ConnectionLimitError::ClientLimitExceeded { .. } => (StatusCode::TOO_MANY_REQUESTS, "Too many connections from client"),
		ConnectionLimitError::QueueFull { .. } => (StatusCode::TOO_MANY_REQUESTS, "Connection queue full"),
		ConnectionLimitError::AcquireTimeout { .. } => (StatusCode::REQUEST_TIMEOUT, "Connection timeout"),
		_ => (StatusCode::INTERNAL_SERVER_ERROR, "Internal error"),
	}
}

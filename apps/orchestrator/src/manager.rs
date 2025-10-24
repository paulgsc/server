use crate::types::{subjects, ClientId, StateUpdate, StreamId};
use prost::Message;
use some_transport::Transport;
use std::sync::Arc;
use tokio::time::Duration;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};
use ws_connection::{Connection, ConnectionStore};
use ws_events::stream_orch::{OrchestratorConfig, StreamOrchestrator};

/// Subscription event key - we only track one type of subscription per stream
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub struct StreamSubscription;

impl ws_connection::core::subscription::EventKey for StreamSubscription {}

/// Manages a single stream orchestrator and its subscribers using ConnectionStore
pub struct ManagedOrchestrator<T>
where
	T: Transport<StateUpdate> + Send + Sync + 'static,
{
	stream_id: StreamId,
	orchestrator: Arc<StreamOrchestrator>,
	subscribers: Arc<ConnectionStore<StreamSubscription>>,
	state_publisher_task: Option<tokio::task::JoinHandle<()>>,
	cancel_token: CancellationToken,
	_phantom: std::marker::PhantomData<T>,
}

impl<T> ManagedOrchestrator<T>
where
	T: Transport<StateUpdate> + Send + Sync + Clone + 'static,
{
	/// Create a new managed orchestrator
	pub fn new(stream_id: StreamId, config: OrchestratorConfig, transport: T, parent_token: &CancellationToken) -> Result<Self, Box<dyn std::error::Error>> {
		let orchestrator = Arc::new(StreamOrchestrator::new(config)?);
		let subscribers = Arc::new(ConnectionStore::new());
		let cancel_token = parent_token.child_token();

		// Spawn state publisher task
		let state_publisher_task = Self::spawn_state_publisher(stream_id.clone(), Arc::clone(&orchestrator), transport, Arc::clone(&subscribers), cancel_token.clone());

		Ok(Self {
			stream_id,
			orchestrator,
			subscribers,
			state_publisher_task: Some(state_publisher_task),
			cancel_token,
			_phantom: std::marker::PhantomData,
		})
	}

	/// Spawn the state publisher task using the transport abstraction
	fn spawn_state_publisher(
		stream_id: StreamId,
		orchestrator: Arc<StreamOrchestrator>,
		transport: T,
		subscribers: Arc<ConnectionStore<StreamSubscription>>,
		cancel_token: CancellationToken,
	) -> tokio::task::JoinHandle<()> {
		tokio::spawn(async move {
			let mut state_rx = orchestrator.subscribe();

			loop {
				tokio::select! {
						_ = cancel_token.cancelled() => {
								debug!("State publisher for stream {} cancelled", stream_id);
								break;
						}
						result = state_rx.changed() => {
								match result {
										Ok(_) => {
												// Don't publish if no subscribers
												let sub_count = subscribers.len();
												if sub_count == 0 {
														continue;
												}

												let state = state_rx.borrow().clone();
												let update = StateUpdate::from_orchestrator_state(
														stream_id.clone(),
														&state,
												);

												// Broadcast state update via transport
												if let Err(e) = transport.broadcast(update).await {
														error!(
																"Failed to broadcast state update for stream {}: {}",
																stream_id, e
														);
												}
										}
										Err(_) => {
												warn!("State channel closed for stream {}", stream_id);
												break;
										}
								}
						}
				}
			}
		})
	}

	/// Get reference to the orchestrator
	pub fn orchestrator(&self) -> &Arc<StreamOrchestrator> {
		&self.orchestrator
	}

	/// Add a subscriber to this stream
	/// Creates a connection actor that tracks the subscription
	pub async fn add_subscriber(&self, client_id: ClientId, source_addr: std::net::SocketAddr) {
		let connection = Connection::new(client_id.clone(), source_addr);
		let connection_key = format!("{}:{}", self.stream_id, client_id);

		let handle = self.subscribers.insert(connection_key.clone(), connection, &self.cancel_token);

		// Subscribe to stream events (just tracking subscription existence)
		if let Err(e) = handle.subscribe(vec![StreamSubscription]).await {
			error!("Failed to subscribe client {}: {}", client_id, e);
		}

		let count = self.subscribers.len();
		info!("Added subscriber {} to stream {}. Total subscribers: {}", client_id, self.stream_id, count);
	}

	/// Remove a subscriber and return remaining count
	pub async fn remove_subscriber(&self, client_id: &ClientId) -> usize {
		let connection_key = format!("{}:{}", self.stream_id, client_id);

		if let Some(_handle) = self.subscribers.remove(&connection_key).await {
			info!("Removed subscriber {} from stream {}", client_id, self.stream_id);
		}

		let count = self.subscribers.len();
		info!("Remaining subscribers for stream {}: {}", self.stream_id, count);

		count
	}

	/// Update heartbeat timestamp for a subscriber
	pub async fn update_heartbeat(&self, client_id: &ClientId) {
		let connection_key = format!("{}:{}", self.stream_id, client_id);

		if let Some(handle) = self.subscribers.get(&connection_key) {
			if let Err(e) = handle.record_activity().await {
				warn!("Failed to record activity for client {}: {}", client_id, e);
			}
		}
	}

	/// Get current subscriber count
	pub async fn subscriber_count(&self) -> usize {
		self.subscribers.len()
	}

	/// Clean up stale subscribers (haven't sent heartbeat within timeout)
	/// Uses the actor's built-in staleness checking
	pub async fn cleanup_stale_subscribers(&self, timeout: Duration) -> usize {
		let mut stale_keys = Vec::new();

		// Check each connection for staleness
		self
			.subscribers
			.for_each_async(|handle| {
				let timeout = timeout;
				let stream_id = self.stream_id.clone();
				async move {
					// Check staleness via actor
					if let Err(e) = handle.check_stale(timeout).await {
						warn!("Failed to check staleness for connection: {}", e);
						return;
					}

					// Get state to see if it's stale
					match handle.get_state().await {
						Ok(state) if state.is_stale => {
							let key = format!("{}:{}", stream_id, handle.connection.client_id);
							// We can't capture stale_keys here due to borrow checker,
							// so we'll mark for disconnect instead
							if let Err(e) = handle.disconnect("Stale connection".to_string()).await {
								warn!("Failed to disconnect stale connection: {}", e);
							}
						}
						Err(e) => {
							warn!("Failed to get connection state: {}", e);
						}
						_ => {}
					}
				}
			})
			.await;

		let remaining = self.subscribers.len();

		if !stale_keys.is_empty() {
			warn!("Cleaned up stale subscribers from stream {}", self.stream_id);
		}

		remaining
	}

	/// Get subscriber statistics using ConnectionStore's built-in stats
	pub async fn get_stats(&self) -> SubscriberStats {
		let store_stats = self.subscribers.stats().await;

		SubscriberStats {
			total: store_stats.total_connections,
			active: store_stats.active_connections,
			stale: store_stats.stale_connections,
			unique_clients: store_stats.unique_clients,
		}
	}

	/// Gracefully shutdown the managed orchestrator
	pub async fn shutdown(mut self) {
		info!("Shutting down orchestrator for stream {}", self.stream_id);

		// Cancel state publisher
		self.cancel_token.cancel();

		// Abort and await state publisher task
		if let Some(task) = self.state_publisher_task.take() {
			task.abort();
			let _ = task.await;
		}

		// Disconnect all subscribers gracefully
		let keys = self.subscribers.keys();
		for key in keys {
			let _ = self.subscribers.remove(&key).await;
		}

		info!("Orchestrator for stream {} shutdown complete", self.stream_id);
	}
}

/// Statistics about subscribers for this stream
#[derive(Debug, Clone)]
pub struct SubscriberStats {
	pub total: usize,
	pub active: usize,
	pub stale: usize,
	pub unique_clients: usize,
}

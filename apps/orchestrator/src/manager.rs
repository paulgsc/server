use some_transport::{NatsTransport, Transport};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info};
use ws_events::{
	events::{Event, EventType, OrchestratorConfig, UnifiedEvent},
	stream_orch::StreamOrchestrator,
};

/// Manages a single stream orchestrator and its subscribers using ConnectionStore
pub struct ManagedOrchestrator {
	stream_id: String,
	orchestrator: Arc<StreamOrchestrator>,
	state_publisher_task: Option<tokio::task::JoinHandle<()>>,
	completion_monitor_task: Option<tokio::task::JoinHandle<()>>,
	cancel_token: CancellationToken,
}

impl ManagedOrchestrator {
	/// Create a new managed orchestrator
	pub fn new(
		stream_id: String,
		config: OrchestratorConfig,
		transport: NatsTransport<UnifiedEvent>,
		parent_token: &CancellationToken,
	) -> Result<Self, Box<dyn std::error::Error>> {
		let orchestrator = Arc::new(StreamOrchestrator::new(config)?);
		let cancel_token = parent_token.child_token();

		// Spawn state publisher task
		let state_publisher_task = Self::spawn_state_publisher(stream_id.clone(), Arc::clone(&orchestrator), transport, cancel_token.clone());

		// Spawn completion monitor task
		let completion_monitor_task = Self::spawn_completion_monitor(stream_id.clone(), Arc::clone(&orchestrator), cancel_token.clone());

		Ok(Self {
			stream_id,
			orchestrator,
			state_publisher_task: Some(state_publisher_task),
			completion_monitor_task: Some(completion_monitor_task),
			cancel_token,
		})
	}

	/// Spawn the state publisher task using the transport abstraction
	fn spawn_state_publisher(
		stream_id: String,
		orchestrator: Arc<StreamOrchestrator>,
		transport: NatsTransport<UnifiedEvent>,
		cancel_token: CancellationToken,
	) -> tokio::task::JoinHandle<()> {
		tokio::spawn(async move {
			info!("🚀 State publisher task started for stream {}", stream_id);

			let mut state_rx = orchestrator.subscribe();
			info!("📡 State publisher subscribed to orchestrator for stream {}", stream_id);

			loop {
				tokio::select! {
					_ = cancel_token.cancelled() => {
						info!("🛑 State publisher for stream {} cancelled", stream_id);
						break;
					}
					result = state_rx.changed() => {
						match result {
							Ok(_) => {

								let state = state_rx.borrow().clone();
								// Create Event with stream_id
								let event = Event::OrchestratorState {
									stream_id: stream_id.clone(),
									state,
								};

								// Convert to UnifiedEvent
								let unified_event = match UnifiedEvent::try_from(event) {
									Ok(v) => v,
									Err(e) => {
										error!("❌ Failed to convert event to unified event for stream {}: {}", stream_id, e);
										continue;
									}
								};

								// Get subject for this event type
								let subject = EventType::OrchestratorState.subject();

								// Send to NATS
								match transport.send_to_subject(subject, unified_event).await {
									Ok(_) => {
									}
									Err(e) => {
										error!(
											"❌ Failed to broadcast state update for stream {} to subject '{}': {}",
											stream_id, subject, e
										);
									}
								}
							}
							Err(e) => {
								error!("❌ State channel closed for stream {}: {}", stream_id, e);
								break;
							}
						}
					}
				}
			}

			info!("👋 State publisher task exiting for stream {}", stream_id);
		})
	}

	/// Get reference to the orchestrator
	pub fn orchestrator(&self) -> &Arc<StreamOrchestrator> {
		&self.orchestrator
	}

	/// Spawn a task that monitors for orchestration completion
	fn spawn_completion_monitor(stream_id: String, orchestrator: Arc<StreamOrchestrator>, cancel_token: CancellationToken) -> tokio::task::JoinHandle<()> {
		tokio::spawn(async move {
			let mut state_rx = orchestrator.subscribe();

			loop {
				tokio::select! {
					_ = cancel_token.cancelled() => {
						debug!("Completion monitor for stream {} cancelled", stream_id);
						break;
					}
					result = state_rx.changed() => {
						match result {
							Ok(_) => {
								let is_complete = {
									let state = state_rx.borrow();
									state.is_complete()
								};

								// Check if orchestration is complete
								if is_complete {
									info!("✅ Orchestration complete for stream {}", stream_id);

									// Auto-shutdown the orchestrator
									orchestrator.shutdown().await;

									info!("🛑 Orchestrator auto-shutdown complete for stream {}", stream_id);
									break;
								}
							}
							Err(_) => {
								error!("State channel closed for stream {} completion monitor", stream_id);
								break;
							}
						}
					}
				}
			}
		})
	}

	/// Gracefully shutdown the managed orchestrator
	pub async fn shutdown(mut self) {
		info!("Shutting down orchestrator for stream {}", self.stream_id);

		// Cancel all tasks
		self.cancel_token.cancel();

		// Wait for state publisher to finish
		if let Some(task) = self.state_publisher_task.take() {
			let _ = task.await;
		}

		// Wait for completion monitor to finish
		if let Some(task) = self.completion_monitor_task.take() {
			let _ = task.await;
		}

		// Shutdown the orchestrator itself (if not already shutdown by completion monitor)
		self.orchestrator.shutdown().await;

		info!("Orchestrator for stream {} shutdown complete", self.stream_id);
	}
}

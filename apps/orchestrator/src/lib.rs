use cursorium::core::StreamOrchestrator;
use dashmap::DashMap;
use some_transport::{NatsTransport, Transport};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};
use ws_events::events::{Event, EventType, OrchestratorCommandData, OrchestratorState, UnifiedEvent};

type StreamId = String;

/// Manages a single stream orchestrator
pub struct ManagedOrchestrator {
	orchestrator: Arc<StreamOrchestrator>,
	cancel_token: CancellationToken,
	state_publisher_handle: tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl ManagedOrchestrator {
	/// Create a new orchestrator (always starts unconfigured)
	pub fn new(parent_token: &CancellationToken) -> anyhow::Result<Self> {
		let orchestrator = Arc::new(StreamOrchestrator::new(None)?);
		let cancel_token = parent_token.child_token();

		Ok(Self {
			orchestrator,
			cancel_token,
			state_publisher_handle: tokio::sync::Mutex::new(None),
		})
	}

	/// Store the state publisher handle
	pub async fn set_publisher_handle(&self, handle: tokio::task::JoinHandle<()>) {
		*self.state_publisher_handle.lock().await = Some(handle);
	}

	/// Send command to orchestrator
	pub async fn send_command(&self, cmd: OrchestratorCommandData) -> anyhow::Result<()> {
		match cmd {
			OrchestratorCommandData::Configure(config) => {
				self.orchestrator.configure(OrchestratorCommandData::Configure(config)).await?;
			}
			OrchestratorCommandData::Start => {
				self.orchestrator.start().await?;
			}
			OrchestratorCommandData::Pause => {
				self.orchestrator.pause().await?;
			}
			OrchestratorCommandData::Resume => {
				self.orchestrator.resume().await?;
			}
			OrchestratorCommandData::Stop => {
				self.orchestrator.stop().await?;
			}
			OrchestratorCommandData::Reset => {
				self.orchestrator.reset().await?;
			}
			// Fire-and-forget commands (synchronous)
			OrchestratorCommandData::ForceScene(scene) => {
				self.orchestrator.force_scene(scene)?;
			}
			OrchestratorCommandData::SkipCurrentScene => {
				self.orchestrator.skip_current_scene()?;
			}
			OrchestratorCommandData::UpdateStreamStatus { .. } => {
				// Not exposed in facade - log warning
				warn!(
					"UpdateStreamStatus called but not implemented in facade. \
		     Consider adding to StreamOrchestrator if needed."
				);
			}
		}
		Ok(())
	}

	pub fn subscribe(&self) -> tokio::sync::watch::Receiver<OrchestratorState> {
		self.orchestrator.subscribe()
	}

	pub fn current_state(&self) -> OrchestratorState {
		self.orchestrator.current_state()
	}

	pub async fn shutdown(&self) {
		self.cancel_token.cancel();

		// Wait for state publisher to finish
		if let Some(handle) = self.state_publisher_handle.lock().await.take() {
			let _ = handle.await;
		}

		// Shutdown orchestrator
		self.orchestrator.shutdown().await;
	}
}

/// Top-level orchestrator service
#[derive(Clone)]
pub struct OrchestratorService {
	orchestrators: Arc<DashMap<StreamId, Arc<ManagedOrchestrator>>>,
	transport: NatsTransport<UnifiedEvent>,
	cancel_token: CancellationToken,
}

impl OrchestratorService {
	pub fn new(transport: NatsTransport<UnifiedEvent>) -> Self {
		Self {
			orchestrators: Arc::new(DashMap::new()),
			transport,
			cancel_token: CancellationToken::new(),
		}
	}

	/// Main event loop: listens for OrchestratorCommandData events
	pub async fn run(&self) -> anyhow::Result<()> {
		info!("🎬 Starting Orchestrator Service event loop");

		let mut command_rx = self.transport.subscribe_to_subject(EventType::OrchestratorCommandData.subject()).await;

		loop {
			tokio::select! {
				_ = self.cancel_token.cancelled() => {
					info!("Orchestrator service shutting down");
					break;
				}
				result = command_rx.recv() => {
					match result {
						Ok(unified_event) => {
							if let Err(e) = self.handle_event(unified_event).await {
								error!("Error handling event: {}", e);
							}
						}
						Err(e) => {
							error!("Command receiver error: {}", e);
							break;
						}
					}
				}
			}
		}

		self.shutdown_all().await;
		info!("Orchestrator service stopped");
		Ok(())
	}

	async fn handle_event(&self, unified_event: UnifiedEvent) -> anyhow::Result<()> {
		let event: Event = Result::<Event, String>::from(unified_event).map_err(|e| anyhow::anyhow!("Failed to convert event: {}", e))?;

		if let Event::OrchestratorCommandData { stream_id, command } = event {
			self.handle_command(stream_id, command).await?;
		} else {
			warn!("Received unexpected event type in command handler");
		}

		Ok(())
	}

	async fn handle_command(&self, stream_id: StreamId, cmd: OrchestratorCommandData) -> anyhow::Result<()> {
		// Get or create orchestrator
		let managed = if let Some(mgr) = self.orchestrators.get(&stream_id) {
			Arc::clone(&mgr)
		} else {
			// Create new orchestrator for this stream
			info!("Creating new orchestrator for stream: {}", stream_id);
			self.create_orchestrator(stream_id.clone()).await?
		};

		// Send command (FSM will enforce state transitions)
		if let Err(e) = managed.send_command(cmd).await {
			error!("Failed to execute command for stream {}: {}", stream_id, e);
			return Err(e);
		}

		Ok(())
	}

	async fn create_orchestrator(&self, stream_id: StreamId) -> anyhow::Result<Arc<ManagedOrchestrator>> {
		let manager = Arc::new(ManagedOrchestrator::new(&self.cancel_token)?);

		// Spawn state publisher for this orchestrator
		let state_publisher_handle = self.spawn_state_publisher(stream_id.clone(), &manager);

		// Store the handle so we can await it on shutdown
		manager.set_publisher_handle(state_publisher_handle).await;

		self.orchestrators.insert(stream_id.clone(), Arc::clone(&manager));

		info!("✅ Orchestrator created for stream: {}", stream_id);

		Ok(manager)
	}

	/// Spawn a task that publishes state updates to NATS
	fn spawn_state_publisher(&self, stream_id: StreamId, manager: &Arc<ManagedOrchestrator>) -> tokio::task::JoinHandle<()> {
		let mut state_rx = manager.subscribe();
		let transport = self.transport.clone();
		let stream_id_clone = stream_id.clone();
		let cancel_token = manager.cancel_token.clone();

		tokio::spawn(async move {
			loop {
				tokio::select! {
					_ = cancel_token.cancelled() => {
						info!("State publisher cancelled for stream: {}", stream_id_clone);
						break;
					}
					result = state_rx.changed() => {
						if result.is_err() {
							info!("State receiver closed for stream: {}", stream_id_clone);
							break;
						}

						let state = state_rx.borrow().clone();

						let event = Event::OrchestratorState {
							stream_id: stream_id_clone.clone(),
							state,
						};

						// Convert to UnifiedEvent and publish
						if let Ok(unified_event) = event.try_into() {
							let subject = EventType::OrchestratorState.subject();
							if let Err(e) = transport.send_to_subject(subject, unified_event).await {
								error!("Failed to publish state for stream {}: {}", stream_id_clone, e);
							}
						} else {
							warn!("Failed to convert OrchestratorState to UnifiedEvent");
						}
					}
				}
			}

			info!("State publisher stopped for stream: {}", stream_id_clone);
		})
	}

	async fn shutdown_all(&self) {
		info!("Shutting down all orchestrators...");

		// Collect all stream IDs first to avoid holding lock
		let stream_ids: Vec<String> = self.orchestrators.iter().map(|e| e.key().clone()).collect();

		// Shutdown each orchestrator
		for stream_id in stream_ids {
			if let Some(entry) = self.orchestrators.remove(&stream_id) {
				info!("Shutting down orchestrator for stream: {}", stream_id);
				entry.1.shutdown().await;
			}
		}

		info!("All orchestrators shut down");
	}

	pub fn shutdown(&self) {
		self.cancel_token.cancel();
	}

	/// Get current state for a stream
	pub fn get_state(&self, stream_id: &str) -> Option<OrchestratorState> {
		self.orchestrators.get(stream_id).map(|mgr| mgr.current_state())
	}

	/// List all active stream IDs
	pub fn list_streams(&self) -> Vec<String> {
		self.orchestrators.iter().map(|entry| entry.key().clone()).collect()
	}

	/// Get count of active orchestrators
	pub fn orchestrator_count(&self) -> usize {
		self.orchestrators.len()
	}
}

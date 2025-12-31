use cursorium::core::StreamOrchestrator;
use dashmap::DashMap;
use some_transport::{NatsTransport, Transport};
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};
use ws_events::events::{Event, EventType, OrchestratorCommandData, OrchestratorState, UnifiedEvent};

type StreamId = String;

/// Internal supervisor messages for lifecycle management
#[derive(Debug)]
enum SupervisorMsg {
	StreamTerminated(StreamId),
}

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
			OrchestratorCommandData::ForceScene(scene) => {
				self.orchestrator.force_scene(scene)?;
			}
			OrchestratorCommandData::SkipCurrentScene => {
				self.orchestrator.skip_current_scene()?;
			}
			OrchestratorCommandData::UpdateStreamStatus { .. } => {
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
		info!("Shutting down managed orchestrator");
		self.cancel_token.cancel();

		// Wait for state publisher to finish
		if let Some(handle) = self.state_publisher_handle.lock().await.take() {
			let _ = handle.await;
		}

		// Shutdown orchestrator
		self.orchestrator.shutdown().await;
	}
}

/// Top-level orchestrator service with supervisor pattern
#[derive(Clone)]
pub struct OrchestratorService {
	orchestrators: Arc<DashMap<StreamId, Arc<ManagedOrchestrator>>>,
	transport: NatsTransport<UnifiedEvent>,
	cancel_token: CancellationToken,
	supervisor_tx: mpsc::UnboundedSender<SupervisorMsg>,
}

impl OrchestratorService {
	pub fn new(transport: NatsTransport<UnifiedEvent>) -> Self {
		let (supervisor_tx, _) = mpsc::unbounded_channel();

		Self {
			orchestrators: Arc::new(DashMap::new()),
			transport,
			cancel_token: CancellationToken::new(),
			supervisor_tx,
		}
	}

	/// Main event loop: listens for commands and supervises lifecycle
	pub async fn run(&self) -> anyhow::Result<()> {
		info!("🎬 Starting Orchestrator Service event loop");

		let mut command_rx = self.transport.subscribe_to_subject(EventType::OrchestratorCommandData.subject()).await;
		let (supervisor_tx, mut supervisor_rx) = mpsc::unbounded_channel::<SupervisorMsg>();

		// Replace the supervisor_tx with the real one
		let service = Self {
			orchestrators: Arc::clone(&self.orchestrators),
			transport: self.transport.clone(),
			cancel_token: self.cancel_token.clone(),
			supervisor_tx,
		};

		loop {
			tokio::select! {
				_ = self.cancel_token.cancelled() => {
					info!("Orchestrator service shutting down");
					break;
				}
				// Handle incoming commands
				result = command_rx.recv() => {
					match result {
						Ok(unified_event) => {
							if let Err(e) = service.handle_event(unified_event).await {
								error!("Error handling event: {}", e);
							}
						}
						Err(e) => {
							error!("Command receiver error: {}", e);
							break;
						}
					}
				}
				// Handle supervisor lifecycle messages
				Some(msg) = supervisor_rx.recv() => {
					service.handle_supervisor_msg(msg).await;
				}
			}
		}

		service.shutdown_all().await;
		info!("Orchestrator service stopped");
		Ok(())
	}

	/// Handle supervisor lifecycle messages
	/// This is the ONLY place orchestrators are removed
	async fn handle_supervisor_msg(&self, msg: SupervisorMsg) {
		match msg {
			SupervisorMsg::StreamTerminated(stream_id) => {
				info!("🧹 Stream terminated, cleaning up: {}", stream_id);

				// Enforce invariant: Terminal state => not in map
				if let Some((_key, manager)) = self.orchestrators.remove(&stream_id) {
					manager.shutdown().await;
					info!("✅ Orchestrator removed and cleaned up for stream: {}", stream_id);
				}
			}
		}
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

		// Spawn state publisher with supervisor channel
		let state_publisher_handle = self.spawn_state_publisher(stream_id.clone(), &manager, self.supervisor_tx.clone());

		manager.set_publisher_handle(state_publisher_handle).await;

		// Enforce invariant: stream exists <=> it's in the map
		self.orchestrators.insert(stream_id.clone(), Arc::clone(&manager));

		info!("✅ Orchestrator created for stream: {}", stream_id);

		Ok(manager)
	}

	/// Spawn a task that publishes state updates and observes terminal states
	fn spawn_state_publisher(
		&self,
		stream_id: StreamId,
		manager: &Arc<ManagedOrchestrator>,
		supervisor_tx: mpsc::UnboundedSender<SupervisorMsg>,
	) -> tokio::task::JoinHandle<()> {
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

						// Publish state to NATS
						let event = Event::OrchestratorState {
							stream_id: stream_id_clone.clone(),
							state: state.clone(),
						};

						if let Ok(unified_event) = event.try_into() {
							let subject = EventType::OrchestratorState.subject();
							if let Err(e) = transport.send_to_subject(subject, unified_event).await {
								error!("Failed to publish state for stream {}: {}", stream_id_clone, e);
							}
						} else {
							warn!("Failed to convert OrchestratorState to UnifiedEvent");
						}

						// Observe terminal state and notify supervisor
						// This is pure observation, not cleanup
						if state.is_terminal() {
							info!("🏁 Terminal state reached for stream: {}", stream_id_clone);
							let _ = supervisor_tx.send(SupervisorMsg::StreamTerminated(stream_id_clone.clone()));
							break;
						}
					}
				}
			}

			info!("State publisher stopped for stream: {}", stream_id_clone);
		})
	}

	async fn shutdown_all(&self) {
		info!("Shutting down all orchestrators...");

		let stream_ids: Vec<String> = self.orchestrators.iter().map(|e| e.key().clone()).collect();

		for stream_id in stream_ids {
			if let Some((_key, manager)) = self.orchestrators.remove(&stream_id) {
				info!("Shutting down orchestrator for stream: {}", stream_id);
				manager.shutdown().await;
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

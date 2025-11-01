use crate::ManagedOrchestrator;
use dashmap::DashMap;
use some_transport::{NatsTransport, Transport};
use std::sync::Arc;
use tokio::time::Duration;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};
use ws_events::events::{Event, EventType, OrchestratorConfig, TickCommand, UnifiedEvent};

type StreamId = String;

/// Main orchestrator service that manages all stream orchestrators
#[derive(Clone)]
pub struct OrchestratorService {
	orchestrators: Arc<DashMap<StreamId, Arc<ManagedOrchestrator>>>,
	transport: NatsTransport<UnifiedEvent>,
	cancel_token: CancellationToken,
}

impl OrchestratorService {
	/// Create a new orchestrator service
	pub fn new(transport: NatsTransport<UnifiedEvent>) -> Self {
		Self {
			orchestrators: Arc::new(DashMap::new()),
			transport,
			cancel_token: CancellationToken::new(),
		}
	}

	/// Run the orchestrator service (main event loop)
	pub async fn run(&self) -> Result<(), Box<dyn std::error::Error>> {
		info!("🎬 Starting Orchestrator Service event loop");

		let mut command_rx = self.transport.subscribe_to_subject(EventType::TickCommand.subject()).await;

		loop {
			tokio::select! {
				_ = self.cancel_token.cancelled() => {
					info!("Orchestrator service shutting down");
					break;
				}
				result = command_rx.recv() => {
					match result {
						Ok(unified_event) => {
							if let Err(e) = self.handle_unified_event(unified_event).await {
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

		// Cleanup
		self.shutdown_all().await;

		info!("Orchestrator service stopped");
		Ok(())
	}

	/// Handle incoming unified event from NATS
	async fn handle_unified_event(&self, unified_event: UnifiedEvent) -> Result<(), Box<dyn std::error::Error>> {
		let event: Event = Result::<Event, String>::from(unified_event).map_err(|e| format!("Failed to convert unified event: {}", e))?;

		match event {
			Event::TickCommand { stream_id, command } => {
				self.handle_tick_command(stream_id, command).await?;
			}
			_ => {
				warn!("Received unexpected event type in command handler");
			}
		}

		Ok(())
	}

	/// Process a specific orchestrator command
	async fn handle_tick_command(&self, stream_id: StreamId, cmd: TickCommand) -> Result<(), Box<dyn std::error::Error>> {
		info!("Handling tick command for stream {}: {:?}", stream_id, cmd);

		match cmd {
			TickCommand::Start(config_opt) => {
				info!("Start command received for stream {}", stream_id);

				// Create orchestrator if it doesn't exist and config is provided
				if !self.orchestrators.contains_key(&stream_id) {
					if let Some(config) = config_opt {
						info!("Creating new orchestrator for stream: {}", stream_id);
						self.create_or_get_orchestrator(stream_id.clone(), config).await?;
					} else {
						return Err(format!("Cannot start orchestrator for stream {} - no config provided and orchestrator doesn't exist", stream_id).into());
					}
				}

				// Start the orchestrator
				if let Some(managed) = self.orchestrators.get(&stream_id) {
					managed.orchestrator().start()?;
				} else {
					return Err(format!("Failed to find orchestrator for stream: {}", stream_id).into());
				}
			}
			TickCommand::Stop => {
				info!("Stopping orchestrator for stream {}", stream_id);
				if let Some(managed) = self.orchestrators.get(&stream_id) {
					managed.orchestrator().stop()?;
				}
			}
			TickCommand::Pause => {
				if let Some(managed) = self.orchestrators.get(&stream_id) {
					managed.orchestrator().pause()?;
				}
			}
			TickCommand::Resume => {
				if let Some(managed) = self.orchestrators.get(&stream_id) {
					managed.orchestrator().resume()?;
				}
			}
			TickCommand::Reset => {
				if let Some(managed) = self.orchestrators.get(&stream_id) {
					managed.orchestrator().reset()?;
				}
			}
			TickCommand::ForceScene(scene_name) => {
				if let Some(managed) = self.orchestrators.get(&stream_id) {
					managed.orchestrator().force_scene(scene_name)?;
				}
			}
			TickCommand::SkipCurrentScene => {
				if let Some(managed) = self.orchestrators.get(&stream_id) {
					managed.orchestrator().skip_current_scene()?;
				}
			}
			TickCommand::UpdateStreamStatus {
				is_streaming,
				stream_time,
				timecode,
			} => {
				if let Some(managed) = self.orchestrators.get(&stream_id) {
					managed.orchestrator().update_stream_status(is_streaming, stream_time, timecode)?;
				}
			}
			TickCommand::Reconfigure(config) => {
				if let Some(managed) = self.orchestrators.get(&stream_id) {
					managed.orchestrator().configure(config)?;
				}
			}
		}

		Ok(())
	}

	/// Create or get an existing orchestrator
	async fn create_or_get_orchestrator(&self, stream_id: StreamId, config: OrchestratorConfig) -> Result<Arc<ManagedOrchestrator>, Box<dyn std::error::Error>> {
		// Check if already exists
		if let Some(existing) = self.orchestrators.get(&stream_id) {
			return Ok(Arc::clone(&existing));
		}

		// Create new with cancellation token
		let manager = Arc::new(ManagedOrchestrator::new(stream_id.clone(), config.into(), self.transport.clone(), &self.cancel_token)?);

		self.orchestrators.insert(stream_id.clone(), Arc::clone(&manager));

		info!("Created new orchestrator for stream {}", stream_id);

		// Schedule automatic cleanup when orchestration completes
		self.schedule_completion_cleanup(stream_id.clone(), Arc::clone(&manager));

		Ok(manager)
	}

	/// Schedule cleanup when orchestration completes
	fn schedule_completion_cleanup(&self, stream_id: StreamId, managed: Arc<ManagedOrchestrator>) {
		let orchestrators = Arc::clone(&self.orchestrators);

		tokio::spawn(async move {
			// Subscribe to state updates
			let mut state_rx = managed.orchestrator().subscribe();

			// Wait for completion
			while state_rx.changed().await.is_ok() {
				let is_complete = {
					let state = state_rx.borrow();
					state.is_complete()
				};

				if is_complete {
					info!("Orchestration complete for stream {}, scheduling removal", stream_id);

					// Wait a bit before cleanup to allow final state broadcasts
					tokio::time::sleep(Duration::from_secs(5)).await;

					// Remove from map
					if let Some((_, removed)) = orchestrators.remove(&stream_id) {
						info!("Removed completed orchestrator for stream {}", stream_id);

						// Try to cleanly shutdown if we can get ownership
						if let Ok(owned) = Arc::try_unwrap(removed) {
							owned.shutdown().await;
						}
					}
					break;
				}
			}
		});
	}

	/// Shutdown all orchestrators
	async fn shutdown_all(&self) {
		info!("Shuttind down all orchestrators");

		let orchestrators: Vec<_> = self.orchestrators.iter().map(|e| Arc::clone(e.value())).collect();

		for manager in orchestrators {
			if let Ok(owned) = Arc::try_unwrap(manager) {
				owned.shutdown().await;
			}
		}

		self.orchestrators.clear();
	}

	/// Request service shutdown
	pub fn shutdown(&self) {
		self.cancel_token.cancel();
	}
}

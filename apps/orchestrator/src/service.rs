use crate::managed_orchestrator::ManagedOrchestrator;
use crate::types::{orchestrator_command, subjects, subscription_command, ClientId, OrchestratorCommand, OrchestratorConfigDto, StateUpdate, StreamId, SubscriptionCommand};
use dashmap::DashMap;
use some_transport::{ReceiverTrait, Transport, TransportReceiver};
use std::sync::Arc;
use tokio::time::{interval, Duration};
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

/// Main orchestrator service that manages all stream orchestrators
pub struct OrchestratorService<T, R>
where
	T: Transport<StateUpdate> + Transport<OrchestratorCommand> + Transport<SubscriptionCommand> + Send + Sync + 'static,
	R: ReceiverTrait<OrchestratorCommand> + ReceiverTrait<SubscriptionCommand> + Send + 'static,
{
	transport: T,
	orchestrators: Arc<DashMap<StreamId, Arc<ManagedOrchestrator<T>>>>,
	cancel_token: CancellationToken,
	_phantom: std::marker::PhantomData<R>,
}

impl<T, R> OrchestratorService<T, R>
where
	T: Transport<StateUpdate> + Transport<OrchestratorCommand> + Transport<SubscriptionCommand> + Send + Sync + Clone + 'static,
	R: ReceiverTrait<OrchestratorCommand> + ReceiverTrait<SubscriptionCommand> + Send + 'static,
{
	/// Create a new orchestrator service
	pub fn new(transport: T) -> Self {
		Self {
			transport,
			orchestrators: Arc::new(DashMap::new()),
			cancel_token: CancellationToken::new(),
			_phantom: std::marker::PhantomData,
		}
	}

	/// Run the orchestrator service (main event loop)
	pub async fn run(
		self: Arc<Self>,
		mut command_rx: TransportReceiver<OrchestratorCommand, R>,
		mut subscription_rx: TransportReceiver<SubscriptionCommand, R>,
	) -> Result<(), Box<dyn std::error::Error>> {
		info!("🎬 Starting Orchestrator Service event loop");

		// Spawn heartbeat cleanup task
		let cleanup_task = self.spawn_cleanup_task();

		// Main event loop
		loop {
			tokio::select! {
					_ = self.cancel_token.cancelled() => {
							info!("Orchestrator service shutting down");
							break;
					}
					result = command_rx.recv() => {
							match result {
									Ok(cmd) => self.handle_command(cmd).await,
									Err(e) => {
											error!("Command receiver error: {}", e);
											break;
									}
							}
					}
					result = subscription_rx.recv() => {
							match result {
									Ok(cmd) => self.handle_subscription_command(cmd).await,
									Err(e) => {
											error!("Subscription receiver error: {}", e);
											break;
									}
							}
					}
			}
		}

		// Cleanup
		cleanup_task.abort();
		let _ = cleanup_task.await;
		self.shutdown_all().await;

		info!("Orchestrator service stopped");
		Ok(())
	}

	/// Spawn the cleanup task for stale subscribers
	fn spawn_cleanup_task(&self) -> tokio::task::JoinHandle<()> {
		let orchestrators = Arc::clone(&self.orchestrators);
		let cancel_token = self.cancel_token.clone();

		tokio::spawn(async move {
			let mut cleanup_interval = interval(Duration::from_secs(30));

			loop {
				tokio::select! {
						_ = cancel_token.cancelled() => {
								info!("Cleanup task cancelled");
								break;
						}
						_ = cleanup_interval.tick() => {
								let timeout = Duration::from_secs(90);
								for entry in orchestrators.iter() {
										entry.value().cleanup_stale_subscribers(timeout).await;
								}
						}
				}
			}
		})
	}

	/// Handle incoming orchestrator commands
	async fn handle_command(&self, cmd: OrchestratorCommand) {
		if let Err(e) = self.process_command(cmd).await {
			error!("Error processing command: {}", e);
		}
	}

	/// Process a specific orchestrator command
	async fn process_command(&self, cmd: OrchestratorCommand) -> Result<(), Box<dyn std::error::Error>> {
		use orchestrator_command::Command;

		match cmd.command {
			Some(Command::Start(start_cmd)) => {
				info!("Starting orchestrator for stream {}", start_cmd.stream_id);
				if let Some(config) = start_cmd.config {
					self.create_or_get_orchestrator(start_cmd.stream_id, config).await?;
				}
			}
			Some(Command::Stop(stop_cmd)) => {
				info!("Stopping orchestrator for stream {}", stop_cmd.stream_id);
				if let Some(managed) = self.orchestrators.get(&stop_cmd.stream_id) {
					managed.orchestrator().stop()?;
				}
			}
			Some(Command::Pause(pause_cmd)) => {
				if let Some(managed) = self.orchestrators.get(&pause_cmd.stream_id) {
					managed.orchestrator().pause()?;
				}
			}
			Some(Command::Resume(resume_cmd)) => {
				if let Some(managed) = self.orchestrators.get(&resume_cmd.stream_id) {
					managed.orchestrator().resume()?;
				}
			}
			Some(Command::ForceScene(force_cmd)) => {
				if let Some(managed) = self.orchestrators.get(&force_cmd.stream_id) {
					managed.orchestrator().force_scene(force_cmd.scene_name)?;
				}
			}
			Some(Command::SkipScene(skip_cmd)) => {
				if let Some(managed) = self.orchestrators.get(&skip_cmd.stream_id) {
					managed.orchestrator().skip_current_scene()?;
				}
			}
			Some(Command::UpdateStreamStatus(status_cmd)) => {
				if let Some(managed) = self.orchestrators.get(&status_cmd.stream_id) {
					managed
						.orchestrator()
						.update_stream_status(status_cmd.is_streaming, status_cmd.stream_time, status_cmd.timecode)?;
				}
			}
			Some(Command::Reconfigure(reconfig_cmd)) => {
				if let Some(managed) = self.orchestrators.get(&reconfig_cmd.stream_id) {
					if let Some(config) = reconfig_cmd.config {
						managed.orchestrator().configure(config.into())?;
					}
				}
			}
			None => {
				warn!("Received empty orchestrator command");
			}
		}
		Ok(())
	}

	/// Handle subscription management commands
	async fn handle_subscription_command(&self, cmd: SubscriptionCommand) {
		use subscription_command::Command;

		match cmd.command {
			Some(Command::Register(reg_cmd)) => {
				self.handle_register(reg_cmd.stream_id, reg_cmd.client_id, reg_cmd.source_addr).await;
			}
			Some(Command::Unregister(unreg_cmd)) => {
				self.handle_unregister(unreg_cmd.stream_id, unreg_cmd.client_id).await;
			}
			Some(Command::Heartbeat(hb_cmd)) => {
				self.handle_heartbeat(hb_cmd.stream_id, hb_cmd.client_id).await;
			}
			None => {
				warn!("Received empty subscription command");
			}
		}
	}

	/// Handle client registration
	async fn handle_register(&self, stream_id: StreamId, client_id: ClientId, source_addr: String) {
		info!("Client {} registering for stream {}", client_id, stream_id);

		if let Some(managed) = self.orchestrators.get(&stream_id) {
			// Parse source address or use placeholder
			let addr = source_addr.parse().unwrap_or_else(|_| "0.0.0.0:0".parse().unwrap());

			managed.add_subscriber(client_id, addr).await;

			// Start orchestrator if it has subscribers
			if managed.subscriber_count().await > 0 {
				let _ = managed.orchestrator().start();
			}
		} else {
			warn!("Client {} tried to register for non-existent stream {}", client_id, stream_id);
		}
	}

	/// Handle client unregistration
	async fn handle_unregister(&self, stream_id: StreamId, client_id: ClientId) {
		info!("Client {} unregistering from stream {}", client_id, stream_id);

		if let Some(managed) = self.orchestrators.get(&stream_id) {
			let remaining = managed.remove_subscriber(&client_id).await;

			// Stop orchestrator if no subscribers
			if remaining == 0 {
				info!("No subscribers left for stream {}, stopping orchestrator", stream_id);
				let _ = managed.orchestrator().stop();

				// Schedule removal after idle timeout
				self.schedule_idle_removal(stream_id).await;
			}
		}
	}

	/// Handle heartbeat update
	async fn handle_heartbeat(&self, stream_id: StreamId, client_id: ClientId) {
		if let Some(managed) = self.orchestrators.get(&stream_id) {
			managed.update_heartbeat(&client_id).await;
		}
	}

	/// Schedule removal of idle orchestrator
	async fn schedule_idle_removal(&self, stream_id: StreamId) {
		let orchestrators = Arc::clone(&self.orchestrators);
		let stream_id_clone = stream_id.clone();

		tokio::spawn(async move {
			// Wait 60 seconds
			tokio::time::sleep(Duration::from_secs(60)).await;

			// Check if still idle and remove
			if let Some((_, managed)) = orchestrators.remove(&stream_id_clone) {
				if managed.subscriber_count().await == 0 {
					info!("Removing idle orchestrator for stream {}", stream_id_clone);
					// managed will be dropped, triggering shutdown
				} else {
					// Subscribers came back, re-insert
					orchestrators.insert(stream_id_clone.clone(), managed);
				}
			}
		});
	}

	/// Create or get an existing orchestrator
	async fn create_or_get_orchestrator(&self, stream_id: StreamId, config: OrchestratorConfigDto) -> Result<Arc<ManagedOrchestrator<T>>, Box<dyn std::error::Error>> {
		// Check if already exists
		if let Some(existing) = self.orchestrators.get(&stream_id) {
			return Ok(Arc::clone(&existing));
		}

		// Create new with cancellation token
		let managed = Arc::new(ManagedOrchestrator::new(stream_id.clone(), config.into(), self.transport.clone(), &self.cancel_token)?);

		self.orchestrators.insert(stream_id.clone(), Arc::clone(&managed));

		info!("Created new orchestrator for stream {}", stream_id);

		Ok(managed)
	}

	/// Shutdown all orchestrators
	async fn shutdown_all(&self) {
		info!("Shutting down all orchestrators");

		let orchestrators: Vec<_> = self.orchestrators.iter().map(|e| Arc::clone(e.value())).collect();

		for managed in orchestrators {
			// Try to extract ownership for clean shutdown
			if let Ok(owned) = Arc::try_unwrap(managed) {
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

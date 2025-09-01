use super::*;
use tokio::sync::mpsc;
use tracing::{error, info, instrument, warn};

/// Messages for the outbound worker
#[derive(Debug)]
enum OutboundMessage {
	Poll(Vec<serde_json::Value>),
	Command(serde_json::Value),
	Disconnect,
}

/// Configurable polling manager with outbound worker
pub struct ObsPollingManager {
	requests: ObsPollingRequests,
	config: PollingConfig,
	command_executor: CommandExecutor,
	high_freq_interval: Duration,
	medium_freq_interval: Duration,
	low_freq_interval: Duration,
	outbound_tx: mpsc::Sender<OutboundMessage>,
}

impl ObsPollingManager {
	pub fn new(config: PollingConfig, command_executor: CommandExecutor, sink: SharedSink) -> Self {
		Self::with_id_strategy(config, command_executor, sink, RequestIdStrategy::Uuid)
	}

	/// Create with specific ID generation strategy
	pub fn with_id_strategy(config: PollingConfig, command_executor: CommandExecutor, sink: SharedSink, id_strategy: RequestIdStrategy) -> Self {
		// Create bounded channel - adjust capacity as needed (512 seems reasonable)
		let (outbound_tx, outbound_rx) = mpsc::channel::<OutboundMessage>(512);

		// Spawn the outbound worker task
		tokio::spawn(Self::outbound_worker(sink, outbound_rx));

		Self {
			requests: ObsPollingRequests::with_strategy(id_strategy),
			config,
			command_executor,
			high_freq_interval: Duration::from_secs(1),
			medium_freq_interval: Duration::from_secs(5),
			low_freq_interval: Duration::from_secs(30),
			outbound_tx,
		}
	}

	/// The outbound worker: handles ALL network I/O to prevent DDOS
	async fn outbound_worker(sink: SharedSink, mut rx: mpsc::Receiver<OutboundMessage>) {
		info!("Starting OBS outbound worker");

		while let Some(msg) = rx.recv().await {
			let mut s_g = sink.lock().await;

			match msg {
				OutboundMessage::Poll(batch) => {
					if batch.is_empty() {
						continue;
					}

					let batch_size = batch.len();

					for req in batch {
						if let Ok(request_text) = serde_json::to_string(&req) {
							if let Err(e) = s_g.send(TungsteniteMessage::Text(request_text.into())).await {
								error!("Poll request send failed: {}", e);
							}
						}
					}

					if let Err(e) = s_g.flush().await {
						error!("Failed to flush poll batch of {}: {}", batch_size, e);
					}
				}
				OutboundMessage::Command(cmd) => {
					if let Ok(request_text) = serde_json::to_string(&cmd) {
						if let Err(e) = s_g.send(TungsteniteMessage::Text(request_text.into())).await {
							error!("Command send failed: {}", e);
						} else if let Err(e) = s_g.flush().await {
							error!("Command flush failed: {}", e);
						} else {
							info!("Successfully sent command request");
						}
					}
				}
				OutboundMessage::Disconnect => {
					warn!("Disconnect requested, outbound worker exiting");
					return;
				}
			}
		}

		warn!("Outbound worker channel closed, worker exiting");
	}

	/// Main polling loop with configurable requests and command handling
	#[instrument(skip(self, cmd_rx))]
	pub async fn start_polling_loop(self, mut cmd_rx: mpsc::Receiver<InternalCommand>) -> Result<(), PollingError> {
		let mut high_freq_timer = interval(self.high_freq_interval);
		let mut medium_freq_timer = interval(self.medium_freq_interval);
		let mut low_freq_timer = interval(self.low_freq_interval);

		// Skip the first tick to avoid immediate execution
		high_freq_timer.tick().await;
		medium_freq_timer.tick().await;
		low_freq_timer.tick().await;

		let mut loop_counter = 0u64;
		let mut high_freq_counter = 0u64;
		let mut medium_freq_counter = 0u64;
		let mut low_freq_counter = 0u64;
		let mut cmd_counter = 0u64;
		let mut dropped_polls = 0u64;

		info!(
			"Starting OBS polling loop with {} high, {} medium, {} low frequency requests",
			self.config.high_frequency_requests.len(),
			self.config.medium_frequency_requests.len(),
			self.config.low_frequency_requests.len()
		);

		loop {
			loop_counter += 1;

			tokio::select! {
					// High frequency polling (1 second)
					_ = high_freq_timer.tick() => {
							high_freq_counter += 1;

							if !self.config.high_frequency_requests.is_empty() {
									let requests = self.requests.generate_requests(&self.config.high_frequency_requests);

									// Use try_send for polls - drop if queue is full (backpressure)
									if let Err(mpsc::error::TrySendError::Full(_)) =
											self.outbound_tx.try_send(OutboundMessage::Poll(requests)) {
											dropped_polls += 1;
											warn!("Dropped high frequency poll batch #{} - outbound queue full", high_freq_counter);
									}
							}
					}

					// Medium frequency polling (5 seconds)
					_ = medium_freq_timer.tick() => {
							medium_freq_counter += 1;

							if !self.config.medium_frequency_requests.is_empty() {
									let requests = self.requests.generate_requests(&self.config.medium_frequency_requests);

									if let Err(mpsc::error::TrySendError::Full(_)) =
											self.outbound_tx.try_send(OutboundMessage::Poll(requests)) {
											dropped_polls += 1;
											warn!("Dropped medium frequency poll batch #{} - outbound queue full", medium_freq_counter);
									}
							}
					}

					// Low frequency polling (30 seconds)
					_ = low_freq_timer.tick() => {
							low_freq_counter += 1;

							if !self.config.low_frequency_requests.is_empty() {
									let requests = self.requests.generate_requests(&self.config.low_frequency_requests);

									if let Err(mpsc::error::TrySendError::Full(_)) =
											self.outbound_tx.try_send(OutboundMessage::Poll(requests)) {
											dropped_polls += 1;
											warn!("Dropped low frequency poll batch #{} - outbound queue full", low_freq_counter);
									}
							}
					}

					// Handle internal commands (Execute or Disconnect)
					Some(internal_cmd) = cmd_rx.recv() => {
							cmd_counter += 1;

							match internal_cmd {
									InternalCommand::Execute(obs_cmd) => {

											// Build the request using the command executor's builder
											let request = self.command_executor.build_request(&obs_cmd);

											// Commands are higher priority - await send to ensure delivery
											if let Err(e) = self.outbound_tx.send(OutboundMessage::Command(request)).await {
													error!("Failed to queue command #{}: {}", cmd_counter, e);
													return Err(PollingError::CriticalLoopTermination {
															reason: format!("Command queue failure on command #{}: {}", cmd_counter, e)
													});
											}

											info!("Successfully queued command #{}: {:?}", cmd_counter, obs_cmd);
									}
									InternalCommand::Disconnect => {
											info!("Received disconnect command, shutting down gracefully");
											let _ = self.outbound_tx.send(OutboundMessage::Disconnect).await;
											return Ok(());
									}
							}
					}

					// Channel closed unexpectedly
					else => {
							error!("Command channel closed unexpectedly after {} loop iterations", loop_counter);
							error!("Loop counters at exit - Total: {}, High: {}, Medium: {}, Low: {}, Commands: {}, Dropped polls: {}",
									loop_counter, high_freq_counter, medium_freq_counter, low_freq_counter, cmd_counter, dropped_polls);

							// Try to signal outbound worker to shut down
							let _ = self.outbound_tx.send(OutboundMessage::Disconnect).await;

							return Err(PollingError::ChannelClosed);
					}
			}
		}
	}
}

use crate::polling::{CommandExecutor, InternalCommand, PollingConfig, PollingError, SharedSink};
use futures_util::sink::SinkExt;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::{interval, Interval};
use tokio_tungstenite::tungstenite::protocol::Message as TungsteniteMessage;
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
	config: PollingConfig,
	command_executor: CommandExecutor,
	high_freq_interval: Duration,
	medium_freq_interval: Duration,
	low_freq_interval: Duration,
	outbound_tx: mpsc::Sender<OutboundMessage>,
}

/// Loop counters for tracking polling activity
#[derive(Debug, Default)]
struct LoopCounters {
	loop_counter: u64,
	high_freq_counter: u64,
	medium_freq_counter: u64,
	low_freq_counter: u64,
	cmd_counter: u64,
	dropped_polls: u64,
}

impl ObsPollingManager {
	pub fn new(config: PollingConfig, command_executor: CommandExecutor, sink: SharedSink) -> Self {
		Self::with_id_strategy(config, command_executor, sink)
	}

	/// Create with specific ID generation strategy
	pub fn with_id_strategy(config: PollingConfig, command_executor: CommandExecutor, sink: SharedSink) -> Self {
		// Create bounded channel - adjust capacity as needed (512 seems reasonable)
		let (outbound_tx, outbound_rx) = mpsc::channel::<OutboundMessage>(512);

		// Spawn the outbound worker task
		tokio::spawn(Self::outbound_worker(sink, outbound_rx));

		Self {
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
					Self::handle_poll_batch(&mut s_g, batch).await;
				}
				OutboundMessage::Command(cmd) => {
					Self::handle_command(&mut s_g, cmd).await;
				}
				OutboundMessage::Disconnect => {
					warn!("Disconnect requested, outbound worker exiting");
					return;
				}
			}
		}

		warn!("Outbound worker channel closed, worker exiting");
	}

	/// Handle sending a batch of poll requests
	async fn handle_poll_batch(
		sink: &mut futures_util::stream::SplitSink<tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>, TungsteniteMessage>,
		batch: Vec<serde_json::Value>,
	) {
		if batch.is_empty() {
			return;
		}

		let batch_size = batch.len();

		for req in batch {
			if let Ok(request_text) = serde_json::to_string(&req) {
				if let Err(e) = sink.send(TungsteniteMessage::Text(request_text.into())).await {
					error!("Poll request send failed: {e}");
				}
			}
		}

		if let Err(e) = sink.flush().await {
			error!("Failed to flush poll batch of {batch_size}: {e}");
		}
	}

	/// Handle sending a single command
	async fn handle_command(
		sink: &mut futures_util::stream::SplitSink<tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>, TungsteniteMessage>,
		cmd: serde_json::Value,
	) {
		if let Ok(request_text) = serde_json::to_string(&cmd) {
			if let Err(e) = sink.send(TungsteniteMessage::Text(request_text.into())).await {
				error!("Command send failed: {e}");
			} else if let Err(e) = sink.flush().await {
				error!("Command flush failed: {e}");
			} else {
				info!("Successfully sent command request");
			}
		}
	}

	/// Initialize timers for polling intervals
	fn initialize_timers(&self) -> (Interval, Interval, Interval) {
		let mut high_freq_timer = interval(self.high_freq_interval);
		let mut medium_freq_timer = interval(self.medium_freq_interval);
		let mut low_freq_timer = interval(self.low_freq_interval);

		// Skip the first tick to avoid immediate execution
		tokio::spawn(async move {
			high_freq_timer.tick().await;
		});
		tokio::spawn(async move {
			medium_freq_timer.tick().await;
		});
		tokio::spawn(async move {
			low_freq_timer.tick().await;
		});

		(interval(self.high_freq_interval), interval(self.medium_freq_interval), interval(self.low_freq_interval))
	}

	/// Handle high frequency polling tick
	fn handle_high_freq_tick(&self, counters: &mut LoopCounters, high_freq_requests: &[serde_json::Value]) {
		counters.high_freq_counter += 1;

		if !self.config.high_frequency_requests.is_empty() {
			if let Err(mpsc::error::TrySendError::Full(_)) = self.outbound_tx.try_send(OutboundMessage::Poll(high_freq_requests.to_vec())) {
				counters.dropped_polls += 1;
				warn!("Dropped high frequency poll batch #{} - outbound queue full", counters.high_freq_counter);
			}
		}
	}

	/// Handle medium frequency polling tick
	fn handle_medium_freq_tick(&self, counters: &mut LoopCounters, medium_freq_requests: &[serde_json::Value]) {
		counters.medium_freq_counter += 1;

		if !self.config.medium_frequency_requests.is_empty() {
			if let Err(mpsc::error::TrySendError::Full(_)) = self.outbound_tx.try_send(OutboundMessage::Poll(medium_freq_requests.to_vec())) {
				counters.dropped_polls += 1;
				warn!("Dropped medium frequency poll batch #{} - outbound queue full", counters.medium_freq_counter);
			}
		}
	}

	/// Handle low frequency polling tick
	fn handle_low_freq_tick(&self, counters: &mut LoopCounters, low_freq_requests: &[serde_json::Value]) {
		counters.low_freq_counter += 1;

		if !self.config.low_frequency_requests.is_empty() {
			if let Err(mpsc::error::TrySendError::Full(_)) = self.outbound_tx.try_send(OutboundMessage::Poll(low_freq_requests.to_vec())) {
				counters.dropped_polls += 1;
				warn!("Dropped low frequency poll batch #{} - outbound queue full", counters.low_freq_counter);
			}
		}
	}

	/// Handle internal command processing
	async fn handle_internal_command(&self, counters: &mut LoopCounters, internal_cmd: InternalCommand) -> Result<bool, PollingError> {
		counters.cmd_counter += 1;

		match internal_cmd {
			InternalCommand::Execute(obs_cmd) => {
				// Build the request using the command executor's builder
				let request = self.command_executor.build_request(&obs_cmd)?;

				// Commands are higher priority - await send to ensure delivery
				if let Err(e) = self.outbound_tx.send(OutboundMessage::Command(request)).await {
					error!("Failed to queue command #{}: {e}", counters.cmd_counter);
					return Err(PollingError::CriticalLoopTermination {
						reason: format!("Command queue failure on command #{}: {e}", counters.cmd_counter),
					});
				}

				info!("Successfully queued command #{}: {:?}", counters.cmd_counter, obs_cmd);
				Ok(false) // Continue loop
			}
			InternalCommand::Disconnect => {
				info!("Received disconnect command, shutting down gracefully");
				let _ = self.outbound_tx.send(OutboundMessage::Disconnect).await;
				Ok(true) // Exit loop
			}
		}
	}

	/// Handle unexpected channel closure
	async fn handle_channel_closed(&self, counters: &LoopCounters) -> Result<(), PollingError> {
		error!("Command channel closed unexpectedly after {} loop iterations", counters.loop_counter);
		error!(
			"Loop counters at exit - Total: {}, High: {}, Medium: {}, Low: {}, Commands: {}, Dropped polls: {}",
			counters.loop_counter, counters.high_freq_counter, counters.medium_freq_counter, counters.low_freq_counter, counters.cmd_counter, counters.dropped_polls
		);

		// Try to signal outbound worker to shut down
		let _ = self.outbound_tx.send(OutboundMessage::Disconnect).await;

		Err(PollingError::ChannelClosed)
	}

	/// Main polling loop with configurable requests and command handling
	#[instrument(skip(self, cmd_rx))]
	pub async fn start_polling_loop(self, mut cmd_rx: mpsc::Receiver<InternalCommand>) -> Result<(), PollingError> {
		let (mut high_freq_timer, mut medium_freq_timer, mut low_freq_timer) = self.initialize_timers();

		// Skip first ticks properly
		high_freq_timer.tick().await;
		medium_freq_timer.tick().await;
		low_freq_timer.tick().await;

		let mut counters = LoopCounters::default();

		info!(
			"Starting OBS polling loop with {} high, {} medium, {} low frequency requests",
			self.config.high_frequency_requests.len(),
			self.config.medium_frequency_requests.len(),
			self.config.low_frequency_requests.len()
		);

		let (high_freq, medium_freq, low_freq) = self.config.generate_all_requests();
		let high_freq = high_freq?;
		let medium_freq = medium_freq?;
		let low_freq = low_freq?;

		loop {
			counters.loop_counter += 1;

			tokio::select! {
				// High frequency polling (1 second)
				_ = high_freq_timer.tick() => {
					self.handle_high_freq_tick(&mut counters, &high_freq);
				}

				// Medium frequency polling (5 seconds)
				_ = medium_freq_timer.tick() => {
					self.handle_medium_freq_tick(&mut counters, &medium_freq);
				}

				// Low frequency polling (30 seconds)
				_ = low_freq_timer.tick() => {
					self.handle_low_freq_tick(&mut counters, &low_freq);
				}

				// Handle internal commands (Execute or Disconnect)
				Some(internal_cmd) = cmd_rx.recv() => {
					let should_exit = self.handle_internal_command(&mut counters, internal_cmd).await?;
					if should_exit {
						return Ok(());
					}
				}

				// Channel closed unexpectedly
				else => {
					return self.handle_channel_closed(&counters).await;
				}
			}
		}
	}
}

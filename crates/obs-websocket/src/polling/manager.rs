use super::*;

/// Configurable polling manager
pub struct ObsPollingManager {
	requests: ObsPollingRequests,
	config: PollingConfig,
	command_executor: CommandExecutor,
	high_freq_interval: Duration,
	medium_freq_interval: Duration,
	low_freq_interval: Duration,
}

impl ObsPollingManager {
	pub fn new(config: PollingConfig, command_executor: CommandExecutor) -> Self {
		Self::with_id_strategy(config, command_executor, RequestIdStrategy::Uuid)
	}

	/// Create with specific ID generation strategy
	pub fn with_id_strategy(config: PollingConfig, command_executor: CommandExecutor, id_strategy: RequestIdStrategy) -> Self {
		Self {
			requests: ObsPollingRequests::with_strategy(id_strategy),
			config,
			command_executor,
			high_freq_interval: Duration::from_secs(1),   // Every second
			medium_freq_interval: Duration::from_secs(5), // Every 5 seconds
			low_freq_interval: Duration::from_secs(30),   // Every 30 seconds
		}
	}

	/// Create from a slice of (RequestType, Frequency) tuples
	pub fn from_request_slice(requests: &[(ObsRequestType, PollingFrequency)], command_executor: CommandExecutor) -> Self {
		Self::new(PollingConfig::from(requests), command_executor)
	}

	/// Main polling loop with configurable requests and command handling
	#[instrument(skip(self, sink, cmd_rx))]
	pub async fn start_polling_loop(self, sink: SharedSink, mut cmd_rx: tokio::sync::mpsc::Receiver<InternalCommand>) -> Result<(), PollingError> {
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
						if let Err(e) = self.send_requests(&sink, requests).await {
							error!("Failed to send high frequency requests (tick #{}): {}", high_freq_counter, e);
							return Err(PollingError::CriticalLoopTermination {
								reason: format!("High frequency request failure on tick #{}: {}", high_freq_counter, e)
							});
						}
					}
				}

				// Medium frequency polling (5 seconds)
				_ = medium_freq_timer.tick() => {
					medium_freq_counter += 1;

					if !self.config.medium_frequency_requests.is_empty() {
						let requests = self.requests.generate_requests(&self.config.medium_frequency_requests);
						if let Err(e) = self.send_requests(&sink, requests).await {
							error!("Failed to send medium frequency requests (tick #{}): {}", medium_freq_counter, e);
							return Err(PollingError::CriticalLoopTermination {
								reason: format!("Medium frequency request failure on tick #{}: {}", medium_freq_counter, e)
							});
						}
					}
				}

				// Low frequency polling (30 seconds)
				_ = low_freq_timer.tick() => {
					low_freq_counter += 1;

					if !self.config.low_frequency_requests.is_empty() {
						let requests = self.requests.generate_requests(&self.config.low_frequency_requests);
						if let Err(e) = self.send_requests(&sink, requests).await {
							error!("Failed to send low frequency requests (tick #{}): {}", low_freq_counter, e);
							return Err(PollingError::CriticalLoopTermination {
								reason: format!("Low frequency request failure on tick #{}: {}", low_freq_counter, e)
							});
						}
					}
				}

				// Handle internal commands (Execute or Disconnect)
				Some(internal_cmd) = cmd_rx.recv() => {
					cmd_counter += 1;

					match internal_cmd {
						InternalCommand::Execute(obs_cmd) => {
							// First validate the command through the executor
							if let Err(e) = self.command_executor.execute(obs_cmd.clone()).await {
								error!("Command validation failed (command #{}): {:?} - Error: {}", cmd_counter, obs_cmd, e);
								// Continue processing other commands even if this one fails validation
								continue;
							}

							// Build the request using the command executor's builder
							let request = self.command_executor.build_request(&obs_cmd);

							// Send the request
							if let Err(e) = self.send_single_request(&sink, request).await {
								error!("Failed to send command request (command #{}): {}", cmd_counter, e);
								return Err(PollingError::CriticalLoopTermination {
									reason: format!("Command request send failure on command #{}: {}", cmd_counter, e)
								});
							}

							info!("Successfully executed command #{}: {:?}", cmd_counter, obs_cmd);
						}
						InternalCommand::Disconnect => {
							info!("Received disconnect command, exiting polling loop gracefully");
							return Ok(());
						}
					}
				}

				// Channel closed unexpectedly
				else => {
					error!("Command channel closed unexpectedly after {} loop iterations", loop_counter);
					error!("Loop counters at exit - Total: {}, High: {}, Medium: {}, Low: {}, Commands: {}",
						loop_counter, high_freq_counter, medium_freq_counter, low_freq_counter, cmd_counter);
					return Err(PollingError::ChannelClosed);
				}
			}
		}
	}

	/// Send a batch of requests to OBS
	async fn send_requests(&self, sink: &SharedSink, requests: Vec<serde_json::Value>) -> Result<(), PollingError> {
		if requests.is_empty() {
			return Ok(());
		}

		let total_count = requests.len();
		let mut send_errors = Vec::new();

		// Send all requests
		{
			let mut s_g = sink.lock().await;
			for (i, req) in requests.iter().enumerate() {
				let request_text = serde_json::to_string(req)?;
				if let Err(e) = s_g.send(TungsteniteMessage::Text(request_text.into())).await {
					error!("Failed to send request {}/{}: {}", i + 1, total_count, e);
					send_errors.push(e);
				}
			}

			// Flush all requests at once
			if let Err(e) = s_g.flush().await {
				return Err(PollingError::FlushFailure {
					request_count: total_count,
					error: e,
				});
			}
		}

		// Check if any sends failed
		if !send_errors.is_empty() {
			let failed_count = send_errors.len();
			let first_error = send_errors.into_iter().next().unwrap();

			return Err(PollingError::BatchSendFailure {
				failed_count,
				total_count,
				first_error,
			});
		}

		Ok(())
	}

	/// Send a single request to OBS (used for commands)
	async fn send_single_request(&self, sink: &SharedSink, request: serde_json::Value) -> Result<(), PollingError> {
		let mut s_g = sink.lock().await;
		let request_text = serde_json::to_string(&request)?;

		s_g.send(TungsteniteMessage::Text(request_text.into())).await?;
		s_g.flush().await.map_err(|e| PollingError::FlushFailure { request_count: 1, error: e })?;

		Ok(())
	}
}

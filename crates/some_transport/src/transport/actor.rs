// ============================================================================
// TRANSPORT ACTOR - NOW GENERIC OVER TRANSPORT
// ============================================================================

pub struct TransportActor<T: Transport> {
	connection_id: ConnectionId,
	config: TransportConfig,
	state: ActorState,
	transport: Option<T>,
	command_rx: mpsc::Receiver<TransportCommand>,
	event_tx: broadcast::Sender<TransportEvent>,
	metrics: Arc<TransportMetrics>,
	keepalive_manager: KeepaliveManager,
	flow_control: FlowControlManager,
	buffer_manager: BufferManager,
}

impl<T: Transport> TransportActor<T> {
	pub fn new(
		config: TransportConfig,
		transport: T,
		command_rx: mpsc::Receiver<TransportCommand>,
		event_tx: broadcast::Sender<TransportEvent>,
		metrics: Arc<TransportMetrics>,
	) -> Self {
		let connection_id = ConnectionId::new();

		Self {
			connection_id,
			config: config.clone(),
			state: ActorState::Idle,
			transport: Some(transport),
			command_rx,
			event_tx,
			metrics,
			keepalive_manager: KeepaliveManager::new(config.keepalive.clone()),
			flow_control: FlowControlManager::new(config.flow_control.clone()),
			buffer_manager: BufferManager::new(config.buffer.clone()),
		}
	}

	/// FIXED: Proper event loop with tokio::select!
	pub async fn run(mut self) {
		info!("Transport actor started for connection {}", self.connection_id);

		let mut keepalive_interval = tokio::time::interval(self.config.keepalive.interval);
		let mut timeout_check_interval = tokio::time::interval(Duration::from_secs(1));

		loop {
			tokio::select! {
					// 1. Handle commands from the public API
					Some(command) = self.command_rx.recv() => {
							self.handle_command(command).await;
					}

					// 2. Receive messages from the transport
					Some(msg_result) = async {
							if let Some(transport) = self.transport.as_mut() {
									transport.incoming_stream().next().await
							} else {
									None
							}
					} => {
							match msg_result {
									Ok(msg) => self.handle_incoming_message(msg).await,
									Err(e) => self.handle_transport_error(e).await,
							}
					}

					// 3. Send keepalive pings when needed
					_ = keepalive_interval.tick(), if self.should_send_keepalive() => {
							self.handle_keepalive_ping().await;
					}

					// 4. Check for connection timeouts
					_ = timeout_check_interval.tick() => {
							self.check_connection_health().await;
					}

					// 5. Shutdown when command channel is closed
					else => {
							info!("Command channel closed, shutting down transport actor");
							break;
					}
			}
		}

		// Cleanup
		if let Some(mut transport) = self.transport.take() {
			let _ = transport.disconnect(CloseCode::Normal, Some("Actor shutdown".to_string())).await;
		}

		info!("Transport actor stopped for connection {}", self.connection_id);
	}

	async fn handle_command(&mut self, command: TransportCommand) {
		match command {
			TransportCommand::Connect { endpoint, config, respond_to } => {
				let result = self.handle_connect(endpoint, config).await;
				let _ = respond_to.send(result);
			}
			TransportCommand::Disconnect { close_code, reason, respond_to } => {
				let result = self.handle_disconnect(close_code, reason).await;
				let _ = respond_to.send(result);
			}
			TransportCommand::SendMessage { message, respond_to } => {
				let result = self.handle_send_message(message).await;
				let _ = respond_to.send(result);
			}
			TransportCommand::SendPing { data, respond_to } => {
				let result = self.handle_send_ping(data).await;
				let _ = respond_to.send(result);
			}
			TransportCommand::UpdateConfig { config, respond_to } => {
				let result = self.handle_update_config(config).await;
				let _ = respond_to.send(result);
			}
			TransportCommand::GetStatistics { respond_to } => {
				let stats = self.get_statistics();
				let _ = respond_to.send(stats);
			}
			TransportCommand::SetFlowControl { enabled, respond_to } => {
				let result = self.handle_set_flow_control(enabled).await;
				let _ = respond_to.send(result);
			}
		}
	}

	async fn handle_connect(&mut self, endpoint: Endpoint, config: TransportConfig) -> Result<ConnectionInfo, TransportError> {
		self.state = ActorState::Connecting {
			start_time: Instant::now(),
			tcp_stream: None,
		};

		self.config = config;

		if let Some(transport) = self.transport.as_mut() {
			match transport.connect(&endpoint).await {
				Ok(connection_info) => {
					self.state = ActorState::Connected {
						connected_at: Instant::now(),
						last_activity: Instant::now(),
						statistics: ConnectionStatistics::default(),
					};

					self.metrics.record_connection_established();

					let _ = self.event_tx.send(TransportEvent::StateChanged {
						from: TransportStateInfo::Idle,
						to: TransportStateInfo::Connected {
							endpoint: endpoint.clone(),
							connection_info: connection_info.clone(),
						},
						timestamp: Instant::now(),
					});

					Ok(connection_info)
				}
				Err(e) => {
					let transport_error = TransportError::Connection {
						source: ConnectionError::TcpConnection(Box::new(e)),
						endpoint,
						retry_after: Some(Duration::from_secs(5)),
					};

					self.state = ActorState::Failed {
						error: transport_error.clone(),
						failed_at: Instant::now(),
						retry_after: Some(Instant::now() + Duration::from_secs(5)),
					};

					Err(transport_error)
				}
			}
		} else {
			Err(TransportError::InvalidState {
				current_state: "No transport available".to_string(),
				expected_states: vec!["Transport initialized".to_string()],
			})
		}
	}

	async fn handle_disconnect(&mut self, close_code: CloseCode, reason: Option<String>) -> Result<(), TransportError> {
		self.state = ActorState::Closing {
			close_code,
			reason: reason.clone(),
			start_time: Instant::now(),
		};

		if let Some(transport) = self.transport.as_mut() {
			transport.disconnect(close_code, reason).await?;
		}

		self.state = ActorState::Idle;
		Ok(())
	}

	/// FIXED: Properly measure send time and generate unique metadata
	async fn handle_send_message(&mut self, message: OutgoingMessage) -> Result<MessageId, SendError> {
		let message_id = MessageId::new();

		// Check flow control
		let message_size = match &message.content {
			MessageContent::Text(text) => text.len(),
			MessageContent::Binary(data) => data.len(),
		};

		if !self.flow_control.can_send(message_size) {
			return Err(SendError::FlowControlBlocked);
		}

		if let Some(transport) = self.transport.as_mut() {
			// Create transport message with fresh metadata
			let metadata = create_default_metadata();
			let transport_msg = match message.content {
				MessageContent::Text(text) => TransportMessage::Text { data: text, metadata },
				MessageContent::Binary(data) => TransportMessage::Binary { data, metadata },
			};

			// FIXED: Measure actual send time
			let start_time = Instant::now();
			match transport.send_message(transport_msg).await {
				Ok(()) => {
					let send_duration = start_time.elapsed();

					// Consume flow control credits
					if let Err(e) = self.flow_control.consume_send_credits(message_size) {
						warn!("Failed to consume send credits: {:?}", e);
					}

					// Record metrics with actual timing
					self.metrics.record_message_sent(message_size, send_duration);

					// Update statistics
					if let ActorState::Connected { statistics, .. } = &mut self.state {
						statistics.messages_sent += 1;
						statistics.bytes_sent += message_size as u64;
					}

					Ok(message_id)
				}
				Err(_) => Err(SendError::ConnectionClosed),
			}
		} else {
			Err(SendError::ConnectionClosed)
		}
	}

	async fn handle_send_ping(&mut self, data: Option<Bytes>) -> Result<(), TransportError> {
		if let Some(transport) = self.transport.as_mut() {
			transport.send_ping(data).await
		} else {
			Err(TransportError::ConnectionClosed {
				code: CloseCode::Abnormal,
				reason: Some("Not connected".to_string()),
				initiated_by: CloseInitiator::Local,
			})
		}
	}

	async fn handle_update_config(&mut self, config: TransportConfig) -> Result<(), ConfigError> {
		self.config = config;
		Ok(())
	}

	/// FIXED: Implement flow control setting
	async fn handle_set_flow_control(&mut self, enabled: bool) -> Result<(), TransportError> {
		self.flow_control.set_enabled(enabled);
		Ok(())
	}

	async fn handle_incoming_message(&mut self, message: TransportMessage) {
		// Update activity timestamp
		if let ActorState::Connected { last_activity, statistics, .. } = &mut self.state {
			*last_activity = Instant::now();
			statistics.messages_received += 1;

			let message_size = match &message {
				TransportMessage::Text { data, .. } => data.len(),
				TransportMessage::Binary { data, .. } => data.len(),
				_ => 0,
			};
			statistics.bytes_received += message_size as u64;
		}

		// Handle keepalive messages
		match &message {
			TransportMessage::Ping { data } => {
				// Respond with pong
				if let Some(transport) = self.transport.as_mut() {
					let pong_msg = TransportMessage::Pong { data: data.clone() };
					if let Err(e) = transport.send_message(pong_msg).await {
						warn!("Failed to send pong response: {}", e);
					}
				}
				return; // Don't broadcast ping messages
			}
			TransportMessage::Pong { data, .. } => {
				if let Ok(rtt) = self.keepalive_manager.handle_pong(data.clone()) {
					debug!("Received pong with RTT: {:?}", rtt);

					if let ActorState::Connected { statistics, .. } = &mut self.state {
						statistics.last_ping_rtt = Some(rtt);
					}
				}
				return; // Don't broadcast pong messages
			}
			TransportMessage::Close { code, reason } => {
				info!("Received close frame: code={:?}, reason={:?}", code, reason);
				self.state = ActorState::Idle;
			}
			_ => {}
		}

		// Broadcast non-control messages
		if message.should_broadcast() {
			let _ = self.event_tx.send(TransportEvent::MessageReceived {
				message,
				timestamp: Instant::now(),
			});
		}
	}

	async fn handle_transport_error(&mut self, error: T::Error) {
		error!("Transport error: {}", error);

		// Convert to TransportError and broadcast
		let transport_error = TransportError::ProtocolViolation {
			details: error.to_string(),
			raw_data: None,
		};

		let _ = self.event_tx.send(TransportEvent::Error {
			error: transport_error,
			recoverable: true, // Could be made more sophisticated
			timestamp: Instant::now(),
		});
	}

	async fn handle_keepalive_ping(&mut self) {
		let (ping_id, data) = self.keepalive_manager.generate_ping();

		if let Some(transport) = self.transport.as_mut() {
			if let Err(e) = transport.send_ping(data).await {
				warn!("Failed to send keepalive ping {}: {}", ping_id, e);
				self.keepalive_manager.handle_ping_failure();
			}
		}
	}

	async fn check_connection_health(&mut self) {
		match self.keepalive_manager.check_health() {
			HealthStatus::Unhealthy { reason, consecutive_failures } => {
				error!("Connection unhealthy: {} (failures: {})", reason, consecutive_failures);

				// Disconnect and transition to failed state
				if let Some(transport) = self.transport.as_mut() {
					let _ = transport.disconnect(CloseCode::Abnormal, Some(reason.clone())).await;
				}

				self.state = ActorState::Failed {
					error: TransportError::ConnectionClosed {
						code: CloseCode::Abnormal,
						reason: Some(reason),
						initiated_by: CloseInitiator::Local,
					},
					failed_at: Instant::now(),
					retry_after: Some(Instant::now() + Duration::from_secs(30)),
				};
			}
			HealthStatus::Stale { idle_duration, .. } => {
				debug!("Connection stale, idle for {:?}", idle_duration);
			}
			HealthStatus::Healthy => {
				// All good
			}
		}
	}

	fn should_send_keepalive(&self) -> bool {
		matches!(self.state, ActorState::Connected { .. }) && self.keepalive_manager.should_send_ping()
	}

	fn get_statistics(&self) -> ConnectionStatistics {
		match &self.state {
			ActorState::Connected { statistics, .. } => statistics.clone(),
			_ => ConnectionStatistics::default(),
		}
	}
}

#[derive(Debug)]
enum ActorState {
	Idle,
	Connecting {
		start_time: Instant,
		tcp_stream: Option<TcpStream>,
	},
	Handshaking {
		start_time: Instant,
		handshake: Option<HandshakeInProgress>,
	},
	Connected {
		connected_at: Instant,
		last_activity: Instant,
		statistics: ConnectionStatistics,
	},
	Closing {
		close_code: CloseCode,
		reason: Option<String>,
		start_time: Instant,
	},
	Failed {
		error: TransportError,
		failed_at: Instant,
		retry_after: Option<Instant>,
	},
}

// Placeholder for handshake state
#[derive(Debug)]
struct HandshakeInProgress;

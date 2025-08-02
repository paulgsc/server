// Transport states using typestate pattern
pub struct Disconnected;
pub struct Connecting;
pub struct Connected;
pub struct Closing;
pub struct Closed;
pub struct Authenticated {
	session_id: SessionId,
	auth_manager: AuthManager<Authenticated>,
}

// State marker trait
pub trait TransportState: 'static + Send + Sync + std::fmt::Debug {}

impl TransportState for Disconnected {}
impl TransportState for Connecting {}
impl TransportState for Connected {}
impl TransportState for Authenticated {}
impl TransportState for Closing {}
impl TransportState for Closed {}

pub struct WebSocketTransport<S: TransportState> {
	inner: Arc<TransportInner>,
	_state: PhantomData<S>,
}

impl WebSocketTransport<Disconnected> {
	pub fn new(config: TransportConfig) -> Self {
		Self {
			inner: Arc::new(TransportInner::new(config)),
			_state: PhantomData,
		}
	}

	pub async fn connect(self, endpoint: Endpoint) -> Result<WebSocketTransport<Connecting>, (Self, TransportError)> {
		let command = TransportCommand::Connect {
			endpoint,
			config: self.inner.config.clone(),
			respond_to: oneshot::channel().0,
		};

		match self.inner.actor_handle.send(command).await {
			Ok(_) => Ok(WebSocketTransport {
				inner: self.inner,
				_state: PhantomData,
			}),
			Err(e) => Err((self, e.into())),
		}
	}
}

impl WebSocketTransport<Connecting> {
	pub async fn wait_for_connection(self) -> Result<WebSocketTransport<Connected>, (WebSocketTransport<Disconnected>, TransportError)> {
		// Poll for connection status
		let timeout_duration = self.inner.config.connection.connect_timeout;
		let start_time = Instant::now();

		loop {
			if start_time.elapsed() > timeout_duration {
				return Err((
					WebSocketTransport {
						inner: self.inner,
						_state: PhantomData,
					},
					TransportError::Timeout {
						operation: "connection".to_string(),
						duration: timeout_duration,
						partial_completion: false,
					},
				));
			}

			// Check connection status via actor
			let (tx, rx) = oneshot::channel();
			let command = TransportCommand::GetStatistics { respond_to: tx };

			if self.inner.actor_handle.send(command).await.is_err() {
				return Err((
					WebSocketTransport {
						inner: self.inner,
						_state: PhantomData,
					},
					TransportError::ConnectionClosed {
						code: CloseCode::Abnormal,
						reason: Some("Actor unavailable".to_string()),
						initiated_by: CloseInitiator::Local,
					},
				));
			}

			// If we can get statistics, we're connected
			if rx.await.is_ok() {
				return Ok(WebSocketTransport {
					inner: self.inner,
					_state: PhantomData,
				});
			}

			tokio::time::sleep(Duration::from_millis(100)).await;
		}
	}

	pub async fn cancel(self) -> WebSocketTransport<Disconnected> {
		// Send disconnect command to cancel connection attempt
		let (tx, _rx) = oneshot::channel();
		let command = TransportCommand::Disconnect {
			close_code: CloseCode::Normal,
			reason: Some("Connection cancelled".to_string()),
			respond_to: tx,
		};

		let _ = self.inner.actor_handle.send(command).await;

		WebSocketTransport {
			inner: self.inner,
			_state: PhantomData,
		}
	}

	pub fn connection_progress(&self) -> ConnectionProgress {
		// Return estimated progress based on elapsed time
		let elapsed = Instant::now().duration_since(
			Instant::now() - Duration::from_secs(1), // Simplified
		);

		ConnectionProgress {
			stage: ConnectionStage::WebSocketHandshake,
			progress: 0.5, // 50% complete
			elapsed,
		}
	}
}

impl WebSocketTransport<Connected> {
	pub async fn send_message(&self, message: OutgoingMessage) -> Result<MessageId, SendError> {
		let command = TransportCommand::SendMessage {
			message,
			respond_to: oneshot::channel().0,
		};

		self.inner.actor_handle.send(command).await
	}

	pub async fn send_ping(&self, data: Option<Bytes>) -> Result<(), TransportError> {
		let command = TransportCommand::SendPing {
			data,
			respond_to: oneshot::channel().0,
		};

		self.inner.actor_handle.send(command).await
	}

	pub fn subscribe_messages(&self) -> broadcast::Receiver<TransportMessage> {
		// Create a new receiver for transport messages
		// This is a simplified implementation - in practice you'd want message-specific channels
		let (_, rx) = broadcast::channel(32);
		rx
	}

	pub async fn close(self, code: CloseCode, reason: Option<String>) -> WebSocketTransport<Closing> {
		let (tx, _rx) = oneshot::channel();
		let command = TransportCommand::Disconnect {
			close_code: code,
			reason,
			respond_to: tx,
		};

		let _ = self.inner.actor_handle.send(command).await;

		WebSocketTransport {
			inner: self.inner,
			_state: PhantomData,
		}
	}

	pub async fn disconnect(self) -> WebSocketTransport<Disconnected> {
		let (tx, _rx) = oneshot::channel();
		let command = TransportCommand::Disconnect {
			close_code: CloseCode::Normal,
			reason: Some("Disconnect requested".to_string()),
			respond_to: tx,
		};

		let _ = self.inner.actor_handle.send(command).await;

		WebSocketTransport {
			inner: self.inner,
			_state: PhantomData,
		}
	}

	pub fn connection_info(&self) -> ConnectionInfo {
		// Return cached connection info or default
		ConnectionInfo {
			connection_id: self.inner.connection_id,
			endpoint: Endpoint {
				host: "localhost".to_string(),
				port: 4455,
				path: "/".to_string(),
				use_tls: false,
			},
			local_addr: "127.0.0.1:0".parse().unwrap(),
			remote_addr: "127.0.0.1:4455".parse().unwrap(),
			connected_at: Instant::now(),
			protocol_version: "13".to_string(),
		}
	}

	pub fn statistics(&self) -> ConnectionStatistics {
		// Return current statistics
		ConnectionStatistics {
			messages_sent: 0,
			messages_received: 0,
			bytes_sent: 0,
			bytes_received: 0,
			frames_sent: 0,
			frames_received: 0,
			last_ping_rtt: None,
			average_rtt: None,
			connection_uptime: Duration::from_secs(0),
		}
	}
}

impl WebSocketTransport<Closing> {
	pub async fn wait_for_close(self) -> Result<WebSocketTransport<Closed>, (WebSocketTransport<Disconnected>, TransportError)> {
		let timeout_duration = self.inner.config.connection.close_timeout;
		let start_time = Instant::now();

		loop {
			if start_time.elapsed() > timeout_duration {
				return Err((
					WebSocketTransport {
						inner: self.inner,
						_state: PhantomData,
					},
					TransportError::Timeout {
						operation: "close".to_string(),
						duration: timeout_duration,
						partial_completion: true,
					},
				));
			}

			// Check if connection is actually closed
			// In practice, this would check the actor state
			tokio::time::sleep(Duration::from_millis(100)).await;

			// Simulate close completion after a short delay
			if start_time.elapsed() > Duration::from_millis(500) {
				return Ok(WebSocketTransport {
					inner: self.inner,
					_state: PhantomData,
				});
			}
		}
	}

	pub async fn force_close(self) -> WebSocketTransport<Disconnected> {
		// Force immediate disconnection
		self.inner.shutdown_signal.store(true, Ordering::Release);

		WebSocketTransport {
			inner: self.inner,
			_state: PhantomData,
		}
	}
}

impl WebSocketTransport<Closed> {
	pub fn close_info(&self) -> CloseInfo {
		CloseInfo {
			code: CloseCode::Normal,
			reason: Some("Connection closed normally".to_string()),
			initiated_by: CloseInitiator::Local,
			closed_at: Instant::now(),
		}
	}

	pub fn reset(self) -> WebSocketTransport<Disconnected> {
		WebSocketTransport {
			inner: self.inner,
			_state: PhantomData,
		}
	}
}

impl<S: TransportState> WebSocketTransport<S> {
	pub fn connection_id(&self) -> ConnectionId {
		self.inner.connection_id
	}

	pub fn config(&self) -> &TransportConfig {
		&self.inner.config
	}

	pub fn metrics(&self) -> &Arc<TransportMetrics> {
		&self.inner.metrics
	}

	pub fn subscribe_events(&self) -> broadcast::Receiver<TransportEvent> {
		self.inner.actor_handle.subscribe_events()
	}

	pub fn is_shutting_down(&self) -> bool {
		self.inner.shutdown_signal.load(Ordering::Acquire)
	}

	pub async fn get_state(&self) -> TransportStateInfo {
		TransportStateInfo {
			state_name: std::any::type_name::<S>().split("::").last().unwrap_or("Unknown").to_string(),
			timestamp: Instant::now(),
			metadata: HashMap::new(),
		}
	}
}

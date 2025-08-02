// ============================================================================
// ABSTRACT TRANSPORT INTERFACE
// ============================================================================

/// Abstract transport interface that enables protocol-agnostic design
#[async_trait]
pub trait Transport: Send + Sync {
	type Incoming: Send;
	type Outgoing: Send;
	type Error: Send + Sync + std::error::Error;

	async fn connect(&mut self, endpoint: &Endpoint) -> Result<ConnectionInfo, Self::Error>;
	async fn send_message(&mut self, msg: Self::Outgoing) -> Result<(), Self::Error>;
	async fn send_ping(&mut self, data: Option<Bytes>) -> Result<(), Self::Error>;
	async fn disconnect(&mut self, code: CloseCode, reason: Option<String>) -> Result<(), Self::Error>;

	/// Get the incoming message stream
	fn incoming_stream(&mut self) -> Pin<Box<dyn Stream<Item = Result<Self::Incoming, Self::Error>> + Send + '_>>;

	/// Get connection information
	fn connection_info(&self) -> Option<&ConnectionInfo>;
}

// ============================================================================
// TUNGSTENITE TRANSPORT IMPLEMENTATION
// ============================================================================

pub struct TungsteniteTransport {
	stream: Option<WebSocketStream<MaybeTlsStream<TcpStream>>>,
	sender: Option<SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>>,
	receiver: Option<SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>>,
	connection_info: Option<ConnectionInfo>,
}

impl TungsteniteTransport {
	pub fn new() -> Self {
		Self {
			stream: None,
			sender: None,
			receiver: None,
			connection_info: None,
		}
	}

	fn convert_message(&self, msg: Message) -> Result<TransportMessage, TransportError> {
		let metadata = create_default_metadata();

		match msg {
			Message::Text(text) => Ok(TransportMessage::Text { data: text, metadata }),
			Message::Binary(data) => Ok(TransportMessage::Binary { data: data.into(), metadata }),
			Message::Ping(data) => Ok(TransportMessage::Ping {
				data: if data.is_empty() { None } else { Some(data.into()) },
			}),
			Message::Pong(data) => Ok(TransportMessage::Pong {
				data: if data.is_empty() { None } else { Some(data.into()) },
			}),
			Message::Close(close_frame) => Ok(TransportMessage::Close {
				code: close_frame.as_ref().map(|f| f.code),
				reason: close_frame.map(|f| f.reason.to_string()),
			}),
			Message::Frame(_) => Err(TransportError::ProtocolViolation {
				details: "Received raw frame".to_string(),
				raw_data: None,
			}),
		}
	}
}

#[async_trait]
impl Transport for TungsteniteTransport {
	type Incoming = TransportMessage;
	type Outgoing = TransportMessage;
	type Error = TransportError;

	async fn connect(&mut self, endpoint: &Endpoint) -> Result<ConnectionInfo, Self::Error> {
		let url = endpoint.to_string();
		let (ws_stream, _) = tokio_tungstenite::connect_async(&url).await.map_err(|e| TransportError::Connection {
			source: ConnectionError::TcpConnection(Box::new(e)),
			endpoint: endpoint.clone(),
			retry_after: Some(Duration::from_secs(5)),
		})?;

		let (sender, receiver) = ws_stream.split();

		// Store connection info
		let connection_info = ConnectionInfo {
			connection_id: ConnectionId::new(),
			endpoint: endpoint.clone(),
			local_addr: "127.0.0.1:0".parse().unwrap(), // Simplified for example
			remote_addr: format!("{}:{}", endpoint.host, endpoint.port).parse().unwrap(),
			connected_at: Instant::now(),
			protocol_version: "13".to_string(),
		};

		self.sender = Some(sender);
		self.receiver = Some(receiver);
		self.connection_info = Some(connection_info.clone());

		Ok(connection_info)
	}

	async fn send_message(&mut self, msg: Self::Outgoing) -> Result<(), Self::Error> {
		let sender = self.sender.as_mut().ok_or(TransportError::ConnectionClosed {
			code: CloseCode::Abnormal,
			reason: Some("Not connected".to_string()),
			initiated_by: CloseInitiator::Local,
		})?;

		let ws_message = match msg {
			TransportMessage::Text { data, .. } => Message::Text(data),
			TransportMessage::Binary { data, .. } => Message::Binary(data.to_vec()),
			TransportMessage::Ping { data } => Message::Ping(data.map(|b| b.to_vec()).unwrap_or_default()),
			TransportMessage::Pong { data } => Message::Pong(data.map(|b| b.to_vec()).unwrap_or_default()),
			TransportMessage::Close { code, reason } => {
				let close_frame = code.map(|c| CloseFrame {
					code: c,
					reason: reason.unwrap_or_default().into(),
				});
				Message::Close(close_frame)
			}
		};

		sender.send(ws_message).await.map_err(|_| TransportError::ConnectionClosed {
			code: CloseCode::Abnormal,
			reason: Some("Send failed".to_string()),
			initiated_by: CloseInitiator::Local,
		})
	}

	async fn send_ping(&mut self, data: Option<Bytes>) -> Result<(), Self::Error> {
		let ping_msg = TransportMessage::Ping { data };
		self.send_message(ping_msg).await
	}

	async fn disconnect(&mut self, code: CloseCode, reason: Option<String>) -> Result<(), Self::Error> {
		let close_msg = TransportMessage::Close { code: Some(code), reason };
		if let Err(e) = self.send_message(close_msg).await {
			warn!("Failed to send close frame: {}", e);
		}

		self.sender = None;
		self.receiver = None;
		self.connection_info = None;

		Ok(())
	}

	fn incoming_stream(&mut self) -> Pin<Box<dyn Stream<Item = Result<Self::Incoming, Self::Error>> + Send + '_>> {
		if let Some(receiver) = self.receiver.as_mut() {
			Box::pin(receiver.map(|msg_result| {
				msg_result
					.map_err(|e| TransportError::ProtocolViolation {
						details: e.to_string(),
						raw_data: None,
					})
					.and_then(|msg| self.convert_message(msg))
			}))
		} else {
			Box::pin(futures::stream::empty())
		}
	}

	fn connection_info(&self) -> Option<&ConnectionInfo> {
		self.connection_info.as_ref()
	}
}

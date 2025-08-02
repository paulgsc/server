use prometheus::{register_counter, register_gauge, register_histogram, register_int_gauge, Counter, Gauge, Histogram, IntGauge};
use std::marker::PhantomData;
use tokio::net::TcpStream;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use tracing::{debug, error, info, span, warn, Instrument, Level};
use tracing_opentelemetry::OpenTelemetrySpanExt;

struct TransportInner {
	config: TransportConfig,
	connection_id: ConnectionId,
	metrics: Arc<TransportMetrics>,
	actor_handle: ActorHandle<TransportActor>,
	shutdown_signal: Arc<AtomicBool>,
}

impl TransportInner {
	fn new(config: TransportConfig) -> Self {
		let connection_id = ConnectionId::new();
		let metrics = Arc::new(TransportMetrics::new(&connection_id.to_string()).unwrap());
		let (command_tx, command_rx) = mpsc::channel(32);
		let (event_tx, event_rx) = broadcast::channel(32);

		let actor = TransportActor {
			connection_id,
			config: config.clone(),
			state: ActorState::Idle,
			websocket: None,
			command_rx,
			event_tx: event_tx.clone(),
			message_tx: mpsc::channel(1024).0,
			message_rx: mpsc::channel(1024).1,
			metrics: metrics.clone(),
			keepalive_manager: KeepaliveManager::new(config.keepalive.clone()),
			flow_control: FlowControlManager::new(config.flow_control.clone()),
		};

		let actor_handle = ActorHandle { command_tx, event_rx };

		// Spawn the actor
		tokio::spawn(async move {
			actor.run().await;
		});

		Self {
			config,
			connection_id,
			metrics,
			actor_handle,
			shutdown_signal: Arc::new(AtomicBool::new(false)),
		}
	}
}

#[derive(Debug, Clone)]
pub struct TransportConfig {
	pub connection: ConnectionConfig,
	pub tls: TlsConfig,
	pub compression: CompressionConfig,
	pub flow_control: FlowControlConfig,
	pub timeouts: TimeoutConfig,
	pub buffer_sizes: BufferConfig,
	pub keepalive: KeepaliveConfig,
}

#[derive(Debug, Clone)]
pub struct ConnectionConfig {
	pub connect_timeout: Duration,
	pub handshake_timeout: Duration,
	pub close_timeout: Duration,
	pub max_frame_size: usize,
	pub max_message_size: usize,
	pub subprotocols: Vec<String>,
	pub custom_headers: HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct TlsConfig {
	pub enabled: bool,
	pub verify_hostname: bool,
	pub ca_certificates: Option<Vec<Certificate>>,
	pub client_certificate: Option<ClientCertificate>,
	pub cipher_suites: Vec<CipherSuite>,
	pub protocol_versions: Vec<ProtocolVersion>,
}

#[derive(Debug, Clone)]
pub struct CompressionConfig {
	pub enabled: bool,
	pub algorithm: CompressionAlgorithm,
	pub window_bits: u8,
	pub compression_level: u8,
	pub threshold: usize, // Minimum message size to compress
}

#[derive(Debug, Clone)]
pub struct FlowControlConfig {
	pub send_buffer_size: usize,
	pub receive_buffer_size: usize,
	pub backpressure_threshold: usize,
	pub max_pending_frames: usize,
	pub credit_based_flow_control: bool,
}

#[derive(Debug, Clone)]
pub struct TimeoutConfig {
	pub ping_interval: Duration,
	pub pong_timeout: Duration,
	pub idle_timeout: Duration,
	pub write_timeout: Duration,
	pub read_timeout: Duration,
}

#[derive(Debug, Clone)]
pub struct BufferConfig {
	pub send_queue_size: usize,
	pub receive_queue_size: usize,
	pub frame_buffer_size: usize,
	pub message_buffer_size: usize,
}

impl BufferConfig {
	pub fn max_send_queue_size(&self) -> usize {
		self.send_queue_size * 10 // Default multiplier
	}

	pub fn max_receive_queue_size(&self) -> usize {
		self.receive_queue_size * 10 // Default multiplier
	}

	pub fn queue_policy(&self) -> QueuePolicy {
		QueuePolicy::DropOldest // Default policy
	}
}

#[derive(Debug, Clone)]
pub struct KeepaliveConfig {
	pub enabled: bool,
	pub interval: Duration,
	pub timeout: Duration,
	pub max_failures: u32,
}

impl KeepaliveConfig {
	pub fn idle_timeout(&self) -> std::time::Duration {
		self.timeout * 4 // Default: 4x the ping timeout
	}
}

#[derive(Debug, Clone)]
pub enum TransportMessage {
	Text { data: String, metadata: MessageMetadata },
	Binary { data: Bytes, metadata: MessageMetadata },
	Ping { data: Option<Bytes> },
	Pong { data: Option<Bytes> },
	Close { code: Option<CloseCode>, reason: Option<String> },
}

impl TransportMessage {
	pub fn message_type(&self) -> &'static str {
		match self {
			Self::Text { .. } => "text",
			Self::Binary { .. } => "binary",
			Self::Ping { .. } => "ping",
			Self::Pong { .. } => "pong",
			Self::Close { .. } => "close",
		}
	}

	pub fn metadata(&self) -> &MessageMetadata {
		match self {
			Self::Text { metadata, .. } | Self::Binary { metadata, .. } => metadata,
			_ => {
				// For control messages, create a default metadata
				static DEFAULT_METADATA: std::sync::LazyLock<MessageMetadata> = std::sync::LazyLock::new(|| MessageMetadata {
					message_id: MessageId::new(),
					timestamp: std::time::Instant::now(),
					priority: MessagePriority::Normal,
					correlation_id: None,
					content_encoding: None,
					delivery_mode: DeliveryMode::BestEffort,
				});
				&DEFAULT_METADATA
			}
		}
	}

	pub fn should_broadcast(&self) -> bool {
		// Only broadcast non-control messages
		matches!(self, Self::Text { .. } | Self::Binary { .. })
	}
}

#[derive(Debug, Clone)]
pub struct MessageMetadata {
	pub message_id: MessageId,
	pub timestamp: Instant,
	pub priority: MessagePriority,
	pub correlation_id: Option<CorrelationId>,
	pub content_encoding: Option<ContentEncoding>,
	pub delivery_mode: DeliveryMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MessagePriority {
	Low = 0,
	Normal = 1,
	High = 2,
	Critical = 3,
}

pub struct TransportActor {
	connection_id: ConnectionId,
	config: TransportConfig,
	state: ActorState,
	websocket: Option<WebSocketConnection>,
	command_rx: mpsc::Receiver<TransportCommand>,
	event_tx: broadcast::Sender<TransportEvent>,
	message_tx: mpsc::Sender<TransportMessage>,
	message_rx: mpsc::Receiver<OutgoingMessage>,
	metrics: Arc<TransportMetrics>,
	keepalive_manager: KeepaliveManager,
	flow_control: FlowControlManager,
}

impl TransportActor {
	async fn run(mut self) {
		info!("Transport actor started for connection {}", self.connection_id);

		while let Some(command) = self.command_rx.recv().await {
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

		info!("Transport actor stopped for connection {}", self.connection_id);
	}

	async fn handle_connect(&mut self, endpoint: Endpoint, config: TransportConfig) -> Result<ConnectionInfo, TransportError> {
		self.state = ActorState::Connecting {
			start_time: Instant::now(),
			tcp_stream: None,
		};

		// Update config
		self.config = config;

		// Connect to the endpoint
		let url = endpoint.to_string();
		match tokio_tungstenite::connect_async(&url).await {
			Ok((ws_stream, _)) => {
				let (sender, receiver) = ws_stream.split();

				// Get socket addresses (simplified)
				let local_addr = "127.0.0.1:0".parse().unwrap();
				let remote_addr = format!("{}:{}", endpoint.host, endpoint.port).parse().unwrap();

				self.websocket = Some(WebSocketConnection {
					stream: ws_stream,
					sender,
					receiver,
					local_addr,
					remote_addr,
				});

				let connection_info = ConnectionInfo {
					connection_id: self.connection_id,
					endpoint,
					local_addr,
					remote_addr,
					connected_at: Instant::now(),
					protocol_version: "13".to_string(),
				};

				self.state = ActorState::Connected {
					connected_at: Instant::now(),
					last_activity: Instant::now(),
					statistics: ConnectionStatistics {
						messages_sent: 0,
						messages_received: 0,
						bytes_sent: 0,
						bytes_received: 0,
						frames_sent: 0,
						frames_received: 0,
						last_ping_rtt: None,
						average_rtt: None,
						connection_uptime: Duration::from_secs(0),
					},
				};

				self.metrics.record_connection_established();

				Ok(connection_info)
			}
			Err(e) => {
				let error = TransportError::Connection {
					source: ConnectionError::TcpConnection(e.into()),
					endpoint,
					retry_after: Some(Duration::from_secs(5)),
				};

				self.state = ActorState::Failed {
					error: error.clone(),
					failed_at: Instant::now(),
					retry_after: Some(Instant::now() + Duration::from_secs(5)),
				};

				Err(error)
			}
		}
	}

	async fn handle_disconnect(&mut self, close_code: CloseCode, reason: Option<String>) -> Result<(), TransportError> {
		self.state = ActorState::Closing {
			close_code,
			reason: reason.clone(),
			start_time: Instant::now(),
		};

		if let Some(ref mut ws) = self.websocket {
			// Send close frame
			let close_frame = tokio_tungstenite::tungstenite::protocol::CloseFrame {
				code: close_code,
				reason: reason.unwrap_or_default().into(),
			};

			if let Err(e) = ws.sender.send(Message::Close(Some(close_frame))).await {
				warn!("Failed to send close frame: {}", e);
			}
		}

		self.websocket = None;
		self.state = ActorState::Idle;

		Ok(())
	}

	async fn handle_send_message(&mut self, message: OutgoingMessage) -> Result<MessageId, SendError> {
		let message_id = MessageId::new();

		if let Some(ref mut ws) = self.websocket {
			let ws_message = match message.content {
				MessageContent::Text(text) => Message::Text(text),
				MessageContent::Binary(data) => Message::Binary(data.to_vec()),
			};

			match ws.sender.send(ws_message).await {
				Ok(()) => {
					self.metrics.record_message_sent(
						match &message.content {
							MessageContent::Text(t) => t.len(),
							MessageContent::Binary(b) => b.len(),
						},
						Duration::from_millis(1), // Simplified timing
					);
					Ok(message_id)
				}
				Err(_) => Err(SendError::ConnectionClosed),
			}
		} else {
			Err(SendError::ConnectionClosed)
		}
	}

	async fn handle_send_ping(&mut self, data: Option<Bytes>) -> Result<(), TransportError> {
		if let Some(ref mut ws) = self.websocket {
			let ping_data = data.map(|b| b.to_vec()).unwrap_or_default();
			match ws.sender.send(Message::Ping(ping_data)).await {
				Ok(()) => Ok(()),
				Err(_) => Err(TransportError::ConnectionClosed {
					code: CloseCode::Abnormal,
					reason: Some("Send failed".to_string()),
					initiated_by: CloseInitiator::Local,
				}),
			}
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

	fn get_statistics(&self) -> ConnectionStatistics {
		match &self.state {
			ActorState::Connected { statistics, .. } => statistics.clone(),
			_ => ConnectionStatistics {
				messages_sent: 0,
				messages_received: 0,
				bytes_sent: 0,
				bytes_received: 0,
				frames_sent: 0,
				frames_received: 0,
				last_ping_rtt: None,
				average_rtt: None,
				connection_uptime: Duration::from_secs(0),
			},
		}
	}

	async fn handle_set_flow_control(&mut self, _enabled: bool) -> Result<(), TransportError> {
		// Implementation would update flow control settings
		Ok(())
	}
}

// Add missing MessageAssembler
struct MessageAssembler {
	// Placeholder for message assembly state
}

impl MessageAssembler {
	fn new() -> Self {
		Self {}
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

struct WebSocketConnection {
	stream: WebSocketStream<MaybeTlsStream<TcpStream>>,
	sender: SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>,
	receiver: SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>,
	local_addr: SocketAddr,
	remote_addr: SocketAddr,
}

#[derive(Debug)]
pub enum TransportCommand {
	Connect {
		endpoint: Endpoint,
		config: TransportConfig,
		respond_to: oneshot::Sender<Result<ConnectionInfo, TransportError>>,
	},
	Disconnect {
		close_code: CloseCode,
		reason: Option<String>,
		respond_to: oneshot::Sender<Result<(), TransportError>>,
	},
	SendMessage {
		message: OutgoingMessage,
		respond_to: oneshot::Sender<Result<MessageId, SendError>>,
	},
	SendPing {
		data: Option<Bytes>,
		respond_to: oneshot::Sender<Result<(), TransportError>>,
	},
	UpdateConfig {
		config: TransportConfig,
		respond_to: oneshot::Sender<Result<(), ConfigError>>,
	},
	GetStatistics {
		respond_to: oneshot::Sender<ConnectionStatistics>,
	},
	SetFlowControl {
		enabled: bool,
		respond_to: oneshot::Sender<Result<(), TransportError>>,
	},
}

#[derive(Debug, Clone)]
pub enum TransportEvent {
	StateChanged {
		from: TransportStateInfo,
		to: TransportStateInfo,
		timestamp: Instant,
	},
	MessageReceived {
		message: TransportMessage,
		timestamp: Instant,
	},
	MessageSent {
		message_id: MessageId,
		size: usize,
		timestamp: Instant,
	},
	PingReceived {
		data: Option<Bytes>,
		timestamp: Instant,
	},
	PongReceived {
		data: Option<Bytes>,
		rtt: Duration,
		timestamp: Instant,
	},
	ConnectionClosed {
		code: CloseCode,
		reason: Option<String>,
		timestamp: Instant,
	},
	Error {
		error: TransportError,
		recoverable: bool,
		timestamp: Instant,
	},
	FlowControlUpdate {
		send_buffer_level: f32,
		receive_buffer_level: f32,
		timestamp: Instant,
	},
}

#[derive(Debug)]
pub struct OutgoingMessage {
	pub content: MessageContent,
	pub priority: MessagePriority,
	pub timeout: Option<Duration>,
	pub correlation_id: Option<CorrelationId>,
	pub response_channel: Option<oneshot::Sender<Result<TransportMessage, TransportError>>>,
}

#[derive(Debug)]
pub enum MessageContent {
	Text(String),
	Binary(Bytes),
}

pub struct FlowControlManager {
	config: FlowControlConfig,
	send_credits: AtomicI32,
	receive_credits: AtomicI32,
	send_buffer: VecDeque<PendingMessage>,
	receive_buffer: VecDeque<ReceivedMessage>,
	backpressure_active: AtomicBool,
	statistics: FlowControlStats,
}

impl FlowControlConfig {
	pub fn initial_send_credits(&self) -> i32 {
		1000 // Default value
	}

	pub fn initial_receive_credits(&self) -> i32 {
		1000 // Default value
	}

	pub fn bytes_per_credit(&self) -> usize {
		1024 // Default: 1KB per credit
	}
}

impl FlowControlManager {
	pub fn new(config: FlowControlConfig) -> Self {
		Self {
			config,
			send_credits: AtomicI32::new(config.initial_send_credits),
			receive_credits: AtomicI32::new(config.initial_receive_credits),
			send_buffer: VecDeque::with_capacity(config.send_buffer_size),
			receive_buffer: VecDeque::with_capacity(config.receive_buffer_size),
			backpressure_active: AtomicBool::new(false),
			statistics: FlowControlStats::default(),
		}
	}

	pub fn can_send(&self, message_size: usize) -> bool {
		let required_credits = self.calculate_credits(message_size);
		self.send_credits.load(Ordering::Acquire) >= required_credits
	}

	pub fn consume_send_credits(&self, message_size: usize) -> Result<(), FlowControlError> {
		let required_credits = self.calculate_credits(message_size);

		loop {
			let current_credits = self.send_credits.load(Ordering::Acquire);
			if current_credits < required_credits {
				return Err(FlowControlError::InsufficientCredits {
					available: current_credits,
					required: required_credits,
				});
			}

			if self
				.send_credits
				.compare_exchange_weak(current_credits, current_credits - required_credits, Ordering::AcqRel, Ordering::Acquire)
				.is_ok()
			{
				break;
			}
		}

		Ok(())
	}

	pub fn replenish_send_credits(&self, credits: i32) {
		self.send_credits.fetch_add(credits, Ordering::AcqRel);

		// Check if we can clear backpressure
		if self.backpressure_active.load(Ordering::Acquire) {
			if self.send_credits.load(Ordering::Acquire) > self.config.backpressure_threshold {
				self.backpressure_active.store(false, Ordering::Release);
			}
		}
	}

	pub fn is_backpressure_active(&self) -> bool {
		self.backpressure_active.load(Ordering::Acquire)
	}

	fn calculate_credits(&self, message_size: usize) -> i32 {
		// Credit calculation based on message size and priority
		std::cmp::max(1, (message_size / self.config.bytes_per_credit) as i32)
	}
}

#[derive(Debug)]
struct PendingMessage {
	message: OutgoingMessage,
	queued_at: Instant,
	credits_required: i32,
	response_channel: Option<oneshot::Sender<Result<(), SendError>>>,
}

#[derive(Debug)]
struct ReceivedMessage {
	message: TransportMessage,
	received_at: Instant,
	credits_consumed: i32,
}

pub struct BufferManager {
	send_queue: PriorityQueue<OutgoingMessage>,
	receive_queue: VecDeque<TransportMessage>,
	frame_buffer: BytesMut,
	message_assembler: MessageAssembler,
	statistics: BufferStatistics,
}

impl BufferManager {
	pub fn new(config: BufferConfig) -> Self {
		Self {
			send_queue: PriorityQueue::new(),
			receive_queue: VecDeque::with_capacity(config.receive_queue_size),
			frame_buffer: BytesMut::with_capacity(config.frame_buffer_size),
			message_assembler: MessageAssembler::new(),
			statistics: BufferStatistics::default(),
		}
	}

	// Add config field to BufferManager
	fn config(&self) -> BufferConfig {
		BufferConfig {
			send_queue_size: 1000,
			receive_queue_size: 1000,
			frame_buffer_size: 64 * 1024,
			message_buffer_size: 1024 * 1024,
			max_send_queue_size: 10000,
			max_receive_queue_size: 10000,
			queue_policy: QueuePolicy::DropOldest,
		}
	}
}

impl BufferManager {
	pub fn enqueue_outgoing(&mut self, message: OutgoingMessage) -> Result<(), BufferError> {
		if self.send_queue.len() >= self.config.max_send_queue_size {
			// Apply queue management policy (drop oldest, drop lowest priority, etc.)
			self.apply_queue_policy()?;
		}

		self.send_queue.push(message.priority, message);
		Ok(())
	}

	pub fn dequeue_outgoing(&mut self) -> Option<OutgoingMessage> {
		self.send_queue.pop()
	}

	pub fn enqueue_incoming(&mut self, message: TransportMessage) -> Result<(), BufferError> {
		if self.receive_queue.len() >= self.config.max_receive_queue_size {
			return Err(BufferError::ReceiveQueueFull);
		}

		self.receive_queue.push_back(message);
		Ok(())
	}

	pub fn dequeue_incoming(&mut self) -> Option<TransportMessage> {
		self.receive_queue.pop_front()
	}

	fn apply_queue_policy(&mut self) -> Result<(), BufferError> {
		match self.config.queue_policy {
			QueuePolicy::DropOldest => {
				while self.send_queue.len() >= self.config.max_send_queue_size {
					if let Some((_, dropped)) = self.send_queue.pop_oldest() {
						self.statistics.messages_dropped += 1;
						// Notify sender of drop
						if let Some(response) = dropped.response_channel {
							let _ = response.send(Err(SendError::QueueFull));
						}
					} else {
						break;
					}
				}
			}
			QueuePolicy::DropLowestPriority => {
				while self.send_queue.len() >= self.config.max_send_queue_size {
					if let Some((_, dropped)) = self.send_queue.pop_lowest_priority() {
						self.statistics.messages_dropped += 1;
						if let Some(response) = dropped.response_channel {
							let _ = response.send(Err(SendError::QueueFull));
						}
					} else {
						break;
					}
				}
			}
			QueuePolicy::RejectNew => {
				return Err(BufferError::SendQueueFull);
			}
		}

		Ok(())
	}
}

#[derive(Debug, Clone, Copy)]
pub enum QueuePolicy {
	DropOldest,
	DropLowestPriority,
	RejectNew,
}

pub struct KeepaliveManager {
	config: KeepaliveConfig,
	state: KeepaliveState,
	last_ping_sent: Option<Instant>,
	last_pong_received: Option<Instant>,
	consecutive_failures: u32,
	ping_id_counter: AtomicU64,
	pending_pings: HashMap<u64, PingInfo>,
}

#[derive(Debug, Clone, Copy)]
enum KeepaliveState {
	Idle,
	PingSent { ping_id: u64, sent_at: Instant },
	PongReceived { rtt: Duration },
	Failed { last_failure: Instant },
}

#[derive(Debug)]
struct PingInfo {
	id: u64,
	sent_at: Instant,
	data: Option<Bytes>,
}

impl KeepaliveManager {
	pub fn new(config: KeepaliveConfig) -> Self {
		Self {
			config,
			state: KeepaliveState::Idle,
			last_ping_sent: None,
			last_pong_received: None,
			consecutive_failures: 0,
			ping_id_counter: AtomicU64::new(0),
			pending_pings: HashMap::new(),
		}
	}

	pub fn should_send_ping(&self) -> bool {
		if !self.config.enabled {
			return false;
		}

		match self.last_ping_sent {
			None => true,
			Some(last_ping) => last_ping.elapsed() >= self.config.interval,
		}
	}

	pub fn generate_ping(&mut self) -> (u64, Option<Bytes>) {
		let ping_id = self.ping_id_counter.fetch_add(1, Ordering::AcqRel);
		let data = Some(ping_id.to_be_bytes().to_vec().into());

		let ping_info = PingInfo {
			id: ping_id,
			sent_at: Instant::now(),
			data: data.clone(),
		};

		self.pending_pings.insert(ping_id, ping_info);
		self.last_ping_sent = Some(Instant::now());
		self.state = KeepaliveState::PingSent { ping_id, sent_at: Instant::now() };

		(ping_id, data)
	}

	pub fn handle_pong(&mut self, data: Option<Bytes>) -> Result<Duration, KeepaliveError> {
		let ping_id = match data {
			Some(ref bytes) if bytes.len() == 8 => u64::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7]]),
			_ => return Err(KeepaliveError::InvalidPongData),
		};

		if let Some(ping_info) = self.pending_pings.remove(&ping_id) {
			let rtt = ping_info.sent_at.elapsed();
			self.last_pong_received = Some(Instant::now());
			self.consecutive_failures = 0;
			self.state = KeepaliveState::PongReceived { rtt };
			Ok(rtt)
		} else {
			Err(KeepaliveError::UnexpectedPong)
		}
	}

	pub fn check_health(&mut self) -> HealthStatus {
		if !self.config.enabled {
			return HealthStatus::Healthy;
		}

		// Check for ping timeout
		if let Some(last_ping) = self.last_ping_sent {
			if last_ping.elapsed() > self.config.timeout {
				self.consecutive_failures += 1;
				self.state = KeepaliveState::Failed { last_failure: Instant::now() };

				if self.consecutive_failures >= self.config.max_failures {
					return HealthStatus::Unhealthy {
						reason: "Keepalive failure".to_string(),
						consecutive_failures: self.consecutive_failures,
					};
				}
			}
		}

		// Check for general activity timeout
		if let Some(last_activity) = self.last_pong_received.or(self.last_ping_sent) {
			if last_activity.elapsed() > self.config.idle_timeout {
				return HealthStatus::Stale {
					last_activity,
					idle_duration: last_activity.elapsed(),
				};
			}
		}

		HealthStatus::Healthy
	}
}

#[derive(Debug, Clone)]
pub enum HealthStatus {
	Healthy,
	Stale { last_activity: Instant, idle_duration: Duration },
	Unhealthy { reason: String, consecutive_failures: u32 },
}

#[derive(Debug)]
pub struct TransportLogger {
	connection_id: ConnectionId,
	span: tracing::Span,
}

impl TransportLogger {
	pub fn new(connection_id: ConnectionId, endpoint: &Endpoint) -> Self {
		let span = span!(
				Level::INFO,
				"websocket_transport",
				connection_id = %connection_id,
				endpoint = %endpoint,
				component = "transport"
		);

		Self { connection_id, span }
	}

	pub fn log_state_transition(&self, from: TransportStateInfo, to: TransportStateInfo, duration: Option<Duration>) {
		let _enter = self.span.enter();

		match to {
			TransportStateInfo::Connected { .. } => {
				info!(
						from_state = ?from,
						to_state = ?to,
						transition_duration_ms = duration.map(|d| d.as_millis()),
						"Transport state transition: connection established"
				);
			}
			TransportStateInfo::Failed { error, .. } => {
				error!(
						from_state = ?from,
						to_state = ?to,
						error = %error,
						transition_duration_ms = duration.map(|d| d.as_millis()),
						"Transport state transition: connection failed"
				);
			}
			_ => {
				debug!(
						from_state = ?from,
						to_state = ?to,
						transition_duration_ms = duration.map(|d| d.as_millis()),
						"Transport state transition"
				);
			}
		}
	}

	pub fn log_message_flow(&self, direction: MessageDirection, message: &TransportMessage, size: usize) {
		let _enter = self.span.enter();

		match direction {
			MessageDirection::Outgoing => {
				debug!(
						direction = "outgoing",
						message_type = ?message.message_type(),
						size_bytes = size,
						message_id = ?message.metadata().message_id,
						priority = ?message.metadata().priority,
						"Message sent"
				);
			}
			MessageDirection::Incoming => {
				debug!(
						direction = "incoming",
						message_type = ?message.message_type(),
						size_bytes = size,
						message_id = ?message.metadata().message_id,
						"Message received"
				);
			}
		}
	}

	pub fn log_keepalive_event(&self, event: KeepaliveEvent) {
		let _enter = self.span.enter();

		match event {
			KeepaliveEvent::PingSent { ping_id, .. } => {
				debug!(ping_id = ping_id, "Keepalive ping sent");
			}
			KeepaliveEvent::PongReceived { ping_id, rtt, .. } => {
				debug!(ping_id = ping_id, rtt_ms = rtt.as_millis(), "Keepalive pong received");
			}
			KeepaliveEvent::Timeout { consecutive_failures, .. } => {
				warn!(consecutive_failures = consecutive_failures, "Keepalive timeout detected");
			}
		}
	}

	pub fn log_flow_control_event(&self, event: FlowControlEvent) {
		let _enter = self.span.enter();

		match event {
			FlowControlEvent::BackpressureActivated { buffer_utilization, .. } => {
				warn!(buffer_utilization = buffer_utilization, "Backpressure activated");
			}
			FlowControlEvent::BackpressureDeactivated { .. } => {
				info!("Backpressure deactivated");
			}
			FlowControlEvent::CreditsExhausted { required, available, .. } => {
				warn!(required_credits = required, available_credits = available, "Flow control credits exhausted");
			}
		}
	}

	pub fn log_error(&self, error: &TransportError, context: Option<&str>) {
		let _enter = self.span.enter();

		let error_span = span!(
				Level::ERROR,
				"transport_error",
				error_type = error.type_name(),
				retryable = error.is_retryable(),
				severity = ?error.severity()
		);

		let _error_enter = error_span.enter();

		match error.severity() {
			ErrorSeverity::Critical => {
				error!(
						error = %error,
						context = context,
						"Critical transport error"
				);
			}
			ErrorSeverity::High => {
				error!(
						error = %error,
						context = context,
						"High severity transport error"
				);
			}
			ErrorSeverity::Medium => {
				warn!(
						error = %error,
						context = context,
						"Medium severity transport error"
				);
			}
			ErrorSeverity::Low => {
				debug!(
						error = %error,
						context = context,
						"Low severity transport error"
				);
			}
		}
	}

	pub async fn with_span<F, T>(&self, future: F) -> T
	where
		F: std::future::Future<Output = T>,
	{
		future.instrument(self.span.clone()).await
	}
}

#[derive(Debug, Clone, Copy)]
pub enum MessageDirection {
	Incoming,
	Outgoing,
}

#[derive(Debug)]
pub enum KeepaliveEvent {
	PingSent { ping_id: u64, timestamp: Instant },
	PongReceived { ping_id: u64, rtt: Duration, timestamp: Instant },
	Timeout { consecutive_failures: u32, timestamp: Instant },
}

#[derive(Debug)]
pub enum FlowControlEvent {
	BackpressureActivated { buffer_utilization: f32, timestamp: Instant },
	BackpressureDeactivated { timestamp: Instant },
	CreditsExhausted { required: i32, available: i32, timestamp: Instant },
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::sync::atomic::{AtomicBool, Ordering};
	use tokio_test::{assert_pending, assert_ready};

	// Mock WebSocket for testing
	struct MockWebSocket {
		messages: Arc<Mutex<VecDeque<Message>>>,
		closed: Arc<AtomicBool>,
		should_fail: Arc<AtomicBool>,
	}

	impl MockWebSocket {
		fn new() -> Self {
			Self {
				messages: Arc::new(Mutex::new(VecDeque::new())),
				closed: Arc::new(AtomicBool::new(false)),
				should_fail: Arc::new(AtomicBool::new(false)),
			}
		}

		fn queue_message(&self, message: Message) {
			self.messages.lock().unwrap().push_back(message);
		}

		fn set_should_fail(&self, should_fail: bool) {
			self.should_fail.store(should_fail, Ordering::Release);
		}

		fn close(&self) {
			self.closed.store(true, Ordering::Release);
		}
	}

	#[tokio::test]
	async fn test_typestate_transitions() {
		let config = TransportConfig::builder().endpoint("ws://localhost:8080").build().unwrap();

		// Test valid state transitions
		let disconnected = WebSocketTransport::new(config);
		assert_eq!(disconnected.get_state().await.state_type(), "Disconnected");

		let connecting = disconnected.connect(test_endpoint()).await.unwrap();
		assert_eq!(connecting.get_state().await.state_type(), "Connecting");

		let connected = connecting.wait_for_connection().await.unwrap();
		assert_eq!(connected.get_state().await.state_type(), "Connected");

		let closing = connected.close(CloseCode::Normal, None).await;
		assert_eq!(closing.get_state().await.state_type(), "Closing");

		let closed = closing.wait_for_close().await.unwrap();
		assert_eq!(closed.get_state().await.state_type(), "Closed");

		let disconnected = closed.reset();
		assert_eq!(disconnected.get_state().await.state_type(), "Disconnected");
	}

	#[tokio::test]
	async fn test_message_sending() {
		let transport = create_connected_transport().await;

		let message = OutgoingMessage {
			content: MessageContent::Text("test message".to_string()),
			priority: MessagePriority::Normal,
			timeout: Some(Duration::from_secs(5)),
			correlation_id: None,
			response_channel: None,
		};

		let message_id = transport.send_message(message).await.unwrap();
		assert!(message_id.is_valid());
	}

	#[tokio::test]
	async fn test_flow_control() {
		let mut config = TransportConfig::default();
		config.flow_control.send_buffer_size = 10;
		config.flow_control.backpressure_threshold = 8;

		let transport = create_transport_with_config(config).await;

		// Fill up the buffer
		for i in 0..15 {
			let message = create_test_message(format!("message {}", i));
			let result = transport.send_message(message).await;

			if i < 10 {
				assert!(result.is_ok(), "Message {} should succeed", i);
			} else {
				// Should fail due to flow control
				assert!(result.is_err(), "Message {} should fail", i);
			}
		}
	}

	#[tokio::test]
	async fn test_keepalive_mechanism() {
		let mut config = TransportConfig::default();
		config.keepalive.enabled = true;
		config.keepalive.interval = Duration::from_millis(100);
		config.keepalive.timeout = Duration::from_millis(200);

		let transport = create_transport_with_config(config).await;

		// Wait for ping to be sent
		tokio::time::sleep(Duration::from_millis(150)).await;

		let stats = transport.statistics();
		assert!(stats.pings_sent > 0);
	}

	#[tokio::test]
	async fn test_error_recovery() {
		let transport = create_connected_transport().await;

		// Simulate network failure
		simulate_network_failure(&transport).await;

		// Verify error is reported
		let mut events = transport.subscribe_events();
		let event = tokio::time::timeout(Duration::from_secs(1), events.recv()).await.unwrap().unwrap();

		match event {
			TransportEvent::Error { error, recoverable, .. } => {
				assert!(matches!(error, TransportError::Connection { .. }));
				assert!(recoverable);
			}
			_ => panic!("Expected error event"),
		}
	}

	#[tokio::test]
	async fn test_concurrent_operations() {
		let transport = Arc::new(create_connected_transport().await);

		let mut handles = Vec::new();

		// Spawn multiple senders
		for i in 0..10 {
			let transport = transport.clone();
			let handle = tokio::spawn(async move {
				for j in 0..100 {
					let message = create_test_message(format!("sender {} message {}", i, j));
					transport.send_message(message).await.unwrap();
				}
			});
			handles.push(handle);
		}

		// Wait for all senders to complete
		for handle in handles {
			handle.await.unwrap();
		}

		let stats = transport.statistics();
		assert_eq!(stats.messages_sent, 1000);
	}

	// Helper functions
	async fn create_connected_transport() -> WebSocketTransport<Connected> {
		let config = TransportConfig::default();
		let transport = WebSocketTransport::new(config);
		let connecting = transport.connect(test_endpoint()).await.unwrap();
		connecting.wait_for_connection().await.unwrap()
	}

	async fn create_transport_with_config(config: TransportConfig) -> WebSocketTransport<Connected> {
		let transport = WebSocketTransport::new(config);
		let connecting = transport.connect(test_endpoint()).await.unwrap();
		connecting.wait_for_connection().await.unwrap()
	}

	fn create_test_message(content: String) -> OutgoingMessage {
		OutgoingMessage {
			content: MessageContent::Text(content),
			priority: MessagePriority::Normal,
			timeout: Some(Duration::from_secs(5)),
			correlation_id: None,
			response_channel: None,
		}
	}

	fn test_endpoint() -> Endpoint {
		Endpoint::from_str("ws://localhost:8080").unwrap()
	}

	async fn simulate_network_failure(transport: &WebSocketTransport<Connected>) {
		// Implementation to simulate network failure
		todo!()
	}
}

#[cfg(test)]
mod integration_tests {
	use super::*;
	use tokio::net::TcpListener;
	use tokio_tungstenite::accept_async;

	struct TestWebSocketServer {
		listener: TcpListener,
		shutdown_tx: broadcast::Sender<()>,
	}

	impl TestWebSocketServer {
		async fn new() -> Self {
			let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
			let (shutdown_tx, _) = broadcast::channel(1);

			Self { listener, shutdown_tx }
		}

		fn local_addr(&self) -> SocketAddr {
			self.listener.local_addr().unwrap()
		}

		async fn run(&self) {
			let mut shutdown_rx = self.shutdown_tx.subscribe();

			loop {
				tokio::select! {
						result = self.listener.accept() => {
								match result {
										Ok((stream, _)) => {
												let ws_stream = accept_async(stream).await.unwrap();
												tokio::spawn(handle_client_connection(ws_stream));
										}
										Err(e) => {
												eprintln!("Failed to accept connection: {}", e);
												break;
										}
								}
						}
						_ = shutdown_rx.recv() => {
								break;
						}
				}
			}
		}

		async fn shutdown(&self) {
			let _ = self.shutdown_tx.send(());
		}
	}

	async fn handle_client_connection(ws_stream: WebSocketStream<TcpStream>) {
		let (mut sender, mut receiver) = ws_stream.split();

		while let Some(message) = receiver.next().await {
			match message {
				Ok(msg) => {
					// Echo the message back
					if sender.send(msg).await.is_err() {
						break;
					}
				}
				Err(_) => break,
			}
		}
	}

	#[tokio::test]
	async fn test_real_websocket_connection() {
		let server = TestWebSocketServer::new().await;
		let server_addr = server.local_addr();

		// Start server
		let server_handle = tokio::spawn(server.run());

		// Give server time to start
		tokio::time::sleep(Duration::from_millis(100)).await;

		// Connect client
		let config = TransportConfig::builder().endpoint(format!("ws://{}", server_addr)).build().unwrap();

		let transport = WebSocketTransport::new(config);
		let connecting = transport.connect(Endpoint::from_str(&format!("ws://{}", server_addr)).unwrap()).await.unwrap();

		let connected = connecting.wait_for_connection().await.unwrap();

		// Send test message
		let message = OutgoingMessage {
			content: MessageContent::Text("Hello, server!".to_string()),
			priority: MessagePriority::Normal,
			timeout: Some(Duration::from_secs(5)),
			correlation_id: None,
			response_channel: None,
		};

		let message_id = connected.send_message(message).await.unwrap();
		assert!(message_id.is_valid());

		// Verify echo response
		let mut message_rx = connected.subscribe_messages();
		let echo_message = tokio::time::timeout(Duration::from_secs(1), message_rx.recv()).await.unwrap().unwrap();

		match echo_message {
			TransportMessage::Text { data, .. } => {
				assert_eq!(data, "Hello, server!");
			}
			_ => panic!("Expected text message"),
		}

		// Clean shutdown
		let closing = connected.close(CloseCode::Normal, Some("Test complete".to_string())).await;
		let _closed = closing.wait_for_close().await.unwrap();

		server.shutdown().await;
		server_handle.abort();
	}

	#[tokio::test]
	async fn test_connection_resilience() {
		// Test reconnection behavior under various failure scenarios
		todo!()
	}

	#[tokio::test]
	async fn test_load_handling() {
		// Test transport behavior under high load
		todo!()
	}
}

use proptest::prelude::*;

proptest! {
		#[test]
		fn test_message_size_handling(
				message_size in 1usize..1_000_000usize
		) {
				let rt = tokio::runtime::Runtime::new().unwrap();

				rt.block_on(async {
						let transport = create_connected_transport().await;

						let large_message = "x".repeat(message_size);
						let message = OutgoingMessage {
								content: MessageContent::Text(large_message),
								priority: MessagePriority::Normal,
								timeout: Some(Duration::from_secs(10)),
								correlation_id: None,
								response_channel: None,
						};

						let result = transport.send_message(message).await;

						if message_size <= transport.config().connection.max_message_size {
								prop_assert!(result.is_ok());
						} else {
								prop_assert!(matches!(result, Err(SendError::MessageTooLarge { .. })));
						}
				});
		}

		#[test]
		fn test_flow_control_credits(
				num_messages in 1u32..1000u32,
				message_size in 1usize..10000usize
		) {
				let rt = tokio::runtime::Runtime::new().unwrap();

				rt.block_on(async {
						let mut config = TransportConfig::default();
						config.flow_control.send_buffer_size = 100;

						let transport = create_transport_with_config(config).await;
						let mut successful_sends = 0u32;

						for _ in 0..num_messages {
								let message = create_test_message("x".repeat(message_size));
								if transport.send_message(message).await.is_ok() {
										successful_sends += 1;
								}
						}

						// Should be able to send at least some messages
						prop_assert!(successful_sends > 0);
						// But not necessarily all if flow control kicks in
						prop_assert!(successful_sends <= num_messages);
				});
		}
}

#[cfg(test)]
mod benchmarks {
	use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};
	use tokio::runtime::Runtime;

	fn benchmark_message_throughput(c: &mut Criterion) {
		let rt = Runtime::new().unwrap();
		let transport = rt.block_on(create_connected_transport());

		let mut group = c.benchmark_group("message_throughput");

		for size in [64, 256, 1024, 4096, 16384].iter() {
			group.throughput(Throughput::Bytes(*size as u64));
			group.bench_with_input(BenchmarkId::new("send_message", size), size, |b, &size| {
				b.to_async(&rt).iter(|| async {
					let message = create_test_message("x".repeat(size));
					black_box(transport.send_message(message).await.unwrap());
				});
			});
		}

		group.finish();
	}

	fn benchmark_connection_establishment(c: &mut Criterion) {
		let rt = Runtime::new().unwrap();

		c.bench_function("connection_establishment", |b| {
			b.to_async(&rt).iter(|| async {
				let config = TransportConfig::default();
				let transport = WebSocketTransport::new(config);
				let connecting = transport.connect(test_endpoint()).await.unwrap();
				let connected = connecting.wait_for_connection().await.unwrap();
				black_box(connected);
			});
		});
	}

	fn benchmark_concurrent_connections(c: &mut Criterion) {
		let rt = Runtime::new().unwrap();

		let mut group = c.benchmark_group("concurrent_connections");

		for num_connections in [10, 100, 1000].iter() {
			group.bench_with_input(BenchmarkId::new("establish_connections", num_connections), num_connections, |b, &num_connections| {
				b.to_async(&rt).iter(|| async {
					let mut handles = Vec::new();

					for _ in 0..num_connections {
						let handle = tokio::spawn(async {
							let config = TransportConfig::default();
							let transport = WebSocketTransport::new(config);
							let connecting = transport.connect(test_endpoint()).await.unwrap();
							connecting.wait_for_connection().await.unwrap()
						});
						handles.push(handle);
					}

					for handle in handles {
						black_box(handle.await.unwrap());
					}
				});
			});
		}

		group.finish();
	}

	criterion_group!(benches, benchmark_message_throughput, benchmark_connection_establishment, benchmark_concurrent_connections);
	criterion_main!(benches);
}

#[cfg(test)]
mod memory_tests {
	use super::*;
	use memory_stats::memory_stats;

	#[tokio::test]
	async fn test_memory_usage_under_load() {
		let initial_memory = memory_stats().unwrap().physical_mem;

		// Create many connections
		let mut transports = Vec::new();
		for _ in 0..1000 {
			let config = TransportConfig::default();
			let transport = WebSocketTransport::new(config);
			transports.push(transport);
		}

		let peak_memory = memory_stats().unwrap().physical_mem;
		let memory_per_transport = (peak_memory - initial_memory) / 1000;

		// Should use less than 64KB per transport (NFR-3)
		assert!(memory_per_transport < 64 * 1024);

		// Test cleanup
		drop(transports);
		tokio::time::sleep(Duration::from_secs(1)).await;

		let final_memory = memory_stats().unwrap().physical_mem;
		let memory_leaked = final_memory.saturating_sub(initial_memory);

		// Should not leak significant memory
		assert!(memory_leaked < 1024 * 1024); // < 1MB leaked
	}

	#[tokio::test]
	async fn test_message_buffer_memory_usage() {
		let mut config = TransportConfig::default();
		config.buffer_sizes.send_queue_size = 10000;
		config.buffer_sizes.receive_queue_size = 10000;

		let transport = create_transport_with_config(config).await;
		let initial_memory = memory_stats().unwrap().physical_mem;

		// Fill buffers with messages
		for i in 0..20000 {
			let message = create_test_message(format!("Message {}", i));
			let _ = transport.send_message(message).await;
		}

		let peak_memory = memory_stats().unwrap().physical_mem;
		let buffer_memory = peak_memory - initial_memory;

		// Memory usage should be reasonable for buffer size
		assert!(buffer_memory < 100 * 1024 * 1024); // < 100MB for large buffers
	}
}

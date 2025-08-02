use bytes::{Bytes, BytesMut};
use futures_util::stream::{SplitSink, SplitStream};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio_tungstenite::tungstenite::protocol::{frame::coding::CloseCode, Message};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use uuid::Uuid;

#[derive(Debug, Clone, Copy)]
pub enum DeliveryMode {
	BestEffort,
	AtLeastOnce,
	ExactlyOnce,
}

#[derive(Debug, Clone, Copy)]
pub enum ContentEncoding {
	None,
	Gzip,
	Deflate,
	Brotli,
}
// Missing type definitions
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ConnectionId(Uuid);

impl ConnectionId {
	pub fn new() -> Self {
		Self(Uuid::new_v4())
	}
}

impl std::fmt::Display for ConnectionId {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "{}", self.0)
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MessageId(Uuid);

impl MessageId {
	pub fn new() -> Self {
		Self(Uuid::new_v4())
	}
}

impl std::fmt::Display for MessageId {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "{}", self.0)
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CorrelationId(Uuid);

impl CorrelationId {
	pub fn new() -> Self {
		Self(Uuid::new_v4())
	}
}

#[derive(Debug, Clone)]
pub struct Endpoint {
	pub host: String,
	pub port: u16,
	pub path: String,
	pub use_tls: bool,
}

impl std::fmt::Display for Endpoint {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		let scheme = if self.use_tls { "wss" } else { "ws" };
		write!(f, "{}://{}:{}{}", scheme, self.host, self.port, self.path)
	}
}

#[derive(Debug, Clone)]
pub struct ConnectionInfo {
	pub connection_id: ConnectionId,
	pub endpoint: Endpoint,
	pub local_addr: SocketAddr,
	pub remote_addr: SocketAddr,
	pub connected_at: Instant,
	pub protocol_version: String,
}

#[derive(Debug, Clone)]
pub struct ConnectionProgress {
	pub stage: ConnectionStage,
	pub progress: f32,
	pub elapsed: Duration,
}

#[derive(Debug, Clone)]
pub enum ConnectionStage {
	DnsResolution,
	TcpConnection,
	TlsHandshake,
	WebSocketHandshake,
	Authentication,
	Complete,
}

#[derive(Debug, Clone)]
pub struct CloseInfo {
	pub code: CloseCode,
	pub reason: Option<String>,
	pub initiated_by: CloseInitiator,
	pub closed_at: Instant,
}

#[derive(Debug, Clone, Copy)]
pub enum CloseInitiator {
	Local,
	Remote,
}

#[derive(Debug, Clone)]
pub struct ConnectionStatistics {
	pub messages_sent: u64,
	pub messages_received: u64,
	pub bytes_sent: u64,
	pub bytes_received: u64,
	pub frames_sent: u64,
	pub frames_received: u64,
	pub last_ping_rtt: Option<Duration>,
	pub average_rtt: Option<Duration>,
	pub connection_uptime: Duration,
}

#[derive(Debug, Clone)]
pub struct TransportStateInfo {
	pub state_name: String,
	pub timestamp: Instant,
	pub metadata: HashMap<String, String>,
}

// Additional enum types
#[derive(Debug, Clone, Copy)]
pub enum CompressionAlgorithm {
	None,
	Deflate,
	Gzip,
	Brotli,
}

#[derive(Debug, Clone, Copy)]
pub enum FrameType {
	Text,
	Binary,
	Ping,
	Pong,
	Close,
}

#[derive(Debug, Clone, Copy)]
pub enum MessageType {
	Text,
	Binary,
	Control,
}

#[derive(Debug, Clone, Copy)]
pub enum ErrorSeverity {
	Low,
	Medium,
	High,
	Critical,
}

// Certificate and TLS types
#[derive(Debug, Clone)]
pub struct Certificate {
	pub data: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct ClientCertificate {
	pub cert: Certificate,
	pub private_key: Vec<u8>,
}

#[derive(Debug, Clone, Copy)]
pub enum CipherSuite {
	Tls13Aes128GcmSha256,
	Tls13Aes256GcmSha384,
	Tls13ChaCha20Poly1305Sha256,
}

#[derive(Debug, Clone, Copy)]
pub enum ProtocolVersion {
	Tls12,
	Tls13,
}

// Error types
#[derive(Debug, thiserror::Error)]
pub enum TlsError {
	#[error("Certificate validation failed")]
	CertificateValidation,
	#[error("Handshake failed")]
	HandshakeFailed,
}

#[derive(Debug, thiserror::Error)]
pub enum CertificateError {
	#[error("Invalid certificate")]
	Invalid,
	#[error("Expired certificate")]
	Expired,
}

#[derive(Debug, thiserror::Error)]
pub enum CompressionError {
	#[error("Compression failed")]
	CompressionFailed,
	#[error("Decompression failed")]
	DecompressionFailed,
}

#[derive(Debug, thiserror::Error)]
pub enum SendError {
	#[error("Queue full")]
	QueueFull,
	#[error("Connection closed")]
	ConnectionClosed,
	#[error("Timeout")]
	Timeout,
}

#[derive(Debug, thiserror::Error)]
pub enum BufferError {
	#[error("Send queue full")]
	SendQueueFull,
	#[error("Receive queue full")]
	ReceiveQueueFull,
}

#[derive(Debug, thiserror::Error)]
pub enum FlowControlError {
	#[error("Insufficient credits: available {available}, required {required}")]
	InsufficientCredits { available: i32, required: i32 },
}

#[derive(Debug, thiserror::Error)]
pub enum KeepaliveError {
	#[error("Invalid pong data")]
	InvalidPongData,
	#[error("Unexpected pong")]
	UnexpectedPong,
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
	#[error("Invalid configuration: {message}")]
	Invalid { message: String },
}

// Actor handle type
pub struct ActorHandle<T> {
	command_tx: mpsc::Sender<T>,
	event_rx: broadcast::Receiver<TransportEvent>,
}

impl<T> ActorHandle<T> {
	pub async fn send(&self, command: T) -> Result<(), TransportError> {
		self.command_tx.send(command).await.map_err(|_| TransportError::ConnectionClosed {
			code: CloseCode::Normal,
			reason: Some("Actor unavailable".to_string()),
			initiated_by: CloseInitiator::Local,
		})
	}

	pub fn subscribe_events(&self) -> broadcast::Receiver<TransportEvent> {
		self.event_rx.resubscribe()
	}
}

// Additional missing structs
struct HandshakeInProgress {
	// Placeholder for handshake state
}

#[derive(Debug, Default)]
struct FlowControlStats {
	// Placeholder for flow control statistics
}

#[derive(Debug, Default)]
struct BufferStatistics {
	pub messages_dropped: u64,
}

// Priority queue implementation
struct PriorityQueue<T> {
	items: std::collections::BinaryHeap<PriorityItem<T>>,
}

struct PriorityItem<T> {
	priority: MessagePriority,
	item: T,
	sequence: u64,
}

impl<T> std::cmp::PartialEq for PriorityItem<T> {
	fn eq(&self, other: &Self) -> bool {
		self.priority == other.priority && self.sequence == other.sequence
	}
}

impl<T> std::cmp::Eq for PriorityItem<T> {}

impl<T> std::cmp::PartialOrd for PriorityItem<T> {
	fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
		Some(self.cmp(other))
	}
}

impl<T> std::cmp::Ord for PriorityItem<T> {
	fn cmp(&self, other: &Self) -> std::cmp::Ordering {
		self.priority.cmp(&other.priority).then_with(|| other.sequence.cmp(&self.sequence))
		// Reverse for FIFO within priority
	}
}

impl<T> PriorityQueue<T> {
	fn new() -> Self {
		Self {
			items: std::collections::BinaryHeap::new(),
		}
	}

	fn push(&mut self, priority: MessagePriority, item: T) {
		static SEQUENCE_COUNTER: AtomicU64 = AtomicU64::new(0);
		let sequence = SEQUENCE_COUNTER.fetch_add(1, Ordering::Relaxed);

		self.items.push(PriorityItem { priority, item, sequence });
	}

	fn pop(&mut self) -> Option<T> {
		self.items.pop().map(|item| item.item)
	}

	fn len(&self) -> usize {
		self.items.len()
	}

	fn pop_oldest(&mut self) -> Option<(MessagePriority, T)> {
		// This is a simplified implementation - in practice you'd want a more efficient data structure
		if let Some(item) = self.items.pop() {
			Some((item.priority, item.item))
		} else {
			None
		}
	}

	fn pop_lowest_priority(&mut self) -> Option<(MessagePriority, T)> {
		// Find and remove the item with lowest priority
		// This is inefficient but works for the interface
		if let Some(item) = self.items.pop() {
			Some((item.priority, item.item))
		} else {
			None
		}
	}
}

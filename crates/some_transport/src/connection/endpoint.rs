use std::net::SocketAddr;
use tokio::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct Endpoint {
	pub host: String,
	pub port: u16,
	pub path: String,
	pub secure: bool,
}

impl std::fmt::Display for Endpoint {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		let scheme = if self.secure { "wss" } else { "ws" };
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

#[derive(Debug, Clone, Default)]
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
pub struct ConnectionId(String);

impl ConnectionId {
	pub fn new() -> Self {
		Self(uuid::Uuid::new_v4().to_string())
	}
}

impl std::fmt::Display for ConnectionId {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "{}", self.0)
	}
}

use std::{fmt, sync::Arc, time::Instant};
use uuid::Uuid;

/// Connection ID type for type safety
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ConnectionId(Uuid);

impl ConnectionId {
	pub fn new() -> Self {
		Self(Uuid::new_v4()) // or new_v7() if you want time-ordered
	}

	pub fn from_uuid(uuid: Uuid) -> Self {
		Self(uuid)
	}

	pub fn as_uuid(&self) -> &Uuid {
		&self.0
	}

	pub fn as_string(&self) -> String {
		self.0.to_string()
	}

	pub fn as_bytes(&self) -> &[u8; 16] {
		self.0.as_bytes()
	}
}

impl fmt::Display for ConnectionId {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{}", self.0)
	}
}

/// Client identifier derived from headers and socket info
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ClientId(Arc<str>);

impl ClientId {
	pub fn new(id: impl Into<Arc<str>>) -> Self {
		Self(id.into())
	}

	pub fn as_str(&self) -> &str {
		&self.0
	}
}

impl fmt::Display for ClientId {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{}", self.0)
	}
}

/// State of a connection lifecycle
#[derive(Debug, Clone)]
pub enum ConnectionState {
	Active { last_ping: Instant },
	Stale { last_ping: Instant, reason: String },
	Disconnected { reason: String, disconnected_at: Instant },
}

impl fmt::Display for ConnectionState {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			ConnectionState::Active { .. } => write!(f, "Active"),
			ConnectionState::Stale { reason, .. } => write!(f, "Stale({})", reason),
			ConnectionState::Disconnected { reason, .. } => {
				write!(f, "Disconnected({})", reason)
			}
		}
	}
}

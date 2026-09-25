use crate::types::{ClientId, ConnectionId};
use std::time::{Duration, Instant};

/// Pure connection metadata - immutable after creation
///
/// Deliberately carries no peer address. It used to hold the socket's
/// `SocketAddr`, which nothing read — but a `Debug` of any connection then
/// printed a client IP. `ClientId` is how connections are grouped; the address
/// stays with the caller's network layer (`file_host`'s `net` module, #372).
#[derive(Clone, Debug)]
pub struct Connection {
	pub id: ConnectionId,
	pub client_id: ClientId,
	pub established_at: Instant,
}

impl Connection {
	/// Create a new connection
	#[must_use]
	pub fn new(client_id: ClientId) -> Self {
		Self {
			id: ConnectionId::new(),
			client_id,
			established_at: Instant::now(),
		}
	}

	/// Get connection duration
	#[must_use]
	pub fn get_duration(&self) -> Duration {
		self.established_at.elapsed()
	}
}

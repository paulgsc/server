use crate::types::{ClientId, ConnectionId};
use std::{
	net::SocketAddr,
	time::{Duration, Instant},
};

/// Pure connection metadata - immutable after creation
#[derive(Clone, Debug)]
pub struct Connection {
	pub id: ConnectionId,
	pub client_id: ClientId,
	pub established_at: Instant,
	pub source_addr: SocketAddr,
}

impl Connection {
	/// Create a new connection
	pub fn new(client_id: ClientId, source_addr: SocketAddr) -> Self {
		Self {
			id: ConnectionId::new(),
			client_id,
			established_at: Instant::now(),
			source_addr,
		}
	}

	/// Get connection duration
	pub fn get_duration(&self) -> Duration {
		self.established_at.elapsed()
	}
}

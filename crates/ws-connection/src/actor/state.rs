use std::time::{Duration, Instant};

/// Actor-owned connection state (not shared)
#[derive(Debug, Clone)]
pub struct ConnectionState {
	pub is_active: bool,
	pub is_stale: bool,
	pub last_activity: Instant,
	pub stale_reason: Option<String>,
	pub disconnect_reason: Option<String>,
}

impl ConnectionState {
	/// Create a new active connection state.
	#[must_use]
	pub fn new() -> Self {
		let now = Instant::now();
		Self {
			is_active: true,
			is_stale: false,
			last_activity: now,
			stale_reason: None,
			disconnect_reason: None,
		}
	}

	/// Record a heartbeat or activity event.
	pub fn record_activity(&mut self) {
		self.last_activity = Instant::now();
	}

	/// Determine if the connection should be marked as stale.
	#[must_use]
	pub fn should_be_stale(&self, timeout: Duration) -> bool {
		self.is_active && Instant::now().duration_since(self.last_activity) > timeout
	}

	/// Mark the connection as stale.
	pub fn mark_stale(&mut self, reason: String) {
		if self.is_active {
			self.is_active = false;
			self.is_stale = true;
			self.stale_reason = Some(reason);
		}
	}

	/// Disconnect the connection for the provided reason.
	pub fn disconnect(&mut self, reason: String) {
		self.is_active = false;
		self.is_stale = false;
		self.disconnect_reason = Some(reason);
	}
}

impl Default for ConnectionState {
	fn default() -> Self {
		Self::new()
	}
}

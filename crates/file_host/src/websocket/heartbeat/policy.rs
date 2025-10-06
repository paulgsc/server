use std::time::Duration;

#[derive(Clone, Debug)]
pub struct HeartbeatPolicy {
	/// If no ping seen for this duration => mark stale
	pub stale_after: Duration,
	/// How long after stale => remove/disconnect
	pub remove_after_stale: Duration,
	/// Scanning interval
	pub scan_interval: Duration,
}

impl Default for HeartbeatPolicy {
	fn default() -> Self {
		Self {
			stale_after: Duration::from_secs(30),
			remove_after_stale: Duration::from_secs(60),
			scan_interval: Duration::from_secs(10),
		}
	}
}

impl HeartbeatPolicy {
	pub fn with_stale_timeout(mut self, timeout: Duration) -> Self {
		self.stale_after = timeout;
		self
	}

	pub fn with_remove_timeout(mut self, timeout: Duration) -> Self {
		self.remove_after_stale = timeout;
		self
	}

	pub fn with_scan_interval(mut self, interval: Duration) -> Self {
		self.scan_interval = interval;
		self
	}
}

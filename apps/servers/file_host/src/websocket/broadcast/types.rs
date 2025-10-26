use tokio::time::Duration;

/// Broadcast result tracking
#[derive(Debug)]
pub struct BroadcastResult {
	pub delivered: usize,
	pub failed: usize,
	pub duration: Duration,
}

impl BroadcastResult {
	pub fn success(count: usize, duration: Duration) -> Self {
		Self {
			delivered: count,
			failed: 0,
			duration,
		}
	}
}

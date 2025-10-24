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

/// Error types for receiving from multiplexed channels
enum ForwardError {
	Lagged(EventType, u64),
	Closed(EventType),
	NoReceivers,
}

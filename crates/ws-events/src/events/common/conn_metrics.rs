// Connection metrics for monitoring
#[derive(Debug, Default)]
pub struct ConnectionMetrics {
	pub total_created: AtomicU64,
	pub total_removed: AtomicU64,
	pub current_active: AtomicU64,
	pub current_stale: AtomicU64,
	pub messages_processed: AtomicU64,
	pub messages_failed: AtomicU64,
	pub broadcast_succeeded: AtomicU64,
	pub broadcast_failed: AtomicU64,
}

impl ConnectionMetrics {
	pub fn connection_created(&self) {
		self.total_created.fetch_add(1, Ordering::Relaxed);
		self.current_active.fetch_add(1, Ordering::Relaxed);
	}

	pub fn connection_removed(&self, was_active: bool) {
		self.total_removed.fetch_add(1, Ordering::Relaxed);
		if was_active {
			self.current_active.fetch_sub(1, Ordering::Relaxed);
		} else {
			self.current_stale.fetch_sub(1, Ordering::Relaxed);
		}
	}

	pub fn connection_marked_stale(&self) {
		self.current_active.fetch_sub(1, Ordering::Relaxed);
		self.current_stale.fetch_add(1, Ordering::Relaxed);
	}

	pub fn message_processed(&self, success: bool) {
		if success {
			self.messages_processed.fetch_add(1, Ordering::Relaxed);
		} else {
			self.messages_failed.fetch_add(1, Ordering::Relaxed);
		}
	}

	pub fn broadcast_attempt(&self, success: bool) {
		if success {
			self.broadcast_succeeded.fetch_add(1, Ordering::Relaxed);
		} else {
			self.broadcast_failed.fetch_add(1, Ordering::Relaxed);
		}
	}

	pub fn get_snapshot(&self) -> ConnectionMetricsSnapshot {
		ConnectionMetricsSnapshot {
			total_created: self.total_created.load(Ordering::Relaxed),
			total_removed: self.total_removed.load(Ordering::Relaxed),
			current_active: self.current_active.load(Ordering::Relaxed),
			current_stale: self.current_stale.load(Ordering::Relaxed),
			messages_processed: self.messages_processed.load(Ordering::Relaxed),
			messages_failed: self.messages_failed.load(Ordering::Relaxed),
			broadcast_succeeded: self.broadcast_succeeded.load(Ordering::Relaxed),
			broadcast_failed: self.broadcast_failed.load(Ordering::Relaxed),
		}
	}
}

#[derive(Debug, Clone, Serialize)]
pub struct ConnectionMetricsSnapshot {
	pub total_created: u64,
	pub total_removed: u64,
	pub current_active: u64,
	pub current_stale: u64,
	pub messages_processed: u64,
	pub messages_failed: u64,
	pub broadcast_succeeded: u64,
	pub broadcast_failed: u64,
}

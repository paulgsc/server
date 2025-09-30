#[derive(Default)]
pub struct NotifierMetrics {
	sent: std::sync::atomic::AtomicU64,
	dropped: std::sync::atomic::AtomicU64,
	errors: std::sync::atomic::AtomicU64,
	no_receivers: std::sync::atomic::AtomicU64,
}

impl NotifierObserver for NotifierMetrics {
	fn on_event(&self, event: &NotifierEvent) {
		if let NotifierEvent::Outcome(outcome) = event {
			match outcome {
				SendOutcome::Sent => self.sent.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
				SendOutcome::DroppedOldest => self.dropped.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
				SendOutcome::Error(_) => self.errors.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
				SendOutcome::NoReceivers => self.no_receivers.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
			};
		}
	}
}

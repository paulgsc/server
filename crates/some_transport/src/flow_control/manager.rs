pub struct FlowControlManager {
	config: FlowControlConfig,
	enabled: bool,
	send_credits: i32,
	receive_credits: i32,
	send_buffer: VecDeque<PendingMessage>,
	receive_buffer: VecDeque<ReceivedMessage>,
	backpressure_active: bool,
	statistics: FlowControlStats,
}

impl FlowControlManager {
	pub fn new(config: FlowControlConfig) -> Self {
		Self {
			enabled: true,
			send_credits: config.initial_send_credits(),
			receive_credits: config.initial_receive_credits(),
			send_buffer: VecDeque::with_capacity(config.send_buffer_size),
			receive_buffer: VecDeque::with_capacity(config.receive_buffer_size),
			backpressure_active: false,
			statistics: FlowControlStats::default(),
			config,
		}
	}

	pub fn set_enabled(&mut self, enabled: bool) {
		self.enabled = enabled;
		if !enabled {
			// Reset credits when disabled
			self.send_credits = self.config.initial_send_credits();
			self.backpressure_active = false;
		}
	}

	pub fn can_send(&self, message_size: usize) -> bool {
		if !self.enabled {
			return true;
		}

		let required_credits = self.calculate_credits(message_size);
		self.send_credits >= required_credits
	}

	pub fn consume_send_credits(&mut self, message_size: usize) -> Result<(), FlowControlError> {
		if !self.enabled {
			return Ok(());
		}

		let required_credits = self.calculate_credits(message_size);

		if self.send_credits < required_credits {
			return Err(FlowControlError::InsufficientCredits {
				available: self.send_credits,
				required: required_credits,
			});
		}

		self.send_credits -= required_credits;

		// Check for backpressure
		if self.send_credits < self.config.backpressure_threshold && !self.backpressure_active {
			self.backpressure_active = true;
			self.statistics.backpressure_events += 1;
		}

		Ok(())
	}

	pub fn replenish_send_credits(&mut self, credits: i32) {
		self.send_credits += credits;

		// Check if we can clear backpressure
		if self.backpressure_active && self.send_credits > self.config.backpressure_threshold {
			self.backpressure_active = false;
		}
	}

	pub fn is_backpressure_active(&self) -> bool {
		self.enabled && self.backpressure_active
	}

	fn calculate_credits(&self, message_size: usize) -> i32 {
		std::cmp::max(1, (message_size / self.config.bytes_per_credit()) as i32)
	}
}

#[derive(Debug, Default)]
pub struct FlowControlStats {
	pub backpressure_events: u64,
	pub credits_exhausted_events: u64,
	pub messages_throttled: u64,
}

#[derive(Debug)]
struct PendingMessage {
	message: OutgoingMessage,
	queued_at: Instant,
	credits_required: i32,
	response_channel: Option<oneshot::Sender<Result<(), SendError>>>,
}

#[derive(Debug)]
struct ReceivedMessage {
	message: TransportMessage,
	received_at: Instant,
	credits_consumed: i32,
}

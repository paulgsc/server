// ============================================================================
// BUFFER MANAGER IMPLEMENTATION
// ============================================================================

pub struct BufferManager {
	send_queue: PriorityQueue<OutgoingMessage>,
	receive_queue: VecDeque<TransportMessage>,
	frame_buffer: bytes::BytesMut,
	message_assembler: MessageAssembler,
	statistics: BufferStatistics,
	config: BufferConfig,
}

impl BufferManager {
	pub fn new(config: BufferConfig) -> Self {
		Self {
			send_queue: PriorityQueue::new(),
			receive_queue: VecDeque::with_capacity(config.receive_queue_size),
			frame_buffer: bytes::BytesMut::with_capacity(config.frame_buffer_size),
			message_assembler: MessageAssembler::new(),
			statistics: BufferStatistics::default(),
			config,
		}
	}

	pub fn enqueue_outgoing(&mut self, message: OutgoingMessage) -> Result<(), BufferError> {
		if self.send_queue.len() >= self.config.max_send_queue_size {
			self.apply_queue_policy()?;
		}

		self.send_queue.push(message.priority, message);
		Ok(())
	}

	pub fn dequeue_outgoing(&mut self) -> Option<OutgoingMessage> {
		self.send_queue.pop()
	}

	pub fn enqueue_incoming(&mut self, message: TransportMessage) -> Result<(), BufferError> {
		if self.receive_queue.len() >= self.config.max_receive_queue_size {
			return Err(BufferError::ReceiveQueueFull);
		}

		self.receive_queue.push_back(message);
		Ok(())
	}

	pub fn dequeue_incoming(&mut self) -> Option<TransportMessage> {
		self.receive_queue.pop_front()
	}

	fn apply_queue_policy(&mut self) -> Result<(), BufferError> {
		match self.config.queue_policy {
			QueuePolicy::DropOldest => {
				while self.send_queue.len() >= self.config.max_send_queue_size {
					if let Some((_, dropped)) = self.send_queue.pop_oldest() {
						self.statistics.messages_dropped += 1;
						if let Some(response) = dropped.response_channel {
							let _ = response.send(Err(TransportError::ConnectionClosed {
								code: CloseCode::Abnormal,
								reason: Some("Message dropped - queue full".to_string()),
								initiated_by: CloseInitiator::Local,
							}));
						}
					} else {
						break;
					}
				}
			}
			QueuePolicy::DropLowestPriority => {
				while self.send_queue.len() >= self.config.max_send_queue_size {
					if let Some((_, dropped)) = self.send_queue.pop_lowest_priority() {
						self.statistics.messages_dropped += 1;
						if let Some(response) = dropped.response_channel {
							let _ = response.send(Err(TransportError::ConnectionClosed {
								code: CloseCode::Abnormal,
								reason: Some("Message dropped - low priority".to_string()),
								initiated_by: CloseInitiator::Local,
							}));
						}
					} else {
						break;
					}
				}
			}
			QueuePolicy::RejectNew => {
				return Err(BufferError::SendQueueFull);
			}
		}

		Ok(())
	}
}

// Simple priority queue implementation
#[derive(Debug)]
struct PriorityQueue<T> {
	items: VecDeque<(MessagePriority, T)>,
}
impl<T> PriorityQueue<T> {
	fn new() -> Self {
		Self { items: VecDeque::new() }
	}

	fn push(&mut self, priority: MessagePriority, item: T) {
		// Insert in priority order (highest first)
		let mut inserted = false;
		for i in 0..self.items.len() {
			if self.items[i].0 < priority {
				self.items.insert(i, (priority, item));
				inserted = true;
				break;
			}
		}
		if !inserted {
			self.items.push_back((priority, item));
		}
	}

	fn pop(&mut self) -> Option<T> {
		self.items.pop_front().map(|(_, item)| item)
	}

	fn pop_oldest(&mut self) -> Option<(MessagePriority, T)> {
		self.items.pop_back()
	}

	fn pop_lowest_priority(&mut self) -> Option<(MessagePriority, T)> {
		if let Some(min_idx) = self.items.iter().enumerate().min_by_key(|(_, (priority, _))| *priority).map(|(idx, _)| idx) {
			self.items.remove(min_idx)
		} else {
			None
		}
	}

	fn len(&self) -> usize {
		self.items.len()
	}
}

#[derive(Debug, Default)]
pub struct BufferStatistics {
	pub messages_dropped: u64,
	pub queue_full_events: u64,
	pub max_queue_depth: usize,
	pub total_bytes_buffered: u64,
}

#[derive(Debug)]
struct MessageAssembler {
	// Placeholder for message assembly logic
}

impl MessageAssembler {
	fn new() -> Self {
		Self {}
	}
}

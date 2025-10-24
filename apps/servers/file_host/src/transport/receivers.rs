use super::Event;
use dashmap::DashMap;
use some_transport::NatsTransportReceiver;
use std::sync::Arc;

/// Per-connection receiver set - one receiver per subscribed event type
#[derive(Clone)]
pub struct ConnectionReceivers {
	receivers: Arc<DashMap<String, NatsTransportReceiver<Event>>>,
}

impl ConnectionReceivers {
	pub fn new() -> Self {
		Self {
			receivers: Arc::new(DashMap::new()),
		}
	}

	pub fn insert(&self, subject: &str, receiver: NatsTransportReceiver<Event>) {
		self.receivers.insert(subject.to_owned(), receiver);
	}

	pub fn remove(&self, subject: &str) {
		self.receivers.remove(subject.to_owned());
	}

	pub fn get(&self, subject: &str) -> Option<NatsTransportReceiver<Event>> {
		self.receivers.get(subject.to_owned()).map(|r| r.clone())
	}

	pub fn len(&self) -> usize {
		self.receivers.len()
	}

	pub fn is_empty(&self) -> bool {
		self.receivers.is_empty()
	}

	pub fn event_types(&self) -> Vec<EventType> {
		let subjects = self.receivers.iter().map(|r| r.key().clone()).collect();
		// TODO: complete this!
	}
}

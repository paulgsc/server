/// Event type discriminator - defines NATS subject namespaces
#[derive(Debug, Clone, PartialEq, Hash)]
pub enum EventType {
	System,
	Audio,
	Chat,
	Obs,
	ClientCount,
	Ping,
	Pong,
	Error,
}

impl EventType {
	/// Get the NATS subject prefix for this event type
	pub fn subject(&self) -> &'static str {
		match self {
			Self::System => "events.system",
			Self::Audio => "events.audio",
			Self::Chat => "events.chat",
			Self::Obs => "events.obs",
			Self::ClientCount => "events.client_count",
			Self::Ping => "events.ping",
			Self::Pong => "events.pong",
			Self::Error => "events.error",
		}
	}

	/// Get the per-connection subject for this event type
	pub fn connection_subject(&self, conn_id: &str) -> String {
		format!("{}.{}", self.subject, conn_id)
	}
}

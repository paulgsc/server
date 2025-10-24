use prost::Message;
use serde::{Deserialize, Serialize};

/// System lifecycle events (connection/disconnection, health)
#[derive(Debug, Clone, Message, Serialize, Deserialize)]
pub struct SystemEvent {
	#[prost(string, tag = "1")]
	pub event_type: String,
	#[prost(bytes, tag = "2")]
	pub payload: Vec<u8>,
}

/// Audio playback events (now playing, track changes)
#[derive(Debug, Clone, Message, Serialize, Deserialize)]
pub struct AudioEvent {
	#[prost(string, tag = "1")]
	pub track_id: String,
	#[prost(string, tag = "2")]
	pub event_type: String,
	#[prost(bytes, tag = "3")]
	pub data: Vec<u8>,
}

/// Chat/utterance events
#[derive(Debug, Clone, Message, Serialize, Deserialize)]
pub struct ChatEvent {
	#[prost(string, tag = "1")]
	pub message_id: String,
	#[prost(string, tag = "2")]
	pub content: String,
	#[prost(int64, tag = "3")]
	pub timestamp: i64,
}

/// OBS control events
#[derive(Debug, Clone, Message, Serialize, Deserialize)]
pub struct ObsEvent {
	#[prost(bytes, tag = "1")]
	pub event_data: Vec<u8>,
	#[prost(int64, tag = "2")]
	pub timestamp: i64,
}

/// Client count updates
#[derive(Debug, Clone, Message, Serialize, Deserialize)]
pub struct ClientCountEvent {
	#[prost(uint64, tag = "1")]
	pub count: u64,
	#[prost(int64, tag = "2")]
	pub timestamp: i64,
}

#[derive(Clone)]
pub struct EventTransports {
	/// System events (connections, health, errors)
	pub system: Arc<NatsTransport<SystemEvent>>,

	/// Audio playback events
	pub audio: Arc<NatsTransport<AudioEvent>>,

	/// Chat/utterance events
	pub chat: Arc<NatsTransport<ChatEvent>>,

	/// OBS control events
	pub obs: Arc<NatsTransport<ObsEvent>>,

	/// Client count broadcasts
	pub client_count: Arc<NatsTransport<ClientCountEvent>>,
}

impl EventTransports {
	/// Initialize all event transports using the global connection pool
	pub async fn new(nats_url: impl Into<String>) -> anyhow::Result<Self> {
		let url = nats_url.into();

		Ok(Self {
			system: Arc::new(NatsTransport::connect_pooled(&url).await?),
			audio: Arc::new(NatsTransport::connect_pooled(&url).await?),
			chat: Arc::new(NatsTransport::connect_pooled(&url).await?),
			obs: Arc::new(NatsTransport::connect_pooled(&url).await?),
			client_count: Arc::new(NatsTransport::connect_pooled(&url).await?),
		})
	}
}

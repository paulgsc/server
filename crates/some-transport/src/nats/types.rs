pub type Result<T> = std::result::Result<T, NatsError>;

/// Event key trait for NATS subjects
pub trait EventKey: Clone + Debug + Serialize + for<'de> Deserialize<'de> + Send + Sync + 'static {
	/// Convert to NATS subject
	fn to_subject(&self) -> String;

	/// Parse from NATS subject
	fn from_subject(subject: &str) -> Option<Self>;
}

// Default implementation for String
impl EventKey for String {
	fn to_subject(&self) -> String {
		self.replace(' ', "_").replace('>', "_").replace('*', "_")
	}

	fn from_subject(subject: &str) -> Option<Self> {
		Some(subject.to_string())
	}
}

/// NATS transport configuration
#[derive(Debug, Clone)]
pub struct NatsConfig {
	pub servers: Vec<String>,
	pub token: Option<String>,
	pub username: Option<String>,
	pub password: Option<String>,
	pub subject_prefix: String,
	pub name: Option<String>,
}

impl Default for NatsConfig {
	fn default() -> Self {
		Self {
			servers: vec!["nats://localhost:4222".to_string()],
			token: None,
			username: None,
			password: None,
			subject_prefix: "events".to_string(),
			name: None,
		}
	}
}

/// Event wrapper for NATS transport
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NatsEvent<K: EventKey> {
	pub event_type: K,
	pub payload: serde_json::Value,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub source: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub correlation_id: Option<String>,
}

impl<K: EventKey> NatsEvent<K> {
	pub fn new(event_type: K, payload: serde_json::Value) -> Self {
		Self {
			event_type,
			payload,
			source: None,
			correlation_id: None,
		}
	}

	pub fn with_source(mut self, source: String) -> Self {
		self.source = Some(source);
		self
	}

	pub fn with_correlation_id(mut self, id: String) -> Self {
		self.correlation_id = Some(id);
		self
	}
}

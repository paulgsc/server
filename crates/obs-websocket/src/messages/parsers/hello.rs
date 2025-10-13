use crate::messages::{HelloData, JsonExtractor, ObsEvent, ObsMessagesError};
use serde_json::Value;
use tracing::{instrument, warn};

type Result<T> = std::result::Result<T, ObsMessagesError>;

/// Handles parsing of Hello messages
pub(crate) struct HelloMessageParser;

impl HelloMessageParser {
	#[instrument(skip(json))]
	pub fn parse(json: &Value) -> Result<ObsEvent> {
		let extractor = JsonExtractor::new(json, "Hello message");
		let d = extractor.get_object("d")?;

		let _d_extractor = JsonExtractor::new(&Value::Object(d.clone()), "Hello message data");

		// Extract OBS version with fallback
		let obs_version = d.get("obsWebSocketVersion").and_then(Value::as_str).map(String::from).unwrap_or_else(|| {
			warn!("obsWebSocketVersion not found or invalid, using 'unknown' as fallback");
			"unknown".to_string()
		});

		Ok(ObsEvent::Hello(HelloData { obs_version }))
	}
}

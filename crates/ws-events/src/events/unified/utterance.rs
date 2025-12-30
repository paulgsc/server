use crate::events::UtteranceMetadata;
use prost::Message;

#[derive(Clone, PartialEq, Message)]
pub struct UtteranceMessage {
	#[prost(string, tag = "1")]
	pub text: String,
	#[prost(bytes, tag = "2")]
	pub metadata: Vec<u8>, // Serialized UtteranceMetadata
}

impl UtteranceMessage {
	/// Create from text and metadata
	pub fn new(text: String, metadata: UtteranceMetadata) -> Result<Self, serde_json::Error> {
		let metadata_bytes = serde_json::to_vec(&metadata)?;
		Ok(Self { text, metadata: metadata_bytes })
	}

	/// Get the metadata
	pub fn get_metadata(&self) -> Result<UtteranceMetadata, serde_json::Error> {
		serde_json::from_slice(&self.metadata)
	}
}

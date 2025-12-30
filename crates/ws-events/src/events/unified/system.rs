use prost::Message;

#[derive(Clone, PartialEq, Message)]
pub struct ClientCountMessage {
	#[prost(uint64, tag = "1")]
	pub count: u64,
}

#[derive(Clone, PartialEq, Message)]
pub struct ErrorMessage {
	#[prost(string, tag = "1")]
	pub message: String,
}

#[derive(Clone, PartialEq, Message)]
pub struct SystemEventMessage {
	#[prost(string, tag = "1")]
	pub event_type: String,
	#[prost(bytes, tag = "2")]
	pub payload: Vec<u8>,
	#[prost(int64, tag = "3")]
	pub timestamp: i64,
}

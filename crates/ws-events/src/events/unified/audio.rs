use prost::Message;

#[derive(Clone, PartialEq, Message)]
pub struct AudioChunkMessage {
	#[prost(uint32, tag = "1")]
	pub sample_rate: u32,

	#[prost(uint32, tag = "2")]
	pub channels: u32,

	#[prost(float, repeated, tag = "3")]
	pub samples: Vec<f32>,
}

impl AudioChunkMessage {
	/// Create from raw audio data
	pub fn new(sample_rate: u32, channels: u32, samples: Vec<f32>) -> Self {
		Self { sample_rate, channels, samples }
	}

	/// Get total sample count
	pub fn sample_count(&self) -> usize {
		self.samples.len()
	}

	/// Get duration in milliseconds
	pub fn duration_ms(&self) -> f64 {
		(self.samples.len() as f64 / self.sample_rate as f64) * 1000.0
	}
}

#[derive(Clone, PartialEq, Message)]
pub struct SubtitleMessage {
	#[prost(string, tag = "1")]
	pub text: String,

	#[prost(uint64, tag = "2")]
	pub timestamp: u64,

	#[prost(float, optional, tag = "3")]
	pub confidence: Option<f32>,
}

impl SubtitleMessage {
	/// Create from text and timestamp
	pub fn new(text: String, timestamp: u64) -> Self {
		Self {
			text,
			timestamp,
			confidence: None,
		}
	}

	/// Create with confidence score
	pub fn with_confidence(text: String, timestamp: u64, confidence: f32) -> Self {
		Self {
			text,
			timestamp,
			confidence: Some(confidence),
		}
	}
}

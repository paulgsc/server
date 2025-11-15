use prost::Message;

#[derive(Clone, PartialEq, Message)]
pub struct AudioChunkMessage {
	/// Raw audio samples as bytes (float32 little-endian encoded)
	#[prost(bytes = "vec", tag = "1")]
	pub samples: Vec<u8>,

	/// Sample rate (e.g., 48000 Hz) - sent in first chunk and every 100 chunks
	#[prost(uint32, optional, tag = "2")]
	pub sample_rate: Option<u32>,

	/// Number of channels (e.g., 2 for stereo) - sent in first chunk and every 100 chunks
	#[prost(uint32, optional, tag = "3")]
	pub channels: Option<u32>,

	/// Chunk sequence number for drop detection
	#[prost(uint64, optional, tag = "4")]
	pub sequence: Option<u64>,

	/// Timestamp in milliseconds - sent in first chunk and every 100 chunks
	#[prost(uint64, optional, tag = "5")]
	pub timestamp_ms: Option<u64>,
}

impl AudioChunkMessage {
	/// Create from sample rate, channels, and f32 samples (converts to bytes)
	pub fn new(sample_rate: u32, channels: u32, samples: Vec<f32>) -> Self {
		// Convert f32 samples to bytes
		let mut bytes = Vec::with_capacity(samples.len() * 4);
		for sample in samples {
			bytes.extend_from_slice(&sample.to_le_bytes());
		}

		Self {
			samples: bytes,
			sample_rate: Some(sample_rate),
			channels: Some(channels),
			sequence: None,
			timestamp_ms: None,
		}
	}

	/// Convert raw bytes to f32 samples
	pub fn decode_samples(&self) -> Result<Vec<f32>, Box<dyn std::error::Error>> {
		if self.samples.len() % 4 != 0 {
			return Err("Invalid sample data: length not multiple of 4".into());
		}

		let mut samples = Vec::with_capacity(self.samples.len() / 4);
		for chunk in self.samples.chunks_exact(4) {
			let bytes: [u8; 4] = chunk.try_into()?;
			samples.push(f32::from_le_bytes(bytes));
		}
		Ok(samples)
	}

	/// Get total sample count
	pub fn sample_count(&self) -> usize {
		self.samples.len() / 4 // Each f32 is 4 bytes
	}

	/// Get duration in milliseconds (requires sample_rate)
	pub fn duration_ms(&self) -> Option<f64> {
		let sample_rate = self.sample_rate?;
		let channels = self.channels.unwrap_or(2) as usize;
		let frame_count = self.sample_count() / channels;
		Some((frame_count as f64 / sample_rate as f64) * 1000.0)
	}

	/// Check if this chunk contains format information
	pub fn has_format_info(&self) -> bool {
		self.sample_rate.is_some() && self.channels.is_some()
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

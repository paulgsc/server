use anyhow::Result;
use tracing::{debug, info, warn};
use webrtc_vad::{Vad, VadMode};

/// Voice Activity Detection processor
///
/// Filters silent or noise-only audio chunks before transcription.
/// Uses WebRTC VAD for high-quality speech detection.
pub struct VadProcessor {
	vad: Vad,
	sample_rate: u32,
	frame_duration_ms: u32,
	frame_samples: usize,
	speech_threshold: f32,
	stats: VadStats,
}

#[derive(Debug, Clone, Default)]
pub struct VadStats {
	pub total_frames: u64,
	pub speech_frames: u64,
	pub silence_frames: u64,
	pub total_chunks: u64,
	pub speech_chunks: u64,
	pub silence_chunks: u64,
}

impl VadStats {
	pub fn speech_ratio(&self) -> f32 {
		if self.total_frames == 0 {
			return 0.0;
		}
		self.speech_frames as f32 / self.total_frames as f32
	}

	pub fn chunk_speech_ratio(&self) -> f32 {
		if self.total_chunks == 0 {
			return 0.0;
		}
		self.speech_chunks as f32 / self.total_chunks as f32
	}
}

impl VadProcessor {
	/// Create new VAD processor
	///
	/// # Arguments
	/// * `sample_rate` - Audio sample rate (8000, 16000, 32000, or 48000)
	/// * `mode` - VAD sensitivity mode
	/// * `speech_threshold` - Minimum ratio of speech frames to consider chunk as speech (0.0-1.0)
	pub fn new(sample_rate: u32, mode: VadMode, speech_threshold: f32) -> Result<Self> {
		// Validate sample rate
		match sample_rate {
			8000 | 16000 | 32000 | 48000 => {}
			_ => {
				return Err(anyhow::anyhow!("Invalid sample rate: {}. Must be 8000, 16000, 32000, or 48000", sample_rate));
			}
		}

		// VAD works on 10ms, 20ms, or 30ms frames
		// For 16kHz: 10ms = 160 samples, 20ms = 320 samples, 30ms = 480 samples
		let frame_duration_ms = 30; // Use 30ms for better accuracy
		let frame_samples = (sample_rate as f32 * frame_duration_ms as f32 / 1000.0) as usize;

		let vad = Vad::new_with_rate_and_mode(webrtc_vad::SampleRate::try_from(sample_rate as usize)?, mode);

		info!(
			sample_rate,
			frame_duration_ms,
			frame_samples,
			mode = format!("{:?}", mode),
			speech_threshold,
			"🎤 VAD initialized"
		);

		Ok(Self {
			vad,
			sample_rate,
			frame_duration_ms,
			frame_samples,
			speech_threshold,
			stats: VadStats::default(),
		})
	}

	/// Check if audio chunk contains speech
	///
	/// Returns true if the percentage of speech frames exceeds the threshold
	pub fn contains_speech(&mut self, audio: &[f32]) -> bool {
		self.stats.total_chunks += 1;

		if audio.is_empty() {
			self.stats.silence_chunks += 1;
			return false;
		}

		// Convert f32 samples to i16 for WebRTC VAD
		let samples_i16: Vec<i16> = audio
			.iter()
			.map(|&sample| {
				// Clamp to [-1.0, 1.0] and convert to i16
				let clamped = sample.clamp(-1.0, 1.0);
				(clamped * i16::MAX as f32) as i16
			})
			.collect();

		// Process audio in frames
		let mut speech_frames = 0;
		let mut total_frames = 0;

		for chunk in samples_i16.chunks(self.frame_samples) {
			// Skip incomplete frames at the end
			if chunk.len() < self.frame_samples {
				continue;
			}

			total_frames += 1;
			self.stats.total_frames += 1;

			match self.vad.is_voice_segment(chunk) {
				Ok(is_speech) => {
					if is_speech {
						speech_frames += 1;
						self.stats.speech_frames += 1;
					} else {
						self.stats.silence_frames += 1;
					}
				}
				Err(e) => {
					warn!(error = %e, "VAD processing error");
					// On error, assume silence to be conservative
					self.stats.silence_frames += 1;
				}
			}
		}

		if total_frames == 0 {
			self.stats.silence_chunks += 1;
			return false;
		}

		let speech_ratio = speech_frames as f32 / total_frames as f32;
		let contains_speech = speech_ratio >= self.speech_threshold;

		if contains_speech {
			self.stats.speech_chunks += 1;
		} else {
			self.stats.silence_chunks += 1;
		}

		debug!(
			speech_frames,
			total_frames,
			speech_ratio = format!("{:.2}", speech_ratio),
			threshold = self.speech_threshold,
			contains_speech,
			"VAD analysis"
		);

		contains_speech
	}

	/// Get VAD statistics
	pub fn stats(&self) -> &VadStats {
		&self.stats
	}

	/// Reset statistics
	pub fn reset_stats(&mut self) {
		self.stats = VadStats::default();
	}

	/// Get speech threshold
	pub fn speech_threshold(&self) -> f32 {
		self.speech_threshold
	}

	/// Update speech threshold
	pub fn set_speech_threshold(&mut self, threshold: f32) {
		self.speech_threshold = threshold.clamp(0.0, 1.0);
		info!(new_threshold = self.speech_threshold, "🎚️ VAD speech threshold updated");
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_vad_creation() {
		let vad = VadProcessor::new(16000, VadMode::Quality, 0.3);
		assert!(vad.is_ok());
	}

	#[test]
	fn test_invalid_sample_rate() {
		let vad = VadProcessor::new(44100, VadMode::Quality, 0.3);
		assert!(vad.is_err());
	}

	#[test]
	fn test_silence_detection() {
		let mut vad = VadProcessor::new(16000, VadMode::Quality, 0.3).unwrap();

		// Generate 1 second of silence
		let silence: Vec<f32> = vec![0.0; 16000];

		let has_speech = vad.contains_speech(&silence);
		assert!(!has_speech);
	}

	#[test]
	fn test_stats_tracking() {
		let mut vad = VadProcessor::new(16000, VadMode::Quality, 0.3).unwrap();

		let silence: Vec<f32> = vec![0.0; 16000];
		vad.contains_speech(&silence);

		let stats = vad.stats();
		assert_eq!(stats.total_chunks, 1);
		assert_eq!(stats.silence_chunks, 1);
	}
}

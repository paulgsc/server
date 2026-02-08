use clap::Parser;

#[derive(Parser, Debug, Clone)]
#[command(name = "transcriber")]
#[command(about = "CPU-optimized audio transcription service", long_about = None)]
pub struct Config {
	/// NATS server URL
	#[arg(long, env = "NATS_URL", default_value = "nats://localhost:4222")]
	pub nats_url: String,

	/// Whisper model path
	#[arg(long, env = "WHISPER_MODELS_PATH")]
	pub whisper_model_path: String,

	/// Number of threads for Whisper processing
	#[arg(long, env = "WHISPER_THREADS", default_value = "2")]
	pub whisper_threads: i32,

	/// Target sample rate for audio processing
	#[arg(long, env = "TARGET_SAMPLE_RATE", default_value = "16000")]
	pub target_sample_rate: u32,

	/// Buffer duration in seconds before transcription
	#[arg(long, env = "BUFFER_DURATION", default_value = "3")]
	pub buffer_duration_secs: usize,

	/// Service name for observability
	#[arg(long, env = "OTEL_SERVICE_NAME", default_value = "transcriber")]
	pub service_name: String,

	/// Heartbeat interval in seconds
	#[arg(long, env = "HEARTBEAT_INTERVAL", default_value = "30")]
	pub heartbeat_interval_secs: u64,

	/// Enable Voice Activity Detection (VAD) for pre-transcription filtering
	#[arg(long, env = "VAD_ENABLED", default_value = "true")]
	pub vad_enabled: bool,

	/// VAD speech threshold (0.0 - 1.0)
	/// Percentage of frames that must contain speech to process the buffer
	#[arg(long, env = "VAD_SPEECH_THRESHOLD", default_value = "0.3")]
	pub vad_speech_threshold: f32,

	/// VAD mode: 0 (Quality), 1 (LowBitrate), 2 (Aggressive), 3 (VeryAggressive)
	#[arg(long, env = "VAD_MODE", default_value = "0")]
	pub vad_mode: u8,
}

impl Config {
	/// Validate configuration values
	pub fn validate(&self) -> Result<(), String> {
		// Validate Whisper model path
		if self.whisper_model_path.is_empty() {
			return Err("WHISPER_MODEL_PATH must be set".to_string());
		}

		if self.whisper_threads < 1 {
			return Err("whisper_threads must be at least 1".to_string());
		}

		// Validate sample rate (WebRTC VAD supports 8000, 16000, 32000, 48000)
		match self.target_sample_rate {
			8000 | 16000 | 32000 | 48000 => {}
			_ => {
				return Err(format!("TARGET_SAMPLE_RATE must be 8000, 16000, 32000, or 48000 (got {})", self.target_sample_rate));
			}
		}

		if self.buffer_duration_secs == 0 {
			return Err("buffer_duration_secs must be greater than 0".to_string());
		}

		if self.heartbeat_interval_secs == 0 {
			return Err("heartbeat_interval_secs must be greater than 0".to_string());
		}

		// Validate VAD threshold
		if self.vad_speech_threshold < 0.0 || self.vad_speech_threshold > 1.0 {
			return Err(format!("VAD_SPEECH_THRESHOLD must be between 0.0 and 1.0 (got {})", self.vad_speech_threshold));
		}

		// Validate VAD mode
		if self.vad_mode > 3 {
			return Err(format!("VAD_MODE must be 0-3 (got {})", self.vad_mode));
		}

		Ok(())
	}

	/// Get VAD mode as webrtc_vad::VadMode
	pub fn get_vad_mode(&self) -> webrtc_vad::VadMode {
		match self.vad_mode {
			0 => webrtc_vad::VadMode::Quality,
			1 => webrtc_vad::VadMode::LowBitrate,
			2 => webrtc_vad::VadMode::Aggressive,
			3 => webrtc_vad::VadMode::VeryAggressive,
			_ => webrtc_vad::VadMode::Quality, // Default fallback
		}
	}
}

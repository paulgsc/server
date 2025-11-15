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
}

impl Config {
	/// Validate configuration values
	pub fn validate(&self) -> Result<(), String> {
		if self.whisper_threads < 1 {
			return Err("whisper_threads must be at least 1".to_string());
		}

		if self.target_sample_rate == 0 {
			return Err("target_sample_rate must be greater than 0".to_string());
		}

		if self.buffer_duration_secs == 0 {
			return Err("buffer_duration_secs must be greater than 0".to_string());
		}

		if self.heartbeat_interval_secs == 0 {
			return Err("heartbeat_interval_secs must be greater than 0".to_string());
		}

		Ok(())
	}
}

use clap::Parser;
use serde::{Deserialize, Serialize};

#[derive(Parser, Clone, Debug, Serialize, Deserialize)]
#[command(author, version, about, long_about = None)]
pub struct Config {
	/// Use JSON formatting for tracing
	#[arg(long, env = "LOG_JSON", default_value = "false")]
	pub log_json: bool,

	/// Log level
	#[arg(long, env = "RUST_LOG")]
	pub rust_log: Option<String>,

	/// Database connection timeout in seconds
	#[arg(long, env = "CONNECTION_TIMEOUT", default_value = "30")]
	pub connection_timeout: u64,

	/// Server host
	#[arg(long, env = "HOST", default_value = "127.0.0.1")]
	pub host: String,

	/// Server port
	#[arg(long, env = "PORT", default_value = "8080")]
	pub port: u16,

	/// Number of worker threads
	#[arg(long, env = "WORKERS", default_value = "4")]
	pub workers: usize,

	/// HMAC signing key for JWT tokens
	#[arg(long, env = "HMAC_KEY")]
	pub hmac_key: String,

	/// JWT token expiration time in seconds
	#[arg(long, env = "TOKEN_EXPIRATION", default_value = "3600")]
	pub token_expiration: u64,

	/// API version
	#[arg(long, env = "API_VERSION", default_value = "v1")]
	pub api_version: String,

	/// Rate limit (requests per minute)
	#[arg(long, env = "RATE_LIMIT", default_value = "10")]
	pub rate_limit: u32,

	/// Enable CORS
	#[arg(long, env = "ENABLE_CORS")]
	pub enable_cors: bool,

	/// Log level
	#[arg(long, env = "LOG_LEVEL", default_value = "info")]
	pub log_level: LogLevel,

	/// Log file path
	#[arg(long, env = "LOG_FILE")]
	pub log_file: Option<String>,

	/// Secret file path
	#[arg(long, env = "CLIENT_SECRET_FILE")]
	pub client_secret_file: String,

	/// Enable user registration
	#[arg(long, env = "ENABLE_USER_REGISTRATION")]
	pub enable_user_registration: bool,

	/// Enable email verification
	#[arg(long, env = "ENABLE_EMAIL_VERIFICATION")]
	pub enable_email_verification: bool,

	/// Enable two-factor authentication
	#[arg(long, env = "ENABLE_TWO_FACTOR_AUTH")]
	pub enable_two_factor_auth: bool,

	/// Email service URL
	#[arg(long, env = "EMAIL_SERVICE_URL")]
	pub email_service_url: Option<String>,

	/// SMS service URL
	#[arg(long, env = "SMS_SERVICE_URL")]
	pub sms_service_url: Option<String>,

	/// Redis URL for caching
	#[arg(long, env = "REDIS_URL")]
	pub redis_url: Option<String>,

	/// Cache TTL in seconds
	#[arg(long, env = "CACHE_TTL", default_value = "600")]
	pub cache_ttl: u64,

	/// Enable Prometheus metrics
	#[arg(long, env = "ENABLE_PROMETHEUS")]
	pub enable_prometheus: bool,

	/// Prometheus metrics port
	#[arg(long, env = "PROMETHEUS_PORT", default_value = "9090")]
	pub prometheus_port: u16,

	/// Enable development mode
	#[arg(long, env = "DEV_MODE")]
	pub dev_mode: bool,

	/// Streaming Chunk Size
	#[arg(long, env = "BUFFER_SIZE", default_value = "65536")]
	pub chunk_size: usize,

	/// Streaming Max Chunk Size
	#[arg(long, env = "MAX_CHUNKS_IN_FLIGHT", default_value = "5")]
	pub max_chunks: usize,

	/// OBS Websocket Server IP Address
	#[arg(long, env = "OBS_WEBSOCKET_IP")]
	pub obs_host: String,

	/// OBS Websocket Server Password
	#[arg(long, env = "OBS_WEBSOCKET_PWD")]
	pub obs_password: String,

	/// GITHUB API SECRET TOKEN
	#[arg(long, env = "GITHUB_API_TOKEN")]
	pub github_token: String,
}

#[derive(clap::ValueEnum, Clone, Debug, Serialize, Deserialize)]
pub enum ProgramMode {
	Run,
	PopulateGameClocks,
}

#[derive(clap::ValueEnum, Clone, Debug, Serialize, Deserialize)]
pub enum LogLevel {
	Error,
	Warn,
	Info,
	Debug,
	Trace,
}

impl Config {
	pub fn new() -> Self {
		Self::parse()
	}
}

use clap::Parser;
use serde::{Deserialize, Serialize};

#[derive(Parser, Clone, Debug, Serialize, Deserialize)]
#[command(author, version, about, long_about = None)]
pub struct Config {
	/// Database URL
	#[arg(long, env = "DATABASE_URL")]
	pub database_url: String,

	/// Maximum number of database connections
	#[arg(long, env = "MAX_CONNECTIONS", default_value = "10")]
	pub max_connections: u32,

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
	#[arg(long, env = "RATE_LIMIT", default_value = "100")]
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
	#[arg(long, env = "CACHE_TTL", default_value = "300")]
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

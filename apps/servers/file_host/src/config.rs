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

	/// Max number of concurrent fetches to deduplicate. Older entries are evicted when exceeded.
	#[arg(long, env = "MAX_IN_FLIGHT", default_value = "256")]
	pub max_in_flight: u64,

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

	/// Rate limit, in requests per minute, per client (`main.rs`'s
	/// `rate_limit_middleware` partitions by peer IP). See #215/#231: this
	/// field previously declared the right unit but was unread — the
	/// limiter's capacity was `max_request_size` (megabytes of upload
	/// payload) instead.
	#[arg(long, env = "RATE_LIMIT", default_value = "60")]
	pub rate_limit: u32,

	/// Enable CORS
	#[arg(long, env = "ENABLE_CORS")]
	pub enable_cors: bool,

	/// Comma-separated list of allowed CORS origins for browser-facing read routes
	/// (sheets, audio, push, sessions). Other routes deliberately allow any origin
	/// (extensions, etc.) and are unaffected by this setting.
	///
	/// The default carries three entries rather than one because the study app is
	/// not on `:6006` — that is Storybook. Service workers, `Notification`, and
	/// `PushManager` are all gated on a secure context, so the origin a
	/// subscription is actually posted from is an HTTPS one, and a list that only
	/// admits the Storybook origin blocks it.
	#[arg(
		long,
		env = "ALLOWED_ORIGINS",
		value_delimiter = ',',
		default_value = "https://nixos.local:5173,https://localhost:5173,http://nixos.local:6006"
	)]
	pub allowed_origins: Vec<String>,

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

	/// NATS URL for broker messaging
	#[arg(long, env = "NATS_URL")]
	pub nats_url: Option<String>,

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

	/// Individual request payload lmit in MB
	#[arg(long, env = "MAX_REQUEST_SIZE_MB", default_value = "10")]
	pub max_request_size: usize,

	/// Active request limit
	#[arg(long, env = "MAX_CONCURRENT_REQUESTS", default_value = "100")]
	pub max_concurrent_req: usize,

	/// Hard timeout for any operation
	#[arg(long, env = "TASK_TIMEOUT_MS", default_value = "15000")]
	pub task_timeout_ms: u64,

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

	/// DATABASE URL
	#[arg(long, env = "DATABASE_URL")]
	pub database_url: String,

	// ── Study nudge ──────────────────────────────────────────────────────────
	//
	// See `docs/study-nudge.md`. Everything below is off by default: this is a
	// feature that speaks without being spoken to, and one that turns itself on
	// because a container happened to have a database is worse than one that
	// never runs.
	/// Whether the daily nudge tick runs at all.
	///
	/// With this off, the `/push` routes still work — a browser can subscribe and
	/// a manual send can be triggered — but nothing fires on a schedule.
	#[arg(long, env = "NUDGE_ENABLED", default_value = "false")]
	pub nudge_enabled: bool,

	/// VAPID public key, base64url, uncompressed P-256 point.
	///
	/// Handed to the browser as `applicationServerKey`. It is baked into every
	/// subscription made with it, so changing it does not re-key them — it
	/// silently invalidates all of them.
	#[arg(long, env = "VAPID_PUBLIC_KEY")]
	pub vapid_public_key: Option<String>,

	/// VAPID private key, base64url, the raw 32-byte P-256 scalar. Secret.
	#[arg(long, env = "VAPID_PRIVATE_KEY")]
	pub vapid_private_key: Option<String>,

	/// The VAPID `sub` claim: a `mailto:` or an origin URL identifying the sender.
	/// Push services reject or deprioritise senders without one.
	#[arg(long, env = "VAPID_SUBJECT", default_value = "mailto:admin@maishatu.com")]
	pub vapid_subject: String,

	/// IANA timezone name deciding when a day starts and when quiet hours apply.
	///
	/// Deliberately an `Option` with no default. A container is UTC unless told
	/// otherwise, and a silent UTC fallback is how an evening reminder arrives at
	/// 3am; leaving this unset is allowed but is logged as the choice it is.
	#[arg(long, env = "NUDGE_TIMEZONE")]
	pub nudge_timezone: Option<String>,

	/// Local hour, 0-23, at which nudges stop.
	#[arg(long, env = "NUDGE_QUIET_HOURS_START", default_value = "22")]
	pub nudge_quiet_hours_start: u32,

	/// Local hour, 0-23, at which nudges resume. May be less than the start hour;
	/// the range wraps midnight. Equal bounds mean no quiet hours at all.
	#[arg(long, env = "NUDGE_QUIET_HOURS_END", default_value = "8")]
	pub nudge_quiet_hours_end: u32,

	/// How often the waker picks up work the engine already decided.
	///
	/// Not a schedule. Eligibility instants are solved when signals arrive and
	/// written to an index; this is the resolution at which they are noticed, so
	/// shortening it lands interventions closer to their computed instant
	/// without changing which ones happen.
	#[arg(long, env = "NUDGE_WAKER_SECONDS", default_value = "300")]
	pub nudge_waker_seconds: u64,

	/// How recently a WebSocket must have been active to count as presence.
	#[arg(long, env = "NUDGE_PRESENCE_FRESHNESS_SECONDS", default_value = "120")]
	pub nudge_presence_freshness_seconds: u64,

	/// Base URL of the study app, used to build the notification's deep link.
	/// Must end with a slash: the client builds `${BASE_URL}sessions/${id}` and
	/// the server has to produce the same shape or clicks land on the dashboard.
	#[arg(long, env = "APP_BASE_URL", default_value = "https://nixos.local:5173/")]
	pub app_base_url: String,

	/// Perform health check and exit
	#[arg(long, help = "Perform health check against running server")]
	pub health_check: bool,

	/// Port for health check (defaults to 3000)
	#[arg(long, default_value = "3000", help = "Port to check for health endpoint")]
	pub health_check_port: u16,

	/// Host for health check (defaults to localhost)
	#[arg(long, default_value = "127.0.0.1", help = "Host to check for health endpoint")]
	pub health_check_host: String,
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

	pub fn as_cache_config(&self) -> some_cache::CacheConfig {
		some_cache::CacheConfig::new(self.redis_url.clone().unwrap_or_else(|| "redis://127.0.0.1:6379".into())).with_ttl(self.cache_ttl)
	}
}

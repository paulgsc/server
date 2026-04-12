/// Configuration for `CacheStore`.
///
/// Constructed directly or via `From` impls in each bin's crate
/// (keeping this crate free of bin-specific config types).
#[derive(Clone, Debug)]
pub struct CacheConfig {
	pub redis_url: String,
	/// Default TTL in seconds applied when callers pass `None`.
	pub default_ttl: u64,
	pub max_retries: u32,
	pub retry_delay_ms: u64,
	pub enable_compression: bool,
	/// Compress serialized payloads larger than this many bytes.
	pub compression_threshold: usize,
	/// Key namespace prefix, e.g. `"cache:"`.
	pub key_prefix: String,
	/// zstd compression level [1–22]. Default 3 (fast, good ratio).
	/// Level 1  → fastest, ~same speed as lz4
	/// Level 3  → balanced default
	/// Level 19 → near-max ratio, slow (don't use in hot paths)
	pub zstd_level: Option<i32>,

	/// Probability [0.0–1.0] that a cache hit will refresh the key's TTL.
	/// Default 0.1 (10% of reads). Set to 1.0 for always-refresh (old behaviour),
	/// 0.0 to disable sliding TTL entirely.
	pub touch_probability: Option<f64>,
}

impl Default for CacheConfig {
	fn default() -> Self {
		Self {
			redis_url: "redis://127.0.0.1:6379".to_string(),
			default_ttl: 3600,
			max_retries: 3,
			retry_delay_ms: 100,
			enable_compression: true,
			compression_threshold: 1024,
			zstd_level: Some(3),
			touch_probability: Some(0.1),
			key_prefix: "cache:".to_string(),
		}
	}
}

impl CacheConfig {
	pub fn new(redis_url: impl Into<String>) -> Self {
		Self {
			redis_url: redis_url.into(),
			..Default::default()
		}
	}

	pub fn with_ttl(mut self, ttl: u64) -> Self {
		self.default_ttl = ttl;
		self
	}

	pub fn with_prefix(mut self, prefix: impl Into<String>) -> Self {
		self.key_prefix = prefix.into();
		self
	}

	pub fn with_compression(mut self, enabled: bool, threshold: usize) -> Self {
		self.enable_compression = enabled;
		self.compression_threshold = threshold;
		self
	}
}

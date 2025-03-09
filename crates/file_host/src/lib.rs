use crate::error::FileHostError;
use redis::Client;
use std::sync::Arc;
use std::time::Duration;

pub mod config;
pub mod error;
pub mod handlers;
pub mod metrics;
pub mod rate_limiter;
pub mod routes;

pub use config::*;
pub use routes::*;

#[derive(Clone)]
pub struct CacheStore {
	pub client: Client,
	pub cache_ttl: Duration,
}

impl CacheStore {
	pub fn new(config: Arc<Config>) -> Result<Self, FileHostError> {
		let redis_url = config.redis_url.as_deref().unwrap_or_else(|| {
			log::warn!("Using default Redis URL: redis://127.0.0.1:6379");
			"redis://127.0.0.1:6379"
		});

		let client = Client::open(redis_url)?;
		let cache_ttl = Duration::from_secs(config.cache_ttl);

		Ok(Self { client, cache_ttl })
	}
}

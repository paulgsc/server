use crate::error::FileHostError;
use redis::Client;
use std::sync::Arc;
use std::time::Duration;

pub mod config;
pub mod error;
pub mod handlers;
pub mod routes;

pub use config::*;

pub struct AppState {
	pub client: Client,
	pub cache_ttl: Duration,
}

impl AppState {
	pub fn new(config: Arc<Config>) -> Result<Arc<Self>, FileHostError> {
		Ok(Arc::new(Self {
			client: Client::open(config.redis_url.as_deref().unwrap_or("redis://127.0.0.1:6379"))?,
			cache_ttl: Duration::from_secs(config.cache_ttl),
		}))
	}
}

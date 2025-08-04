use crate::error::{FileHostError, GSheetDeriveError};
use redis::{AsyncCommands, Client};
use std::sync::Arc;

pub mod cache;
pub mod config;
pub mod error;
pub mod handlers;
pub mod streaming_service;
pub mod websocket;
// pub mod image_processing;
pub mod metrics;
pub mod models;
pub mod rate_limiter;
pub mod routes;
pub mod utils;

pub use crate::websocket::{Event, NowPlaying, UtterancePrompt, WebSocketFsm};
pub use config::*;
pub use handlers::utterance::UtteranceMetadata;
pub use metrics::http::*;

#[derive(Clone)]
pub struct AppState {
	pub cache_store: CacheStore,
	pub config: Arc<Config>,
}

#[derive(Clone)]
pub struct CacheStore {
	pub redis_client: Client,
	pub cache_ttl: u64,
	pub config: Arc<Config>,
}

impl CacheStore {
	pub fn new(config: Arc<Config>) -> Result<Self, FileHostError> {
		let redis_url = config.redis_url.as_deref().unwrap_or_else(|| {
			log::warn!("Using default Redis URL: redis://127.0.0.1:6379");
			"redis://127.0.0.1:6379"
		});

		let redis_client = Client::open(redis_url)?;
		let cache_ttl = config.cache_ttl;

		Ok(Self { redis_client, cache_ttl, config })
	}

	async fn reset_cache_ttl(&self, key: &str) -> Result<(), FileHostError> {
		let mut con = self.redis_client.get_multiplexed_async_connection().await?;
		let _: () = con.expire(key, self.cache_ttl.try_into().unwrap()).await?;
		Ok(())
	}

	pub async fn set_json<T: serde::Serialize>(&self, key: &str, data: &T) -> Result<(), FileHostError> {
		let mut con = self.redis_client.get_multiplexed_async_connection().await?;
		let serialized = serde_json::to_string(data)?;
		let _: () = con.set_ex(key, serialized, self.cache_ttl).await?;
		Ok(())
	}

	pub async fn get_json<T: serde::de::DeserializeOwned>(&self, key: &str) -> Result<Option<T>, FileHostError> {
		let mut con = self.redis_client.get_multiplexed_async_connection().await?;
		let data: Option<String> = con.get(key).await?;

		match data {
			Some(json_str) => {
				self.reset_cache_ttl(key).await?;
				Ok(Some(serde_json::from_str(&json_str)?))
			}
			None => Ok(None),
		}
	}

	#[allow(dead_code)]
	async fn set_bytes(&self, key: &str, data: &[u8]) -> Result<(), FileHostError> {
		let mut con = self.redis_client.get_multiplexed_async_connection().await?;
		let _: () = con.set_ex(key, data, self.cache_ttl).await?;
		Ok(())
	}

	#[allow(dead_code)]
	async fn get_bytes(&self, key: &str) -> Result<Option<Vec<u8>>, FileHostError> {
		let mut con = self.redis_client.get_multiplexed_async_connection().await?;
		let data: Option<Vec<u8>> = con.get(key).await?;

		if data.is_some() {
			self.reset_cache_ttl(key).await?;
		}

		Ok(data)
	}
}

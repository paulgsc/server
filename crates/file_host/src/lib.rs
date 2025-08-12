use crate::error::{FileHostError, GSheetDeriveError};
use std::sync::Arc;

pub mod cache;
pub mod config;
pub mod error;
pub mod handlers;
// pub mod streaming_service;
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
pub use metrics::ws::*;

pub use cache::{CacheConfig, CacheStore};

#[derive(Clone)]
pub struct AppState {
	pub cache_store: CacheStore,
	pub config: Arc<Config>,
}

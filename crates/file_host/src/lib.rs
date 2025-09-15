use crate::error::{FileHostError, GSheetDeriveError};
use axum::extract::FromRef;
use sqlx::SqlitePool;
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
use sdk::{GitHubClient, ReadDrive, ReadSheets};

pub use cache::{CacheConfig, CacheStore, DedupCache, DedupError};
pub use handlers::audio_files::error::AudioServiceError;

#[derive(Clone)]
pub struct AppState {
	pub dedup_cache: Arc<DedupCache>,
	pub gsheet_reader: Arc<ReadSheets>,
	pub gdrive_reader: Arc<ReadDrive>,
	pub github_client: Arc<GitHubClient>,
	pub shared_db: SqlitePool,
	pub ws: WebSocketFsm,
	// TODO: might remove the Arc here!
	pub config: Arc<Config>,
}

// Implement for the non-Arc field `DedupCache`
impl FromRef<AppState> for Arc<DedupCache> {
	fn from_ref(state: &AppState) -> Self {
		state.dedup_cache.clone()
	}
}

// Implement for the Arc-wrapped field `ReadSheets`
impl FromRef<AppState> for Arc<ReadSheets> {
	fn from_ref(state: &AppState) -> Self {
		state.gsheet_reader.clone()
	}
}

// Implement for your `Config`
impl FromRef<AppState> for Arc<Config> {
	fn from_ref(state: &AppState) -> Self {
		state.config.clone()
	}
}

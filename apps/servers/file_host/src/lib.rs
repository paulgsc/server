use crate::error::{FileHostError, GSheetDeriveError};
use axum::extract::FromRef;
use sdk::{GitHubClient, ReadDrive, ReadSheets};
use some_transport::NatsTransport;
use sqlx::SqlitePool;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use ws_conn_manager::{AcquireErrorKind, ConnectionGuard, ConnectionPermit};

pub mod cache;
pub mod config;
pub mod error;
pub mod handlers;
pub mod health;
pub mod metrics;
pub mod models;
pub mod rate_limiter;
pub mod routes;
pub mod transport;
pub mod utils;
pub mod websocket;

pub use crate::websocket::{init_websocket, Event, NowPlaying, UtterancePrompt, WebSocketFsm};
pub use cache::{CacheConfig, CacheStore, DedupCache, DedupError};
pub use config::*;
pub use handlers::audio_files::error::AudioServiceError;
pub use handlers::utterance::UtteranceMetadata;
pub use health::perform_health_check;
pub use metrics::http::*;
pub use metrics::ws::*;

/// Core: defines the universe - stable, global, rarely changes
#[derive(Clone)]
pub struct CoreContext {
	pub config: Arc<Config>,
	pub cancel_token: CancellationToken,
	pub shared_db: SqlitePool,
	pub connection_guard: ConnectionGuard,
}

/// External APIs: third-party integrations with independent lifecycles
#[derive(Clone)]
pub struct ExternalApis {
	pub gsheet_reader: Arc<ReadSheets>,
	pub gdrive_reader: Arc<ReadDrive>,
	pub github_client: Arc<GitHubClient>,
}

/// Realtime: websocket and ephemeral caching subsystem
#[derive(Clone)]
pub struct RealtimeContext {
	pub ws: WebSocketFsm,
	pub dedup_cache: Arc<DedupCache>,
	pub transport: Arc<NatsTransport<Event>>,
}

#[derive(Clone)]
pub struct AppState {
	pub core: CoreContext,
	pub external: ExternalApis,
	pub realtime: RealtimeContext,
}

impl AppState {
	/// Build the entire universe in one explicit place
	pub async fn build(config: Arc<Config>, pool: SqlitePool, cancel_token: CancellationToken) -> anyhow::Result<Self> {
		let core = CoreContext {
			config: config.clone(),
			cancel_token: cancel_token.clone(),
			shared_db: pool,
			connection_guard: ConnectionGuard::new(),
		};

		let secret_file = config.client_secret_file.clone();
		let use_email = config.email_service_url.clone().unwrap_or_default();

		let external = ExternalApis {
			gsheet_reader: Arc::new(ReadSheets::new(use_email.clone(), secret_file.clone())?),
			gdrive_reader: Arc::new(ReadDrive::new(use_email.clone(), secret_file.clone())?),
			github_client: Arc::new(GitHubClient::new(config.github_token.clone())?),
		};

		let cache_store = CacheStore::new(CacheConfig::from(config.clone()))?;
		let dedup_cache = Arc::new(DedupCache::new(cache_store.into(), config.max_in_flight.clone()));

		// Initialize NATS transports
		let nats_url = config.nats_url.as_deref().unwrap_or("nats://localhost:4222");
		let transports = Arc::new(NatsTransport::connect_pooled(nats_url).await?);

		let ws = init_websocket(cancel_token.clone()).await;

		let realtime = RealtimeContext { ws, dedup_cache, transport };

		Ok(Self { core, external, realtime })
	}
}

// TODO: Ask eh eye! do we need this?!
impl FromRef<AppState> for Arc<DedupCache> {
	fn from_ref(state: &AppState) -> Self {
		state.realtime.dedup_cache.clone()
	}
}

impl FromRef<AppState> for Arc<ReadSheets> {
	fn from_ref(state: &AppState) -> Self {
		state.external.gsheet_reader.clone()
	}
}

impl FromRef<AppState> for Arc<Config> {
	fn from_ref(state: &AppState) -> Self {
		state.core.config.clone()
	}
}

impl FromRef<AppState> for SqlitePool {
	fn from_ref(state: &AppState) -> Self {
		state.core.shared_db.clone()
	}
}

impl FromRef<AppState> for WebSocketFsm {
	fn from_ref(state: &AppState) -> Self {
		state.realtime.ws.clone()
	}
}

impl FromRef<AppState> for CancellationToken {
	fn from_ref(state: &AppState) -> Self {
		state.core.cancel_token.clone()
	}
}

impl FromRef<AppState> for ConnectionGuard {
	fn from_ref(state: &AppState) -> Self {
		state.core.connection_guard.clone()
	}
}

impl FromRef<AppState> for Arc<NatsTransport<Event>> {
	fn from_ref(state: &AppState) -> Self {
		state.realtime.transport.clone()
	}
}

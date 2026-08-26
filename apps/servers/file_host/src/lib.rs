use crate::error::FileHostError;
use axum::extract::FromRef;
use some_transport::{nats::JetStreamPublisher, NatsTransport};
use sqlx::SqlitePool;
use std::sync::{Arc, Mutex};
use tokio_util::sync::CancellationToken;
use ws_conn_manager::ConnectionGuard;
use ws_events::{tabsched::JobEnvelope, UnifiedEvent};

pub mod cache;
pub mod config;
pub mod error;
pub mod handlers;
pub mod health;
pub mod metrics;
pub mod models;
pub mod nudge;
pub mod rate_limiter;
pub mod routes;
pub mod subject;
pub mod utils;
pub mod websocket;

/// Base path all versioned HTTP routers are nested under in `main.rs`.
/// `/health` is the deliberate exception: it stays unversioned so load
/// balancers and orchestrators don't need to track API version bumps.
pub const API_V1_BASE_PATH: &str = "/api/v1";

pub use crate::websocket::WebSocketFsm;
pub use cache::{CacheConfig, CacheStore, DedupCache};
pub use config::*;
pub use handlers::utterance::UtteranceMetadata;
pub use health::perform_health_check;
pub use metrics::{ObservabilityError, OtelGuard};

/// Core: defines the universe - stable, global, rarely changes
#[derive(Clone)]
pub struct CoreContext {
	pub config: Arc<Config>,
	pub cancel_token: CancellationToken,
	pub shared_db: SqlitePool,
	pub connection_guard: ConnectionGuard,
	// Wrap in Mutex<Option<>> so we can take ownership during shutdown
	pub otel_guard: Arc<Mutex<Option<OtelGuard>>>,
}

/// Realtime: websocket and ephemeral caching subsystem
#[derive(Clone)]
pub struct RealtimeContext {
	pub ws: WebSocketFsm,
	pub dedup_cache: Arc<DedupCache>,
	pub transport: NatsTransport<UnifiedEvent>,
	pub pipeline_publisher: Arc<JetStreamPublisher<JobEnvelope>>,
}

/// Everything the study nudge needs, or `None` when the deployment has not
/// configured it.
///
/// An `Option` rather than a set of `Option` fields inside `CoreContext`,
/// because "push is configured" is a single fact: without a VAPID keypair
/// there is no identity to sign with, and every part of the feature is
/// downstream of that. Callers check once.
#[derive(Clone)]
pub struct NudgeContext {
	pub clock: nudge::clock::NudgeClock,
	pub vapid: push_kit::VapidIdentity,
	pub sender: Arc<push_kit::Sender<push_kit::ReqwestTransport>>,
	pub enabled: bool,
	pub quiet_hours_start: u32,
	pub quiet_hours_end: u32,
	pub presence_freshness: std::time::Duration,
	pub base_url: String,
}

#[derive(Clone)]
pub struct AppState {
	pub core: CoreContext,
	pub realtime: RealtimeContext,
	/// `None` when `VAPID_*` is unset. The `/push` routes answer `503` in that
	/// case and the tick never starts, which is deliberately different from
	/// failing: a deployment that does not want notifications should not have
	/// to configure them to boot.
	pub nudge: Option<NudgeContext>,
}

impl AppState {
	/// Build the entire universe in one explicit place
	pub async fn build(config: Arc<Config>, pool: SqlitePool, cancel_token: CancellationToken) -> anyhow::Result<Self> {
		let otel_guard = Arc::new(Mutex::new(Some(OtelGuard::new()?)));
		let core = CoreContext {
			config: config.clone(),
			cancel_token: cancel_token.clone(),
			shared_db: pool,
			connection_guard: ConnectionGuard::new(),
			otel_guard,
		};

		let cache_store = CacheStore::new(config.as_cache_config())?;
		let dedup_cache = Arc::new(DedupCache::new(cache_store.into(), config.max_in_flight.clone()));

		// Initialize NATS transports
		let nats_url = config.nats_url.as_deref().unwrap_or("nats://localhost:4222");
		let transport = NatsTransport::connect_pooled(nats_url).await?;

		// Reuse the Arc<Client> from the transport for JetStream
		let client = transport.client().clone();
		let pipeline_publisher = Arc::new(JetStreamPublisher::from_client(client));

		let ws = WebSocketFsm::new();

		let realtime = RealtimeContext {
			ws,
			dedup_cache,
			transport,
			pipeline_publisher,
		};

		let nudge = Self::build_nudge(&config)?;

		Ok(Self { core, realtime, nudge })
	}

	/// Validate the nudge's configuration once, at startup.
	///
	/// Two rules, and both are about failing at a moment someone is watching:
	///
	/// - Keys absent and `NUDGE_ENABLED` off — fine, the feature is simply not
	///   deployed here. Logged so nobody wonders why nothing arrives.
	/// - Keys absent and `NUDGE_ENABLED` on — a hard error. Discovering a
	///   missing `VAPID_PRIVATE_KEY` at the first send is discovering it hours
	///   later, in a log nobody is reading, from a feature whose only symptom
	///   is silence.
	///
	/// A mismatched keypair fails here too. It is the failure with the longest
	/// fuse: every subscription is made with one public key, and signing with
	/// the other returns `403` on all of them, forever, until every browser
	/// re-subscribes.
	fn build_nudge(config: &Config) -> anyhow::Result<Option<NudgeContext>> {
		use push_kit::VapidIdentity;

		let identity = VapidIdentity::from_config(config.vapid_private_key.as_deref(), config.vapid_public_key.as_deref(), &config.vapid_subject);

		let identity = match identity {
			Ok(identity) => identity,
			Err(err) if config.nudge_enabled => {
				anyhow::bail!("NUDGE_ENABLED is set but the VAPID identity is unusable: {err}");
			}
			Err(err) => {
				tracing::warn!(reason = %err, "study nudge is not configured; /api/v1/push will answer 503 and no tick will run");
				return Ok(None);
			}
		};

		let (clock, zone_source) = nudge::clock::NudgeClock::resolve(config.nudge_timezone.as_deref());
		match &zone_source {
			nudge::clock::ZoneSource::Configured(name) => {
				tracing::info!(zone = %name, "study nudge is using the configured timezone");
			}
			nudge::clock::ZoneSource::Unset => {
				// Not an error, but never silent: a container is UTC by default,
				// and a UTC day boundary is how an evening reminder arrives at 3am.
				tracing::warn!("NUDGE_TIMEZONE is unset; the day boundary and quiet hours will be evaluated in UTC");
			}
			nudge::clock::ZoneSource::Unparseable(name) => {
				crate::metrics::instruments::record_error("nudge_config", "timezone_unparseable");
				tracing::error!(zone = %name, "NUDGE_TIMEZONE is not an IANA zone name; falling back to UTC");
			}
		}

		Ok(Some(NudgeContext {
			clock,
			sender: Arc::new(push_kit::Sender::new(identity.clone(), push_kit::ReqwestTransport::default())),
			vapid: identity,
			enabled: config.nudge_enabled,
			quiet_hours_start: config.nudge_quiet_hours_start,
			quiet_hours_end: config.nudge_quiet_hours_end,
			presence_freshness: std::time::Duration::from_secs(config.nudge_presence_freshness_seconds),
			base_url: config.app_base_url.clone(),
		}))
	}
}

// TODO: Ask eh eye! do we need this?!
impl FromRef<AppState> for Arc<DedupCache> {
	fn from_ref(state: &AppState) -> Self {
		state.realtime.dedup_cache.clone()
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

impl FromRef<AppState> for Arc<Mutex<Option<OtelGuard>>> {
	fn from_ref(state: &AppState) -> Self {
		state.core.otel_guard.clone()
	}
}

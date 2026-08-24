// These modules live in the library crate (`src/lib.rs`) and are used from
// there rather than re-declared here. Declaring `mod routes;` compiled a second
// copy of the whole tree into this binary, which meant anything the binary did
// not happen to call — the route inventory, for one — tripped `dead_code` under
// `-D warnings` despite being used by the library's other consumers.
use anyhow::Result;
use axum::{
	error_handling::HandleErrorLayer, extract::Request as AxumRequest, http::StatusCode, middleware::from_fn, middleware::from_fn_with_state, middleware::Next,
	response::Response, Router,
};
use clap::Parser;
use file_host::metrics::http::{track_http_metrics, unmatched, HTTP_DURATION_BUCKETS, HTTP_DURATION_METRIC};
use file_host::metrics::refusals;
use file_host::rate_limiter::token_bucket::rate_limit_middleware;
use file_host::routes::{
	db::{activities, mood_events, sessions, tabs},
	health::get_health,
	metrics::get_metrics,
	push::push,
	readiness::get_readiness,
	signals::signals,
	tab_metadata::post_now_playing,
	utterance::post_utterance,
};
use file_host::{error::FileHostError, nudge, perform_health_check, AppState, Config, API_V1_BASE_PATH};
use some_services::rate_limiter::{PartitionedTokenBucketLimiter, DEFAULT_REFILL_PERIOD_MS};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use std::{net::SocketAddr, str::FromStr, sync::Arc};
use tokio::{net::TcpListener, time::Duration};
use tokio_util::sync::CancellationToken;
use tower::{limit::ConcurrencyLimitLayer, load_shed::LoadShedLayer, timeout::TimeoutLayer, BoxError, ServiceBuilder};
use tower_http::{add_extension::AddExtensionLayer, limit::RequestBodyLimitLayer, trace::TraceLayer};

async fn handle_tower_error(error: BoxError) -> FileHostError {
	if error.is::<tower::timeout::error::Elapsed>() {
		tracing::warn!("Request timeout: {}", error);
		refusals::record_http("timeout");
		FileHostError::RequestTimeout
	} else if error.is::<tower::load_shed::error::Overloaded>() {
		tracing::warn!("Service overloaded: {}", error);
		refusals::record_http("load_shed");
		FileHostError::ServiceOverloaded
	} else {
		tracing::error!("Unhandled tower error: {}", error);
		FileHostError::TowerError(error)
	}
}

/// `RequestBodyLimitLayer` answers an over-limit request directly — a `413`
/// built inside the layer itself, never routed through `handle_tower_error`
/// (see #223/F3). This wraps it from the outside instead, watching for that
/// status on the way back out. Placed as the first layer inside
/// `ServiceBuilder`'s chain so it sees everything `RequestBodyLimitLayer`
/// (and everything beneath it) produces.
async fn track_body_limit_refusals(req: AxumRequest, next: Next) -> Response {
	let response = next.run(req).await;
	if response.status() == StatusCode::PAYLOAD_TOO_LARGE {
		tracing::warn!("Request body exceeded the configured limit");
		refusals::record_http("body_limit");
	}
	response
}

#[tokio::main]
async fn main() -> Result<()> {
	dotenvy::dotenv().ok();
	let config = Config::parse();

	// Handle health check flag
	if config.health_check {
		return perform_health_check(&config).await;
	}

	// Installs the global `metrics` recorder — see #139 (P1). Every
	// `counter!`/`histogram!` call anywhere in this process, direct or via
	// `crate::metrics::instruments`, writes through this recorder from here
	// on; the handle below is what turns its state into a scrape payload.
	//
	// `install_with` rather than `install`: the request-duration histogram
	// (#213/F2) needs a bucket above `TASK_TIMEOUT_MS`'s 15s, and the
	// exporter's default buckets top out at 10s — see
	// `metrics::http::HTTP_DURATION_BUCKETS`'s own doc comment.
	let metrics_builder = some_metrics::PrometheusBuilder::new().set_buckets_for_metric(some_metrics::Matcher::Full(HTTP_DURATION_METRIC.to_string()), HTTP_DURATION_BUCKETS)?;
	let metrics_handle = some_metrics::install_with(metrics_builder)?;

	// #213 (F4): which build the dashboard is describing, stamped once at
	// startup so a restart shows as a discontinuity rather than an inference
	// from a gap in an unrelated series.
	file_host::metrics::build_info::record();

	// #227 (C2): the configured ceiling `ws_connection_guard_occupancy` is
	// measured against, stamped once for the same reason build info is —
	// fixed for the process's life, so no reason to recompute it every tick.
	file_host::websocket::connection::instrument::set_guard_capacity(ws_conn_manager::MAX_GLOBAL);

	let config = Arc::new(config);
	let connection_options = SqliteConnectOptions::from_str(&config.database_url)?
		.create_if_missing(true)
		.journal_mode(SqliteJournalMode::Wal) // Allows concurrent reads/writes
		.synchronous(SqliteSynchronous::Normal) // Best performance for WAL mode
		.busy_timeout(std::time::Duration::from_secs(5)); // Prevents instant crashes on locks

	let pool = SqlitePoolOptions::new()
		.max_connections(5) // Don't go too high with SQLite
		.connect_with(connection_options)
		.await?;
	let shutdown_token = CancellationToken::new();

	let app_state = AppState::build(config.clone(), pool, shutdown_token.clone()).await?;

	let mut versioned_routes = Router::new()
		.merge(mood_events(&config))
		.merge(tabs(&config))
		.merge(sessions(&config))
		.merge(activities(&config))
		.merge(push(&config))
		.merge(signals(&config))
		.merge(post_now_playing())
		.merge(post_utterance());

	// #215 (A1/A2): capacity is `config.rate_limit` — documented as requests
	// per minute — never `max_request_size`, which is megabytes of payload
	// and was only ever the rate limiter's capacity by coincidence of both
	// living on the same `Config` struct. Partitioned per client (keyed like
	// `ConnectionGuard`'s WS admission) so one client's burst can't deny
	// every other client; see `some_services::rate_limiter` for both fixes.
	let rate_limiter = Arc::new(PartitionedTokenBucketLimiter::new(config.rate_limit, DEFAULT_REFILL_PERIOD_MS));
	file_host::metrics::rate_limit::record_capacity(config.rate_limit);
	versioned_routes = versioned_routes.layer(from_fn_with_state(rate_limiter.clone(), rate_limit_middleware));

	let app = Router::new()
		.nest(API_V1_BASE_PATH, versioned_routes)
		.merge(get_health())
		.merge(get_readiness())
		.merge(get_metrics(metrics_handle))
		.merge(app_state.realtime.ws.clone().router())
		.with_state(app_state.clone());

	// #213 (F2): applied to the fully merged router so `MatchedPath` covers
	// `/health`, `/ready`, `/metrics` and `/ws` alongside the versioned
	// surface, not just `/api/v1/*`. `.layer(...)` only runs for requests
	// axum's router actually matched; `.fallback(unmatched)` is the
	// complementary path for everything else, recorded with a fixed
	// `route="unmatched"` label so an unbounded set of garbage paths can
	// never grow this metric's cardinality.
	let app = app.layer(from_fn(track_http_metrics)).fallback(unmatched);

	let app = app.layer(
		ServiceBuilder::new()
			.layer(TraceLayer::new_for_http())
			.layer(HandleErrorLayer::new(|error: BoxError| async move { handle_tower_error(error).await }))
			.layer(RequestBodyLimitLayer::new(config.clone().max_request_size * 1024 * 1024))
			.layer(ConcurrencyLimitLayer::new(config.clone().max_concurrent_req))
			.layer(TimeoutLayer::new(Duration::from_millis(config.clone().task_timeout_ms)))
			.layer(LoadShedLayer::new())
			.layer(AddExtensionLayer::new(config.clone())),
	);

	// #223 (F3): outermost, so it wraps `RequestBodyLimitLayer` above (and
	// everything else) from the outside — see `track_body_limit_refusals`'s
	// own doc comment for why that layer's own rejection can't be caught via
	// `handle_tower_error`. `axum::middleware::from_fn` doesn't compose
	// cleanly with `HandleErrorLayer` from inside the same `ServiceBuilder`
	// chain (ambiguous `Service` inference), hence applying it as a separate
	// `Router::layer` call rather than folding it into the chain above.
	let app = app.layer(from_fn(track_body_limit_refusals));

	// Only starts when the deployment asked for it *and* has a usable VAPID
	// identity — `AppState::build` has already refused to boot if those two
	// disagree, so reaching here with `nudge_enabled` and no context is
	// impossible rather than merely unlikely.
	//
	// Not a scheduler. The waker runs `WHERE eligible_at <= now` — an index
	// probe over instants the arithmetic already solved — so this interval is
	// the resolution at which decided work is picked up, not a cadence at which
	// anything is decided. See `nudge::waker`.
	if app_state.nudge.is_some() && config.nudge_enabled {
		// One-time reconciliation: a subscription recorded before #278 landed
		// has no reason to be re-sent, so without this pass it would never
		// reach `engagement_gate` at all. See `waker::backfill_first_contact`.
		match nudge::waker::backfill_first_contact(&app_state.core.shared_db).await {
			Ok(seeded) => tracing::info!(seeded, "first-contact backfill complete"),
			Err(err) => tracing::error!(error = %err, "first-contact backfill failed; existing subscribers may stay ungated until next boot"),
		}
		nudge::waker::spawn(&app_state, Duration::from_secs(config.nudge_waker_seconds.max(30)));
	} else {
		tracing::info!("engagement waker is not running; /api/v1/push and /api/v1/signals still work");
	}

	// #227 (C2) / #228 (C3): `connected`/`live`/`subscribed`, guard
	// occupancy, and SQLite pool gauges — unconditional, unlike the nudge
	// waker above, since none of them depend on optional configuration.
	// `file_host::websocket::STALE_TIMEOUT` is the same "recently heard
	// from" window the WS message loop already uses for its own stale
	// check, so `live` here and the connections it actually closes agree on
	// what "recent" means.
	file_host::metrics::periodic::spawn(app_state.clone(), rate_limiter, file_host::websocket::STALE_TIMEOUT, Duration::from_secs(10));

	let listener = TcpListener::bind("0.0.0.0:3000").await?;
	tracing::debug!("listening on {}", listener.local_addr()?);

	// Spawn signal handler task with proper shutdown coordination
	let signal_shutdown_token = shutdown_token.clone();
	tokio::spawn(async move {
		tokio::signal::ctrl_c().await.ok();
		tracing::info!("Received Ctrl+C, initiating shutdown...");
		signal_shutdown_token.cancel();
	});

	// Run server with graceful shutdown
	let server_token = shutdown_token.clone();
	let server = axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>()).with_graceful_shutdown(async move {
		server_token.cancelled().await;
	});

	server.await?;
	tracing::info!("Server stopped");

	// Shutdown with timeout to prevent hanging forever
	tracing::info!("Starting cleanup...");

	let cleanup = async {
		shutdown_token.cancel();

		// Give tasks time to observe cancellation and run Drop cleanup
		tokio::time::sleep(Duration::from_millis(200)).await;

		app_state.core.shared_db.close().await;
		tracing::info!("Database closed");
		app_state.realtime.transport.client().flush().await.ok();
		tracing::info!("Nats channel closed");
		app_state.realtime.ws.shutdown().await;
		tracing::info!("All WebSocket connections cleanup up");

		// Take ownership of OtelGuard for shutdown
		if let Some(guard) = app_state.core.otel_guard.lock().unwrap().take() {
			if let Err(e) = guard.shutdown().await {
				tracing::error!("Failed to shutdown OpenTelemetry: {}", e);
			}
			tracing::info!("OpenTelemetry shutdown");
		}
	};

	// Add a timeout to prevent infinite hang
	match tokio::time::timeout(Duration::from_secs(5), cleanup).await {
		Ok(_) => tracing::info!("Graceful shutdown completed"),
		Err(_) => {
			tracing::error!("Shutdown timeout - forcing exit");
		}
	}

	tracing::info!("Shutdown complete");
	Ok(())
}

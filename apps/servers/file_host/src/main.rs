// These modules live in the library crate (`src/lib.rs`) and are used from
// there rather than re-declared here. Declaring `mod routes;` compiled a second
// copy of the whole tree into this binary, which meant anything the binary did
// not happen to call — the route inventory, for one — tripped `dead_code` under
// `-D warnings` despite being used by the library's other consumers.
use anyhow::Result;
use axum::{error_handling::HandleErrorLayer, middleware::from_fn_with_state, Router};
use clap::Parser;
use file_host::rate_limiter::token_bucket::rate_limit_middleware;
use file_host::routes::{
	audio_files::get_audio,
	db::{mood_events, sessions, tabs},
	gdrive::{get_gdrive_image, write_gdrive_fs},
	github::get_repos,
	health::get_health,
	metrics::get_metrics,
	push::push,
	sheets::get_sheets,
	signals::signals,
	tab_metadata::post_now_playing,
	utterance::post_utterance,
};
use file_host::{error::FileHostError, nudge, perform_health_check, AppState, Config, API_V1_BASE_PATH};
use some_services::rate_limiter::TokenBucketRateLimiter;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use std::{net::SocketAddr, str::FromStr, sync::Arc};
use tokio::{net::TcpListener, time::Duration};
use tokio_util::sync::CancellationToken;
use tower::{limit::ConcurrencyLimitLayer, load_shed::LoadShedLayer, timeout::TimeoutLayer, BoxError, ServiceBuilder};
use tower_http::{add_extension::AddExtensionLayer, limit::RequestBodyLimitLayer, trace::TraceLayer};

async fn handle_tower_error(error: BoxError) -> FileHostError {
	if error.is::<tower::timeout::error::Elapsed>() {
		tracing::warn!("Request timeout: {}", error);
		FileHostError::RequestTimeout
	} else if error.is::<tower::load_shed::error::Overloaded>() {
		tracing::warn!("Service overloaded: {}", error);
		FileHostError::ServiceOverloaded
	} else {
		tracing::error!("Unhandled tower error: {}", error);
		FileHostError::TowerError(error)
	}
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
	let metrics_handle = some_metrics::install()?;

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
		.merge(get_sheets(&config))
		.merge(get_gdrive_image())
		.merge(write_gdrive_fs(&config))
		.merge(get_repos(&config))
		.merge(mood_events(&config))
		.merge(tabs(&config))
		.merge(sessions(&config))
		.merge(push(&config))
		.merge(signals(&config))
		.merge(get_audio(&config))
		.merge(post_now_playing())
		.merge(post_utterance());

	let max_requests = config.clone().max_request_size.try_into()?;
	// TODO: Is this even working! boyo needs to know!
	versioned_routes = versioned_routes.layer(from_fn_with_state(Arc::new(TokenBucketRateLimiter::new(max_requests)), rate_limit_middleware));

	let app = Router::new()
		.nest(API_V1_BASE_PATH, versioned_routes)
		.merge(get_health())
		.merge(get_metrics(metrics_handle))
		.merge(app_state.realtime.ws.clone().router())
		.with_state(app_state.clone());

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
		nudge::waker::spawn(app_state.clone(), Duration::from_secs(config.nudge_waker_seconds.max(30)));
	} else {
		tracing::info!("engagement waker is not running; /api/v1/push and /api/v1/signals still work");
	}

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

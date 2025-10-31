mod handlers;
mod metrics;
mod models;
mod routes;

use crate::routes::{
	audio_files::get_audio, db::mood_events, gdrive::get_gdrive_image, github::get_repos, health::get_health, sheets::get_sheets, tab_metadata::post_now_playing,
	utterance::post_utterance,
};
use anyhow::Result;
use axum::{error_handling::HandleErrorLayer, middleware::from_fn_with_state, routing::get, Router};
use clap::Parser;
use file_host::rate_limiter::token_bucket::rate_limit_middleware;
use file_host::{
	error::{FileHostError, GSheetDeriveError},
	perform_health_check,
	websocket::{Event, EventType, NowPlaying},
	AppState, AudioServiceError, Config, DedupCache, DedupError, UtterancePrompt,
};
use sdk::ReadDrive;
use some_services::rate_limiter::TokenBucketRateLimiter;
use sqlx::SqlitePool;
use std::{net::SocketAddr, sync::Arc};
use tokio::{net::TcpListener, time::Duration};
use tokio_util::sync::CancellationToken;
use tower::{limit::ConcurrencyLimitLayer, load_shed::LoadShedLayer, timeout::TimeoutLayer, BoxError, ServiceBuilder};
use tower_http::{add_extension::AddExtensionLayer, limit::RequestBodyLimitLayer, trace::TraceLayer};
use tracing_subscriber::{filter::EnvFilter, fmt::format::JsonFields, util::SubscriberInitExt, Layer};

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
	dotenv::dotenv().ok();
	let config = Config::parse();

	// Handle health check flag
	if config.health_check {
		return perform_health_check(&config).await;
	}

	let _ = init_tracing(&config);

	let config = Arc::new(config);
	let pool = SqlitePool::connect(&config.database_url).await?;
	let shutdown_token = CancellationToken::new();

	let app_state = AppState::build(config.clone(), pool, shutdown_token.clone()).await?;

	let ws_bridge = Arc::new(app_state.realtime.ws.clone());
	ws_bridge.clone().bridge_obs_events(EventType::ObsStatus);

	let mut protected_routes = Router::new()
		.merge(get_sheets())
		.merge(get_gdrive_image())
		.merge(get_repos())
		.merge(mood_events())
		.merge(get_audio())
		.merge(post_now_playing())
		.merge(get_health())
		.merge(post_utterance());

	let max_requests = config.clone().max_request_size.try_into()?;
	// TODO: Is this even working! boyo needs to know!
	protected_routes = protected_routes.layer(from_fn_with_state(Arc::new(TokenBucketRateLimiter::new(max_requests)), rate_limit_middleware));

	let public_routes = Router::new().route("/metrics", get(metrics::http::metrics_handler));

	let app = Router::new()
		.merge(protected_routes)
		.merge(public_routes)
		.merge(app_state.realtime.ws.clone().router())
		.with_state(app_state.clone());

	let app = app.layer(
		ServiceBuilder::new()
			.layer(axum::middleware::from_fn(metrics::http::metrics_middleware))
			.layer(TraceLayer::new_for_http())
			.layer(HandleErrorLayer::new(|error: BoxError| async move { handle_tower_error(error).await }))
			.layer(RequestBodyLimitLayer::new(config.clone().max_request_size * 1024 * 1024))
			.layer(ConcurrencyLimitLayer::new(config.clone().max_concurrent_req))
			.layer(TimeoutLayer::new(Duration::from_millis(config.clone().task_timeout_ms)))
			.layer(LoadShedLayer::new())
			.layer(AddExtensionLayer::new(config.clone())),
	);

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
		ws_bridge.shutdown().await;
		tracing::info!("WebSocket shutdown complete");

		app_state.core.shared_db.close().await;
		tracing::info!("Database closed");
		transport
			.close_channel(client_key)
			.await
			.map_err(|e| ConnectionError::TransportCloseFailed(e.to_string()))?;
		tracing::info!("Nats channel closed");
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

#[must_use]
pub fn init_tracing(config: &Config) -> Option<()> {
	use std::str::FromStr;
	use tracing_subscriber::layer::SubscriberExt;

	let filter = EnvFilter::from_str(config.rust_log.as_deref()?).unwrap();

	tracing_subscriber::registry()
		.with(if config.log_json {
			Box::new(
				tracing_subscriber::fmt::layer()
					.fmt_fields(JsonFields::default())
					.event_format(tracing_subscriber::fmt::format().json().flatten_event(true).with_span_list(false))
					.with_filter(filter),
			) as Box<dyn Layer<_> + Send + Sync>
		} else {
			Box::new(
				tracing_subscriber::fmt::layer()
					.event_format(tracing_subscriber::fmt::format().pretty())
					.with_filter(filter),
			)
		})
		.init();
	None
}

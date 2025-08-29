mod handlers;
mod metrics;
mod models;
mod routes;
use crate::routes::{
	audio_files::get_audio, gdrive::get_gdrive_image, github::get_repos, health::get_health, sheets::get_sheets, tab_metadata::post_now_playing, utterance::post_utterance,
};
use anyhow::Result;
use axum::{error_handling::HandleErrorLayer, middleware::from_fn_with_state, routing::get, Router};
use clap::Parser;
use file_host::rate_limiter::token_bucket::{rate_limit_middleware, TokenBucketRateLimiter};
use file_host::{
	error::{FileHostError, GSheetDeriveError},
	websocket::{init_websocket, middleware::connection_limit_middleware, ConnectionLimitConfig, ConnectionLimiter, Event, NowPlaying},
	AppState, AudioServiceError, CacheConfig, CacheStore, Config, DedupCache, DedupError, UtterancePrompt,
};
use sdk::{GitHubClient, ReadDrive, ReadSheets};
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
	let _ = init_tracing(&config);

	let config = Arc::new(config);
	let cache_store = CacheStore::new(CacheConfig::from(config.clone()))?;
	let dedup_cache = DedupCache::new(cache_store.into(), config.max_in_flight.clone());

	let secret_file = config.client_secret_file.clone();
	let use_email = config.email_service_url.clone().unwrap_or("".to_string());
	let gsheet_reader = ReadSheets::new(use_email.clone(), secret_file.clone())?;
	let gdrive_reader = ReadDrive::new(use_email.clone(), secret_file.clone())?;
	let github_client = GitHubClient::new(config.github_token.clone())?;
	let ws = init_websocket().await;

	let app_state = AppState {
		dedup_cache: dedup_cache.into(),
		gsheet_reader: gsheet_reader.into(),
		gdrive_reader: gdrive_reader.into(),
		github_client: github_client.into(),
		ws,
		config: config.clone(),
	};

	let ws_state = init_websocket().await;

	// Create cancellation token for coordinated shutdown
	let shutdown_token = CancellationToken::new();
	let shutdown_token_clone = shutdown_token.clone();

	// Create connection limiter with configuration
	let connection_limits = ConnectionLimitConfig {
		max_per_client: 2,
		max_global: 10,
		acquire_timeout: Duration::from_secs(10),
		enable_queuing: true,
		queue_size_per_client: 3,
		max_queue_time: Duration::from_secs(30),
	};

	let connection_limiter = ConnectionLimiter::new(connection_limits);

	// Start cleanup task for inactive client states with cancellation support
	let mut cleanup_handle = {
		let limiter = connection_limiter.clone();
		let token = shutdown_token.clone();
		tokio::spawn(async move {
			let _ = limiter
				.start_cleanup_task_with_cancellation(
					Duration::from_secs(300),  // cleanup every 5 minutes
					Duration::from_secs(3600), // remove clients inactive for 1 hour
					token,
				)
				.await;
		})
	};

	let mut protected_routes = Router::new()
		.merge(get_sheets())
		.merge(get_gdrive_image())
		.merge(get_repos())
		.merge(get_audio())
		.merge(post_now_playing())
		.merge(get_health())
		.merge(post_utterance());

	protected_routes = protected_routes.layer(from_fn_with_state(Arc::new(TokenBucketRateLimiter::new(config.clone())), rate_limit_middleware));

	// Add connection stats endpoint (if needed)
	// let stats_route = Router::new()
	//	.route(
	//		"/connection-stats",
	//		axum::routing::get({
	//			let limiter = connection_limiter.clone();
	//			move || async move {
	//				let stats = limiter.get_stats().await;
	//				axum::Json(stats)
	//			}
	//		}),
	//	)
	//	.with_state(connection_limiter);

	let public_routes = Router::new().route("/metrics", get(metrics::http::metrics_handler));

	let app = Router::new().merge(protected_routes).merge(public_routes).merge(ws_state.router()).with_state(app_state);

	let app = app.layer(
		ServiceBuilder::new()
			.layer(from_fn_with_state(connection_limiter.clone(), connection_limit_middleware))
			.layer(axum::middleware::from_fn(metrics::http::metrics_middleware))
			.layer(TraceLayer::new_for_http())
			.layer(HandleErrorLayer::new(|error: BoxError| async move { handle_tower_error(error).await }))
			.layer(RequestBodyLimitLayer::new(config.clone().max_request_size * 1024 * 1024))
			.layer(ConcurrencyLimitLayer::new(config.clone().max_concurrent_req))
			.layer(TimeoutLayer::new(Duration::from_millis(config.clone().task_timeout_ms)))
			.layer(LoadShedLayer::new())
			.layer(AddExtensionLayer::new(config)),
	);

	let listener = TcpListener::bind("0.0.0.0:3000").await?;
	tracing::debug!("listening on {}", listener.local_addr()?);
	let server = axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>());

	// Spawn signal handler task
	let signal_task = tokio::spawn(async move {
		let _ = tokio::signal::ctrl_c().await;
		tracing::info!("Received shutdown signal");
		shutdown_token_clone.cancel();
	});

	// Main server loop with proper cancellation handling
	tokio::select! {
		result = server => {
			if let Err(e) = result {
				tracing::error!("Server error: {}", e);
			}
			tracing::info!("Server stopped");
		}
		_ = shutdown_token.cancelled() => {
			tracing::info!("Shutdown initiated by signal");
		}
	}

	// Graceful shutdown sequence
	tracing::info!("Starting graceful shutdown...");

	// Cancel the shutdown token to notify all tasks
	shutdown_token.cancel();

	// Give background tasks a moment to clean up
	tokio::select! {
		_ = &mut cleanup_handle => {
			tracing::debug!("Cleanup task finished");
		}
		_ = tokio::time::sleep(Duration::from_secs(10)) => {
			tracing::warn!("Cleanup task didn't finish within timeout, proceeding with shutdown");
			cleanup_handle.abort();
		}
	}

	// Clean up signal handler
	signal_task.abort();

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

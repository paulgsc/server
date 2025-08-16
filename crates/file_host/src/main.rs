mod handlers;
mod metrics;
mod models;
mod routes;
use crate::routes::{gdrive::get_gdrive_image, github::get_repos, health::get_health, sheets::get_sheets, tab_metadata::post_now_playing, utterance::post_utterance};
use anyhow::Result;
use axum::{error_handling::HandleErrorLayer, middleware::from_fn_with_state, routing::get, Router};
use clap::Parser;
use file_host::rate_limiter::token_bucket::{rate_limit_middleware, TokenBucketRateLimiter};
use file_host::{
	error::{FileHostError, GSheetDeriveError},
	websocket::{init_websocket, middleware::connection_limit_middleware, ConnectionLimitConfig, ConnectionLimiter, Event, NowPlaying},
	AppState, CacheConfig, CacheStore, Config, DedupCache, UtterancePrompt,
};
// use obs_websocket::{create_obs_client_with_broadcast, ObsConfig, ObsRequestType, PollingFrequency, RetryConfig};
use sdk::{GitHubClient, ReadDrive, ReadSheets};
use std::{net::SocketAddr, sync::Arc};
use tokio::{net::TcpListener, time::Duration};
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

	// Start cleanup task for inactive client states
	connection_limiter.clone().start_cleanup_task(
		Duration::from_secs(300),  // cleanup every 5 minutes
		Duration::from_secs(3600), // remove clients inactive for 1 hour
	);

	// let obs_config = ObsConfig {
	// 	host: context.obs_host.clone(),
	// 	port: 4455,
	// 	password: context.obs_password.clone(),
	// };

	// let polling_requests = [
	// 	// High frequency - every second
	// 	(ObsRequestType::StreamStatus, PollingFrequency::High),
	// 	(ObsRequestType::RecordStatus, PollingFrequency::High),
	// 	(ObsRequestType::CurrentScene, PollingFrequency::High),
	// 	(ObsRequestType::Stats, PollingFrequency::High),
	// 	// Medium frequency - every 5 seconds
	// 	(ObsRequestType::SceneList, PollingFrequency::Medium),
	// 	(ObsRequestType::SourcesList, PollingFrequency::Medium),
	// 	(ObsRequestType::InputsList, PollingFrequency::Medium),
	// 	(ObsRequestType::VirtualCamStatus, PollingFrequency::Medium),
	// 	(ObsRequestType::InputMute("Desktop Audio".to_string()), PollingFrequency::Medium),
	// 	(ObsRequestType::InputVolume("Microphone".to_string()), PollingFrequency::Medium),
	// 	// Low frequency - every 30 seconds
	// 	(ObsRequestType::ProfileList, PollingFrequency::Low),
	// 	(ObsRequestType::CurrentProfile, PollingFrequency::Low),
	// 	(ObsRequestType::Version, PollingFrequency::Low),
	// ];

	// let retry_config = RetryConfig::default();

	// let client = Arc::new(create_obs_client_with_broadcast(obs_config));
	// let obs_client = client.clone();
	// let obs_handle = obs_client.start(Box::new(polling_requests), retry_config);

	// let obs_monitor = {
	// 	let obs_task_client = client.clone();
	// 	tokio::spawn(async move {
	// 		// Log the startup

	// 		// Keep the handle alive and monitor it
	// 		tokio::select! {
	// 			_ = tokio::signal::ctrl_c() => {
	// 				tracing::info!("Received shutdown signal, stopping OBS handler...");
	// 				obs_task_client.disconnect().await;
	// 				obs_handle.stop().await;
	// 			}
	// 			_ = async {
	// 				// Monitor the handle and restart if it fails
	// 				loop {
	// 					if !obs_handle.is_running() {
	// 						tracing::error!("OBS background handler stopped unexpectedly");
	// 						break;
	// 					}
	// 					tracing::warn!("going to sleep for 30s");
	// 					tokio::time::sleep(Duration::from_secs(30)).await;
	// 				}
	// 			} => {
	// 				tracing::error!("OBS background handler monitoring ended");
	// 			}
	// 		}
	// 	})
	// };

	// ws_state.bridge_obs_events(client.clone());

	let mut protected_routes = Router::new()
		.merge(get_sheets())
		.merge(get_gdrive_image())
		.merge(get_repos())
		.merge(post_now_playing())
		.merge(get_health())
		.merge(post_utterance());

	protected_routes = protected_routes.layer(from_fn_with_state(Arc::new(TokenBucketRateLimiter::new(config.clone())), rate_limit_middleware));

	// Add connection stats endpoint
	// let stats_route = Router::new()
	// 	.route(
	// 		"/connection-stats",
	// 		axum::routing::get({
	// 			let limiter = connection_limiter.clone();
	// 			move || async move {
	// 				let stats = limiter.get_stats().await;
	// 				axum::Json(stats)
	// 			}
	// 		}),
	// 	)
	// 	.with_state(connection_limiter);

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

	tokio::select! {
		result = server => {
			if let Err(e) = result {
				tracing::error!("Server error: {}", e);
			}
		}
		_ = tokio::signal::ctrl_c() => {
			tracing::info!("Received shutdown signal");
		}
	}

	// Clean shutdown
	tracing::info!("Shutting down...");
	// client.disconnect().await;
	// obs_monitor.abort();
	// let _ = obs_monitor.await;

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

#[cfg(test)]
mod tests {
	use super::*;
	use axum::{
		body::Body,
		extract::ConnectInfo,
		http::{Request, StatusCode},
	};
	use std::net::SocketAddr;
	use tower::ServiceExt;

	#[tokio::test]
	async fn test_rate_limiter_without_server() {
		dotenv::dotenv().ok();
		let context = Arc::new(Config::parse());

		// Create a test app
		let app = Router::new().route("/test", get(|| async { "Success" })).layer(axum::middleware::from_fn_with_state(
			Arc::new(SlidingWindowRateLimiter::new(context.clone())),
			rate_limit_middleware,
		));

		let app_service = app.clone().into_service();
		// Make requests quickly
		let remote_addr = "127.0.0.1:12345".parse::<SocketAddr>().unwrap();

		// Test with same IP (should trigger rate limit)
		for i in 1..=12 {
			// Assuming limit is 10
			let request = Request::builder().uri("/test").extension(ConnectInfo(remote_addr)).body(Body::empty()).unwrap();

			let response = app_service.clone().oneshot(request).await.unwrap();

			if i <= 10 {
				assert_eq!(response.status(), StatusCode::OK);
			} else {
				assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
			}
		}
	}
}

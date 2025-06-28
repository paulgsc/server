mod handlers;
mod metrics;
mod models;
mod routes;
use crate::routes::{gdrive::get_gdrive_image, github::get_repos, sheets::get_sheets, tab_metadata::post_now_playing};
use anyhow::Result;
use axum::{routing::get, Router};
use clap::Parser;
use file_host::rate_limiter::sliding_window::{rate_limit_middleware, SlidingWindowRateLimiter};
use file_host::{
	error::{FileHostError, GSheetDeriveError},
	record_cache_op, timed_operation,
	websocket::init_websocket,
	CacheStore,
};
use file_host::{AppState, Config};
use obs_websocket::{create_obs_client_with_broadcast, ObsConfig, ObsRequestType, PollingFrequency, RetryConfig};
use std::sync::Arc;
use tokio::net::TcpListener;
use tower::ServiceBuilder;
use tower_http::add_extension::AddExtensionLayer;
use tower_http::trace::TraceLayer;
use tracing_subscriber::{filter::EnvFilter, fmt::format::JsonFields, util::SubscriberInitExt, Layer};

#[tokio::main]
async fn main() -> Result<()> {
	dotenv::dotenv().ok();
	let config = Config::parse();
	let _ = init_tracing(&config);

	let context = Arc::new(config);

	let ws_state = init_websocket().await;

	let obs_config = ObsConfig {
		host: context.obs_host.clone(),
		port: 4455,
		password: context.obs_password.clone(),
	};

	let polling_requests = [
		// High frequency - every second
		(ObsRequestType::StreamStatus, PollingFrequency::High),
		(ObsRequestType::RecordStatus, PollingFrequency::High),
		(ObsRequestType::CurrentScene, PollingFrequency::High),
		(ObsRequestType::Stats, PollingFrequency::High),
		// Medium frequency - every 5 seconds
		(ObsRequestType::SceneList, PollingFrequency::Medium),
		(ObsRequestType::SourcesList, PollingFrequency::Medium),
		(ObsRequestType::InputsList, PollingFrequency::Medium),
		(ObsRequestType::VirtualCamStatus, PollingFrequency::Medium),
		(ObsRequestType::InputMute("Desktop Audio".to_string()), PollingFrequency::Medium),
		(ObsRequestType::InputVolume("Microphone".to_string()), PollingFrequency::Medium),
		// Low frequency - every 30 seconds
		(ObsRequestType::ProfileList, PollingFrequency::Low),
		(ObsRequestType::CurrentProfile, PollingFrequency::Low),
		(ObsRequestType::Version, PollingFrequency::Low),
	];

	let retry_config = RetryConfig::default();

	let client = Arc::new(create_obs_client_with_broadcast(obs_config));
	let obs_client = client.clone();

	let obs_handle = {
		tokio::spawn(async move {
			let handle = obs_client.start(Box::new(polling_requests), retry_config);

			// Log the startup
			tracing::info!("OBS background handler started");

			// Keep the handle alive and monitor it
			tokio::select! {
				_ = tokio::signal::ctrl_c() => {
					tracing::info!("Received shutdown signal, stopping OBS handler...");
					handle.stop().await;
				}
				_ = async {
					// Monitor the handle and restart if it fails
					loop {
						if !handle.is_running() {
							tracing::error!("OBS background handler stopped unexpectedly");
							break;
						}
						tokio::time::sleep(std::time::Duration::from_secs(30)).await;
					}
				} => {
					tracing::error!("OBS background handler monitoring ended");
				}
			}
		})
	};

	ws_state.bridge_obs_events(client.clone());

	let mut protected_routes = Router::new()
		.merge(get_sheets(context.clone())?)
		.merge(get_gdrive_image(context.clone())?)
		.merge(get_repos(context.clone())?)
		.merge(post_now_playing(context.clone())?);

	protected_routes = protected_routes.layer(axum::middleware::from_fn_with_state(
		Arc::new(SlidingWindowRateLimiter::new(context.clone())),
		rate_limit_middleware,
	));

	let public_routes = Router::new().route("/metrics", get(metrics::metrics_handler));

	let app = Router::new()
		.merge(protected_routes)
		.merge(public_routes)
		.merge(ws_state.router())
		.layer(axum::middleware::from_fn(metrics::metrics_middleware));

	let app = app.layer(ServiceBuilder::new().layer(AddExtensionLayer::new(context)).layer(TraceLayer::new_for_http()));
	let listener = TcpListener::bind("0.0.0.0:3000").await?;
	tracing::debug!("listening on {}", listener.local_addr()?);
	let server = axum::serve(listener, app);

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
	obs_handle.abort();
	let _ = obs_handle.await;

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

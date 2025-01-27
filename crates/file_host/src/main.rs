pub mod handlers;
pub mod routes;
use crate::routes::get_attributions;
use anyhow::Result;
use axum::Router;
use clap::Parser;
use nest::config::Config;
use nest::init_tracing;
use std::sync::Arc;
use tokio::net::TcpListener;
use tower::ServiceBuilder;
use tower_http::add_extension::AddExtensionLayer;
use tower_http::trace::TraceLayer;

#[tokio::main]
async fn main() -> Result<()> {
	dotenv::dotenv().ok();
	let config = Config::parse();
	let _ = init_tracing(&config);

	let context = Arc::new(config);

	let mut app = Router::new();

	app = app.merge(get_attributions());

	let app = app.layer(ServiceBuilder::new().layer(AddExtensionLayer::new(context)).layer(TraceLayer::new_for_http()));
	let listener = TcpListener::bind("127.0.0.1:8000").await?;
	tracing::debug!("listening on {}", listener.local_addr()?);
	axum::serve(listener, app).await?;
	Ok(())
}

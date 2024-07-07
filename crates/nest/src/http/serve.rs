use crate::config::Config;
use axum::Router;
use sqlx::SqlitePool;
use std::sync::Arc;
use tokio::net::TcpListener;
use tower::ServiceBuilder;
use tower_http::add_extension::AddExtensionLayer;
use tower_http::trace::TraceLayer;

use crate::http::routes;

#[derive(Clone)]
struct ApiContext {
	config: Arc<Config>,
	db: SqlitePool,
}

pub async fn serve(config: Config, db: SqlitePool) {
	let app = api_router().layer(
		ServiceBuilder::new()
			.layer(AddExtensionLayer::new(ApiContext { config: Arc::new(config), db }))
			// Enables logging. Use `RUST_LOG=tower_http=debug`
			.layer(TraceLayer::new_for_http()),
	);

	// run it with hyper
	let listener = TcpListener::bind("127.0.0.1:8000").await.unwrap();
	tracing::debug!("listening on {}", listener.local_addr().unwrap());
	axum::serve(listener, app).await.unwrap();
}

fn api_router() -> Router {
	// This is the order that the modules were authored in.
	routes::browser_tabs::routes()
}

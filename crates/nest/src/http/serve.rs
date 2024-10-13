use crate::config::Config;
use axum::Router;
use sqlx::SqlitePool;
use std::sync::Arc;
use std::collections::HashMap;
use tokio::net::TcpListener;
use tower::ServiceBuilder;
use tower_http::add_extension::AddExtensionLayer;
use tower_http::trace::TraceLayer;

use crate::http::routes;

#[derive(Clone)]
struct ApiContext {
	config: Arc<Config>,
    dbs: HashMap<String, SqlitePool>,
}

pub async fn serve(config: Config, dbs: HashMap<String, SqlitePool>) {
	let app = api_router(dbs.clone()).layer(
		ServiceBuilder::new()
			.layer(AddExtensionLayer::new(ApiContext { config: Arc::new(config), dbs }))
			// Enables logging. Use `RUST_LOG=tower_http=debug`
			.layer(TraceLayer::new_for_http()),
	);

	// run it with hyper
	let listener = TcpListener::bind("127.0.0.1:8000").await.unwrap();
	tracing::debug!("listening on {}", listener.local_addr().unwrap());
	axum::serve(listener, app).await.unwrap();
}

fn api_router(dbs: HashMap<String, SqlitePool>) -> Router {
    let mut router = Router::new();

    for (db_name, db_pool) in dbs {
        router = router.merge(routes::browser_tabs::routes(db_name, db_pool));
    }

    router
}


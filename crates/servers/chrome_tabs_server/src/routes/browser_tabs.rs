use crate::handlers::{create_tab, delete_tab, get_all_tabs, update_tab};
use axum::routing::{delete, get, post, put};
use axum::{http::Method, Router};
use nest::http::Error;
use nest::{MigrationHandler, MultiDbHandler};
use sqlx::SqlitePool;
use std::future::Future;
use std::pin::Pin;
use tower_http::cors::{Any, CorsLayer};

pub struct BrowserTabsHandler;

impl MultiDbHandler for BrowserTabsHandler {
	fn create_routes(&self, db_name: &str, pool: SqlitePool) -> Router {
		let cors = CorsLayer::new().allow_methods([Method::GET, Method::POST, Method::PUT, Method::DELETE]).allow_origin(Any);

		Router::new()
			.route(&format!("/api/{}/chrome/tabs/update", db_name), put(update_tab))
			.route(&format!("/api/{}/chrome/tabs/delete", db_name), delete(delete_tab))
			.route(&format!("/api/{}/chrome/tabs/all", db_name), get(get_all_tabs))
			.route(&format!("/api/{}/chrome/tabs/post", db_name), post(create_tab))
			.layer(cors)
			.with_state(pool)
	}
}

pub struct BrowserTabsMigrationHandler;

impl MigrationHandler for BrowserTabsMigrationHandler {
	fn run_migrations<'a>(&'a self, pool: &'a SqlitePool) -> Pin<Box<dyn Future<Output = Result<(), Error>> + Send + 'a>> {
		Box::pin(async move {
			sqlx::migrate!().run(pool).await?;
			Ok(())
		})
	}
}

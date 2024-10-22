use crate::routes::play_type_routes as routes;
use axum::routing::{delete, get, post, put};
use axum::{http::Method, Router};
use nest::http::Error;
use nest::{MigrationHandler, MultiDbHandler};
use sqlx::SqlitePool;
use std::future::Future;
use std::pin::Pin;
use tower_http::cors::{Any, CorsLayer};

pub struct PlayTypeHandlers;

impl MultiDbHandler for PlayTypeHandlers {
	fn create_routes(&self, db_name: &str, pool: SqlitePool) -> Router {
		let cors = CorsLayer::new().allow_methods([Method::GET, Method::POST, Method::PUT, Method::DELETE]).allow_origin(Any);

		Router::new()
			.route(&format!("/api/{}/play_type/update", db_name), put(routes::update))
			.route(&format!("/api/{}/play_type/delete", db_name), delete(routes::delete))
			.route(&format!("/api/{}/play_type/post", db_name), post(routes::create))
			.route(&format!("/api/{}/play_type/get", db_name), get(routes::get))
			.layer(cors)
			.with_state(pool)
	}
}

pub struct PlayTypeMigrationHandler;

impl MigrationHandler for PlayTypeMigrationHandler {
	fn run_migrations<'a>(&'a self, pool: &'a SqlitePool) -> Pin<Box<dyn Future<Output = Result<(), Error>> + Send + 'a>> {
		Box::pin(async move {
			sqlx::migrate!().run(pool).await?;
			Ok(())
		})
	}
}

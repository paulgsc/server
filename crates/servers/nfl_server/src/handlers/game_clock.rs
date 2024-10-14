use crate::routes;
use axum::routing::{delete, get, post, put};
use axum::{http::Method, Router};
use nest::{MultiDbHandler, MigrationHandler};
use nest::http::Error;
use sqlx::SqlitePool;
use std::future::Future;
use std::pin::Pin;
use tower_http::cors::{Any, CorsLayer};

pub struct GameClockHandlers;

impl MultiDbHandler for GameClockHandlers {
	fn create_routes(&self, db_name: &str, pool: SqlitePool) -> Router {
		let cors = CorsLayer::new().allow_methods([Method::GET, Method::POST, Method::PUT, Method::DELETE]).allow_origin(Any);

		Router::new()
			.route(&format!("/api/{}/game_clock/update", db_name), put(routes::update))
			.route(&format!("/api/{}/game_clock/delete", db_name), delete(routes::delete))
			.route(&format!("/api/{}/game_clock/all", db_name), get(routes::get))
			.route(&format!("/api/{}/game_clock/post", db_name), post(routes::create))
			.layer(cors)
			.with_state(pool)
	}
}

pub struct GameClockMigrationHandler;

impl MigrationHandler for GameClockMigrationHandler {
    fn run_migrations<'a>(&'a self, pool: &'a SqlitePool) -> Pin<Box<dyn Future<Output = Result<(), Error>> + Send + 'a>> {
        Box::pin(async move {
            sqlx::migrate!().run(pool).await?;
            Ok(())
        })
    }
}

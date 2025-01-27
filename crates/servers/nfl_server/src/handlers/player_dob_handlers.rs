use crate::routes::player_dob_routes as routes;
use axum::routing::{delete, get, post, put};
use axum::{http::Method, Router};
use nest::http::Error;
use nest::{MigrationHandler, MultiDbHandler};
use sqlx::SqlitePool;
use std::future::Future;
use std::pin::Pin;
use tower_http::cors::{Any, CorsLayer};

pub struct PlayerDOBHandlers;

impl MultiDbHandler for PlayerDOBHandlers {
	fn create_routes(&self, db_name: &str, pool: Option<SqlitePool>) -> Router {
		println!("routes set for player_dob with db_name: {}", &db_name);

		let cors = CorsLayer::new().allow_methods([Method::GET, Method::POST, Method::PUT, Method::DELETE]).allow_origin(Any);
		let pool = pool.unwrap();

		Router::new()
			.route(&format!("/api/{}/player_dob/update/{}", db_name, "{id}"), put(routes::update))
			.route(&format!("/api/{}/player_dob/delete/{}", db_name, "{id}"), delete(routes::delete))
			.route(&format!("/api/{}/player_dob/{}", db_name, "{id}"), get(routes::get))
			.route(&format!("/api/{}/player_dob", db_name), post(routes::create))
			.layer(cors)
			.with_state(pool)
	}
}

pub struct PlayerDOBMigrationHandler;

impl MigrationHandler for PlayerDOBMigrationHandler {
	fn run_migrations<'a>(&'a self, pool: &'a SqlitePool) -> Pin<Box<dyn Future<Output = Result<(), Error>> + Send + 'a>> {
		Box::pin(async move {
			sqlx::migrate!().run(pool).await?;
			Ok(())
		})
	}
}

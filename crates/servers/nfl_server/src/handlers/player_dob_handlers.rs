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
	fn create_routes(&self, db_name: &str, pool: SqlitePool) -> Router {
		println!("routes set for player_dob with db_name: {}", &db_name);

		let cors = CorsLayer::new().allow_methods([Method::GET, Method::POST, Method::PUT, Method::DELETE]).allow_origin(Any);

		Router::new()
			.route(&format!("/api/{}/player_dob/update/{}", db_name, "{id}"), put(routes::update))
			.route(&format!("/api/{}/player_dob/delete/{}", db_name, "{id}"), delete(routes::delete))
			.route(&format!("/api/{}/player_dob/{}", db_name, "{id}"), get(routes::get))
			.route(&format!("/api/{}/player_dob", db_name), post(routes::create))
			.route(&format!("/api/{}/player_dob/age_range", db_name), get(routes::get_by_age_range))
			.route(&format!("/api/{}/player_dob/cleanup", db_name), delete(routes::delete_older_than))
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

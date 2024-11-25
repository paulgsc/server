use crate::common::nfl_server_error::NflServerError as Error;
use crate::common::CrudOperations;
use crate::models::game_clock::{CreateGameClock, GameClock};
use axum::{
	extract::{Path, State},
	Json,
};
use nest::http::Error as NestError;
use sqlx::SqlitePool;

pub async fn create(State(pool): State<SqlitePool>, Json(payload): Json<CreateGameClock>) -> Result<Json<i64>, Error> {
	if !payload.is_valid() {
		return Err(Error::NestError(NestError::unprocessable_entity(vec![("game_clock", "Invalid game clock values")])));
	}
	let game_clock = GameClock::create(&pool, payload).await?;
	Ok(Json(game_clock))
}

pub async fn get(State(pool): State<SqlitePool>, Path(id): Path<i64>) -> Result<Json<GameClock>, Error> {
	let game_clock = GameClock::get(&pool, id).await?;
	Ok(Json(game_clock))
}

pub async fn update(State(pool): State<SqlitePool>, Path(id): Path<i64>, Json(payload): Json<CreateGameClock>) -> Result<Json<()>, Error> {
	if !payload.is_valid() {
		return Err(Error::NestError(NestError::unprocessable_entity(vec![("game_clock", "Invalid game clock values")])));
	}
	let game_clock = GameClock::update(&pool, id, payload).await?;
	Ok(Json(game_clock))
}

pub async fn delete(State(pool): State<SqlitePool>, Path(id): Path<i64>) -> Result<(), Error> {
	GameClock::delete(&pool, id).await
}

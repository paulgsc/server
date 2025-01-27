use crate::common::nfl_server_error::NflServerError as Error;
use crate::common::CrudOperations;
use crate::models::player_dob::PlayerDOB;
use axum::{
	extract::{Path, State},
	Json,
};
use nest::http::Error as NestError;
use sqlx::SqlitePool;

pub async fn create(State(pool): State<SqlitePool>, Json(payload): Json<PlayerDOB>) -> Result<Json<i64>, Error> {
	if !payload.is_valid() {
		return Err(Error::NestError(NestError::unprocessable_entity(vec![("player_dob", "Invalid date of birth values")])));
	}

	let player_dob = PlayerDOB::create(&pool, payload).await?;
	Ok(Json(player_dob))
}

pub async fn get(State(pool): State<SqlitePool>, Path(id): Path<i64>) -> Result<Json<PlayerDOB>, Error> {
	let player_dob = PlayerDOB::get(&pool, id).await?;
	Ok(Json(player_dob))
}

pub async fn update(State(pool): State<SqlitePool>, Path(id): Path<i64>, Json(payload): Json<PlayerDOB>) -> Result<Json<()>, Error> {
	if !payload.is_valid() {
		return Err(Error::NestError(NestError::unprocessable_entity(vec![("player_dob", "Invalid date of birth values")])));
	}

	let player_dob = PlayerDOB::update(&pool, id, payload).await?;
	Ok(Json(player_dob))
}

pub async fn delete(State(pool): State<SqlitePool>, Path(id): Path<i64>) -> Result<(), Error> {
	PlayerDOB::delete(&pool, id).await
}

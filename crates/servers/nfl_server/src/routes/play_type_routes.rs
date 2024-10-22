use crate::common::CrudOperations;
use crate::models::play_type::{CreatePlayType, PlayTypeRecord};
use axum::{
	extract::{Path, State},
	Json,
};
use crate::common::nfl_server_error::NflServerError as Error;
use sqlx::SqlitePool;

pub async fn create(State(pool): State<SqlitePool>, Json(payload): Json<CreatePlayType>) -> Result<Json<PlayTypeRecord>, Error> {
	let play_type = PlayTypeRecord::create(&pool, payload).await?;
	Ok(Json(play_type))
}

pub async fn get(State(pool): State<SqlitePool>, Path(id): Path<i64>) -> Result<Json<PlayTypeRecord>, Error> {
	let play_type = PlayTypeRecord::get(&pool, id).await?;
	Ok(Json(play_type))
}

pub async fn update(State(pool): State<SqlitePool>, Path(id): Path<i64>, Json(payload): Json<CreatePlayType>) -> Result<Json<PlayTypeRecord>, Error> {
	let play_type = PlayTypeRecord::update(&pool, id, payload).await?;
	Ok(Json(play_type))
}

pub async fn delete(State(pool): State<SqlitePool>, Path(id): Path<i64>) -> Result<(), Error> {
	PlayTypeRecord::delete(&pool, id).await
}

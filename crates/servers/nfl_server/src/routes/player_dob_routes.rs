use crate::common::nfl_server_error::NflServerError as Error;
use crate::common::CrudOperations;
use crate::models::player_dob::{AgeOperations, CreatePlayerDOB, PlayerDOB};
use axum::{
	extract::{Path, Query, State},
	Json,
};
use chrono::NaiveDate;
use nest::http::Error as NestError;
use serde::Deserialize;
use sqlx::SqlitePool;

pub async fn create(State(pool): State<SqlitePool>, Json(payload): Json<CreatePlayerDOB>) -> Result<Json<PlayerDOB>, Error> {
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

pub async fn update(State(pool): State<SqlitePool>, Path(id): Path<i64>, Json(payload): Json<CreatePlayerDOB>) -> Result<Json<PlayerDOB>, Error> {
	if !payload.is_valid() {
		return Err(Error::NestError(NestError::unprocessable_entity(vec![("player_dob", "Invalid date of birth values")])));
	}

	let player_dob = PlayerDOB::update(&pool, id, payload).await?;
	Ok(Json(player_dob))
}

pub async fn delete(State(pool): State<SqlitePool>, Path(id): Path<i64>) -> Result<(), Error> {
	PlayerDOB::delete(&pool, id).await
}

#[derive(Debug, Deserialize)]
pub struct AgeRangeQuery {
	min_age: u16,
	max_age: u16,
	#[serde(default = "default_reference_date")]
	reference_date: NaiveDate,
}

fn default_reference_date() -> NaiveDate {
	chrono::Local::now().date_naive()
}

pub async fn get_by_age_range(State(pool): State<SqlitePool>, Query(query): Query<AgeRangeQuery>) -> Result<Json<Vec<PlayerDOB>>, Error> {
	if query.min_age > query.max_age {
		return Err(Error::NestError(NestError::unprocessable_entity(vec![(
			"age_range",
			"Minimum age cannot be greater than maximum age",
		)])));
	}

	let players = PlayerDOB::get_by_age_range(&pool, query.min_age, query.max_age, query.reference_date).await?;

	Ok(Json(players))
}

#[derive(Debug, Deserialize)]
pub struct DeleteOlderThanQuery {
	cutoff_date: NaiveDate,
}

pub async fn delete_older_than(State(pool): State<SqlitePool>, Query(query): Query<DeleteOlderThanQuery>) -> Result<(), Error> {
	PlayerDOB::delete_older_than(&pool, query.cutoff_date).await
}

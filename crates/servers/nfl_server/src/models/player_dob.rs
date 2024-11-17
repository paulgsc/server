use crate::common::nfl_server_error::NflServerError as Error;
use crate::common::CrudOperations;
use async_trait::async_trait;
use chrono::{Datelike, NaiveDate};
use nest::http::Error as NestError;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use std::convert::TryFrom;

const YEAR_MASK: u16 = 0b1111111_0000_00000;
const MONTH_MASK: u16 = 0b0000000_1111_00000;
const DAY_MASK: u16 = 0b0000000_0000_11111;
const BASE_YEAR: i32 = 1970;

#[derive(Debug, Serialize, Deserialize)]
pub struct PlayerDOB {
	pub id: i64,
	pub dob_encoded: u16,
}

#[derive(Debug)]
struct PlayerDOBRecord {
	id: i64,
	dob_encoded: i64,
}

impl TryFrom<PlayerDOBRecord> for PlayerDOB {
	type Error = Error;

	fn try_from(row: PlayerDOBRecord) -> Result<Self, Self::Error> {
		Ok(PlayerDOB {
			id: row.id,
			dob_encoded: u16::try_from(row.dob_encoded).map_err(|_| Error::NestError(NestError::unprocessable_entity(vec![("dob", "Invalid date of birth encoding")])))?,
		})
	}
}

#[derive(Debug, Deserialize)]
pub struct CreatePlayerDOB {
	pub year: i32,
	pub month: u32,
	pub day: u32,
}

impl PlayerDOB {
	fn encode_date(date: NaiveDate) -> u16 {
		let year_offset = (date.year() - BASE_YEAR) as u16;
		let month = date.month() as u16;
		let day = date.day() as u16;

		(year_offset << 9) | (month << 5) | day
	}

	pub fn decode_date(&self) -> NaiveDate {
		let year = ((self.dob_encoded & YEAR_MASK) >> 9) as i32 + BASE_YEAR;
		let month = ((self.dob_encoded & MONTH_MASK) >> 5) as u32;
		let day = (self.dob_encoded & DAY_MASK) as u32;

		NaiveDate::from_ymd_opt(year, month, day).unwrap()
	}

	pub fn is_valid(&self) -> bool {
		let date = self.decode_date();
		date.year() >= BASE_YEAR && date.year() <= (BASE_YEAR + 127)
	}
}

impl CreatePlayerDOB {
	pub fn is_valid(&self) -> bool {
		if let Some(_) = NaiveDate::from_ymd_opt(self.year, self.month, self.day) {
			return self.year >= BASE_YEAR && self.year <= (BASE_YEAR + 127);
		}
		false
	}

	fn to_encoded(&self) -> Option<u16> {
		NaiveDate::from_ymd_opt(self.year, self.month, self.day).map(PlayerDOB::encode_date)
	}
}

#[async_trait]
pub trait AgeOperations {
	async fn get_by_age_range(pool: &SqlitePool, min_age: u16, max_age: u16, reference_date: NaiveDate) -> Result<Vec<Self>, Error>
	where
		Self: Sized;

	async fn delete_older_than(pool: &SqlitePool, cutoff_date: NaiveDate) -> Result<(), Error>;
}

#[async_trait]
impl CrudOperations<PlayerDOB, CreatePlayerDOB> for PlayerDOB {
	async fn create(pool: &SqlitePool, item: CreatePlayerDOB) -> Result<PlayerDOB, Error> {
		let dob_encoded = item
			.to_encoded()
			.ok_or_else(|| Error::NestError(NestError::unprocessable_entity(vec![("dob", "Invalid dob values")])))?;
		let dob_encoded_i64: i64 = i64::from(dob_encoded);

		let result = sqlx::query!("INSERT INTO player_dobs (dob_encoded) VALUES (?)", dob_encoded_i64)
			.execute(pool)
			.await
			.map_err(NestError::from)?;

		Ok(Self {
			id: result.last_insert_rowid(),
			dob_encoded,
		})
	}

	async fn batch_create(pool: &SqlitePool, items: &[CreatePlayerDOB]) -> Result<Vec<PlayerDOB>, Error> {
		let mut tx = pool.begin().await.map_err(NestError::from)?;
		let mut created_dobs = Vec::with_capacity(items.len());

		for item in items {
			let dob_encoded = item
				.to_encoded()
				.ok_or_else(|| Error::NestError(NestError::unprocessable_entity(vec![("dob", "Invalid dob values")])))?;
			let dob_encoded_i64: i64 = i64::from(dob_encoded);

			let result = sqlx::query!("INSERT INTO player_dobs (dob_encoded) VALUES (?)", dob_encoded_i64)
				.execute(&mut *tx)
				.await
				.map_err(NestError::from)?;

			created_dobs.push(PlayerDOB {
				id: result.last_insert_rowid(),
				dob_encoded,
			});
		}

		tx.commit().await.map_err(NestError::from)?;
		Ok(created_dobs)
	}

	async fn get(pool: &SqlitePool, id: i64) -> Result<Self, Error> {
		let dob = sqlx::query_as!(PlayerDOBRecord, "SELECT id, dob_encoded FROM player_dobs WHERE id = ?", id)
			.fetch_optional(pool)
			.await
			.map_err(NestError::from)?
			.ok_or(Error::NestError(NestError::NotFound))?;

		Self::try_from(dob)
	}

	async fn update(pool: &SqlitePool, id: i64, item: CreatePlayerDOB) -> Result<PlayerDOB, Error> {
		let dob_encoded = item
			.to_encoded()
			.ok_or_else(|| Error::NestError(NestError::unprocessable_entity(vec![("dob", "Invalid dob values")])))?;
		let dob_encoded_i64: i64 = i64::from(dob_encoded);

		let result = sqlx::query!("UPDATE player_dobs SET dob_encoded = ? WHERE id = ?", dob_encoded_i64, id)
			.execute(pool)
			.await
			.map_err(NestError::from)?;

		if result.rows_affected() == 0 {
			return Err(Error::NestError(NestError::NotFound));
		}

		Ok(PlayerDOB { id, dob_encoded })
	}

	async fn delete(pool: &SqlitePool, id: i64) -> Result<(), Error> {
		let result = sqlx::query!("DELETE FROM player_dobs WHERE id = ?", id).execute(pool).await.map_err(NestError::from)?;

		if result.rows_affected() == 0 {
			return Err(Error::NestError(NestError::NotFound));
		}

		Ok(())
	}
}

#[async_trait]
impl AgeOperations for PlayerDOB {
	#[allow(unused_variables)]
	async fn get_by_age_range(pool: &SqlitePool, min_age: u16, max_age: u16, reference_date: NaiveDate) -> Result<Vec<Self>, Error> {
		// let days = chrono::Days::new((min_age as u64) * 365);
		// let max_date = reference_date
		// 	.checked_sub_days(days)
		// 	.ok_or_else(|| Error::NestError(NestError::unprocessable_entity(vec![("age_range", "Invalid age calculation")])))?;

		// let days = chrono::Days::new((max_age as u64) * 365);
		// let min_date = reference_date
		// 	.checked_sub_days(days)
		// 	.ok_or_else(|| Error::NestError(NestError::unprocessable_entity(vec![("age_range", "Invalid age calculation")])))?;

		// let max_encoded = Self::encode_date(max_date);
		// let min_encoded = Self::encode_date(min_date);
		// let max_encoded_i64: i64 = i64::from(max_encoded);
		// let min_encoded_i64: i64 = i64::from(min_encoded);

		// TODO: This is broken, query all without condition for now
		let ages = sqlx::query_as!(
			PlayerDOBRecord,
			r#"
            SELECT id, dob_encoded
            FROM player_dobs
            "#,
		)
		.fetch_all(pool)
		.await
		.map_err(NestError::from)?;

		Ok(ages.into_iter().map(Self::try_from).collect::<Result<Vec<_>, _>>()?)
	}

	async fn delete_older_than(pool: &SqlitePool, cutoff_date: NaiveDate) -> Result<(), Error> {
		let cutoff_encoded = Self::encode_date(cutoff_date);

		sqlx::query!(
			r#"
            DELETE FROM player_dobs
            WHERE dob_encoded < ?
            "#,
			cutoff_encoded
		)
		.execute(pool)
		.await
		.map_err(NestError::from)?;

		Ok(())
	}
}

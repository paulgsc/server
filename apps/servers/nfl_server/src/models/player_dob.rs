use crate::common::nfl_server_error::NflServerError as Error;
use crate::common::CrudOperations;
use async_trait::async_trait;
use chrono::{Datelike, NaiveDate};
use nest::http::Error as NestError;
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};

const YEAR_MASK: u16 = 0b1111111_0000_00000;
const MONTH_MASK: u16 = 0b0000000_1111_00000;
const DAY_MASK: u16 = 0b0000000_0000_11111;
const BASE_YEAR: i32 = 1970;

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct PlayerDOB {
	pub id: i64,
	pub dob_encoded: i64,
}

impl PlayerDOB {
	#[allow(dead_code)]
	fn encode_date(date: NaiveDate) -> u16 {
		let year_offset = (date.year() - BASE_YEAR) as u16;
		let month = date.month() as u16;
		let day = date.day() as u16;

		(year_offset << 9) | (month << 5) | day
	}

	pub fn decode_date(&self) -> NaiveDate {
		let dob_encoded: u16 = self.dob_encoded.try_into().map_err(|_| "Value out of range for u16".to_string()).unwrap();
		let year = ((dob_encoded & YEAR_MASK) >> 9) as i32 + BASE_YEAR;
		let month = ((dob_encoded & MONTH_MASK) >> 5) as u32;
		let day = (dob_encoded & DAY_MASK) as u32;

		NaiveDate::from_ymd_opt(year, month, day).unwrap()
	}

	pub fn is_valid(&self) -> bool {
		let date = self.decode_date();
		date.year() >= BASE_YEAR && date.year() <= (BASE_YEAR + 127)
	}
}

#[async_trait]
impl CrudOperations<PlayerDOB> for PlayerDOB {
	type CreateResult = i64;
	type BatchCreateResult = ();
	type GetResult = Self;
	type UpdateResult = ();

	async fn create(pool: &SqlitePool, item: PlayerDOB) -> Result<Self::CreateResult, Error> {
		let result = sqlx::query!("INSERT INTO player_dobs (dob_encoded) VALUES (?)", item.dob_encoded)
			.execute(pool)
			.await
			.map_err(NestError::from)?;

		Ok(result.last_insert_rowid())
	}

	async fn batch_create(pool: &SqlitePool, items: &[PlayerDOB]) -> Result<Self::BatchCreateResult, Error> {
		let mut tx = pool.begin().await.map_err(NestError::from)?;

		for item in items {
			sqlx::query!("INSERT INTO player_dobs (dob_encoded) VALUES (?)", item.dob_encoded)
				.execute(&mut *tx)
				.await
				.map_err(NestError::from)?;
		}

		tx.commit().await.map_err(NestError::from)?;
		Ok(())
	}

	async fn get(pool: &SqlitePool, id: i64) -> Result<Self, Error> {
		let dob = sqlx::query_as!(PlayerDOB, "SELECT id, dob_encoded FROM player_dobs WHERE id = ?", id)
			.fetch_optional(pool)
			.await
			.map_err(NestError::from)?
			.ok_or(Error::NestError(NestError::NotFound))?;

		Ok(Self {
			id: dob.id,
			dob_encoded: dob.dob_encoded,
		})
	}

	async fn update(pool: &SqlitePool, id: i64, item: PlayerDOB) -> Result<Self::UpdateResult, Error> {
		let result = sqlx::query!("UPDATE player_dobs SET dob_encoded = ? WHERE id = ?", item.dob_encoded, id)
			.execute(pool)
			.await
			.map_err(NestError::from)?;

		if result.rows_affected() == 0 {
			return Err(Error::NestError(NestError::NotFound));
		}

		Ok(())
	}

	async fn delete(pool: &SqlitePool, id: i64) -> Result<(), Error> {
		let result = sqlx::query!("DELETE FROM player_dobs WHERE id = ?", id).execute(pool).await.map_err(NestError::from)?;

		if result.rows_affected() == 0 {
			return Err(Error::NestError(NestError::NotFound));
		}

		Ok(())
	}
}

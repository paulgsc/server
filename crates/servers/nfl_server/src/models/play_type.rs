use crate::common::CrudOperations;
use async_trait::async_trait;
use crate::common::nfl_server_error::NflServerError as Error;
use nest::http::Error as NestError;
use nfl_play_parser::schema::PlayType;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use std::str::FromStr;

#[derive(Debug, Serialize, Deserialize)]
pub struct PlayTypeRecord {
	pub id: i64,
	pub play_type: PlayType,
}

#[derive(Debug, Deserialize)]
pub struct CreatePlayType {
	pub play_type: PlayType,
}

#[derive(sqlx::FromRow)]
struct PlayTypeRecordRaw {
	id: i64,
	play_type: String,
}

#[async_trait]
impl CrudOperations<PlayTypeRecord, CreatePlayType> for PlayTypeRecord {
	async fn create(pool: &SqlitePool, item: CreatePlayType) -> Result<PlayTypeRecord, Error> {
		let play_type_str = item.play_type.to_string();
		let result = sqlx::query!("INSERT INTO play_types (play_type) VALUES (?)", play_type_str).execute(pool).await .map_err(NestError::from)?;

		Ok(PlayTypeRecord {
			id: result.last_insert_rowid(),
			play_type: item.play_type,
		})
	}

	async fn batch_create(pool: &SqlitePool, items: &[CreatePlayType]) -> Result<Vec<PlayTypeRecord>, Error> {
        let mut tx = pool.begin().await.map_err(NestError::from)?;
		let mut created_records = Vec::with_capacity(items.len());

		for item in items {
			let play_type_str = item.play_type.to_string();
			let result = sqlx::query!("INSERT INTO play_types (play_type) VALUES (?)", play_type_str)
				.execute(&mut *tx)
                .await.map_err(NestError::from)?;

			created_records.push(PlayTypeRecord {
				id: result.last_insert_rowid(),
				play_type: item.play_type.clone(),
			});
		}

        tx.commit().await.map_err(NestError::from)?;

		Ok(created_records)
	}

	async fn get(pool: &SqlitePool, id: i64) -> Result<PlayTypeRecord, Error> {
		let record = sqlx::query_as!(PlayTypeRecordRaw, "SELECT id, play_type FROM play_types WHERE id = ?", id)
			.fetch_optional(pool)
			.await.map_err(NestError::from)?
			.ok_or(Error::NestError(NestError::NotFound))?;

		Ok(PlayTypeRecord {
			id: record.id,
			play_type: PlayType::from_str(&record.play_type)?,
		})
	}

	async fn update(pool: &SqlitePool, id: i64, item: CreatePlayType) -> Result<PlayTypeRecord, Error> {
		let play_type_str = item.play_type.to_string();
		let result = sqlx::query!("UPDATE play_types SET play_type = ? WHERE id = ?", play_type_str, id).execute(pool).await.map_err(NestError::from)?;

		if result.rows_affected() == 0 {
			return Err(Error::NestError(NestError::NotFound));
		}

		Ok(PlayTypeRecord { id, play_type: item.play_type })
	}

	async fn delete(pool: &SqlitePool, id: i64) -> Result<(), Error> {
		let result = sqlx::query!("DELETE FROM play_types WHERE id = ?", id).execute(pool).await.map_err(NestError::from)?;

		if result.rows_affected() == 0 {
			return Err(Error::NestError(NestError::NotFound));
		}

		Ok(())
	}
}

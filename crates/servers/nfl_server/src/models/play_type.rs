use crate::common::CrudOperations;
use async_trait::async_trait;
use nest::http::Error;
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
	name: String,
}

#[async_trait]
impl CrudOperations<PlayTypeRecord, CreatePlayType> for PlayTypeRecord {
	async fn create(pool: &SqlitePool, item: CreatePlayType) -> Result<PlayTypeRecord, Error> {
		let play_type_str = item.play_type.to_string();
		let result = sqlx::query!("INSERT INTO play_types (name) VALUES (?)", play_type_str).execute(pool).await?;

		Ok(PlayTypeRecord {
			id: result.last_insert_rowid(),
			play_type: item.play_type,
		})
	}

	async fn batch_create(pool: &SqlitePool, items: &[CreatePlayType]) -> Result<Vec<PlayTypeRecord>, Error> {
        let mut tx = pool.begin().await?;
		let mut created_records = Vec::with_capacity(items.len());

		for item in items {
			let play_type_str = item.play_type.to_string();
			let result = sqlx::query!("INSERT INTO play_types (name) VALUES (?)", play_type_str)
				.execute(&mut *tx)
                .await?;

			created_records.push(PlayTypeRecord {
				id: result.last_insert_rowid(),
				play_type: item.play_type.clone(),
			});
		}

        tx.commit().await?;

		Ok(created_records)
	}

	async fn get(pool: &SqlitePool, id: i64) -> Result<PlayTypeRecord, Error> {
		let record = sqlx::query_as!(PlayTypeRecordRaw, "SELECT id, name FROM play_types WHERE id = ?", id)
			.fetch_optional(pool)
			.await?
			.ok_or(Error::NotFound)?;

		Ok(PlayTypeRecord {
			id: record.id,
			play_type: PlayType::from_str(&record.name)?,
		})
	}

	async fn update(pool: &SqlitePool, id: i64, item: CreatePlayType) -> Result<PlayTypeRecord, Error> {
		let play_type_str = item.play_type.to_string();
		let result = sqlx::query!("UPDATE play_types SET name = ? WHERE id = ?", play_type_str, id).execute(pool).await?;

		if result.rows_affected() == 0 {
			return Err(Error::NotFound);
		}

		Ok(PlayTypeRecord { id, play_type: item.play_type })
	}

	async fn delete(pool: &SqlitePool, id: i64) -> Result<(), Error> {
		let result = sqlx::query!("DELETE FROM play_types WHERE id = ?", id).execute(pool).await?;

		if result.rows_affected() == 0 {
			return Err(Error::NotFound);
		}

		Ok(())
	}
}

use crate::common::CrudOperations;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use crate::common::nfl_server_error::NflServerError as Error;
use nest::http::Error as NestError;

#[derive(Debug, Serialize, Deserialize)]
pub struct GameClock {
	pub id: i64,
	pub minutes: i64,
	pub seconds: i64,
}

#[derive(Debug, Deserialize)]
pub struct CreateGameClock {
	pub minutes: i64,
	pub seconds: i64,
}

impl GameClock {
	pub fn is_valid(&self) -> bool {
		(0..=15).contains(&self.minutes) && (0..=59).contains(&self.seconds)
	}
}

impl CreateGameClock {
	pub fn is_valid(&self) -> bool {
		(0..=15).contains(&self.minutes) && (0..=59).contains(&self.seconds)
	}
}

#[async_trait]
impl CrudOperations<GameClock, CreateGameClock> for GameClock {
	async fn create(pool: &SqlitePool, item: CreateGameClock) -> Result<GameClock, Error> {
		let count = sqlx::query!("SELECT COUNT(*) as count FROM game_clock").fetch_one(pool).await.map_err(NestError::from)?.count;

		if count >= 960 {
            return Err(Error::NestError(NestError::MaxRecordLimitExceeded));
		}

		let result = sqlx::query!("INSERT INTO game_clock (minutes, seconds) VALUES (?, ?)", item.minutes, item.seconds)
			.execute(pool)
            .await.map_err(NestError::from)?;  

		Ok(GameClock {
			id: result.last_insert_rowid(),
			minutes: item.minutes,
			seconds: item.seconds,
		})
	}

	async fn batch_create(pool: &SqlitePool, items: &[CreateGameClock]) -> Result<Vec<GameClock>, Error> {
		let mut tx = pool.begin().await.map_err(NestError::from)?;

		let count: i32 = sqlx::query_scalar!("SELECT COUNT(*) FROM game_clock").fetch_one(&mut *tx).await.map_err(NestError::from)?;

		if count + items.len() as i32 > 960 {
            return Err(Error::NestError(NestError::MaxRecordLimitExceeded));
		}

		let mut created_clocks = Vec::with_capacity(items.len());

		for item in items {
			let result = sqlx::query!("INSERT INTO game_clock (minutes, seconds) VALUES (?, ?)", item.minutes, item.seconds)
				.execute(&mut *tx)
                .await.map_err(NestError::from)?;

			created_clocks.push(GameClock {
				id: result.last_insert_rowid(),
				minutes: item.minutes,
				seconds: item.seconds,
			});
		}

		tx.commit().await.map_err(NestError::from)?;

		Ok(created_clocks)
	}

	async fn get(pool: &SqlitePool, id: i64) -> Result<GameClock, Error> {
		let game_clock = sqlx::query_as!(GameClock, "SELECT id, minutes, seconds FROM game_clock WHERE id = ?", id)
			.fetch_optional(pool)
			.await.map_err(NestError::from)?
            .ok_or(Error::NestError(NestError::NotFound))?;

		Ok(game_clock)
	}

	async fn update(pool: &SqlitePool, id: i64, item: CreateGameClock) -> Result<GameClock, Error> {
		let result = sqlx::query!("UPDATE game_clock SET minutes = ?, seconds = ? WHERE id = ?", item.minutes, item.seconds, id)
			.execute(pool)
            .await.map_err(NestError::from)?;

		if result.rows_affected() == 0 {
            return Err(Error::NestError(NestError::NotFound));
		}

		Ok(GameClock {
			id,
			minutes: item.minutes,
			seconds: item.seconds,
		})
	}

	async fn delete(pool: &SqlitePool, id: i64) -> Result<(), Error> {
        let result = sqlx::query!("DELETE FROM game_clock WHERE id = ?", id).execute(pool).await.map_err(NestError::from)?;

		if result.rows_affected() == 0 {
            return Err(Error::NestError(NestError::NotFound));
		}

		Ok(())
	}
}

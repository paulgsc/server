use crate::common::nfl_server_error::NflServerError as Error;
use crate::common::{CrudOperations, Identifiable, ModelId};
use crate::models::team_models::NFLGame;
use async_trait::async_trait;
use nest::http::Error as NestError;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

#[derive(Debug, Serialize, Deserialize)]
pub struct GameScore {
	pub id: u32,
	pub game: ModelId<NFLGame>,
	pub home_quarter_pts: [u8; 4],
	pub away_quarter_pts: [u8; 4],
}

impl Identifiable for GameScore {
	fn id(&self) -> u32 {
		self.id
	}
}

#[derive(Debug, Deserialize)]
pub struct CreateGameScore {
	pub game: ModelId<NFLGame>,
	pub home_quarter_pts: [u8; 4],
	pub away_quarter_pts: [u8; 4],
}

impl GameScore {
	pub fn is_valid(&self) -> bool {
		let max_quarter_points = 50;

		self.home_quarter_pts.iter().all(|&pts| pts <= max_quarter_points) && self.away_quarter_pts.iter().all(|&pts| pts <= max_quarter_points)
	}

	pub fn total_home_points(&self) -> u16 {
		self.home_quarter_pts.iter().map(|&pts| pts as u16).sum()
	}

	pub fn total_away_points(&self) -> u16 {
		self.away_quarter_pts.iter().map(|&pts| pts as u16).sum()
	}
}

impl CreateGameScore {
	pub fn is_valid(&self) -> bool {
		// Use the same validation logic as GameScore
		let max_quarter_points = 50;

		self.home_quarter_pts.iter().all(|&pts| pts <= max_quarter_points) && self.away_quarter_pts.iter().all(|&pts| pts <= max_quarter_points)
	}
}

#[async_trait]
impl CrudOperations<GameScore, CreateGameScore> for GameScore {
	type CreateResult = i64;
	type BatchCreateResult = ();
	type GetResult = Self;
	type UpdateResult = ();

	async fn create(pool: &SqlitePool, item: CreateGameScore) -> Result<Self::CreateResult, Error> {
		if !item.is_valid() {
			return Err(Error::NestError(NestError::Forbidden));
		}

		let result = sqlx::query!(
			r#"
            INSERT INTO game_scores (
                game_id,
                home_q1, home_q2, home_q3, home_q4,
                away_q1, away_q2, away_q3, away_q4
            ) 
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
			item.game.0,
			item.home_quarter_pts[0],
			item.home_quarter_pts[1],
			item.home_quarter_pts[2],
			item.home_quarter_pts[3],
			item.away_quarter_pts[0],
			item.away_quarter_pts[1],
			item.away_quarter_pts[2],
			item.away_quarter_pts[3],
		)
		.execute(pool)
		.await
		.map_err(NestError::from)?;

		Ok(result.last_insert_rowid())
	}

	async fn batch_create(pool: &SqlitePool, items: &[CreateGameScore]) -> Result<Self::BatchCreateResult, Error> {
		let mut tx = pool.begin().await.map_err(NestError::from)?;

		for item in items {
			if !item.is_valid() {
				tx.rollback().await.map_err(NestError::from)?;
				return Err(Error::NestError(NestError::Forbidden));
			}

			let result = sqlx::query!(
				r#"
                INSERT INTO game_scores (
                    game_id,
                    home_q1, home_q2, home_q3, home_q4,
                    away_q1, away_q2, away_q3, away_q4
                ) 
                VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
                "#,
				item.game.0,
				item.home_quarter_pts[0],
				item.home_quarter_pts[1],
				item.home_quarter_pts[2],
				item.home_quarter_pts[3],
				item.away_quarter_pts[0],
				item.away_quarter_pts[1],
				item.away_quarter_pts[2],
				item.away_quarter_pts[3],
			)
			.execute(&mut *tx)
			.await
			.map_err(NestError::from)?;
		}

		tx.commit().await.map_err(NestError::from)?;
		Ok(())
	}

	async fn get(pool: &SqlitePool, id: i64) -> Result<Self::GetResult, Error> {
		let score = sqlx::query!(
			r#"
            SELECT 
                id, game_id,
                home_q1, home_q2, home_q3, home_q4,
                away_q1, away_q2, away_q3, away_q4
            FROM game_scores 
            WHERE id = ?
            "#,
			id
		)
		.fetch_optional(pool)
		.await
		.map_err(NestError::from)?
		.ok_or(Error::NestError(NestError::NotFound))?;

		Ok(Self {
			id: score.id as u32,
			game: ModelId::new(score.game_id as u32),
			home_quarter_pts: [score.home_q1 as u8, score.home_q2 as u8, score.home_q3 as u8, score.home_q4 as u8],
			away_quarter_pts: [score.away_q1 as u8, score.away_q2 as u8, score.away_q3 as u8, score.away_q4 as u8],
		})
	}

	async fn update(pool: &SqlitePool, id: i64, item: CreateGameScore) -> Result<Self::UpdateResult, Error> {
		if !item.is_valid() {
			return Err(Error::NestError(NestError::Forbidden));
		}

		let mut tx = pool.begin().await.map_err(NestError::from)?;

		let result = sqlx::query!(
			r#"
            UPDATE game_scores SET
                game_id = ?,
                home_q1 = ?, home_q2 = ?, home_q3 = ?, home_q4 = ?,
                away_q1 = ?, away_q2 = ?, away_q3 = ?, away_q4 = ?
            WHERE id = ?
            "#,
			item.game.0,
			item.home_quarter_pts[0],
			item.home_quarter_pts[1],
			item.home_quarter_pts[2],
			item.home_quarter_pts[3],
			item.away_quarter_pts[0],
			item.away_quarter_pts[1],
			item.away_quarter_pts[2],
			item.away_quarter_pts[3],
			id
		)
		.execute(&mut *tx)
		.await
		.map_err(NestError::from)?;

		if result.rows_affected() == 0 {
			return Err(Error::NestError(NestError::NotFound));
		}
		tx.commit().await.map_err(NestError::from)?;

		Ok(())
	}

	async fn delete(pool: &SqlitePool, id: i64) -> Result<(), Error> {
		let result = sqlx::query!("DELETE FROM game_scores WHERE id = ?", id).execute(pool).await.map_err(NestError::from)?;

		if result.rows_affected() == 0 {
			return Err(Error::NestError(NestError::NotFound));
		}
		Ok(())
	}
}

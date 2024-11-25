use crate::common::nfl_server_error::NflServerError as Error;
use crate::common::{CrudOperations, Identifiable, ModelId};
use crate::models::{GameClock, NFLGame, Team};
use async_trait::async_trait;
use nest::http::Error as NestError;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

#[derive(Debug, Copy, Clone, Serialize, Deserialize)]
pub enum ScoringEvent {
	OffensiveTouchdown,
	FieldGoal,
	PAT,
	TwoPointScore,
	Safety,
	DefensiveTouchdown,
}

impl TryFrom<u32> for ScoringEvent {
	type Error = Error;

	fn try_from(value: u32) -> Result<Self, Self::Error> {
		match value {
			0 => Ok(ScoringEvent::OffensiveTouchdown),
			1 => Ok(ScoringEvent::FieldGoal),
			2 => Ok(ScoringEvent::PAT),
			3 => Ok(ScoringEvent::TwoPointScore),
			4 => Ok(ScoringEvent::DefensiveTouchdown),
			5 => Ok(ScoringEvent::Safety),

			_ => Err(Error::NestError(NestError::unprocessable_entity(vec![("scoring event", "Invalid ScoringEvent")]))),
		}
	}
}

impl From<ScoringEvent> for u32 {
	fn from(value: ScoringEvent) -> u32 {
		match value {
			ScoringEvent::OffensiveTouchdown => 0,
			ScoringEvent::FieldGoal => 1,
			ScoringEvent::PAT => 2,
			ScoringEvent::TwoPointScore => 3,
			ScoringEvent::DefensiveTouchdown => 4,
			ScoringEvent::Safety => 5,
		}
	}
}

#[derive(Debug, Copy, Clone, Serialize, Deserialize)]
pub enum Quarter {
	First,
	Second,
	Third,
	Fourth,
	OT,
}

impl TryFrom<u32> for Quarter {
	type Error = Error;

	fn try_from(value: u32) -> Result<Self, Self::Error> {
		match value {
			1 => Ok(Quarter::First),
			2 => Ok(Quarter::Second),
			3 => Ok(Quarter::Third),
			4 => Ok(Quarter::Fourth),
			5 => Ok(Quarter::OT),
			_ => Err(Error::NestError(NestError::unprocessable_entity(vec![("weather", "Invalid Quarter")]))),
		}
	}
}

impl From<Quarter> for u32 {
	fn from(value: Quarter) -> u32 {
		match value {
			Quarter::First => 1,
			Quarter::Second => 2,
			Quarter::Third => 3,
			Quarter::Fourth => 4,
			Quarter::OT => 5,
		}
	}
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GameScore {
	pub id: u32,
	pub game: ModelId<NFLGame>,
	pub team: ModelId<Team>,
	pub scoring_event: ScoringEvent,
	pub quarter: Quarter,
	pub clock: ModelId<GameClock>,
}

impl Identifiable for GameScore {
	fn id(&self) -> u32 {
		self.id
	}
}

#[derive(Debug, Deserialize)]
pub struct CreateGameScore {
	pub game: ModelId<NFLGame>,
	pub team: ModelId<Team>,
	pub scoring_event: ScoringEvent,
	pub quarter: Quarter,
	pub clock: ModelId<GameClock>,
}

impl GameScore {
	pub fn points(&self) -> u32 {
		self.scoring_event.into()
	}
}

#[async_trait]
impl CrudOperations<GameScore, CreateGameScore> for GameScore {
	type CreateResult = i64;
	type BatchCreateResult = ();
	type GetResult = Self;
	type UpdateResult = ();

	async fn create(pool: &SqlitePool, item: CreateGameScore) -> Result<Self::CreateResult, Error> {
		let scoring_event = u32::from(item.scoring_event);
		let quarter = u32::from(item.quarter);
		let game_id = item.game.value();
		let team_id = item.team.value();
		let clock_id = item.clock.value();

		let result = sqlx::query!(
			r#"
            INSERT INTO game_scores (
                game_id,
                team_id,
                scoring_event,
                quarter,
                clock_id
            ) 
            VALUES (?, ?, ?, ?, ?)
            "#,
			game_id,
			team_id,
			scoring_event,
			quarter,
			clock_id,
		)
		.execute(pool)
		.await
		.map_err(NestError::from)?;

		Ok(result.last_insert_rowid())
	}

	async fn batch_create(pool: &SqlitePool, items: &[CreateGameScore]) -> Result<Self::BatchCreateResult, Error> {
		let mut tx = pool.begin().await.map_err(NestError::from)?;

		for item in items {
			let scoring_event = u32::from(item.scoring_event);
			let quarter = u32::from(item.quarter);
			let game_id = item.game.value();
			let team_id = item.team.value();
			let clock_id = item.clock.value();

			sqlx::query!(
				r#"
                INSERT INTO game_scores (
                    game_id,
                    team_id,
                    scoring_event,
                    quarter,
                    clock_id
                ) 
                VALUES (?, ?, ?, ?, ?)
                "#,
				game_id,
				team_id,
				scoring_event,
				quarter,
				clock_id,
			)
			.execute(&mut *tx)
			.await
			.map_err(NestError::from)?;
		}

		tx.commit().await.map_err(NestError::from)?;
		Ok(())
	}

	async fn get(pool: &SqlitePool, id: i64) -> Result<Self::GetResult, Error> {
		let score = sqlx::query_as!(
			GameScoreRow,
			r#"
            SELECT 
                id,
                game_id,
                team_id,
                scoring_event,
                quarter,
                clock_id
            FROM game_scores 
            WHERE id = ?
            "#,
			id
		)
		.fetch_optional(pool)
		.await
		.map_err(NestError::from)?
		.ok_or(Error::NestError(NestError::NotFound))?;

		let game_id = u32::try_from(score.game_id).map_err(NestError::from)?;
		let team_id = u32::try_from(score.team_id).map_err(NestError::from)?;
		let clock_id = u32::try_from(score.clock_id).map_err(NestError::from)?;

		Ok(Self {
			id: score.id as u32,
			game: ModelId::new(game_id),
			team: ModelId::new(team_id),
			clock: ModelId::new(clock_id),
			scoring_event: u32::try_from(score.scoring_event)
				.map_err(|e| Error::NestError(NestError::from(e)))
				.and_then(|v| ScoringEvent::try_from(v))?,
			quarter: u32::try_from(score.quarter)
				.map_err(|e| Error::NestError(NestError::from(e)))
				.and_then(|v| Quarter::try_from(v))?,
		})
	}

	async fn update(pool: &SqlitePool, id: i64, item: CreateGameScore) -> Result<Self::UpdateResult, Error> {
		let mut tx = pool.begin().await.map_err(NestError::from)?;

		let scoring_event = u32::from(item.scoring_event);
		let quarter = u32::from(item.quarter);
		let game_id = item.game.value();
		let team_id = item.team.value();
		let clock_id = item.clock.value();

		let result = sqlx::query!(
			r#"
            UPDATE game_scores SET
                game_id = ?,
                team_id = ?,
                scoring_event = ?,
                quarter = ?,
                clock_id = ?
            WHERE id = ?
            "#,
			game_id,
			team_id,
			scoring_event,
			quarter,
			clock_id,
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

pub struct GameScoreRow {
	id: i64,
	game_id: i64,
	team_id: i64,
	scoring_event: i64,
	quarter: i64,
	clock_id: i64,
}

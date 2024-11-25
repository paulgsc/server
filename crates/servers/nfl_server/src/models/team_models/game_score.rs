use crate::common::nfl_server_error::NflServerError as Error;
use crate::common::{CrudOperations, Identifiable, ModelId};
use crate::models::team_models::NFLGame;
use async_trait::async_trait;
use nest::http::Error as NestError;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

#[derive(Debug, Clone, PartialEq)]
pub enum ScoringEvent {
	OffensiveTouchdown,
	FieldGoal,
	PAT,
	TwoPointScore,
	Safety,
	DefensiveTouchdown,
}

impl TryFrom<u16> for ScoringEvent {
	type Error = Error;

	fn try_from(value: u16) -> Result<Self, Self::Error> {
		match value {
			0 => Ok(ScoringEvent::OffensiveTouchdown),
			1 => Ok(ScoringEvent::FieldGoal),
			2 => Ok(ScoringEvent::PAT),
			3 => Ok(ScoringEvent::TwoPointScore),
			4 => Ok(ScoringEvent::DefensiveTouchdown),
			_ => Err(Error::NestError(NestError::unprocessable_entity(vec![("scoring event", "Invalid ScoringEvent")]))),
		}
	}
}

impl From<ScoringEvent> for u16 {
	fn from(value: ScoringEvent) -> u16 {
		match value {
			ScoringEvent::OffensiveTouchdown => 0,
			ScoringEvent::FieldGoal => 1,
			ScoringEvent::PAT => 2,
			ScoringEvent::TowPointScore => 3,
			ScoringEvent::DefensiveTouchdown => 4,
		}
	}
}

#[derive(Debug)]
pub enum Quarter {
	First,
	Second,
	Third,
	Fourth,
	OT,
}

impl TryFrom<u16> for Quarter {
	type Error = Error;

	fn try_from(value: u16) -> Result<Self, Self::Error> {
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

impl From<Quarter> for u16 {
	fn from(value: Quarter) -> u16 {
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
	pub time: ModelId<GameClock>,
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
	pub time: ModelId<GameClock>,
}

impl GameScore {
	pub const fn points(&self) -> u16 {
		self.scoring_event.into()
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

		let event_type = u16::from(item.event_type);
		let quarter = u16::from(item.quarter);
		let game_id = item.game.value();
		let team_id = item.team.value();
		let clock_id = item.clock.value();

		let result = sqlx::query!(
			r#"
            INSERT INTO game_scores (
                game_id,
                team_id,
                event_type,
                quarter,
                clock_id,
            ) 
            VALUES (?, ?, ?, ?, ?)
            "#,
			game_id,
			team_id,
			event_type,
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
			if !item.is_valid() {
				tx.rollback().await.map_err(NestError::from)?;
				return Err(Error::NestError(NestError::Forbidden));
			}

			let event_type = u16::from(item.event_type);
			let quarter = u16::from(item.quarter);
			let game_id = item.game.value();
			let team_id = item.team.value();
			let clock_id = item.clock.value();

			let result = sqlx::query!(
				r#"
                INSERT INTO game_scores (
                    game_id,
                    team_id,
                    event_type,
                    quarter,
                    clock_id,
                ) 
                VALUES (?, ?, ?, ?, ?)
                "#,
				game_id,
				team_id,
				event_type,
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
                event_type,
                quarter,
                clock_id,
            FROM game_scores 
            WHERE id = ?
            "#,
			id
		)
		.fetch_optional(pool)
		.await
		.map_err(NestError::from)?
		.ok_or(Error::NestError(NestError::NotFound))?;

		let game_id = u16::try_from(score.game_id).map_err(NestError::from)?;
		let team_id = u16::try_from(score.team_id).map_err(NestError::from)?;
		let clock_id = u16::try_from(score.clock_id).map_err(NestError::from)?;

		Ok(Self {
			id: score.id as u32,
			game: ModelId::new(game_id),
			team: ModelId::new(team_id),
			clock: ModelId::new(clock_id),
		}
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

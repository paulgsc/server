use crate::common::nfl_server_error::NflServerError as Error;
use crate::common::{CrudOperations, Identifiable, ModelId};
use crate::models::{GameClock, NFLGame, Team};
use async_trait::async_trait;
use nest::http::Error as NestError;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use sqlx::{sqlite::SqliteTypeInfo, Encode, Type};

#[derive(Debug, Copy, Clone, Serialize, Deserialize)]
pub enum ScoringEvent {
	OffensiveTouchdown,
	FieldGoal,
	PAT,
	TwoPointScore,
	Safety,
	DefensiveTouchdown,
}

impl From<i64> for ScoringEvent {
	fn from(value: i64) -> Self {
		match value {
			0 => Self::OffensiveTouchdown,
			1 => Self::FieldGoal,
			2 => Self::PAT,
			3 => Self::TwoPointScore,
			4 => Self::DefensiveTouchdown,
			5 => Self::Safety,
			_ => panic!("Invalid value for ScoringEvent: {}", value),
		}
	}
}

impl From<ScoringEvent> for i64 {
	fn from(value: ScoringEvent) -> i64 {
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

impl From<i64> for Quarter {
	fn from(value: i64) -> Self {
		match value {
			1 => Self::First,
			2 => Self::Second,
			3 => Self::Third,
			4 => Self::Fourth,
			5 => Self::OT,
			_ => panic!("Invalid value for Quarter: {}", value),
		}
	}
}

impl From<Quarter> for i64 {
	fn from(value: Quarter) -> i64 {
		match value {
			Quarter::First => 1,
			Quarter::Second => 2,
			Quarter::Third => 3,
			Quarter::Fourth => 4,
			Quarter::OT => 5,
		}
	}
}

impl Type<sqlx::Sqlite> for Quarter {
	fn type_info() -> SqliteTypeInfo {
		<i64 as Type<sqlx::Sqlite>>::type_info()
	}

	fn compatible(ty: &SqliteTypeInfo) -> bool {
		<i64 as Type<sqlx::Sqlite>>::compatible(ty)
	}
}
impl<'q> sqlx::Encode<'q, sqlx::Sqlite> for Quarter {
	fn encode_by_ref(&self, args: &mut <sqlx::Sqlite as sqlx::database::HasArguments<'q>>::ArgumentBuffer) -> sqlx::encode::IsNull {
		let encoded_value = *self as i64;
		<i64 as Encode<sqlx::Sqlite>>::encode_by_ref(&encoded_value, args)
	}
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GameScore {
	pub id: i64,
	pub game_id: ModelId<NFLGame>,
	pub team_id: ModelId<Team>,
	pub scoring_event: ScoringEvent,
	pub quarter: Quarter,
	pub clock_id: ModelId<GameClock>,
}

impl Identifiable for GameScore {
	fn id(&self) -> i64 {
		self.id
	}
}

impl GameScore {
	pub fn points(&self) -> i64 {
		self.scoring_event.into()
	}
}

#[async_trait]
impl CrudOperations<GameScore> for GameScore {
	type CreateResult = i64;
	type BatchCreateResult = ();
	type GetResult = Self;
	type UpdateResult = ();

	async fn create(pool: &SqlitePool, item: GameScore) -> Result<Self::CreateResult, Error> {
		let scoring_event = i64::from(item.scoring_event);
		let quarter = item.quarter;
		let game_id = item.game_id.value();
		let team_id = item.team_id.value();
		let clock_id = item.clock_id.value();

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

	async fn batch_create(pool: &SqlitePool, items: &[GameScore]) -> Result<Self::BatchCreateResult, Error> {
		let mut tx = pool.begin().await.map_err(NestError::from)?;

		for item in items {
			let scoring_event = i64::from(item.scoring_event);
			let quarter = i64::from(item.quarter);
			let game_id = item.game_id.value();
			let team_id = item.team_id.value();
			let clock_id = item.clock_id.value();

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
			GameScore,
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

		let game_id = score.game_id;
		let team_id = score.team_id;
		let clock_id = score.clock_id;

		Ok(Self {
			id: score.id as i64,
			game_id,
			team_id,
			clock_id,
			scoring_event: score.scoring_event,
			quarter: score.quarter,
		})
	}

	async fn update(pool: &SqlitePool, id: i64, item: GameScore) -> Result<Self::UpdateResult, Error> {
		let mut tx = pool.begin().await.map_err(NestError::from)?;

		let scoring_event = i64::from(item.scoring_event);
		let quarter = i64::from(item.quarter);
		let game_id = item.game_id.value();
		let team_id = item.team_id.value();
		let clock_id = item.clock_id.value();

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

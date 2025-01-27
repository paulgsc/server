use crate::common::nfl_server_error::NflServerError as Error;
use crate::common::EncodedDate;
use crate::common::{is_timestamp, CrudOperations, Identifiable, ModelId};
use crate::models::team_models::{Stadium, TeamNameMeta};
use async_trait::async_trait;
use nest::http::Error as NestError;
use serde::{Deserialize, Serialize};
use sqlite_macros::SqliteType;
use sqlx::{Encode, FromRow, Sqlite, SqlitePool, Type};
use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum WeatherCondition {
	Sunny,
	Cloudy,
	Rainy,
	Snowy,
	Windy,
	Foggy,
}

impl Type<Sqlite> for WeatherCondition {
	fn type_info() -> sqlx::sqlite::SqliteTypeInfo {
		<i64 as Type<Sqlite>>::type_info()
	}
}

impl<'q> sqlx::Encode<'q, sqlx::Sqlite> for WeatherCondition {
	fn encode_by_ref(&self, args: &mut <sqlx::Sqlite as sqlx::database::HasArguments<'q>>::ArgumentBuffer) -> sqlx::encode::IsNull {
		let encoded_value = *self as i64;
		<i64 as Encode<sqlx::Sqlite>>::encode_by_ref(&encoded_value, args)
	}
}

impl From<i64> for WeatherCondition {
	fn from(value: i64) -> Self {
		match value {
			0 => Self::Sunny,
			1 => Self::Cloudy,
			2 => Self::Rainy,
			3 => Self::Snowy,
			4 => Self::Windy,
			5 => Self::Foggy,
			_ => panic!("Invalid Weather Condition: {}", value),
		}
	}
}

impl From<WeatherCondition> for i64 {
	fn from(value: WeatherCondition) -> i64 {
		match value {
			WeatherCondition::Sunny => 0,
			WeatherCondition::Cloudy => 1,
			WeatherCondition::Rainy => 2,
			WeatherCondition::Snowy => 3,
			WeatherCondition::Windy => 4,
			WeatherCondition::Foggy => 5,
		}
	}
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum DayNight {
	Day,
	Night,
}

impl Type<Sqlite> for DayNight {
	fn type_info() -> sqlx::sqlite::SqliteTypeInfo {
		<i64 as Type<Sqlite>>::type_info()
	}
}

impl<'q> sqlx::Encode<'q, sqlx::Sqlite> for DayNight {
	fn encode_by_ref(&self, args: &mut <sqlx::Sqlite as sqlx::database::HasArguments<'q>>::ArgumentBuffer) -> sqlx::encode::IsNull {
		let encoded_value = *self as i64;
		<i64 as Encode<sqlx::Sqlite>>::encode_by_ref(&encoded_value, args)
	}
}

impl From<i64> for DayNight {
	fn from(value: i64) -> Self {
		match value {
			0 => Self::Day,
			1 => Self::Night,
			_ => panic!("Invalid Daynight: {value}"),
		}
	}
}

impl From<DayNight> for i64 {
	fn from(value: DayNight) -> i64 {
		match value {
			DayNight::Day => 0,
			DayNight::Night => 1,
		}
	}
}

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct Weather {
	pub id: i64,
	pub condition: WeatherCondition,
	pub day_night: DayNight,
	pub temperature: f64,
	pub wind_speed: Option<f64>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Team {
	pub id: i64,
	pub name_id: ModelId<TeamNameMeta>,
	pub stadium_id: ModelId<Stadium>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, SqliteType)]
pub enum GameStatus {
	Scheduled,
	InProgress,
	Completed,
	Postponed,
	Cancelled,
	TBD,
}

impl fmt::Display for GameStatus {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{}", self)
	}
}

impl FromStr for GameStatus {
	type Err = Error;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		let normalized = s.trim().to_lowercase();

		match normalized.as_str() {
			"scheduled" => Ok(GameStatus::Scheduled),
			"in_progress" | "inprogress" | "in progress" => Ok(GameStatus::InProgress),
			"completed" | "final" | "complete" => Ok(GameStatus::Completed),
			"postponed" => Ok(GameStatus::Postponed),
			"cancelled" | "canceled" => Ok(GameStatus::Cancelled),

			"live" => Ok(GameStatus::InProgress),
			"final/ot" | "final (ot)" | "final (overtime)" => Ok(GameStatus::Completed),

			status if is_timestamp(status) => Ok(GameStatus::Scheduled),

			_ => Err(Error::GameStatusParseError(s.to_string())),
		}
	}
}

impl From<i64> for GameStatus {
	fn from(value: i64) -> Self {
		match value {
			0 => GameStatus::Scheduled,
			1 => GameStatus::InProgress,
			2 => GameStatus::Completed,
			3 => GameStatus::Postponed,
			4 => GameStatus::Cancelled,
			5 => GameStatus::TBD,
			_ => panic!("Invalid i64 value for GameStatus: {}", value),
		}
	}
}

impl From<GameStatus> for i64 {
	fn from(value: GameStatus) -> i64 {
		match value {
			GameStatus::Scheduled => 0,
			GameStatus::InProgress => 1,
			GameStatus::Completed => 2,
			GameStatus::Postponed => 3,
			GameStatus::Cancelled => 4,
			GameStatus::TBD => 5,
		}
	}
}

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct NFLGame {
	pub id: i64,
	pub encoded_date: EncodedDate,
	pub home_team_id: ModelId<Team>,
	pub away_team_id: ModelId<Team>,
	pub game_status_id: GameStatus,
	pub weather_id: ModelId<Weather>,
}

impl Identifiable for Weather {
	fn id(&self) -> i64 {
		self.id
	}
}

impl Weather {
	pub fn is_valid(&self) -> bool {
		(-50.0..=150.0).contains(&self.temperature)
			&& match self.wind_speed {
				Some(speed) => (0.0..=200.0).contains(&speed),
				None => true,
			}
	}
}

#[async_trait]
impl CrudOperations<Weather> for Weather {
	type CreateResult = i64;
	type BatchCreateResult = ();
	type GetResult = Self;
	type UpdateResult = ();

	async fn create(pool: &SqlitePool, item: Weather) -> Result<Self::CreateResult, Error> {
		if !item.is_valid() {
			return Err(Error::NestError(NestError::Forbidden));
		}

		let result = sqlx::query!(
			"INSERT INTO weather (condition, day_night, temperature, wind_speed) VALUES (?, ?, ?, ?)",
			item.condition,
			item.day_night,
			item.temperature,
			item.wind_speed
		)
		.execute(pool)
		.await
		.map_err(NestError::from)?;

		Ok(result.last_insert_rowid())
	}

	async fn batch_create(pool: &SqlitePool, items: &[Weather]) -> Result<Self::BatchCreateResult, Error> {
		let mut tx = pool.begin().await.map_err(NestError::from)?;

		for item in items {
			let condition_i64 = i64::from(item.condition);
			let day_night_i64 = i64::from(item.day_night);

			sqlx::query!(
				"INSERT INTO weather (condition, day_night, temperature, wind_speed) VALUES (?, ?, ?, ?)",
				condition_i64,
				day_night_i64,
				item.temperature,
				item.wind_speed
			)
			.execute(&mut *tx)
			.await
			.map_err(NestError::from)?;
		}

		tx.commit().await.map_err(NestError::from)?;
		Ok(())
	}

	async fn get(pool: &SqlitePool, id: i64) -> Result<Self::GetResult, Error> {
		let weather = sqlx::query_as!(
			Weather,
			r#"
            SELECT
                id,
                condition,
                day_night,
                temperature,
                wind_speed
            FROM
                weather
            WHERE
                id = ?
            "#,
			id
		)
		.fetch_optional(pool)
		.await
		.map_err(NestError::from)?
		.ok_or(Error::NestError(NestError::NotFound))?;

		Ok(Self {
			id: weather.id as i64,
			condition: weather.condition,
			day_night: weather.day_night,
			temperature: weather.temperature,
			wind_speed: weather.wind_speed.map(|speed| speed),
		})
	}

	async fn update(pool: &SqlitePool, id: i64, item: Weather) -> Result<Self::UpdateResult, Error> {
		let result = sqlx::query!(
			"UPDATE weather SET condition = ?, day_night = ?, temperature = ?, wind_speed = ? WHERE id = ?",
			item.condition,
			item.day_night,
			item.temperature,
			item.wind_speed,
			id
		)
		.execute(pool)
		.await
		.map_err(NestError::from)?;

		if result.rows_affected() == 0 {
			return Err(Error::NestError(NestError::NotFound));
		}

		Ok(())
	}

	async fn delete(pool: &SqlitePool, id: i64) -> Result<(), Error> {
		let result = sqlx::query!("DELETE FROM weather WHERE id = ?", id).execute(pool).await.map_err(NestError::from)?;

		if result.rows_affected() == 0 {
			return Err(Error::NestError(NestError::NotFound));
		}
		Ok(())
	}
}

impl Identifiable for Team {
	fn id(&self) -> i64 {
		self.id
	}
}

#[async_trait]
impl CrudOperations<Team> for Team {
	type CreateResult = i64;
	type BatchCreateResult = ();
	type GetResult = Self;
	type UpdateResult = ();

	async fn create(pool: &SqlitePool, item: Team) -> Result<Self::CreateResult, Error> {
		// recieve a str then parse to make teamname and stadium
		//
		let name_id = item.name_id.value();
		let stadium_id = item.stadium_id.value();

		let result = sqlx::query!("INSERT INTO teams (name_id, stadium_id) VALUES (?, ?)", name_id, stadium_id)
			.execute(pool)
			.await
			.map_err(NestError::from)?;

		Ok(result.last_insert_rowid())
	}

	async fn batch_create(pool: &SqlitePool, items: &[Team]) -> Result<Self::BatchCreateResult, Error> {
		let mut tx = pool.begin().await.map_err(NestError::from)?;

		for item in items {
			let name_id = item.name_id.value();
			let stadium_id = item.stadium_id.value();

			sqlx::query!("INSERT INTO teams (name_id, stadium_id) VALUES (?, ?)", name_id, stadium_id)
				.execute(&mut *tx)
				.await
				.map_err(NestError::from)?;
		}

		tx.commit().await.map_err(NestError::from)?;
		Ok(())
	}

	async fn get(pool: &SqlitePool, id: i64) -> Result<Self::GetResult, Error> {
		let team = sqlx::query_as!(Team, "SELECT id, name_id, stadium_id FROM teams WHERE id = ?", id)
			.fetch_optional(pool)
			.await
			.map_err(NestError::from)?
			.ok_or(Error::NestError(NestError::NotFound))?;

		let name_id = team.name_id;
		let stadium_id = team.stadium_id;
		let id = team.id;

		Ok(Self { id, name_id, stadium_id })
	}

	async fn update(pool: &SqlitePool, id: i64, item: Team) -> Result<Self::UpdateResult, Error> {
		let name_id = item.name_id.value();
		let stadium_id = item.stadium_id.value();

		let result = sqlx::query!("UPDATE teams SET name_id = ?, stadium_id = ? WHERE id = ?", name_id, stadium_id, id,)
			.execute(pool)
			.await
			.map_err(NestError::from)?;

		if result.rows_affected() == 0 {
			return Err(Error::NestError(NestError::NotFound));
		}

		Ok(())
	}

	async fn delete(pool: &SqlitePool, id: i64) -> Result<(), Error> {
		let result = sqlx::query!("DELETE FROM weather WHERE id = ?", id).execute(pool).await.map_err(NestError::from)?;

		if result.rows_affected() == 0 {
			return Err(Error::NestError(NestError::NotFound));
		}
		Ok(())
	}
}

impl Identifiable for NFLGame {
	fn id(&self) -> i64 {
		self.id
	}
}

#[async_trait]
impl CrudOperations<NFLGame> for NFLGame {
	type CreateResult = i64;
	type BatchCreateResult = ();
	type GetResult = Self;
	type UpdateResult = ();

	async fn create(pool: &SqlitePool, item: NFLGame) -> Result<Self::CreateResult, Error> {
		let game_status_id = item.game_status_id;
		let home_team_id = item.home_team_id.value();
		let away_team_id = item.away_team_id.value();
		let weather_id = item.weather_id.value();
		let result = sqlx::query!(
			"INSERT INTO nfl_games
            (encoded_date, home_team_id, away_team_id, game_status_id, weather_id)
            VALUES (?, ?, ?, ?, ?)
            ",
			item.encoded_date,
			home_team_id,
			away_team_id,
			game_status_id,
			weather_id,
		)
		.execute(pool)
		.await
		.map_err(NestError::from)?;

		Ok(result.last_insert_rowid())
	}

	async fn batch_create(pool: &SqlitePool, items: &[NFLGame]) -> Result<Self::BatchCreateResult, Error> {
		let mut tx = pool.begin().await.map_err(NestError::from)?;

		for item in items {
			let game_status_id = i64::from(item.game_status_id);
			let home_team_id = item.home_team_id.value();
			let away_team_id = item.away_team_id.value();
			let weather_id = item.weather_id.value();
			sqlx::query!(
				"INSERT INTO nfl_games (encoded_date, home_team_id, away_team_id, game_status_id, weather_id) VALUES (?, ?, ?, ?, ?)",
				item.encoded_date,
				home_team_id,
				away_team_id,
				game_status_id,
				weather_id,
			)
			.execute(&mut *tx)
			.await
			.map_err(NestError::from)?;
		}

		tx.commit().await.map_err(NestError::from)?;
		Ok(())
	}

	async fn get(pool: &SqlitePool, id: i64) -> Result<Self::GetResult, Error> {
		let nfl_game = sqlx::query_as!(
			NFLGame,
			"SELECT id, encoded_date, home_team_id, away_team_id, game_status_id, weather_id from nfl_games WHERE id = ?",
			id
		)
		.fetch_optional(pool)
		.await
		.map_err(NestError::from)?
		.ok_or(Error::NestError(NestError::NotFound))?;

		let home_team_id = nfl_game.home_team_id;
		let away_team_id = nfl_game.away_team_id;
		let weather_id = nfl_game.weather_id;
		let encoded_date = nfl_game.encoded_date;

		Ok(NFLGame {
			id: nfl_game.id,
			encoded_date,
			home_team_id,
			away_team_id,
			game_status_id: nfl_game.game_status_id,
			weather_id,
		})
	}

	async fn update(pool: &SqlitePool, id: i64, item: NFLGame) -> Result<Self::UpdateResult, Error> {
		let game_status_id = item.game_status_id;
		let home_team_id = item.home_team_id.value();
		let away_team_id = item.away_team_id.value();
		let weather_id = item.weather_id.value();
		let result = sqlx::query!(
			"UPDATE nfl_games SET encoded_date = ?, home_team_id = ?, away_team_id = ?, game_status_id = ?, weather_id = ? WHERE id = ?",
			item.encoded_date,
			home_team_id,
			away_team_id,
			game_status_id,
			weather_id,
			id
		)
		.execute(pool)
		.await
		.map_err(NestError::from)?;

		if result.rows_affected() == 0 {
			return Err(Error::NestError(NestError::NotFound));
		}

		Ok(())
	}

	async fn delete(pool: &SqlitePool, id: i64) -> Result<(), Error> {
		let result = sqlx::query!("DELETE FROM nfl_games WHERE id = ?", id).execute(pool).await.map_err(NestError::from)?;

		if result.rows_affected() == 0 {
			return Err(Error::NestError(NestError::NotFound));
		}
		Ok(())
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::str::FromStr;

	#[test]
	fn test_game_status_parsing() {
		// Test standard variants
		assert_eq!(GameStatus::from_str("Scheduled").unwrap(), GameStatus::Scheduled);
		assert_eq!(GameStatus::from_str("in_progress").unwrap(), GameStatus::InProgress);
		assert_eq!(GameStatus::from_str("FINAL").unwrap(), GameStatus::Completed);
		assert_eq!(GameStatus::from_str("TBD").unwrap(), GameStatus::TBD);

		// Test variations
		assert_eq!(GameStatus::from_str("live").unwrap(), GameStatus::InProgress);
		assert_eq!(GameStatus::from_str("final/ot").unwrap(), GameStatus::Completed);

		// Test timestamps
		assert_eq!(GameStatus::from_str("10:00 AM").unwrap(), GameStatus::Scheduled);
		assert_eq!(GameStatus::from_str("9:30 pm").unwrap(), GameStatus::Scheduled);

		// Test error case
		assert!(GameStatus::from_str("unknown").is_err());
	}
}

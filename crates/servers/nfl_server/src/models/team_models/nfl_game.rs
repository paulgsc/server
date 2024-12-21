use crate::common::nfl_server_error::NflServerError as Error;
use crate::common::EncodedDate;
use crate::common::{is_timestamp, CrudOperations, Identifiable, ModelId};
use crate::models::team_models::{Stadium, TeamNameMeta};
use async_trait::async_trait;
use nest::http::Error as NestError;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
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

impl TryFrom<i64> for WeatherCondition {
	type Error = Error;

	fn try_from(value: i64) -> Result<Self, Self::Error> {
		match value {
			0 => Ok(WeatherCondition::Sunny),
			1 => Ok(WeatherCondition::Cloudy),
			2 => Ok(WeatherCondition::Rainy),
			3 => Ok(WeatherCondition::Snowy),
			4 => Ok(WeatherCondition::Windy),
			5 => Ok(WeatherCondition::Foggy),
			_ => Err(Error::NestError(NestError::unprocessable_entity(vec![("weather", "Invalid Weather Condition")]))),
		}
	}
}

impl From<WeatherCondition> for u32 {
	fn from(value: WeatherCondition) -> u32 {
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

impl TryFrom<i64> for DayNight {
	type Error = Error;

	fn try_from(value: i64) -> Result<Self, Self::Error> {
		match value {
			0 => Ok(DayNight::Day),
			1 => Ok(DayNight::Night),
			_ => Err(Error::NestError(NestError::unprocessable_entity(vec![("Daynight", "Invalid DayNight")]))),
		}
	}
}

impl From<DayNight> for u32 {
	fn from(value: DayNight) -> u32 {
		match value {
			DayNight::Day => 0,
			DayNight::Night => 1,
		}
	}
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Weather {
	pub id: u32,
	pub condition: WeatherCondition,
	pub day_night: DayNight,
	pub temperature: f32,
	pub wind_speed: Option<f32>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Team {
	pub id: u32,
	pub name: ModelId<TeamNameMeta>,
	pub stadium: ModelId<Stadium>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum GameStatus {
	Scheduled,
	InProgress,
	Completed,
	Postponed,
	Cancelled,
	TBD,
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

impl TryFrom<i64> for GameStatus {
	type Error = Error;

	fn try_from(value: i64) -> Result<Self, Self::Error> {
		match value {
			0 => Ok(GameStatus::Scheduled),
			1 => Ok(GameStatus::InProgress),
			2 => Ok(GameStatus::Completed),
			3 => Ok(GameStatus::Postponed),
			4 => Ok(GameStatus::Cancelled),
			5 => Ok(GameStatus::TBD),

			_ => Err(Error::NestError(NestError::unprocessable_entity(vec![("GameStatus", "Invalid GameStatus")]))),
		}
	}
}

impl From<GameStatus> for u32 {
	fn from(value: GameStatus) -> u32 {
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

#[derive(Debug, Serialize, Deserialize)]
pub struct NFLGame {
	pub id: u32,
	pub date: EncodedDate,
	pub home_team: ModelId<Team>,
	pub away_team: ModelId<Team>,
	pub game_status: GameStatus,
	pub weather: ModelId<Weather>,
}

impl Identifiable for Weather {
	fn id(&self) -> u32 {
		self.id
	}
}

#[derive(Debug, Deserialize)]
pub struct CreateWeather {
	pub condition: WeatherCondition,
	pub day_night: DayNight,
	pub temperature: f32,
	pub wind_speed: Option<f32>,
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

impl CreateWeather {
	pub fn is_valid(&self) -> bool {
		(-50.0..=150.0).contains(&self.temperature)
			&& match self.wind_speed {
				Some(speed) => (0.0..=200.0).contains(&speed),
				None => true,
			}
	}
}

#[async_trait]
impl CrudOperations<Weather, CreateWeather> for Weather {
	type CreateResult = i64;
	type BatchCreateResult = ();
	type GetResult = Self;
	type UpdateResult = ();

	async fn create(pool: &SqlitePool, item: CreateWeather) -> Result<Self::CreateResult, Error> {
		if !item.is_valid() {
			return Err(Error::NestError(NestError::Forbidden));
		}

		let condition_u32 = u32::from(item.condition);
		let day_night_u32 = u32::from(item.day_night);
		let result = sqlx::query!(
			"INSERT INTO weather (condition, day_night, temperature, wind_speed) VALUES (?, ?, ?, ?)",
			condition_u32,
			day_night_u32,
			item.temperature,
			item.wind_speed
		)
		.execute(pool)
		.await
		.map_err(NestError::from)?;

		Ok(result.last_insert_rowid())
	}

	async fn batch_create(pool: &SqlitePool, items: &[CreateWeather]) -> Result<Self::BatchCreateResult, Error> {
		let mut tx = pool.begin().await.map_err(NestError::from)?;

		for item in items {
			let condition_u32 = u32::from(item.condition);
			let day_night_u32 = u32::from(item.day_night);

			sqlx::query!(
				"INSERT INTO weather (condition, day_night, temperature, wind_speed) VALUES (?, ?, ?, ?)",
				condition_u32,
				day_night_u32,
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
			WeatherRow,
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
			id: weather.id as u32,
			condition: WeatherCondition::try_from(weather.condition)?,
			day_night: DayNight::try_from(weather.day_night)?,
			temperature: weather.temperature as f32,
			wind_speed: weather.wind_speed.map(|speed| speed as f32),
		})
	}

	async fn update(pool: &SqlitePool, id: i64, item: CreateWeather) -> Result<Self::UpdateResult, Error> {
		let mut tx = pool.begin().await.map_err(NestError::from)?;

		let condition_u32 = u32::from(item.condition);
		let day_night_u32 = u32::from(item.day_night);

		let result = sqlx::query!(
			"UPDATE weather SET condition = ?, day_night = ?, temperature = ?, wind_speed = ? WHERE id = ?",
			condition_u32,
			day_night_u32,
			item.temperature,
			item.wind_speed,
			id
		)
		.execute(&mut tx)
		.await
		.map_err(NestError::from)?;

		if result.rows_affected() == 0 {
			return Err(Error::NestError(NestError::NotFound));
		}
		tx.commit().await.map_err(NestError::from)?;

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
	fn id(&self) -> u32 {
		self.id
	}
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CreateTeam {
	pub name: ModelId<TeamNameMeta>,
	pub stadium: ModelId<Stadium>,
}

#[async_trait]
impl CrudOperations<Team, CreateTeam> for Team {
	type CreateResult = i64;
	type BatchCreateResult = ();
	type GetResult = Self;
	type UpdateResult = ();

	async fn create(pool: &SqlitePool, item: CreateTeam) -> Result<Self::CreateResult, Error> {
		// recieve a str then parse to make teamname and stadium
		//
		let name = item.name.value();
		let stadium = item.stadium.value();

		let result = sqlx::query!("INSERT INTO teams (name_id, stadium_id) VALUES (?, ?)", name, stadium)
			.execute(pool)
			.await
			.map_err(NestError::from)?;

		Ok(result.last_insert_rowid())
	}

	async fn batch_create(pool: &SqlitePool, items: &[CreateTeam]) -> Result<Self::BatchCreateResult, Error> {
		let mut tx = pool.begin().await.map_err(NestError::from)?;

		for item in items {
			let name = item.name.value();
			let stadium = item.stadium.value();

			sqlx::query!("INSERT INTO teams (name_id, stadium_id) VALUES (?, ?)", name, stadium)
				.execute(&mut *tx)
				.await
				.map_err(NestError::from)?;
		}

		tx.commit().await.map_err(NestError::from)?;
		Ok(())
	}

	async fn get(pool: &SqlitePool, id: i64) -> Result<Self::GetResult, Error> {
		let team = sqlx::query_as!(TeamRow, "SELECT id, name_id, stadium_id FROM teams WHERE id = ?", id)
			.fetch_optional(pool)
			.await
			.map_err(NestError::from)?
			.ok_or(Error::NestError(NestError::NotFound))?;

		let name_id = u32::try_from(team.name_id).map_err(NestError::from)?;
		let stadium_id = u32::try_from(team.stadium_id).map_err(NestError::from)?;
		let id = u32::try_from(team.id).map_err(NestError::from)?;

		Ok(Self {
			id,
			name: ModelId::<TeamNameMeta>::new(name_id),
			stadium: ModelId::<Stadium>::new(stadium_id),
		})
	}

	async fn update(pool: &SqlitePool, id: i64, item: CreateTeam) -> Result<Self::UpdateResult, Error> {
		let mut tx = pool.begin().await.map_err(NestError::from)?;

		let name = item.name.value();
		let stadium = item.stadium.value();

		let result = sqlx::query!("UPDATE teams SET name_id = ?, stadium_id = ? WHERE id = ?", name, stadium, id,)
			.execute(&mut tx)
			.await
			.map_err(NestError::from)?;

		if result.rows_affected() == 0 {
			return Err(Error::NestError(NestError::NotFound));
		}

		tx.commit().await.map_err(NestError::from)?;

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
	fn id(&self) -> u32 {
		self.id
	}
}

#[derive(Debug, Deserialize)]
pub struct CreateNFLGame {
	pub date: EncodedDate,
	pub home_team: ModelId<Team>,
	pub away_team: ModelId<Team>,
	pub game_status: GameStatus,
	pub weather: ModelId<Weather>,
}

#[async_trait]
impl CrudOperations<NFLGame, CreateNFLGame> for NFLGame {
	type CreateResult = i64;
	type BatchCreateResult = ();
	type GetResult = Self;
	type UpdateResult = ();

	async fn create(pool: &SqlitePool, item: CreateNFLGame) -> Result<Self::CreateResult, Error> {
		let game_status = u32::from(item.game_status);
		let home_team = item.home_team.value();
		let away_team = item.away_team.value();
		let weather = item.weather.value();
		let result = sqlx::query!(
			"INSERT INTO nfl_games
            (encoded_date, home_team_id, away_team_id, game_status_id, weather_id)
            VALUES (?, ?, ?, ?, ?)
            ",
			item.date.value,
			home_team,
			away_team,
			game_status,
			weather,
		)
		.execute(pool)
		.await
		.map_err(NestError::from)?;

		Ok(result.last_insert_rowid())
	}

	async fn batch_create(pool: &SqlitePool, items: &[CreateNFLGame]) -> Result<Self::BatchCreateResult, Error> {
		let mut tx = pool.begin().await.map_err(NestError::from)?;

		for item in items {
			let game_status = u32::from(item.game_status);
			let home_team = item.home_team.value();
			let away_team = item.away_team.value();
			let weather = item.weather.value();
			sqlx::query!(
				"INSERT INTO nfl_games (encoded_date, home_team_id, away_team_id, game_status_id, weather_id) VALUES (?, ?, ?, ?, ?)",
				item.date.value,
				home_team,
				away_team,
				game_status,
				weather,
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
			NFLGameRow,
			"SELECT id, encoded_date, home_team_id, away_team_id, game_status_id, weather_id from nfl_games WHERE id = ?",
			id
		)
		.fetch_optional(pool)
		.await
		.map_err(NestError::from)?
		.ok_or(Error::NestError(NestError::NotFound))?;

		let home_team = u32::try_from(nfl_game.home_team_id).map_err(NestError::from)?;
		let away_team = u32::try_from(nfl_game.away_team_id).map_err(NestError::from)?;
		let weather = u32::try_from(nfl_game.weather_id).map_err(NestError::from)?;
		let encoded_date = EncodedDate::try_from(nfl_game.encoded_date)?;

		Ok(NFLGame {
			id: nfl_game.id as u32,
			date: encoded_date,
			home_team: ModelId::<Team>::new(home_team),
			away_team: ModelId::<Team>::new(away_team),
			game_status: GameStatus::try_from(nfl_game.game_status_id)?,
			weather: ModelId::<Weather>::new(weather),
		})
	}

	async fn update(pool: &SqlitePool, id: i64, item: CreateNFLGame) -> Result<Self::UpdateResult, Error> {
		let mut tx = pool.begin().await.map_err(NestError::from)?;

		let game_status = u32::from(item.game_status);
		let home_team = item.home_team.value();
		let away_team = item.away_team.value();
		let weather = item.weather.value();
		let result = sqlx::query!(
			"UPDATE nfl_games SET encoded_date = ?, home_team_id = ?, away_team_id = ?, game_status_id = ?, weather_id = ? WHERE id = ?",
			item.date.value,
			home_team,
			away_team,
			game_status,
			weather,
			id
		)
		.execute(&mut tx)
		.await
		.map_err(NestError::from)?;

		if result.rows_affected() == 0 {
			return Err(Error::NestError(NestError::NotFound));
		}

		tx.commit().await.map_err(NestError::from)?;

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

// Helper struct for Weather database rows
#[derive(Debug)]
struct WeatherRow {
	id: i64,
	condition: i64,
	day_night: i64,
	temperature: f64,
	wind_speed: Option<f64>,
}

#[derive(Debug)]
struct TeamRow {
	id: i64,
	name_id: i64,
	stadium_id: i64,
}

pub struct NFLGameRow {
	id: i64,
	encoded_date: i64,
	home_team_id: i64,
	away_team_id: i64,
	game_status_id: i64,
	weather_id: i64,
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

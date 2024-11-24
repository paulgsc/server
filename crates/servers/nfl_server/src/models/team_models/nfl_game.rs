use crate::common::nfl_server_error::NflServerError as Error;
use crate::common::EncodedDate;
use crate::common::{CrudOperations, Identifiable, ModelId};
use crate::models::team_models::TeamNameMeta;
use async_trait::async_trait;
use nest::http::Error as NestError;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

#[derive(Debug, Serialize, Deserialize)]
pub enum WeatherCondition {
	Sunny,
	Cloudy,
	Rainy,
	Snowy,
	Windy,
	Foggy,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum DayNight {
	Day,
	Night,
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
	pub abbreviation: ModelId<TeamNameMeta>,
	pub name: ModelId<TeamNameMeta>,
	pub mascot: ModelId<TeamNameMeta>,
	pub stadium: ModelId<Stadium>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct NFLGame {
	pub id: u32,
	pub date: EncodedDate,
	pub home_team: ModelId<Team>,
	pub away_team: ModelId<Team>,
	pub game_score: ModelId<GameScore>,
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
	async fn create(pool: &SqlitePool, item: CreateWeather) -> Result<Weather, Error> {
		if !item.is_valid() {
			return Err(Error::NestError(NestError::Forbidden));
		}

		let result = sqlx::query!(
			"INSERT INTO weather (condition, day_night, temperature, wind_speed) VALUES (?, ?, ?, ?)",
			item.condition as i32,
			item.day_night as i32,
			item.temperature,
			item.wind_speed
		)
		.execute(pool)
		.await
		.map_err(NestError::from)?;

		Ok(Self {
			id: result.last_insert_rowid() as u32,
			condition: item.condition,
			day_night: item.day_night,
			temperature: item.temperature,
			wind_speed: item.wind_speed,
		})
	}

	async fn batch_create(pool: &SqlitePool, items: &[CreateWeather]) -> Result<Vec<Weather>, Error> {
		let mut tx = pool.begin().await.map_err(NestError::from)?;
		let mut created_weather = Vec::with_capacity(items.len());

		for item in items {
			let result = sqlx::query!(
				"INSERT INTO weather (condition, day_night, temperature, wind_speed) VALUES (?, ?, ?, ?)",
				item.condition as i32,
				item.day_night as i32,
				item.temperature,
				item.wind_speed
			)
			.execute(&mut *tx)
			.await
			.map_err(NestError::from)?;

			created_weather.push(Weather {
				id: result.last_insert_rowid() as u32,
				condition: item.condition,
				day_night: item.day_night,
				temperature: item.temperature,
				wind_speed: item.wind_speed,
			});
		}

		tx.commit().await.map_err(NestError::from)?;
		Ok(created_weather)
	}

	async fn get(pool: &SqlitePool, id: i64) -> Result<Weather, Error> {
		let weather = sqlx::query_as!(WeatherRow, "SELECT id, condition, day_night, temperature, wind_speed FROM weather WHERE id = ?", id)
			.fetch_optional(pool)
			.await
			.map_err(NestError::from)?
			.ok_or(Error::NestError(NestError::NotFound))?;

		Ok(Self {
			id: weather.id as u32,
			condition: WeatherCondition::try_from(weather.condition)?,
			day_night: DayNight::try_from(weather.day_night)?,
			temperature: weather.temperature,
			wind_speed: weather.wind_speed,
		})
	}

	async fn update(pool: &SqlitePool, id: i64, item: CreateWeather) -> Result<Weather, Error> {
		let mut tx = pool.begin().await.map_err(NestError::from)?;

		let result = sqlx::query!(
			"UPDATE weather SET condition = ?, day_night = ?, temperature = ?, wind_speed = ? WHERE id = ?",
			item.condition as i32,
			item.day_night as i32,
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

		Ok(Weather {
			id: id as u32,
			condition: item.condition,
			day_night: item.day_night,
			temperature: item.temperature,
			wind_speed: item.wind_speed,
		})
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
	pub abbreviation: ModelId<TeamNameMeta>,
	pub name: ModelId<TeamNameMeta>,
	pub mascot: ModelId<TeamNameMeta>,
	pub stadium: ModelId<Stadium>,
}

#[async_trait]
impl CrudOperations<Team, CreateTeam> for Team {
	async fn create(pool: &SqlitePool, item: CreateTeam) -> Result<Self, Error> {
		let result = sqlx::query!(
			"INSERT INTO teams (abbreviation_id, name_id, mascot_id, stadium_id) VALUES (?, ?, ?, ?)",
			item.abbreviation.value(),
			item.name.value(),
			item.mascot.value(),
			item.stadium.value()
		)
		.execute(pool)
		.await
		.map_err(NestError::from)?;

		Ok(Self {
			id: result.last_insert_rowid() as u32,
			abbreviation: item.abbreviation,
			name: item.name,
			mascot: item.mascot,
			stadium: item.stadium,
		})
	}

	async fn batch_create(pool: &SqlitePool, items: &[CreateTeam]) -> Result<Vec<Self>, Error> {
		let mut tx = pool.begin().await.map_err(NestError::from)?;
		let mut teams = Vec::with_capacity(items.len());

		for item in items {
			let result = sqlx::query!(
				"INSERT INTO teams (abbreviation_id, name_id, mascot_id, stadium_id) VALUES (?, ?, ?, ?)",
				item.abbreviation.value(),
				item.name.value(),
				item.mascot.value(),
				item.stadium.value()
			)
			.execute(&mut *tx)
			.await
			.map_err(NestError::from)?;

			teams.push(Team {
				id: result.last_insert_rowid() as u32,
				abbreviation: item.abbreviation,
				name: item.name,
				mascot: item.mascot,
				stadium: item.stadium,
			});
		}

		tx.commit().await.map_err(NestError::from)?;
		Ok(teams)
	}

	async fn get(pool: &SqlitePool, id: i64) -> Result<Self, Error> {
		let team = sqlx::query_as!(WeatherRow, "SELECT id, abbreviation_id, name_id, mascot_id, stadium_id FROM teams WHERE id = ?", id)
			.fetch_optional(pool)
			.await
			.map_err(NestError::from)?
			.ok_or(Error::NestError(NestError::NotFound))?;

		Ok(Self {
			id: team.id,
			abbreviation: team.abbreviation,
			name: team.name,
			mascot: team.mascot,
			stadium: team.stadium,
		})
	}

	async fn update(pool: &SqlitePool, id: i64, item: CreateTeam) -> Result<Self, Error> {
		let mut tx = pool.begin().await.map_err(NestError::from)?;

		let result = sqlx::query!(
			"UPDATE teams SET abbreviation_id = ?, name_id = ?, mascot_id = ?, stadium_id = ? WHERE id = ?",
			item.abbreviation.value(),
			item.name.value(),
			item.mascot.value(),
			item.stadium.value(),
			id,
		)
		.execute(&mut tx)
		.await
		.map_err(NestError::from)?;

		if result.rows_affected() == 0 {
			return Err(Error::NestError(NestError::NotFound));
		}

		tx.commit().await.map_err(NestError::from)?;

		Ok(Self {
			id: result.id,
			abbreviation: item.abbreviation,
			name: item.name,
			mascot: item.mascot,
			stadium: item.stadium,
		})
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
	pub game_score: ModelId<GameScore>,
	pub weather: ModelId<Weather>,
}

#[async_trait]
impl CrudOperations<NFLGame, CreateNFLGame> for NFLGame {
	async fn create(pool: &SqlitePool, item: CreateNFLGame) -> Result<Self, Error> {
		let result = sqlx::query!(
			"INSERT INTO nfl_games (date, home_team_id, away_team_id, game_score_id, weather_id) VALUES (?, ?, ?, ?, ?)",
			item.date.value,
			item.home_team.value(),
			item.away_team.value(),
			item.game_score.value(),
			item.weather.value()
		)
		.execute(pool)
		.await
		.map_err(NestError::from)?;

		Ok(Self {
			id: result.last_insert_rowid() as u32,
			date: item.date,
			home_team: item.home_team,
			away_team: item.away_team,
			game_score: item.game_score,
			weather: item.weather,
		})
	}

	async fn batch_create(pool: &SqlitePool, items: &[CreateNFLGame]) -> Result<Vec<Self>, Error> {
		let mut tx = pool.begin().await.map_err(NestError::from)?;
		let mut created_nfl_game = Vec::with_capacity(items.len());

		for item in items {
			let result = sqlx::query!(
				"INSERT INTO nfl_games (date, home_team_id, away_team_id, game_score_id, weather_id) VALUES (?, ?, ?, ?, ?)",
				item.date.value,
				item.home_team.value(),
				item.away_team.value(),
				item.game_score.value(),
				item.weather.value()
			)
			.execute(&mut *tx)
			.await
			.map_err(NestError::from)?;

			created_nfl_game.push(NFLGame {
				id: result.last_insert_rowid() as u32,
				date: item.date,
				home_team: item.home_team,
				away_team: item.away_team,
				game_score: item.game_score,
				weather: item.weather,
			});
		}

		tx.commit().await.map_err(NestError::from)?;
		Ok(created_nfl_game)
	}

	async fn get(pool: &SqlitePool, id: i64) -> Result<Self, Error> {
		let nfl_game = sqlx::query_as!(
			NFLGame,
			"SELECT date, home_team_id, away_team_id, game_score_id, weather_id from nfl_games WHERE id = ?",
			id
		)
		.fetch_optional(pool)
		.await
		.map_err(NestError::from)?
		.ok_or(Error::NestError(NestError::NotFound))?;

		Ok(NFLGame {
			id: nfl_game.id,
			date: nfl_game.date,
			home_team: nfl_game.home_team,
			away_team: nfl_game.away_team,
			game_score: nfl_game.game_score,
			weather: nfl_game.weather,
		})
	}

	async fn update(pool: &SqlitePool, id: i64, item: CreateNFLGame) -> Result<Self, Error> {
		let mut tx = pool.begin().await.map_err(NestError::from)?;

		let result = sqlx::query!(
			"UPDATE nfl_games SET date = ?, home_team_id = ?, away_team_id = ?, game_score_id, weather_id = ? WHERE id = ?",
			item.date,
			item.home_team,
			item.away_team,
			item.game_score,
			item.weather,
			id
		)
		.execute(&mut tx)
		.await
		.map_err(NestError::from)?;

		if result.rows_affected() == 0 {
			return Err(Error::NestError(NestError::NotFound));
		}

		tx.commit().await.map_err(NestError::from)?;

		Ok(Self {
			id: id as u32,
			date: item.date,
			home_team: item.home_team,
			away_team: item.away_team,
			weather: item.weather,
		})
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
	condition: i32,
	day_night: i32,
	temperature: f32,
	wind_speed: Option<f32>,
}

// Conversion implementations for enums
impl TryFrom<i32> for WeatherCondition {
	type Error = Error;
	fn try_from(value: i32) -> Result<Self, Self::Error> {
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

impl TryFrom<i32> for DayNight {
	type Error = Error;
	fn try_from(value: i32) -> Result<Self, Self::Error> {
		match value {
			0 => Ok(DayNight::Day),
			1 => Ok(DayNight::Night),
			_ => Err(Error::NestError(NestError::unprocessable_entity(vec![("Daynight", "Invalid DayNight")]))),
		}
	}
}

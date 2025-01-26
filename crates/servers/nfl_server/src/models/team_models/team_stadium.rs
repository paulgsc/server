use crate::common::nfl_server_error::NflServerError as Error;
use crate::common::{CrudOperations, Identifiable};
use async_trait::async_trait;
use nest::http::Error as NestError;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Location {
	pub state: i64,
	pub city: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum StadiumType {
	Indoor,
	Outdoor,
	Retractable,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum SurfaceType {
	Grass,
	AstroTurf,
	Hybrid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Stadium {
	pub id: i64,
	pub name: String,
	pub location: Location,
	pub stadium_type: StadiumType,
	pub surface_type: SurfaceType,
	pub capacity: i64,
}

#[derive(Debug, Deserialize)]
pub struct CreateStadium {
	pub name: String,
	pub location: Location,
	pub stadium_type: StadiumType,
	pub surface_type: SurfaceType,
	pub capacity: i64,
}

impl Identifiable for Stadium {
	fn id(&self) -> i64 {
		self.id
	}
}

impl Stadium {
	pub fn is_valid(&self) -> bool {
		todo!("Implement this function later");
	}
}

impl CreateStadium {
	pub fn is_valid(&self) -> bool {
		!self.name.trim().is_empty() && !self.location.city.trim().is_empty() && self.capacity >= 1_000 && self.capacity <= 150_000
	}
}

#[async_trait]
impl CrudOperations<Stadium, CreateStadium> for Stadium {
	type CreateResult = i64;
	type BatchCreateResult = ();
	type GetResult = Self;
	type UpdateResult = ();

	async fn create(pool: &SqlitePool, item: CreateStadium) -> Result<Self::CreateResult, Error> {
		if !item.is_valid() {
			return Err(Error::NestError(NestError::Forbidden));
		}

		let stadium_type_val = i64::from(item.stadium_type);
		let surface_type_val = i64::from(item.surface_type);
		let result = sqlx::query!(
			r#"
            INSERT INTO stadiums (
                name, 
                state,
                city,
                stadium_type,
                surface_type,
                capacity
            ) 
            VALUES (?, ?, ?, ?, ?, ?)
            "#,
			item.name,
			item.location.state,
			item.location.city,
			stadium_type_val,
			surface_type_val,
			item.capacity,
		)
		.execute(pool)
		.await
		.map_err(NestError::from)?;

		Ok(result.last_insert_rowid())
	}

	async fn batch_create(pool: &SqlitePool, items: &[CreateStadium]) -> Result<Self::BatchCreateResult, Error> {
		let mut tx = pool.begin().await.map_err(NestError::from)?;

		for item in items {
			if !item.is_valid() {
				tx.rollback().await.map_err(NestError::from)?;
				return Err(Error::NestError(NestError::Forbidden));
			}

			let stadium_type_val = i64::from(item.stadium_type);
			let surface_type_val = i64::from(item.surface_type);
			sqlx::query!(
				r#"
                INSERT INTO stadiums (
                    name, 
                    state,
                    city,
                    stadium_type,
                    surface_type,
                    capacity
                ) 
                VALUES (?, ?, ?, ?, ?, ?)
                "#,
				item.name,
				item.location.state,
				item.location.city,
				stadium_type_val,
				surface_type_val,
				item.capacity,
			)
			.execute(&mut *tx)
			.await
			.map_err(NestError::from)?;
		}

		tx.commit().await.map_err(NestError::from)?;
		Ok(())
	}

	async fn get(pool: &SqlitePool, id: i64) -> Result<Self::GetResult, Error> {
		let stadium = sqlx::query_as!(
			StadiumRow,
			r#"
            SELECT 
                id,
                name,
                state,
                city,
                stadium_type,
                surface_type,
                capacity
            FROM stadiums 
            WHERE id = ?
            "#,
			id
		)
		.fetch_optional(pool)
		.await
		.map_err(NestError::from)?
		.ok_or(Error::NestError(NestError::NotFound))?;

		Ok(Self {
			id: stadium.id as i64,
			name: stadium.name,
			location: Location {
				state: stadium.state,
				city: stadium.city,
			},
			stadium_type: StadiumType::try_from(stadium.stadium_type)?,
			surface_type: SurfaceType::try_from(stadium.surface_type)?,
			capacity: stadium.capacity,
		})
	}

	async fn update(pool: &SqlitePool, id: i64, item: CreateStadium) -> Result<Self::UpdateResult, Error> {
		if !item.is_valid() {
			return Err(Error::NestError(NestError::Forbidden));
		}

		let mut tx = pool.begin().await.map_err(NestError::from)?;

		let stadium_type_val = i64::from(item.stadium_type);
		let surface_type_val = i64::from(item.surface_type);
		let result = sqlx::query!(
			r#"
            UPDATE stadiums SET
                name = ?,
                state = ?,
                city = ?,
                stadium_type = ?,
                surface_type = ?,
                capacity = ?
            WHERE id = ?
            "#,
			item.name,
			item.location.state,
			item.location.city,
			stadium_type_val,
			surface_type_val,
			item.capacity,
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
		let result = sqlx::query!("DELETE FROM stadiums WHERE id = ?", id).execute(pool).await.map_err(NestError::from)?;

		if result.rows_affected() == 0 {
			return Err(Error::NestError(NestError::NotFound));
		}
		Ok(())
	}
}

// Database migrations to create the stadiums table
#[derive(sqlx::FromRow)]
struct StadiumRow {
	id: i64,
	name: String,
	state: i64,
	city: String,
	stadium_type: i64,
	surface_type: i64,
	capacity: i64,
}

impl TryFrom<i64> for StadiumType {
	type Error = Error;

	fn try_from(value: i64) -> Result<Self, Self::Error> {
		match value {
			0 => Ok(StadiumType::Indoor),
			1 => Ok(StadiumType::Outdoor),
			2 => Ok(StadiumType::Retractable),
			_ => Err(Error::NestError(NestError::InvalidData)),
		}
	}
}

impl From<StadiumType> for i64 {
	fn from(stadium_type: StadiumType) -> i64 {
		match stadium_type {
			StadiumType::Indoor => 0,
			StadiumType::Outdoor => 1,
			StadiumType::Retractable => 2,
		}
	}
}

impl TryFrom<i64> for SurfaceType {
	type Error = Error;

	fn try_from(value: i64) -> Result<Self, Self::Error> {
		match value {
			0 => Ok(SurfaceType::Grass),
			1 => Ok(SurfaceType::AstroTurf),
			2 => Ok(SurfaceType::Hybrid),
			_ => Err(Error::NestError(NestError::InvalidData)),
		}
	}
}

impl From<SurfaceType> for i64 {
	fn from(surface: SurfaceType) -> i64 {
		match surface {
			SurfaceType::Grass => 0,
			SurfaceType::AstroTurf => 1,
			SurfaceType::Hybrid => 2,
		}
	}
}

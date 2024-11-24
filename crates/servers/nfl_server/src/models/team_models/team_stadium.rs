use crate::common::nfl_server_error::NflServerError as Error;
use crate::common::{CrudOperations, Identifiable};
use async_trait::async_trait;
use nest::http::Error as NestError;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Location {
	pub state: State,
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
	pub id: u32,
	pub name: String,
	pub location: Location,
	pub stadium_type: StadiumType,
	pub surface_type: SurfaceType,
	pub capacity: u32,
}

#[derive(Debug, Deserialize)]
pub struct CreateStadium {
	pub name: String,
	pub location: Location,
	pub stadium_type: StadiumType,
	pub surface_type: SurfaceType,
	pub capacity: u32,
}

impl Identifiable for Stadium {
	fn id(&self) -> u32 {
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
	async fn create(pool: &SqlitePool, item: CreateStadium) -> Result<Stadium, Error> {
		if !item.is_valid() {
			return Err(Error::NestError(NestError::Forbidden));
		}

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
			item.location.state as i32,
			item.location.city,
			item.stadium_type as i32,
			item.surface_type as i32,
			item.capacity,
		)
		.execute(pool)
		.await
		.map_err(NestError::from)?;

		Ok(Self {
			id: result.last_insert_rowid() as u32,
			name: item.name,
			location: item.location,
			stadium_type: item.stadium_type,
			surface_type: item.surface_type,
			capacity: item.capacity,
		})
	}

	async fn batch_create(pool: &SqlitePool, items: &[CreateStadium]) -> Result<Vec<Stadium>, Error> {
		let mut tx = pool.begin().await.map_err(NestError::from)?;
		let mut created_stadiums = Vec::with_capacity(items.len());

		for item in items {
			if !item.is_valid() {
				tx.rollback().await.map_err(NestError::from)?;
				return Err(Error::NestError(NestError::Forbidden));
			}

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
				item.location.state as i32,
				item.location.city,
				item.stadium_type as i32,
				item.surface_type as i32,
				item.capacity,
			)
			.execute(&mut *tx)
			.await
			.map_err(NestError::from)?;

			created_stadiums.push(Stadium {
				id: result.last_insert_rowid() as u32,
				name: item.name.clone(),
				location: item.location.clone(),
				stadium_type: item.stadium_type,
				surface_type: item.surface_type,
				capacity: item.capacity,
			});
		}

		tx.commit().await.map_err(NestError::from)?;
		Ok(created_stadiums)
	}

	async fn get(pool: &SqlitePool, id: i64) -> Result<Stadium, Error> {
		let stadium = sqlx::query!(
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
			id: stadium.id as u32,
			name: stadium.name,
			location: Location {
				state: State::try_from(stadium.state)?,
				city: stadium.city,
			},
			stadium_type: StadiumType::try_from(stadium.stadium_type)?,
			surface_type: SurfaceType::try_from(stadium.surface_type)?,
			capacity: stadium.capacity as u32,
		})
	}

	async fn update(pool: &SqlitePool, id: i64, item: CreateStadium) -> Result<Stadium, Error> {
		if !item.is_valid() {
			return Err(Error::NestError(NestError::Forbidden));
		}

		let mut tx = pool.begin().await.map_err(NestError::from)?;

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
			item.location.state as i32,
			item.location.city,
			item.stadium_type as i32,
			item.surface_type as i32,
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

		Ok(Stadium {
			id: id as u32,
			name: item.name,
			location: item.location,
			stadium_type: item.stadium_type,
			surface_type: item.surface_type,
			capacity: item.capacity,
		})
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
	state: i32,
	city: String,
	stadium_type: i32,
	surface_type: i32,
	capacity: i64,
}

impl TryFrom<i32> for StadiumType {
	type Error = Error;

	fn try_from(value: i32) -> Result<Self, Self::Error> {
		match value {
			0 => Ok(StadiumType::Indoor),
			1 => Ok(StadiumType::Outdoor),
			2 => Ok(StadiumType::Retractable),
			_ => Err(Error::NestError(NestError::InvalidData)),
		}
	}
}

impl TryFrom<i32> for SurfaceType {
	type Error = Error;

	fn try_from(value: i32) -> Result<Self, Self::Error> {
		match value {
			0 => Ok(SurfaceType::Grass),
			1 => Ok(SurfaceType::AstroTurf),
			2 => Ok(SurfaceType::Hybrid),
			_ => Err(Error::NestError(NestError::InvalidData)),
		}
	}
}

use crate::common::nfl_server_error::NflServerError as Error;
use crate::common::{CrudOperations, Identifiable};
use async_trait::async_trait;
use nest::http::Error as NestError;
use serde::{Deserialize, Serialize};
use sqlx::sqlite::{SqliteArgumentValue, SqliteValueRef};
use sqlx::{Decode, Encode, Row, Sqlite, SqlitePool};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Location {
	pub state: i32,
	pub city: String,
}

impl Encode<'_, Sqlite> for Location {
	fn encode_by_ref(&self, buf: &mut Vec<SqliteArgumentValue<'_>>) -> sqlx::encode::IsNull {
		buf.push(SqliteArgumentValue::Int(self.state));
		buf.push(SqliteArgumentValue::Text(self.city.clone().into()));

		sqlx::encode::IsNull::No
	}
}

impl<'r> Decode<'r, Sqlite> for Location {
	fn decode(_value: SqliteValueRef<'r>) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
		Err("Decode for Location must use separate columns for state and city.".into())
	}
}

impl sqlx::FromRow<'_, sqlx::sqlite::SqliteRow> for Location {
	fn from_row(row: &sqlx::sqlite::SqliteRow) -> Result<Self, sqlx::Error> {
		let state: i32 = row.try_get("state")?;
		let city: String = row.try_get("city")?;
		Ok(Self { state, city })
	}
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

#[async_trait]
impl CrudOperations<Stadium> for Stadium {
	type CreateResult = i64;
	type BatchCreateResult = ();
	type GetResult = Self;
	type UpdateResult = ();

	async fn create(pool: &SqlitePool, item: Stadium) -> Result<Self::CreateResult, Error> {
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

	async fn batch_create(pool: &SqlitePool, items: &[Stadium]) -> Result<Self::BatchCreateResult, Error> {
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
			Stadium,
			r#"
            SELECT 
                id,
                name,
		json_object('state', COALESCE(state, 0), 'city', COALESCE(city, '')) as "location!: Location",
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
			location: stadium.location,
			stadium_type: stadium.stadium_type,
			surface_type: stadium.surface_type,
			capacity: stadium.capacity,
		})
	}

	async fn update(pool: &SqlitePool, id: i64, item: Stadium) -> Result<Self::UpdateResult, Error> {
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

impl From<i64> for StadiumType {
	fn from(value: i64) -> Self {
		match value {
			0 => Self::Indoor,
			1 => Self::Outdoor,
			2 => Self::Retractable,
			_ => panic!("Invalid StadiumType: {value}"),
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

impl From<i64> for SurfaceType {
	fn from(value: i64) -> Self {
		match value {
			0 => Self::Grass,
			1 => Self::AstroTurf,
			2 => Self::Hybrid,
			_ => panic!("Invalid SurfaceType: {value}"),
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

use async_trait::async_trait;
use sqlx::SqlitePool;

use nest::http::Error;

#[async_trait]
pub trait CrudOperations<T, C> {
	async fn create(pool: &SqlitePool, item: &C) -> Result<T, Error>;
	async fn get(pool: &SqlitePool, id: i64) -> Result<T, Error>;
	async fn update(pool: &SqlitePool, id: i64, item: &C) -> Result<T, Error>;
	async fn delete(pool: &SqlitePool, id: i64) -> Result<(), Error>;
}

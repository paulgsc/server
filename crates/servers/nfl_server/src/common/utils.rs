use crate::common::nfl_server_error::NflServerError as Error;
use async_trait::async_trait;
use regex::Regex;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

#[derive(Debug, Serialize, Deserialize)]
pub struct ModelId<T>(pub u32, pub std::marker::PhantomData<T>);

impl<T> ModelId<T> {
	pub const fn new(id: u32) -> Self {
		Self(id, std::marker::PhantomData)
	}

	pub const fn value(&self) -> u32 {
		self.0
	}
}

pub trait Identifiable {
	fn model_id(&self) -> ModelId<Self>
	where
		Self: Sized,
	{
		ModelId::new(self.id())
	}

	fn id(&self) -> u32;
}

#[async_trait]
pub trait CrudOperations<T, C>
where
	T: Send + Sync + 'static,
	C: Send + Sync + 'static,
{
	type CreateResult: Send + Sync + 'static;
	type BatchCreateResult: Send + Sync + 'static;
	type GetResult: Send + Sync + 'static;
	type UpdateResult: Send + Sync + 'static;

	async fn create(pool: &SqlitePool, item: C) -> Result<Self::CreateResult, Error>;
	async fn batch_create(pool: &SqlitePool, items: &[C]) -> Result<Self::BatchCreateResult, Error>;
	async fn get(pool: &SqlitePool, id: i64) -> Result<Self::GetResult, Error>;
	async fn update(pool: &SqlitePool, id: i64, item: C) -> Result<Self::UpdateResult, Error>;
	async fn delete(pool: &SqlitePool, id: i64) -> Result<(), Error>;
}

pub fn is_timestamp(s: &str) -> bool {
	let time_patterns = [
		r"^\d{1,2}:\d{2}\s*[ap]m$",     // 10:00 am, 9:30 PM
		r"^\d{1,2}:\d{2}\s*[AP]M$",     // 10:00 AM, 9:30 PM
		r"^\d{1,2}:\d{2}\s*[ap]\.m\.$", // 10:00 a.m., 9:30 p.m.
		r"^\d{1,2}:\d{2}$",             // 10:00, 09:30
	];

	time_patterns.iter().any(|pattern| Regex::new(pattern).expect("Invalid regex").is_match(s))
}

use crate::common::nfl_server_error::NflServerError as Error;
use crate::common::CrudOperations;
use async_trait::async_trait;
use nest::http::Error as NestError;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

#[derive(Debug, Serialize, Deserialize)]
enum ChatbotPosition {
	Left,
	Right,
}

#[derive(Debug, Serialize, Deserialize)]
struct Avatar {
	src: String,
	alt: Box<str>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ChatbotMessage {
	pub id: Box<str>,
	pub character: Box<str>,
	pub position: ChatbotPosition,
	pub content: String,
	pub is_thinking: bool,
	pub avatar: Avatar,
}

#[async_trait]
impl CrudOperations<ChatbotMessage> for ChatbotMessage {
	type CreateResult = i64;
	type BatchCreateResult = ();
	type GetResult = Self;
	type UpdateResult = ();

	async fn create(pool: &SqlitePool, item: ChatbotMessage) -> Result<Self::CreateResult, Error> {
		let count = sqlx::query!("SELECT COUNT(*) as count FROM chatbot_messages")
			.fetch_one(pool)
			.await
			.map_err(NestError::from)?
			.count;

		if count >= 960 {
			return Err(Error::NestError(NestError::MaxRecordLimitExceeded));
		}

		let result = sqlx::query!("INSERT INTO chatbot_messages (minutes, seconds) VALUES (?, ?)", item.minutes, item.seconds)
			.execute(pool)
			.await
			.map_err(NestError::from)?;

		Ok(result.last_insert_rowid())
	}

	async fn batch_create(pool: &SqlitePool, items: &[ChatbotMessage]) -> Result<Self::BatchCreateResult, Error> {
		let mut tx = pool.begin().await.map_err(NestError::from)?;

		let count: i32 = sqlx::query_scalar!("SELECT COUNT(*) FROM chatbot_messages")
			.fetch_one(&mut *tx)
			.await
			.map_err(NestError::from)?;

		if count + items.len() as i32 > 960 {
			return Err(Error::NestError(NestError::MaxRecordLimitExceeded));
		}

		for item in items {
			sqlx::query!("INSERT INTO chatbot_messages (minutes, seconds) VALUES (?, ?)", item.minutes, item.seconds)
				.execute(&mut *tx)
				.await
				.map_err(NestError::from)?;
		}

		tx.commit().await.map_err(NestError::from)?;

		Ok(())
	}

	async fn get(pool: &SqlitePool, id: i64) -> Result<Self::GetResult, Error> {
		let chatbot_messages = sqlx::query_as!(ChatbotMessage, "SELECT id, minutes, seconds FROM chatbot_messages WHERE id = ?", id)
			.fetch_optional(pool)
			.await
			.map_err(NestError::from)?
			.ok_or(Error::NestError(NestError::NotFound))?;

		Ok(chatbot_messages)
	}

	async fn update(pool: &SqlitePool, id: i64, item: ChatbotMessage) -> Result<Self::UpdateResult, Error> {
		let result = sqlx::query!("UPDATE chatbot_messages SET minutes = ?, seconds = ? WHERE id = ?", item.minutes, item.seconds, id)
			.execute(pool)
			.await
			.map_err(NestError::from)?;

		if result.rows_affected() == 0 {
			return Err(Error::NestError(NestError::NotFound));
		}

		Ok(())
	}

	async fn delete(pool: &SqlitePool, id: i64) -> Result<(), Error> {
		let result = sqlx::query!("DELETE FROM chatbot_messages WHERE id = ?", id).execute(pool).await.map_err(NestError::from)?;

		if result.rows_affected() == 0 {
			return Err(Error::NestError(NestError::NotFound));
		}

		Ok(())
	}
}

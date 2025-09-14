//! # SQLite Template
//!
//! A generic SQLite CRUD template that provides database operations for any schema.
//!
//! ## Features
//!
//! - Generic CRUD operations for any entity type
//! - Transaction support
//! - Query building with conditions, ordering, and pagination
//! - Batch operations
//! - Schema management and migrations
//! - Derive macros for easy setup
//!
//! ## Quick Start

//! # SQLite Template Demo - Fixed Version
//!
//! A generic SQLite CRUD template that provides database operations for any schema.

use chrono::Utc;
use serde::{Deserialize, Serialize};
use some_sqlite::{
	client::SqliteRepository,
	traits::Repository, // Import the Repository trait
	DatabaseConfig,
	OrderBy,
	QueryCondition,
	QueryParams,
	SqliteDatabaseManager,
};
pub use sqlite_macros_2::*;
use uuid::Uuid;

// Define your entity
#[derive(Debug, Clone, Serialize, Deserialize, Entity, Schema, sqlx::FromRow)]
#[schema(table = "users", primary_key = "id")]
struct User {
	#[primary_key]
	pub id: Uuid,
	pub name: String,
	pub email: String,
	pub created_at: i64,
}

// Implement columns_and_values for User if the derive macro doesn't provide it
impl User {
	pub fn columns_and_values(&self) -> (Vec<String>, Vec<some_sqlite::QueryValue>) {
		use some_sqlite::QueryValue;

		let columns = vec!["id".to_string(), "name".to_string(), "email".to_string(), "created_at".to_string()];

		let values = vec![
			QueryValue::String(self.id.to_string()),
			QueryValue::String(self.name.clone()),
			QueryValue::String(self.email.clone()),
			QueryValue::Integer(self.created_at),
		];

		(columns, values)
	}
}

// Define the creation struct
#[derive(Debug, Clone, Serialize, Deserialize, NewEntity)]
#[new_entity(entity = "User", table = "users")]
struct NewUser {
	pub name: String,
	pub email: String,
	pub created_at: i64,
}

// Implement columns_and_values for NewUser if the derive macro doesn't provide it
impl NewUser {
	pub fn columns_and_values(&self) -> (Vec<String>, Vec<some_sqlite::QueryValue>) {
		use some_sqlite::QueryValue;

		let columns = vec!["id".to_string(), "name".to_string(), "email".to_string(), "created_at".to_string()];

		let values = vec![
			QueryValue::String(Uuid::new_v4().to_string()), // Generate new UUID
			QueryValue::String(self.name.clone()),
			QueryValue::String(self.email.clone()),
			QueryValue::Integer(self.created_at),
		];

		(columns, values)
	}
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
	// ✅ 1. Initialize database configuration
	let config = DatabaseConfig {
		database_url: "sqlite:demo.db".to_string(),
		max_connections: Some(5),
		min_connections: Some(1),
		acquire_timeout: Some(std::time::Duration::from_secs(30)),
		idle_timeout: Some(std::time::Duration::from_secs(600)),
	};
	let db = SqliteDatabaseManager::new(config).await?;

	// ✅ 2. Initialize schema manually (since UserSchema is not available)
	// Create the users table manually
	let pool = db.pool(); // Assuming SqliteDatabaseManager has a pool() method
	sqlx::query(
		r#"
					        CREATE TABLE IF NOT EXISTS users (
							            id TEXT PRIMARY KEY,
										            name TEXT NOT NULL,
													            email TEXT NOT NULL UNIQUE,
																            created_at INTEGER NOT NULL
																			        )
																					    "#,
	)
	.execute(pool)
	.await?;

	// ✅ 3. Get repository
	let user_repo: SqliteRepository<User> = db.repository();

	// ✅ 4. Create user
	let new_user = NewUser {
		name: "John Doe".to_string(),
		email: "john@example.com".to_string(),
		created_at: Utc::now().timestamp(),
	};
	let user = user_repo.create(new_user).await?;
	println!("Created user: {:?}", user);

	// ✅ 5. Find user by ID
	let found_user = user_repo.find_by_id(&user.id).await?.expect("User not found");
	println!("Found user: {:?}", found_user);

	// ✅ 6. Query users with filters (use find_by instead of find_by_query)
	let params = QueryParams {
		conditions: vec![QueryCondition::Like("email".to_string(), "%example.com".to_string())],
		order_by: vec![OrderBy {
			column: "name".to_string(),
			ascending: true,
		}],
		limit: Some(10),
		offset: None,
	};

	let users = user_repo.find_by(params).await?; // Use find_by method
	println!("Found {} users matching query", users.len());

	// ✅ 7. Update user
	let mut updated_user = found_user.clone();
	updated_user.name = "Jane Doe".to_string();
	let saved_user = user_repo.update(&updated_user).await?;
	println!("Updated user: {:?}", saved_user);

	// ✅ 8. Delete user
	let deleted_count = user_repo.delete_by_id(&saved_user.id).await?;
	println!("Deleted {} user(s)", deleted_count);

	// ✅ 9. Close DB gracefully
	db.close().await;

	Ok(())
}

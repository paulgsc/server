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
use some_sqlite::{client::SqliteRepository, traits::Repository, DatabaseConfig, Entity, OrderBy, QueryCondition, QueryParams, SqliteDatabaseManager};
pub use sqlite_macros_2::*;
use uuid::Uuid;

// ---------------------- Entities ----------------------

#[derive(Debug, Clone, Serialize, Deserialize, Entity, sqlx::FromRow)]
struct User {
	#[primary_key]
	pub id: Uuid,
	pub name: String,
	pub email: String,
	pub created_at: i64,
	pub nickname: Option<String>, // Example of optional field
}

// You can omit implementing columns_and_values manually; derive macro handles it

#[derive(Debug, Clone, Serialize, Deserialize, NewEntity)]
#[new_entity(entity = "User", table_name = "users")]
struct NewUser {
	pub name: String,
	pub email: String,
	pub created_at: i64,
	pub nickname: Option<String>, // optional fields are automatically handled
}

// ---------------------- Main ----------------------

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
	// 1️⃣ Database config
	let config = DatabaseConfig {
		database_url: "sqlite:demo.db".to_string(),
		max_connections: Some(5),
		min_connections: Some(1),
		acquire_timeout: Some(std::time::Duration::from_secs(30)),
		idle_timeout: Some(std::time::Duration::from_secs(600)),
	};
	let db = SqliteDatabaseManager::new(config).await?;

	// 2️⃣ Create table manually (or via Schema::create_table_sql)
	let pool = db.pool();
	sqlx::query(
		r#"
										CREATE TABLE IF NOT EXISTS users (
													id TEXT PRIMARY KEY,
																name TEXT NOT NULL,
																			email TEXT NOT NULL UNIQUE,
																						created_at INTEGER NOT NULL,
																									nickname TEXT
																											)
																													"#,
	)
	.execute(pool)
	.await?;

	// 3️⃣ Get repository
	let user_repo: SqliteRepository<User> = db.repository();

	// 4️⃣ Create new user
	let new_user = NewUser {
		name: "Alice".to_string(),
		email: "alice@example.com".to_string(),
		created_at: Utc::now().timestamp(),
		nickname: Some("Ally".to_string()),
	};
	let user = user_repo.create(new_user).await?;
	println!("Created user: {:?}", user);

	// 5️⃣ Find user by ID
	let found_user = user_repo.find_by_id(&user.id).await?.expect("User not found");
	println!("Found user: {:?}", found_user);

	// 6️⃣ Query users
	let params = QueryParams {
		conditions: vec![QueryCondition::Like("email".to_string(), "%example.com".to_string())],
		order_by: vec![OrderBy {
			column: "name".to_string(),
			ascending: true,
		}],
		limit: Some(10),
		offset: None,
	};
	let users = user_repo.find_by(params).await?;
	println!("Found {} users matching query", users.len());

	// 7️⃣ Update user
	let mut updated_user = found_user.clone();
	updated_user.name = "Alice Smith".to_string();
	let saved_user = user_repo.update(&updated_user).await?;
	println!("Updated user: {:?}", saved_user);

	// 8️⃣ Delete user
	let deleted_count = user_repo.delete_by_id(&saved_user.id).await?;
	println!("Deleted {} user(s)", deleted_count);

	// 9️⃣ Close DB
	db.close().await;

	Ok(())
}

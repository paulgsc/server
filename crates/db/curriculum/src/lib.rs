//! Server-owned lesson content (#256).
//!
//! One row per lesson — see `20260924001300_create_curriculum.up.sql` for
//! which fields are columns, which is a blob, and where each was transcribed
//! from. [`content_hash`] is the one definition of "did this lesson change"
//! the importer (#275), the routes' `ETag` (#276), and the `CurriculumUpdated`
//! producer (#277) all share.

pub mod importer;
pub mod model;
pub mod repository;

pub use importer::{import_dir, ImportError, ImportReport, PUBLICATION_SOURCE};
pub use model::{content_hash, CurriculumEntry, Level, ManifestEntry};
pub use repository::{Change, CurriculumRepository, MANIFEST_CEILING};

#[cfg(test)]
mod tests {
	use sqlx::sqlite::SqlitePoolOptions;
	use sqlx::SqlitePool;

	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");
	const PREVIOUS_MIGRATION: i64 = 20_260_924_001_200;

	async fn pool() -> SqlitePool {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();
		pool
	}

	async fn table_exists(pool: &SqlitePool) -> bool {
		sqlx::query_scalar!(r#"SELECT COUNT(*) AS "count!: i64" FROM sqlite_master WHERE type = 'table' AND name = 'curriculum'"#)
			.fetch_one(pool)
			.await
			.unwrap()
			== 1
	}

	#[tokio::test]
	async fn the_migration_round_trips() {
		let pool = pool().await;
		assert!(table_exists(&pool).await);
		MIGRATOR.undo(&pool, PREVIOUS_MIGRATION).await.unwrap();
		assert!(!table_exists(&pool).await, "down drops the table");
		MIGRATOR.run(&pool).await.unwrap();
		assert!(table_exists(&pool).await, "up recreates it");
	}

	/// The schema refuses a level outside `TopikMetadataSchema`'s
	/// `difficulty` enum, and admits NULL because that field is optional.
	#[tokio::test]
	async fn level_is_the_clients_difficulty_vocabulary_or_null() {
		let pool = pool().await;
		for (key, level, ok) in [("a", Some("beginner"), true), ("b", None, true), ("c", Some("expert"), false)] {
			let inserted = sqlx::query!(
				"INSERT INTO curriculum (key, activity_id, level, display_name, description, batch_count, total_questions, total_messages, tags, published_at, version, content_hash, body) VALUES (?, 'topik', ?, 'x', 'x', 1, 1, 1, NULL, '2026-09-24T00:00:00+00:00', 1, 'h', '{}')",
				key,
				level
			)
			.execute(&pool)
			.await;
			assert_eq!(inserted.is_ok(), ok, "{key}: {level:?}");
		}
	}
}

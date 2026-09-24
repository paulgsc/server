//! Per-block activity outcomes (#258).
//!
//! One row per activity block a subject played — see
//! `20260924001100_create_activity_outcome.up.sql` for the grain decision and
//! why each column is what it is. This crate is storage only: the model the
//! table round-trips through, and the retention sweep #265's rule requires.
//! Ingestion (`POST /outcomes`, deriving `ScoredBelowTarget`) is #287 (TEL2);
//! reads for the recommender are #289 (TEL4).

pub mod model;
pub mod repository;

pub use model::OutcomeKind;
pub use repository::{ActivityStats, OutcomeRecord, OutcomeRepository, Recorded, StatsError, ACTIVITY_OUTCOME_RETENTION_DAYS, STATS_CEILING};

#[cfg(test)]
mod tests {
	use super::{OutcomeKind, OutcomeRepository};
	use sqlx::sqlite::SqlitePoolOptions;
	use sqlx::SqlitePool;

	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");

	/// The migration this crate owns, and the one immediately before it —
	/// the `undo` target that removes exactly this table and nothing else.
	const THIS_MIGRATION: i64 = 20_260_924_001_100;
	const PREVIOUS_MIGRATION: i64 = 20_260_924_001_000;

	async fn pool() -> SqlitePool {
		// One connection: `:memory:` is private to the connection that opened
		// it, so a second one would see an empty, unmigrated schema.
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();
		pool
	}

	async fn insert(pool: &SqlitePool, session_id: &str, block_index: i64, outcome: &str, score: Option<f64>) -> Result<(), sqlx::Error> {
		sqlx::query!(
			r#"
			INSERT INTO activity_outcome
			    (subject_id, session_id, activity_id, block_index, started_at, ended_at, planned_ms, elapsed_ms, outcome, score)
			VALUES ('subject-local', ?1, 'honeycomb', ?2, '2026-09-24T10:00:00+00:00', '2026-09-24T10:05:00+00:00', 300000, 300000, ?3, ?4)
			"#,
			session_id,
			block_index,
			outcome,
			score
		)
		.execute(pool)
		.await
		.map(|_| ())
	}

	async fn table_exists(pool: &SqlitePool) -> bool {
		sqlx::query_scalar!(r#"SELECT COUNT(*) AS "count!: i64" FROM sqlite_master WHERE type = 'table' AND name = 'activity_outcome'"#)
			.fetch_one(pool)
			.await
			.unwrap()
			== 1
	}

	/// A row goes in and comes back out with its `outcome` parsing to the
	/// variant that went in, and a NULL score comes back NULL — not 0.0.
	#[tokio::test]
	async fn a_row_round_trips_and_a_null_score_stays_null() {
		let pool = pool().await;
		insert(&pool, "session-a", 0, OutcomeKind::Completed.as_str(), Some(0.0)).await.unwrap();
		insert(&pool, "session-a", 1, OutcomeKind::Abandoned.as_str(), None).await.unwrap();

		let rows = sqlx::query!(r#"SELECT block_index, outcome, score FROM activity_outcome WHERE session_id = 'session-a' ORDER BY block_index"#)
			.fetch_all(&pool)
			.await
			.unwrap();

		assert_eq!(OutcomeKind::parse(&rows[0].outcome), Some(OutcomeKind::Completed));
		assert_eq!(rows[0].score, Some(0.0), "assessed, and got nothing right");
		assert_eq!(OutcomeKind::parse(&rows[1].outcome), Some(OutcomeKind::Abandoned));
		assert_eq!(rows[1].score, None, "not assessed — a different fact from scoring zero");
	}

	/// `(session_id, block_index)` is the idempotency key: one block of one
	/// session yields one outcome. The same block index in a *different*
	/// session is a different block.
	#[tokio::test]
	async fn one_block_of_one_session_has_exactly_one_outcome() {
		let pool = pool().await;
		insert(&pool, "session-a", 0, "completed", Some(0.8)).await.unwrap();

		let replay = insert(&pool, "session-a", 0, "completed", Some(0.8)).await;
		assert!(replay.is_err(), "a second row for the same block must be refused, not appended");

		insert(&pool, "session-b", 0, "completed", Some(0.8)).await.unwrap();
	}

	/// The schema refuses what `OutcomeKind::parse` refuses, a score outside
	/// `[0, 1]`, and a score on anything but a completed block — so a path
	/// that forgot to validate cannot write one.
	#[tokio::test]
	async fn an_unknown_outcome_or_an_out_of_range_score_is_refused_by_the_schema() {
		let pool = pool().await;
		assert!(insert(&pool, "session-a", 0, "finished", None).await.is_err(), "not one of the three outcomes");
		assert!(insert(&pool, "session-a", 1, "completed", Some(1.5)).await.is_err(), "score above 1");
		assert!(insert(&pool, "session-a", 2, "completed", Some(-0.1)).await.is_err(), "score below 0");
		insert(&pool, "session-a", 3, "completed", Some(1.0)).await.unwrap();
		assert!(
			insert(&pool, "session-a", 4, "abandoned", Some(0.2)).await.is_err(),
			"an abandoned block is unassessed by definition"
		);
		assert!(insert(&pool, "session-a", 5, "skipped", Some(0.0)).await.is_err(), "and so is a skipped one");
		insert(&pool, "session-a", 6, "abandoned", None).await.unwrap();
	}

	/// `.down.sql` removes exactly this table, and `.up.sql` puts it back.
	#[tokio::test]
	async fn the_migration_round_trips() {
		let pool = pool().await;
		assert!(table_exists(&pool).await);

		MIGRATOR.undo(&pool, PREVIOUS_MIGRATION).await.unwrap();
		assert!(!table_exists(&pool).await, "down drops the table");
		let applied: Vec<i64> = sqlx::query_scalar!(r#"SELECT version AS "version!" FROM _sqlx_migrations ORDER BY version"#)
			.fetch_all(&pool)
			.await
			.unwrap();
		assert_eq!(applied.last(), Some(&PREVIOUS_MIGRATION), "and only this migration was undone");

		MIGRATOR.run(&pool).await.unwrap();
		assert!(table_exists(&pool).await, "up recreates it");
		assert!(
			sqlx::query_scalar!(r#"SELECT version AS "version!" FROM _sqlx_migrations WHERE version = ?"#, THIS_MIGRATION)
				.fetch_optional(&pool)
				.await
				.unwrap()
				.is_some()
		);
		insert(&pool, "session-a", 0, "skipped", None).await.unwrap();
		assert_eq!(OutcomeRepository::new(pool.clone()).prune("9999-01-01T00:00:00+00:00", 10).await.unwrap(), 1);
	}

	/// #289: counts per activity, skipped blocks excluded from plays, and the
	/// mean over assessed completed blocks only — a NULL score is excluded,
	/// never averaged in as zero.
	#[tokio::test]
	async fn stats_count_plays_and_average_only_assessed_scores() {
		let pool = pool().await;
		insert(&pool, "session-a", 0, "completed", Some(0.8)).await.unwrap();
		insert(&pool, "session-a", 1, "completed", None).await.unwrap();
		insert(&pool, "session-b", 0, "completed", Some(0.4)).await.unwrap();
		insert(&pool, "session-b", 1, "abandoned", None).await.unwrap();
		insert(&pool, "session-c", 0, "skipped", None).await.unwrap();

		let stats = OutcomeRepository::new(pool.clone()).stats("subject-local").await.unwrap();
		assert_eq!(stats.len(), 1, "every fixture row is honeycomb");
		let honeycomb = &stats[0];
		assert_eq!((honeycomb.plays, honeycomb.completed, honeycomb.abandoned, honeycomb.skipped), (4, 3, 1, 1));
		let mean = honeycomb.mean_score.unwrap();
		assert!((mean - 0.6).abs() < 1e-9, "(0.8 + 0.4) / 2, not (0.8 + 0 + 0.4) / 3: got {mean}");
		assert_eq!(honeycomb.completion_rate(), Some(0.75));
		assert_eq!(honeycomb.abandonment_rate(), Some(0.25));

		assert!(OutcomeRepository::new(pool.clone()).stats("subject-nobody").await.unwrap().is_empty());
	}

	/// Over the ceiling is refused, never a silent prefix (from a
	/// `chatgpt-codex-connector` finding on #361).
	#[tokio::test]
	async fn stats_over_the_ceiling_are_refused_not_truncated() {
		let pool = pool().await;
		for i in 0..=super::STATS_CEILING {
			let mut session = String::from("session-");
			session.push_str(&i.to_string());
			let mut activity = String::from("activity-");
			activity.push_str(&i.to_string());
			sqlx::query!(
				"INSERT INTO activity_outcome (subject_id, session_id, activity_id, block_index, started_at, ended_at, planned_ms, elapsed_ms, outcome, score) VALUES ('subject-local', ?, ?, 0, '2026-09-24T10:00:00+00:00', '2026-09-24T10:05:00+00:00', 1, 1, 'completed', NULL)",
				session,
				activity
			)
			.execute(&pool)
			.await
			.unwrap();
		}
		assert!(matches!(
			OutcomeRepository::new(pool.clone()).stats("subject-local").await,
			Err(super::StatsError::OverCeiling)
		));
	}
}

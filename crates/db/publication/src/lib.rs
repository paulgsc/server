//! The append-only log of published material, and its epoch (#273, CAT5).
//!
//! Storage for the `CurriculumUpdated` producer. Detecting that an activity
//! `(id, version)` is new appends it here; the log's `MAX(id)` is the
//! **epoch**. Nothing in this crate knows about subjects: who a publication
//! reaches is each subject's own watermark (`engagement_gate.curriculum_epoch`)
//! falling behind the epoch, and the waker catching them up — see
//! `20260924001200_create_curriculum_publication.up.sql` and
//! `study_domain::CURRICULUM_AUDIENCE`.

use sqlx::SqlitePool;

/// One entry in the log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Publication {
	/// The epoch this publication moved the log to.
	pub id: i64,
	pub source: String,
	pub curriculum_id: String,
	pub version: i64,
	pub detected_at: String,
}

/// The most catalogue rows one detection pass reads — `activity_repo::CATALOG_CEILING`,
/// restated so this crate need not depend on that one.
pub const ACTIVITY_DETECTION_CEILING: i64 = 500;

/// Why a detection pass could not run.
#[derive(Debug)]
pub enum PublicationError {
	/// The source table holds more rows than one pass reads; refused rather
	/// than read as a prefix.
	OverCeiling {
		rows: i64,
		ceiling: i64,
	},
	Storage(sqlx::Error),
}

impl std::fmt::Display for PublicationError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::OverCeiling { rows, ceiling } => write!(f, "{rows} rows, over the {ceiling} one detection pass reads; refusing to announce a partial catalogue"),
			Self::Storage(err) => write!(f, "{err}"),
		}
	}
}

impl std::error::Error for PublicationError {}

impl From<sqlx::Error> for PublicationError {
	fn from(err: sqlx::Error) -> Self {
		Self::Storage(err)
	}
}

pub struct PublicationRepository {
	pool: SqlitePool,
}

impl PublicationRepository {
	#[must_use]
	pub const fn new(pool: SqlitePool) -> Self {
		Self { pool }
	}

	/// Record every catalogue `(id, version)` not seen before as a publication,
	/// and return how many were new.
	///
	/// Reads the whole catalogue, which is bounded by
	/// [`ACTIVITY_DETECTION_CEILING`] (`activity_repo::CATALOG_CEILING`'s
	/// value): a catalogue over it is **refused** with
	/// [`PublicationError::OverCeiling`], never read as a prefix — a silent
	/// `LIMIT` would leave every activity past it permanently unannounced. On
	/// a pass where nothing was published this inserts nothing.
	///
	/// # Errors
	/// [`PublicationError::OverCeiling`] for an over-ceiling catalogue, or any
	/// `sqlx` failure.
	pub async fn detect_activity_publications(&self, now: &str) -> Result<u64, PublicationError> {
		let rows = sqlx::query_scalar!(r#"SELECT COUNT(*) AS "count!: i64" FROM activities"#).fetch_one(&self.pool).await?;
		if rows > ACTIVITY_DETECTION_CEILING {
			return Err(PublicationError::OverCeiling {
				rows,
				ceiling: ACTIVITY_DETECTION_CEILING,
			});
		}
		let inserted = sqlx::query!(
			r#"
			INSERT INTO curriculum_publication (source, curriculum_id, version, detected_at)
			SELECT 'activity', id, version, ?1 FROM activities WHERE true
			ON CONFLICT (source, curriculum_id, version) DO NOTHING
			"#,
			now
		)
		.execute(&self.pool)
		.await?;
		Ok(inserted.rows_affected())
	}

	/// The newest publication — whose `id` is the epoch — or `None` for an
	/// empty log. One primary-key read, whatever the size of the log or the
	/// number of subjects.
	///
	/// # Errors
	/// Propagates any `sqlx` failure.
	pub async fn newest(&self) -> Result<Option<Publication>, sqlx::Error> {
		sqlx::query_as!(
			Publication,
			r#"
			SELECT id AS "id!", source, curriculum_id, version, detected_at
			FROM curriculum_publication
			ORDER BY id DESC
			LIMIT 1
			"#
		)
		.fetch_optional(&self.pool)
		.await
	}
}

#[cfg(test)]
mod tests {
	use super::PublicationRepository;
	use sqlx::sqlite::SqlitePoolOptions;
	use sqlx::SqlitePool;

	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");
	const PREVIOUS_MIGRATION: i64 = 20_260_924_001_100;

	async fn pool() -> SqlitePool {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();
		pool
	}

	/// The seeded catalogue is the baseline: nothing already served is a
	/// publication, and a second detection pass finds nothing either.
	#[tokio::test]
	async fn the_catalogue_that_exists_at_migration_is_not_a_publication() {
		let pool = pool().await;
		let repo = PublicationRepository::new(pool.clone());
		let baseline = repo.newest().await.unwrap().map(|publication| publication.id);
		assert_eq!(repo.detect_activity_publications("2026-09-24T00:00:00+00:00").await.unwrap(), 0);
		assert_eq!(repo.newest().await.unwrap().map(|publication| publication.id), baseline, "the epoch did not move");
	}

	/// A new activity, or a version bump, moves the epoch — once; detecting
	/// again moves nothing.
	#[tokio::test]
	async fn a_publication_moves_the_epoch_once() {
		let pool = pool().await;
		let repo = PublicationRepository::new(pool.clone());
		let before = repo.newest().await.unwrap().map_or(0, |publication| publication.id);
		sqlx::query!(
			"INSERT INTO activities (id, name, description, icon, registry_key, layout_tree, maturity, min_duration_ms, published_at, version, fields, default_config, audio) VALUES ('new-thing', 'n', 'd', 'hexagon', 'new-thing', 'study', 'ready', NULL, '2026-09-24T00:00:00Z', 1, '[]', '{}', NULL)"
		)
		.execute(&pool)
		.await
		.unwrap();
		assert_eq!(repo.detect_activity_publications("now").await.unwrap(), 1);
		assert_eq!(repo.detect_activity_publications("now").await.unwrap(), 0, "and never twice");
		let newest = repo.newest().await.unwrap().unwrap();
		assert!(newest.id > before);
		assert_eq!((newest.source.as_str(), newest.curriculum_id.as_str(), newest.version), ("activity", "new-thing", 1));

		sqlx::query!("UPDATE activities SET version = 2 WHERE id = 'new-thing'").execute(&pool).await.unwrap();
		assert_eq!(repo.detect_activity_publications("now").await.unwrap(), 1, "a version bump is news again");
		assert!(repo.newest().await.unwrap().unwrap().id > newest.id);
	}

	/// A catalogue over the detection ceiling is refused, never read as a
	/// prefix that would leave the rest permanently unannounced.
	#[tokio::test]
	async fn an_over_ceiling_catalogue_is_refused_not_truncated() {
		let pool = pool().await;
		let baseline = PublicationRepository::new(pool.clone()).newest().await.unwrap();
		let seeded: i64 = sqlx::query_scalar!(r#"SELECT COUNT(*) AS "count!: i64" FROM activities"#).fetch_one(&pool).await.unwrap();
		for i in seeded..=super::ACTIVITY_DETECTION_CEILING {
			let mut id = String::from("bulk-");
			id.push_str(&i.to_string());
			sqlx::query!(
				"INSERT INTO activities (id, name, description, icon, registry_key, layout_tree, maturity, min_duration_ms, published_at, version, fields, default_config, audio) VALUES (?1, ?1, 'd', 'hexagon', ?1, 'study', 'ready', NULL, '2026-09-24T00:00:00Z', 1, '[]', '{}', NULL)",
				id
			)
			.execute(&pool)
			.await
			.unwrap();
		}
		let result = PublicationRepository::new(pool.clone()).detect_activity_publications("now").await;
		assert!(matches!(result, Err(super::PublicationError::OverCeiling { .. })), "{result:?}");
		assert_eq!(
			PublicationRepository::new(pool.clone()).newest().await.unwrap(),
			baseline,
			"and nothing partial was recorded"
		);
	}

	#[tokio::test]
	async fn the_migration_round_trips() {
		let pool = pool().await;
		MIGRATOR.undo(&pool, PREVIOUS_MIGRATION).await.unwrap();
		let tables: i64 = sqlx::query_scalar!(r#"SELECT COUNT(*) AS "count!: i64" FROM sqlite_master WHERE type = 'table' AND name = 'curriculum_publication'"#)
			.fetch_one(&pool)
			.await
			.unwrap();
		assert_eq!(tables, 0, "down drops the log");
		let watermark: i64 = sqlx::query_scalar!(r#"SELECT COUNT(*) AS "count!: i64" FROM pragma_table_info('engagement_gate') WHERE name = 'curriculum_epoch'"#)
			.fetch_one(&pool)
			.await
			.unwrap();
		assert_eq!(watermark, 0, "and the watermark column");
		MIGRATOR.run(&pool).await.unwrap();
		assert_eq!(
			PublicationRepository::new(pool.clone()).detect_activity_publications("now").await.unwrap(),
			0,
			"and up re-baselines"
		);
	}
}

//! The append-only log of published material, and its epoch (#273, CAT5).
//!
//! Storage for the `CurriculumUpdated` producer. Detecting that an activity
//! `(id, version)` is new appends it here; the log's `MAX(id)` is the
//! **epoch**. Nothing in this crate knows about subjects: who a publication
//! reaches is each subject's own watermark (`engagement_gate.curriculum_epoch`)
//! falling behind the epoch, and the waker catching them up — see
//! `20260924001200_create_curriculum_publication.up.sql`. Lessons (#277) append
//! to the same log; which publications apply to whom is answered per subject at
//! catch-up time by [`PublicationRepository::relevant_since`], under
//! `study_domain::CURRICULUM_AUDIENCE` and `study_domain::LESSON_AUDIENCE`.

use sqlx::{SqliteConnection, SqlitePool};

/// `curriculum_publication.source` for a catalogue activity (#273).
pub const ACTIVITY_SOURCE: &str = "activity";

/// `curriculum_publication.source` for a lesson (#277).
pub const LESSON_SOURCE: &str = "curriculum";

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

/// The most lessons one detection pass reads — `curriculum_repo::MANIFEST_CEILING`,
/// restated so this crate need not depend on that one.
pub const LESSON_DETECTION_CEILING: i64 = 1_000;

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

	/// Record every lesson `(key, version)` not seen before as a publication
	/// (#277, CUR4), and return how many were new.
	///
	/// A lesson's `version` moves only when its file's bytes change (#275), so
	/// a re-import of unchanged content, or a manifest rename, is never a
	/// publication; the first import of an existing corpus is recorded as a
	/// baseline by the importer itself ([`Self::record_baseline`]). Bounded by
	/// [`LESSON_DETECTION_CEILING`] (`curriculum_repo::MANIFEST_CEILING`'s
	/// value), and refused — never read as a prefix — over it, like
	/// [`Self::detect_activity_publications`].
	///
	/// # Errors
	/// [`PublicationError::OverCeiling`] for an over-ceiling corpus, or any
	/// `sqlx` failure.
	pub async fn detect_curriculum_publications(&self, now: &str) -> Result<u64, PublicationError> {
		let rows = sqlx::query_scalar!(r#"SELECT COUNT(*) AS "count!: i64" FROM curriculum"#).fetch_one(&self.pool).await?;
		if rows > LESSON_DETECTION_CEILING {
			return Err(PublicationError::OverCeiling {
				rows,
				ceiling: LESSON_DETECTION_CEILING,
			});
		}
		let inserted = sqlx::query!(
			r#"
			INSERT INTO curriculum_publication (source, curriculum_id, version, detected_at)
			SELECT 'curriculum', key, version, ?1 FROM curriculum WHERE true
			ON CONFLICT (source, curriculum_id, version) DO NOTHING
			"#,
			now
		)
		.execute(&self.pool)
		.await?;
		Ok(inserted.rows_affected())
	}

	/// The newest publication after `watermark`, up to and including `epoch`,
	/// that applies to `subject_id` — or `None` if nothing they missed is news
	/// to them (#277, CUR4).
	///
	/// This is where each audience rule is applied: per subject, when they are
	/// caught up, never as an audience query at publish time.
	///
	/// - A catalogue activity (`study_domain::CURRICULUM_AUDIENCE`) applies to
	///   everyone behind it.
	/// - A lesson (`study_domain::LESSON_AUDIENCE`) applies to a subject who had
	///   played its activity — a completed or abandoned block in
	///   `activity_outcome` that ended by the lesson's `detected_at` — answered
	///   by a point lookup on `idx_activity_outcome_subject_activity`. A lesson
	///   whose key the `curriculum` table no longer holds applies to nobody.
	///
	/// Walks the publications in the gap newest first and stops at the first
	/// that applies: O(publications missed), never O(subjects).
	///
	/// # Errors
	/// Propagates any `sqlx` failure.
	pub async fn relevant_since(&self, subject_id: &str, watermark: i64, epoch: i64) -> Result<Option<Publication>, sqlx::Error> {
		sqlx::query_as!(
			Publication,
			r#"
			SELECT p.id AS "id!", p.source, p.curriculum_id, p.version, p.detected_at
			FROM curriculum_publication p
			WHERE p.id > ?2 AND p.id <= ?3 AND p.baseline = 0
			  AND (
				p.source = 'activity'
				OR (
					p.source = 'curriculum'
					AND EXISTS (
						SELECT 1
						FROM curriculum c
						JOIN activity_outcome o ON o.subject_id = ?1 AND o.activity_id = c.activity_id
						WHERE c.key = p.curriculum_id
						  AND o.outcome != 'skipped'
						  AND COALESCE(julianday(o.ended_at) <= julianday(p.detected_at), 1)
					)
				)
			  )
			ORDER BY p.id DESC
			LIMIT 1
			"#,
			subject_id,
			watermark,
			epoch
		)
		.fetch_optional(&self.pool)
		.await
	}

	/// Record `(source, curriculum_id, version)` as seen **without announcing
	/// it** — a baseline row (#275, CUR2). Used for material the app was
	/// already serving before this server knew about it: the first import of
	/// an existing lesson corpus is what everyone has been studying all along
	/// and must not drain anyone.
	///
	/// A baseline row is in the log, so detection never mistakes it for new,
	/// but it is not an epoch: the epoch is the newest row with `baseline = 0`
	/// ([`Self::newest`], and the stamp on a new gate row), so recording one
	/// puts nobody behind. That keeps a corpus baseline O(1) in subjects —
	/// no watermark is touched. A no-op for a triple already recorded.
	///
	/// On `conn`, so the importer records a baseline in the same transaction
	/// as the lesson it covers.
	///
	/// # Errors
	/// Propagates any `sqlx` failure.
	pub async fn record_baseline(conn: &mut SqliteConnection, source: &str, curriculum_id: &str, version: i64, now: &str) -> Result<(), sqlx::Error> {
		sqlx::query!(
			r#"
			INSERT INTO curriculum_publication (source, curriculum_id, version, detected_at, baseline)
			VALUES (?1, ?2, ?3, ?4, 1)
			ON CONFLICT (source, curriculum_id, version) DO NOTHING
			"#,
			source,
			curriculum_id,
			version,
			now
		)
		.execute(&mut *conn)
		.await?;
		Ok(())
	}

	/// The newest publication — whose `id` is the epoch — or `None` for an
	/// empty log. Baseline rows ([`Self::record_baseline`]) are skipped: they
	/// are seen, not announced. One read from the top of the primary key,
	/// whatever the number of subjects.
	///
	/// # Errors
	/// Propagates any `sqlx` failure.
	pub async fn newest(&self) -> Result<Option<Publication>, sqlx::Error> {
		sqlx::query_as!(
			Publication,
			r#"
			SELECT id AS "id!", source, curriculum_id, version, detected_at
			FROM curriculum_publication
			WHERE baseline = 0
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

	/// The epoch lookups read the partial index on non-baseline rows, not the
	/// baseline rows a corpus's first import appends after them (from a
	/// `chatgpt-codex-connector` finding on #365).
	#[tokio::test]
	async fn the_epoch_lookups_read_the_partial_index() {
		let pool = pool().await;
		for query in [
			"EXPLAIN QUERY PLAN SELECT id, source, curriculum_id, version, detected_at FROM curriculum_publication WHERE baseline = 0 ORDER BY id DESC LIMIT 1",
			"EXPLAIN QUERY PLAN SELECT COALESCE(MAX(id), 0) FROM curriculum_publication WHERE baseline = 0",
		] {
			let plan: Vec<(i64, i64, i64, String)> = sqlx::query_as(query).fetch_all(&pool).await.unwrap();
			assert!(
				plan.iter().any(|(_, _, _, detail)| detail.contains("idx_curriculum_publication_epoch")),
				"{query}: {plan:?}"
			);
		}
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

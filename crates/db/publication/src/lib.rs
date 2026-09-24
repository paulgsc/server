//! Which published material has been announced to whom (#273, CAT5).
//!
//! Storage for the `CurriculumUpdated` producer: detecting that an activity
//! `(id, version)` is new, working out who it applies to, and recording — per
//! subject, before the signal is applied — that it has been applied, so a
//! publish is announced to each subject at most once however many times the
//! fan-out runs. See `20260924001200_create_curriculum_publication.up.sql` for
//! the schema's reasoning and `study_domain::CURRICULUM_AUDIENCE` for the
//! audience rule's.

use sqlx::SqlitePool;

/// One publication whose fan-out is still in progress.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Publication {
	pub id: i64,
	pub source: String,
	pub curriculum_id: String,
	pub version: i64,
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

	/// Publications still being fanned out, oldest first.
	///
	/// # Errors
	/// Propagates any `sqlx` failure.
	pub async fn pending(&self, limit: i64) -> Result<Vec<Publication>, sqlx::Error> {
		sqlx::query_as!(
			Publication,
			r#"
			SELECT id AS "id!", source, curriculum_id, version
			FROM curriculum_publication
			WHERE fanned_out_at IS NULL
			ORDER BY id
			LIMIT ?
			"#,
			limit
		)
		.fetch_all(&self.pool)
		.await
	}

	/// Up to `limit` subjects this publication applies to and has not yet been
	/// applied to.
	///
	/// **The audience rule** (`study_domain::CURRICULUM_AUDIENCE`): subjects who
	/// have started at least one session. A subject who has never studied starts
	/// with a full charge on purpose and has nothing that new material could
	/// make stale; draining them would nudge someone on their first day for
	/// something they have not missed.
	///
	/// # Errors
	/// Propagates any `sqlx` failure.
	pub async fn audience(&self, publication_id: i64, limit: i64) -> Result<Vec<String>, sqlx::Error> {
		sqlx::query_scalar!(
			r#"
			SELECT DISTINCT sessions.subject_id AS "subject_id!"
			FROM sessions
			WHERE sessions.started_at IS NOT NULL
			  AND NOT EXISTS (
			      SELECT 1 FROM curriculum_delivery
			      WHERE curriculum_delivery.publication_id = ?1
			        AND curriculum_delivery.subject_id = sessions.subject_id
			  )
			ORDER BY sessions.subject_id
			LIMIT ?2
			"#,
			publication_id,
			limit
		)
		.fetch_all(&self.pool)
		.await
	}

	/// Claim `(publication, subject)` before applying the signal. `true` means
	/// this call made the claim and the caller should apply the signal; `false`
	/// means it was already applied, and applying it again would drain twice.
	///
	/// # Errors
	/// Propagates any `sqlx` failure.
	pub async fn claim_delivery(&self, publication_id: i64, subject_id: &str, now: &str) -> Result<bool, sqlx::Error> {
		let inserted = sqlx::query!(
			"INSERT INTO curriculum_delivery (publication_id, subject_id, applied_at) VALUES (?, ?, ?) ON CONFLICT DO NOTHING",
			publication_id,
			subject_id,
			now
		)
		.execute(&self.pool)
		.await?;
		Ok(inserted.rows_affected() == 1)
	}

	/// Mark a publication's fan-out complete: its audience query came back
	/// short of what was asked for, so everyone it applies to has it.
	///
	/// # Errors
	/// Propagates any `sqlx` failure.
	pub async fn mark_fanned_out(&self, publication_id: i64, now: &str) -> Result<(), sqlx::Error> {
		sqlx::query!("UPDATE curriculum_publication SET fanned_out_at = ? WHERE id = ?", now, publication_id)
			.execute(&self.pool)
			.await?;
		Ok(())
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
		assert_eq!(repo.detect_activity_publications("2026-09-24T00:00:00+00:00").await.unwrap(), 0);
		assert!(repo.pending(10).await.unwrap().is_empty());
	}

	/// A delivery is claimed exactly once per (publication, subject).
	#[tokio::test]
	async fn a_delivery_is_claimed_once() {
		let pool = pool().await;
		let repo = PublicationRepository::new(pool.clone());
		assert!(repo.claim_delivery(1, "subject-a", "now").await.unwrap());
		assert!(!repo.claim_delivery(1, "subject-a", "now").await.unwrap(), "a second claim is a replay");
		assert!(repo.claim_delivery(2, "subject-a", "now").await.unwrap(), "a different publication is different news");
	}

	/// A catalogue over the detection ceiling is refused, never read as a
	/// prefix that would leave the rest permanently unannounced.
	#[tokio::test]
	async fn an_over_ceiling_catalogue_is_refused_not_truncated() {
		let pool = pool().await;
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
		assert!(
			PublicationRepository::new(pool.clone()).pending(10).await.unwrap().is_empty(),
			"and nothing partial was recorded"
		);
	}

	#[tokio::test]
	async fn the_migration_round_trips() {
		let pool = pool().await;
		MIGRATOR.undo(&pool, PREVIOUS_MIGRATION).await.unwrap();
		let tables: i64 =
			sqlx::query_scalar!(r#"SELECT COUNT(*) AS "count!: i64" FROM sqlite_master WHERE type = 'table' AND name IN ('curriculum_publication', 'curriculum_delivery')"#)
				.fetch_one(&pool)
				.await
				.unwrap();
		assert_eq!(tables, 0, "down drops both tables");
		MIGRATOR.run(&pool).await.unwrap();
		assert_eq!(
			PublicationRepository::new(pool.clone()).detect_activity_publications("now").await.unwrap(),
			0,
			"and up re-baselines"
		);
	}
}

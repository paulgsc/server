use sqlx::{FromRow, SqlitePool};

/// One lease as stored.
///
/// `observed_at` is ISO-8601 UTC, stored as the caller gave it — this crate
/// never generates a timestamp itself, so a test can hand it a fixed one and
/// stay independent of wall-clock time.
#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct PresenceLeaseRow {
	pub context_key: String,
	pub observed_at: String,
}

/// How many distinct contexts one subject may hold a lease on at once.
///
/// A renewal rewrites the same row (see [`PresenceLeaseRepository::observe`]),
/// but a *new* `context_key` is a new row, and this endpoint has no
/// authentication beyond the CORS allowlist — the same trust model as every
/// other route in this deployment, see `docs/study-nudge.md`. Without a cap,
/// a subject's row count (and so `for_subject`'s read cost, paid once per
/// `waker::consider` pass for that subject, forever) grows with how many
/// distinct contexts they have *ever* visited rather than how many they
/// plausibly hold open at once — a handful of tabs, not an unbounded
/// lifetime history. Chosen generously above that realistic number rather
/// than tuned tight.
const MAX_LEASES_PER_SUBJECT: i64 = 16;

pub struct PresenceLeaseRepository {
	pool: SqlitePool,
}

impl PresenceLeaseRepository {
	#[must_use]
	pub const fn new(pool: SqlitePool) -> Self {
		Self { pool }
	}

	/// Record a subject looking at a context, right now (or renew a lease
	/// they already hold on it).
	///
	/// An upsert rather than an insert: a client's sparse renewal heartbeat
	/// rewrites the one row for `(subject_id, context_key)` in place, so a
	/// subject who keeps a tab open all day still owns exactly one row per
	/// context rather than a growing history nobody reads. Followed, in the
	/// same transaction, by trimming this subject down to
	/// [`MAX_LEASES_PER_SUBJECT`] contexts (oldest `observed_at` first) —
	/// see that constant's own doc comment for why a cap is needed at all.
	///
	/// # Errors
	/// Propagates any `sqlx` failure.
	pub async fn observe(&self, subject_id: &str, context_key: &str, observed_at: &str) -> Result<(), sqlx::Error> {
		let mut tx = self.pool.begin().await?;

		sqlx::query!(
			"INSERT INTO presence_leases (subject_id, context_key, observed_at)
			 VALUES (?1, ?2, ?3)
			 ON CONFLICT (subject_id, context_key) DO UPDATE SET observed_at = excluded.observed_at",
			subject_id,
			context_key,
			observed_at
		)
		.execute(&mut *tx)
		.await?;

		sqlx::query!(
			"DELETE FROM presence_leases
			 WHERE subject_id = ?1
			   AND context_key NOT IN (
			       SELECT context_key FROM presence_leases
			       WHERE subject_id = ?1
			       ORDER BY observed_at DESC
			       LIMIT ?2
			   )",
			subject_id,
			MAX_LEASES_PER_SUBJECT
		)
		.execute(&mut *tx)
		.await?;

		tx.commit().await?;

		Ok(())
	}

	/// Every lease this subject currently holds, fresh or not.
	///
	/// Freshness is deliberately not filtered here: it is a function of `now`
	/// and a TTL, both of which live with the caller (`nudge::presence`), not
	/// with storage. Returning the raw rows keeps this repository ignorant of
	/// what "fresh enough" means, which is the same split `engagement_repo`
	/// draws between stored levels and decay.
	///
	/// # Errors
	/// Propagates any `sqlx` failure.
	pub async fn for_subject(&self, subject_id: &str) -> Result<Vec<PresenceLeaseRow>, sqlx::Error> {
		sqlx::query_as!(PresenceLeaseRow, "SELECT context_key, observed_at FROM presence_leases WHERE subject_id = ?1", subject_id)
			.fetch_all(&self.pool)
			.await
	}
}

#[cfg(test)]
mod tests {
	use super::{PresenceLeaseRepository, PresenceLeaseRow};
	use sqlx::sqlite::SqlitePoolOptions;

	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");

	async fn pool() -> sqlx::SqlitePool {
		let pool = SqlitePoolOptions::new()
			.max_connections(1)
			.connect("sqlite::memory:")
			.await
			.expect("open an in-memory sqlite database");
		MIGRATOR.run(&pool).await.expect("run the workspace migration history");
		pool
	}

	#[tokio::test]
	async fn a_subject_with_no_leases_has_none() {
		let repo = PresenceLeaseRepository::new(pool().await);
		assert_eq!(repo.for_subject("subject-1").await.expect("read leases"), Vec::new());
	}

	#[tokio::test]
	async fn observing_a_context_makes_it_findable() {
		let repo = PresenceLeaseRepository::new(pool().await);
		repo.observe("subject-1", "session-a", "2026-08-26T12:00:00Z").await.expect("record a lease");

		let leases = repo.for_subject("subject-1").await.expect("read leases");
		assert_eq!(
			leases,
			vec![PresenceLeaseRow {
				context_key: "session-a".to_owned(),
				observed_at: "2026-08-26T12:00:00Z".to_owned(),
			}]
		);
	}

	/// The renewal heartbeat's whole point: observing the same context again
	/// renews the one row rather than accumulating a second.
	#[tokio::test]
	async fn observing_the_same_context_twice_renews_rather_than_duplicates() {
		let repo = PresenceLeaseRepository::new(pool().await);
		repo.observe("subject-1", "session-a", "2026-08-26T12:00:00Z").await.expect("first observation");
		repo.observe("subject-1", "session-a", "2026-08-26T12:01:00Z").await.expect("renewal");

		let leases = repo.for_subject("subject-1").await.expect("read leases");
		assert_eq!(leases.len(), 1, "a renewal must not grow the table");
		assert_eq!(leases[0].observed_at, "2026-08-26T12:01:00Z");
	}

	/// The schema's other half of the design: a subject can hold more than
	/// one fresh context at once (e.g. two session tabs), and a lease on one
	/// context must not disturb a lease on another.
	#[tokio::test]
	async fn a_subject_can_hold_leases_on_more_than_one_context_at_once() {
		let repo = PresenceLeaseRepository::new(pool().await);
		repo.observe("subject-1", "session-a", "2026-08-26T12:00:00Z").await.expect("lease on session-a");
		repo.observe("subject-1", "session-b", "2026-08-26T12:00:05Z").await.expect("lease on session-b");

		let mut leases = repo.for_subject("subject-1").await.expect("read leases");
		leases.sort_by(|a, b| a.context_key.cmp(&b.context_key));
		assert_eq!(leases.len(), 2);
		assert_eq!(leases[0].context_key, "session-a");
		assert_eq!(leases[1].context_key, "session-b");
	}

	/// Leases are scoped per subject: one subject's context must never leak
	/// into another's read, however similar the context key looks.
	#[tokio::test]
	async fn leases_do_not_cross_subjects() {
		let repo = PresenceLeaseRepository::new(pool().await);
		repo.observe("subject-1", "session-a", "2026-08-26T12:00:00Z").await.expect("subject-1's lease");
		repo.observe("subject-2", "session-a", "2026-08-26T12:00:00Z").await.expect("subject-2's lease");

		let leases = repo.for_subject("subject-1").await.expect("read subject-1's leases");
		assert_eq!(leases.len(), 1, "subject-2's lease on the same context key must not appear here");
	}

	/// The bug an unbounded per-subject history would be: an unauthenticated
	/// caller writes far more distinct contexts than anyone plausibly holds
	/// open, and every one of them is retained and read back forever. This
	/// proves the table stays capped and keeps the freshest contexts rather
	/// than the oldest.
	#[tokio::test]
	async fn a_subject_is_capped_at_max_leases_keeping_the_freshest() {
		use super::MAX_LEASES_PER_SUBJECT;

		let repo = PresenceLeaseRepository::new(pool().await);
		#[allow(clippy::cast_sign_loss)]
		let flood = MAX_LEASES_PER_SUBJECT as usize * 2;
		for i in 0..flood {
			let context_key = format!("session-{i}");
			let observed_at = format!("2026-08-26T12:{i:02}:00Z");
			repo.observe("subject-1", &context_key, &observed_at).await.expect("write one more distinct context");
		}

		let leases = repo.for_subject("subject-1").await.expect("read leases");
		#[allow(clippy::cast_sign_loss)]
		let cap = MAX_LEASES_PER_SUBJECT as usize;
		assert_eq!(leases.len(), cap, "row count must stay capped regardless of how many distinct contexts were ever written");

		// The most recently observed contexts survive; the earliest ones,
		// written before the flood pushed them out, do not.
		let survivors: std::collections::BTreeSet<_> = leases.iter().map(|l| l.context_key.clone()).collect();
		assert!(survivors.contains(&format!("session-{}", flood - 1)), "the most recent context must survive the trim");
		assert!(!survivors.contains("session-0"), "the oldest context must be trimmed once the cap is exceeded");
	}
}

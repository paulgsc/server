use sqlx::SqlitePool;

/// How long `activity_outcome` keeps a row: one year from `ended_at` (#286,
/// applying #265's rule).
///
/// Longer than `intervention_log`'s ninety days because the question is
/// different. That log answers "why did I get that notification?", which is
/// asked soon or never. This table is a person's own study history, and the
/// questions the recommender asks of it — has this activity ever been played,
/// was it abandoned, is it landing — are about the recent past but not only
/// the last quarter: someone returning after a summer away should still read
/// as having played the thing they played in spring. A year covers every
/// realistic gap of that kind; past it, "you played this once, fourteen months
/// ago" is honestly closer to "never played" than to a signal worth ranking
/// on. At one row per block this is still small — a daily learner writes a
/// few thousand rows a year.
///
/// Everything else is #265's rule unchanged: time-based on the row's own event
/// timestamp, indexed (`idx_activity_outcome_ended_at`), deleted in bounded,
/// oldest-first bites from the waker's pass — see `docs/study-nudge.md`,
/// "History has a horizon".
pub const ACTIVITY_OUTCOME_RETENTION_DAYS: i64 = 365;

pub struct OutcomeRepository {
	pool: SqlitePool,
}

impl OutcomeRepository {
	#[must_use]
	pub const fn new(pool: SqlitePool) -> Self {
		Self { pool }
	}

	/// Delete up to `limit` rows that ended before `horizon`, oldest first,
	/// and return how many went — #265's bounded sweep, applied to this table.
	///
	/// # Errors
	/// Propagates any `sqlx` failure.
	pub async fn prune(&self, horizon: &str, limit: i64) -> Result<u64, sqlx::Error> {
		let deleted = sqlx::query!(
			r#"
			DELETE FROM activity_outcome
			WHERE id IN (
			    SELECT id FROM activity_outcome
			    WHERE ended_at < ?1
			    ORDER BY ended_at ASC
			    LIMIT ?2
			)
			"#,
			horizon,
			limit,
		)
		.execute(&self.pool)
		.await?;
		Ok(deleted.rows_affected())
	}
}

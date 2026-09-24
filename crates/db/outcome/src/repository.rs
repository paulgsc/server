use crate::model::OutcomeKind;
use serde::Serialize;
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

/// One block's outcome, as written — and as read back on a replay.
///
/// `outcome` is the parsed [`OutcomeKind`], never the raw text: the schema's
/// CHECK and [`OutcomeKind::parse`] agree on the vocabulary, so a stored value
/// that does not parse is a row this schema did not write, and reading it
/// fails loudly rather than defaulting.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutcomeRecord {
	pub session_id: String,
	pub activity_id: String,
	pub block_index: i64,
	pub started_at: String,
	pub ended_at: String,
	pub planned_ms: i64,
	pub elapsed_ms: i64,
	pub outcome: OutcomeKind,
	/// `None` is *not assessed* — never the same fact as `Some(0.0)`.
	pub score: Option<f64>,
}

/// What [`OutcomeRepository::record`] did.
#[derive(Debug, Clone, PartialEq)]
pub enum Recorded {
	/// A new row: the first report of this block. The only case in which the
	/// caller may derive and fold a signal from it.
	New,
	/// `(session_id, block_index)` already had a row — a replay. Carries what
	/// is stored so the caller can tell an identical retry from a
	/// contradictory one. Nothing was written.
	Existing(OutcomeRecord),
}

pub struct OutcomeRepository {
	pool: SqlitePool,
}

impl OutcomeRepository {
	#[must_use]
	pub const fn new(pool: SqlitePool) -> Self {
		Self { pool }
	}

	/// Write one block's outcome, unless that block already has one (#287).
	///
	/// `INSERT … ON CONFLICT (session_id, block_index) DO NOTHING` is the
	/// whole idempotency mechanism: of any number of concurrent or replayed
	/// writes for one block, exactly one sees a row inserted, in one
	/// statement, with no read-then-write window for two of them to both
	/// believe they were first. That one — [`Recorded::New`] — is the only
	/// caller licensed to fold a signal, which is how a replay writes no second
	/// row *and* folds no second signal.
	///
	/// First report wins. A later, different report for the same block is not
	/// applied; it comes back as [`Recorded::Existing`] for the caller to refuse.
	///
	/// # Errors
	/// Propagates any `sqlx` failure, including a stored `outcome` that
	/// [`OutcomeKind::parse`] refuses (`sqlx::Error::Decode`).
	pub async fn record(&self, subject_id: &str, outcome: &OutcomeRecord) -> Result<Recorded, sqlx::Error> {
		let kind = outcome.outcome.as_str();
		let inserted = sqlx::query!(
			r#"
			INSERT INTO activity_outcome
			    (subject_id, session_id, activity_id, block_index, started_at, ended_at, planned_ms, elapsed_ms, outcome, score)
			VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
			ON CONFLICT (session_id, block_index) DO NOTHING
			"#,
			subject_id,
			outcome.session_id,
			outcome.activity_id,
			outcome.block_index,
			outcome.started_at,
			outcome.ended_at,
			outcome.planned_ms,
			outcome.elapsed_ms,
			kind,
			outcome.score,
		)
		.execute(&self.pool)
		.await?
		.rows_affected();

		if inserted == 1 {
			return Ok(Recorded::New);
		}

		let row = sqlx::query!(
			r#"
			SELECT session_id, activity_id, block_index, started_at, ended_at, planned_ms, elapsed_ms, outcome, score
			FROM activity_outcome
			WHERE session_id = ?1 AND block_index = ?2
			"#,
			outcome.session_id,
			outcome.block_index,
		)
		.fetch_one(&self.pool)
		.await?;

		let parsed = OutcomeKind::parse(&row.outcome).ok_or_else(|| sqlx::Error::Decode("activity_outcome.outcome is not completed, abandoned, or skipped".into()))?;
		Ok(Recorded::Existing(OutcomeRecord {
			session_id: row.session_id,
			activity_id: row.activity_id,
			block_index: row.block_index,
			started_at: row.started_at,
			ended_at: row.ended_at,
			planned_ms: row.planned_ms,
			elapsed_ms: row.elapsed_ms,
			outcome: parsed,
			score: row.score,
		}))
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

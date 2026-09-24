use serde::Serialize;
use sqlx::{FromRow, SqlitePool};

/// One class's stored level.
///
/// `class` is the durable discriminant, not a dense index — parsing it back
/// into a domain type is the caller's job, so the quarantine arm for an
/// unrecognised value stays where the types are.
#[derive(Debug, Clone, FromRow, Serialize)]
pub struct ChargeRow {
	pub class: i64,
	pub level: f64,
	pub as_of: String,
}

#[derive(Debug, Clone, FromRow, Serialize)]
pub struct GateRow {
	pub subject_id: String,
	pub eligible_at: String,
	pub last_intervened_at: Option<String>,
	pub last_action: Option<String>,
	pub intervention_count: i64,
}

/// How long `intervention_log` keeps a row: ninety days from `decided_at`
/// (#265, SLI4).
///
/// The log exists to answer "why did I get that notification?", and that
/// question arrives late — someone notices a pattern, or a confusing nudge,
/// weeks after the fact. The schema comment names "weeks later" as the
/// horizon; ninety days covers every realistic version of it with room to
/// spare, and at one row per intervention (at most one per subject per
/// `REFRACTORY`) it costs almost nothing to keep. A shorter horizon would
/// save little; "forever" is not a policy, it is the absence of one.
///
/// **Time-based, not count-based.** A per-subject "keep the last N" bounds
/// storage regardless of activity rate, but answers the wrong question: the
/// question is *when* something happened, and "the last N" of a quiet subject
/// can reach back a year while a busy one's forgets last week.
///
/// **Rows with `actuated_at IS NULL` are not exempt.** `NULL` means claimed but
/// never confirmed — a crash between claim and send, or (#264) a delivery that
/// timed out. That window matters for minutes, not months: nothing reads those
/// rows to recover or retry anything (the claim is deliberately final), so at
/// ninety days one says no more than a sent row does. Exempting them would make
/// them the only rows in the table with no horizon at all.
///
/// This is the rule a history table in this schema inherits: a horizon on the
/// row's own event timestamp, an index on that timestamp, and a sweep bounded
/// by [`RETENTION_SWEEP_LIMIT`] run from the waker's pass — see
/// `docs/study-nudge.md`, "History has a horizon".
pub const INTERVENTION_LOG_RETENTION_DAYS: i64 = 90;

/// The most rows one retention sweep deletes (#265, SLI4).
///
/// A first run against a table that has been growing since launch must not
/// become the long pole in a waker pass, so the sweep deletes at most this
/// many and leaves the rest for the next pass — the same bound-not-page shape
/// as the waker's own `BATCH`. At one pass per five minutes that is still
/// well over a hundred thousand rows a day, far ahead of any rate this table
/// is written at.
pub const RETENTION_SWEEP_LIMIT: i64 = 500;

pub struct EngagementRepository {
	pool: SqlitePool,
}

impl EngagementRepository {
	#[must_use]
	pub const fn new(pool: SqlitePool) -> Self {
		Self { pool }
	}

	/// Every stored level for a subject. Empty for someone never seen, which
	/// the caller reads as "start full" rather than "start empty" — an empty
	/// charge is instantly eligible, and a new account should not be.
	///
	/// # Errors
	/// Propagates any `sqlx` failure.
	pub async fn charge(&self, subject_id: &str) -> Result<Vec<ChargeRow>, sqlx::Error> {
		sqlx::query_as!(
			ChargeRow,
			r#"SELECT class as "class!: i64", level as "level!: f64", as_of FROM engagement_charge WHERE subject_id = ?"#,
			subject_id
		)
		.fetch_all(&self.pool)
		.await
	}

	/// Write levels and the solved eligibility in one transaction.
	///
	/// The two must move together: a charge without its recomputed
	/// `eligible_at` is a subject the waker will look at on the wrong schedule,
	/// and there is no background process that would notice and repair it.
	///
	/// # Errors
	/// Propagates any `sqlx` failure.
	pub async fn save(&self, subject_id: &str, levels: &[(u16, f64)], as_of: &str, eligible_at: &str) -> Result<(), sqlx::Error> {
		let mut tx = self.pool.begin().await?;

		for (class, level) in levels {
			let class = i64::from(*class);
			sqlx::query!(
				r#"
				INSERT INTO engagement_charge (subject_id, class, level, as_of)
				VALUES (?, ?, ?, ?)
				ON CONFLICT(subject_id, class) DO UPDATE SET level = excluded.level, as_of = excluded.as_of
				"#,
				subject_id,
				class,
				level,
				as_of,
			)
			.execute(&mut *tx)
			.await?;
		}

		sqlx::query!(
			r#"
			INSERT INTO engagement_gate (subject_id, eligible_at, intervention_count)
			VALUES (?, ?, 0)
			ON CONFLICT(subject_id) DO UPDATE SET eligible_at = excluded.eligible_at
			"#,
			subject_id,
			eligible_at,
		)
		.execute(&mut *tx)
		.await?;

		tx.commit().await
	}

	/// Give a subject nobody has yet observed a gate row, seeded full.
	///
	/// The only caller is first contact (`waker::first_contact`, from
	/// `POST /push/subscriptions`) — see that function for why subscribing is
	/// the honest "someone exists" event. This is deliberately **not** built on
	/// [`Self::save`]: `save`'s `ON CONFLICT` upserts both tables
	/// unconditionally, which is exactly right for folding in a new signal and
	/// exactly wrong here — subscribing a second device, or re-subscribing the
	/// same one, must not reset an already-drifting subject back to full.
	///
	/// `INSERT OR IGNORE` against `engagement_gate`'s primary key is the guard,
	/// not a read-then-write: two devices racing to be the first ever
	/// subscription for a subject cannot both win, so the charge rows are only
	/// ever written by whichever transaction's gate insert actually landed.
	/// Returns whether this call was the one that created the row, so the
	/// caller can log without a second read.
	///
	/// # Errors
	/// Propagates any `sqlx` failure.
	pub async fn seed_if_absent(&self, subject_id: &str, levels: &[(u16, f64)], as_of: &str, eligible_at: &str) -> Result<bool, sqlx::Error> {
		let mut tx = self.pool.begin().await?;

		let inserted = sqlx::query!(
			r#"INSERT OR IGNORE INTO engagement_gate (subject_id, eligible_at, intervention_count) VALUES (?, ?, 0)"#,
			subject_id,
			eligible_at,
		)
		.execute(&mut *tx)
		.await?;

		if inserted.rows_affected() == 0 {
			tx.rollback().await?;
			return Ok(false);
		}

		for (class, level) in levels {
			let class = i64::from(*class);
			sqlx::query!(
				r#"INSERT OR IGNORE INTO engagement_charge (subject_id, class, level, as_of) VALUES (?, ?, ?, ?)"#,
				subject_id,
				class,
				level,
				as_of,
			)
			.execute(&mut *tx)
			.await?;
		}

		tx.commit().await?;
		Ok(true)
	}

	/// # Errors
	/// Propagates any `sqlx` failure.
	pub async fn gate(&self, subject_id: &str) -> Result<Option<GateRow>, sqlx::Error> {
		sqlx::query_as!(
			GateRow,
			r#"
			SELECT subject_id as "subject_id!", eligible_at, last_intervened_at, last_action,
			       intervention_count as "intervention_count!: i64"
			FROM engagement_gate WHERE subject_id = ?
			"#,
			subject_id
		)
		.fetch_optional(&self.pool)
		.await
	}

	/// **The waker's entire query.** Subjects the arithmetic already marked
	/// eligible, oldest first.
	///
	/// Note what is absent: no scan, no per-subject decay, no decision. The
	/// crossing instant was solved when the last signal arrived, so this is an
	/// index range read that returns nothing on a quiet day.
	///
	/// # Errors
	/// Propagates any `sqlx` failure.
	pub async fn due(&self, now: &str, limit: i64) -> Result<Vec<GateRow>, sqlx::Error> {
		sqlx::query_as!(
			GateRow,
			r#"
			SELECT subject_id as "subject_id!", eligible_at, last_intervened_at, last_action,
			       intervention_count as "intervention_count!: i64"
			FROM engagement_gate
			WHERE eligible_at <= ?
			ORDER BY eligible_at ASC
			LIMIT ?
			"#,
			now,
			limit
		)
		.fetch_all(&self.pool)
		.await
	}

	/// Claim a subject for one intervention, atomically.
	///
	/// Returns the log id if this call won the claim, `None` if it did not. The
	/// check and the claim are one statement so that two waker passes — or one
	/// pass and a just-restarted process — cannot both conclude the subject is
	/// due. `eligible_at` moves forward as part of the same write, which is
	/// what closes the window rather than a lock.
	///
	/// Claiming happens *before* delivery. A crash in between therefore costs
	/// the intervention rather than duplicating it, which is the right way
	/// round for a feature whose whole value is not being annoying.
	///
	/// # Errors
	/// Propagates any `sqlx` failure.
	pub async fn claim(&self, subject_id: &str, now: &str, next_eligible_at: &str, action_kind: &str, action: &str) -> Result<Option<i64>, sqlx::Error> {
		let mut tx = self.pool.begin().await?;

		let claimed = sqlx::query!(
			r#"
			UPDATE engagement_gate
			SET eligible_at        = ?2,
			    last_intervened_at = ?3,
			    last_action        = ?4,
			    intervention_count = intervention_count + 1
			WHERE subject_id = ?1 AND eligible_at <= ?3
			RETURNING intervention_count as "intervention_count!: i64"
			"#,
			subject_id,
			next_eligible_at,
			now,
			action_kind,
		)
		.fetch_optional(&mut *tx)
		.await?;

		if claimed.is_none() {
			tx.rollback().await?;
			return Ok(None);
		}

		let id = sqlx::query!(
			r#"
			INSERT INTO intervention_log (subject_id, action_kind, action, decided_at)
			VALUES (?, ?, ?, ?)
			RETURNING id as "id!: i64"
			"#,
			subject_id,
			action_kind,
			action,
			now,
		)
		.fetch_one(&mut *tx)
		.await?;

		tx.commit().await?;
		Ok(Some(id.id))
	}

	/// Mark a claimed intervention as having reached a push service.
	///
	/// Deliberately not "delivered": what `push_kit` reports is acceptance, and
	/// this column inherits that meaning exactly.
	///
	/// # Errors
	/// Propagates any `sqlx` failure.
	pub async fn mark_actuated(&self, log_id: i64, at: &str) -> Result<(), sqlx::Error> {
		sqlx::query!("UPDATE intervention_log SET actuated_at = ? WHERE id = ?", at, log_id)
			.execute(&self.pool)
			.await?;
		Ok(())
	}

	/// Delete up to `limit` `intervention_log` rows decided before `horizon`,
	/// oldest first, and return how many went (#265, SLI4).
	///
	/// Bounded on purpose — see [`RETENTION_SWEEP_LIMIT`]. Reads
	/// `idx_intervention_log_decided_at`, so a sweep with nothing to do is one
	/// index probe and a sweep with work stops after its own `LIMIT`.
	///
	/// # Errors
	/// Propagates any `sqlx` failure.
	pub async fn prune_intervention_log(&self, horizon: &str, limit: i64) -> Result<u64, sqlx::Error> {
		let deleted = sqlx::query!(
			r#"
			DELETE FROM intervention_log
			WHERE id IN (
			    SELECT id FROM intervention_log
			    WHERE decided_at < ?1
			    ORDER BY decided_at ASC
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

	/// Hand a claim back when nothing reached anyone.
	///
	/// Without this, a subject whose every subscription failed would have spent
	/// their refractory period on a notification nobody saw.
	///
	/// # Errors
	/// Propagates any `sqlx` failure.
	pub async fn release(&self, subject_id: &str, log_id: i64, eligible_at: &str) -> Result<(), sqlx::Error> {
		let mut tx = self.pool.begin().await?;

		sqlx::query!(
			r#"
			UPDATE engagement_gate
			SET eligible_at = ?2, last_intervened_at = NULL, intervention_count = MAX(intervention_count - 1, 0)
			WHERE subject_id = ?1
			"#,
			subject_id,
			eligible_at,
		)
		.execute(&mut *tx)
		.await?;

		sqlx::query!("DELETE FROM intervention_log WHERE id = ?", log_id).execute(&mut *tx).await?;

		tx.commit().await
	}
}

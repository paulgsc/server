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

//! What a pass does before it considers anyone: catch a subject up on corpus it has not
//! seen (`catch_up`), and sweep history past its horizon (`sweep_history`). Split out of
//! `waker` (S9, #489).

use super::observe::fold_signal;
use chrono::{DateTime, Utc};
use engagement_repo::{CatchUpWrite, EngagementRepository, INTERVENTION_LOG_RETENTION_DAYS, RETENTION_SWEEP_LIMIT};
use outcome_repo::{OutcomeRepository, ACTIVITY_OUTCOME_RETENTION_DAYS};
use publication_repo::{Publication, PublicationRepository};
use sqlx::SqlitePool;
use study_domain::StudySignal;
use tracing::{debug, error, info};

/// What [`catch_up`] did for a subject behind the curriculum epoch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CatchUp {
	/// Something they missed applies to them: drained once, watermark moved to
	/// the epoch; the re-solved `eligible_at`.
	Drained(DateTime<Utc>),
	/// Nothing they missed applies to them (#277: only lessons for activities
	/// they have not played): watermark moved, no drain.
	NothingApplies,
	/// Caught up in the meantime; nothing written.
	AlreadyCaughtUp,
}

/// Catch a subject behind the curriculum epoch up to it (#273, CAT5; #277,
/// CUR4).
///
/// Finds the newest publication since their `watermark` that applies to them
/// (`PublicationRepository::relevant_since` — where `CURRICULUM_AUDIENCE` and
/// `LESSON_AUDIENCE` are applied). If there is one, folds **one**
/// `CurriculumUpdated` naming it and advances the watermark to the epoch in
/// the same transaction (`EngagementRepository::catch_up`); if not, advances
/// the watermark alone.
///
/// One drain however many publications the subject missed: the signal means
/// "there is new material", and a drain per publication would let a burst of
/// catalogue edits empty someone's freshness in a single pass.
///
/// # Errors
/// Propagates any storage failure; nothing is written if any step fails.
pub(super) async fn catch_up(db: &SqlitePool, subject_id: &str, watermark: i64, newest: &Publication) -> Result<CatchUp, sqlx::Error> {
	let engagement = EngagementRepository::new(db.clone());
	let Some(relevant) = PublicationRepository::new(db.clone()).relevant_since(subject_id, watermark, newest.id).await? else {
		let moved = engagement.advance_watermark(subject_id, newest.id).await?;
		debug!(subject = %subject_id, epoch = newest.id, "nothing new applies; watermark advanced");
		return Ok(if moved { CatchUp::NothingApplies } else { CatchUp::AlreadyCaughtUp });
	};
	let now = Utc::now();
	let signal = StudySignal::CurriculumUpdated {
		curriculum_id: relevant.curriculum_id.clone(),
	};
	Ok(
		match engagement.catch_up(subject_id, relevant.id, newest.id, |stored| fold_signal(stored, &signal, now)).await? {
			CatchUpWrite::Drained(eligible_at) => {
				debug!(subject = %subject_id, epoch = newest.id, curriculum_id = %relevant.curriculum_id, eligible_at = %eligible_at, "caught up to the curriculum epoch");
				CatchUp::Drained(eligible_at)
			}
			// Another pass consumed `relevant` since this one read the
			// watermark: advanced without draining it again.
			CatchUpWrite::Advanced => CatchUp::NothingApplies,
			CatchUpWrite::NotBehind => CatchUp::AlreadyCaughtUp,
		},
	)
}

/// Enforce each history table's retention horizon, one bounded bite per
/// table per pass (#265, SLI4).
///
/// Run from the waker rather than a second scheduled task: the waker is
/// already a bounded periodic loop with a cancellation token, so the sweep
/// inherits both — and #264's pass deadline — for free. Each delete is capped
/// at `RETENTION_SWEEP_LIMIT`, so a first run against a table that has been
/// growing since launch drains over several passes instead of becoming the
/// long pole in one; with nothing past the horizon it is one index probe.
///
/// Covers `intervention_log` (`INTERVENTION_LOG_RETENTION_DAYS`) and, since
/// #286, `activity_outcome` (`ACTIVITY_OUTCOME_RETENTION_DAYS`) — the next
/// history table joins this list rather than growing its own loop.
///
/// Returns how many rows went, across both. A failure is logged and swallowed
/// rather than failing the pass: retention is housekeeping, and the subjects
/// this pass already handled were handled — `error!` still reaches the
/// workspace-wide tracing-error panel.
pub(super) async fn sweep_history(engagement: &EngagementRepository, outcomes: &OutcomeRepository, now: DateTime<Utc>) -> u64 {
	let log_horizon = now - chrono::Duration::days(INTERVENTION_LOG_RETENTION_DAYS);
	let outcome_horizon = now - chrono::Duration::days(ACTIVITY_OUTCOME_RETENTION_DAYS);
	let sweeps = [
		(
			"intervention_log",
			log_horizon,
			engagement.prune_intervention_log(&log_horizon.to_rfc3339(), RETENTION_SWEEP_LIMIT).await,
		),
		(
			"activity_outcome",
			outcome_horizon,
			outcomes.prune(&outcome_horizon.to_rfc3339(), RETENTION_SWEEP_LIMIT).await,
		),
	];

	let mut total = 0;
	for (table, horizon, result) in sweeps {
		match result {
			Ok(0) => {}
			Ok(pruned) => {
				info!(table, pruned, horizon = %horizon, "history rows past the retention horizon deleted");
				total += pruned;
			}
			Err(err) => error!(table, error = %err, "retention sweep failed; the next pass will try again"),
		}
	}
	total
}

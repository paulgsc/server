//! `/api/v1/outcomes` — where a graded block stops being a number rendered to
//! a screen and discarded (#287, TEL2).
//!
//! ```text
//! POST /outcomes   { sessionId, blockIndex, activityId, startedAt, endedAt,
//!                    plannedMs, elapsedMs, outcome, score? }
//! ```
//!
//! Not another `/signals` call. `/signals` is the engine's front door and takes
//! a signal directly; an outcome is a richer fact a signal is *derived* from,
//! and the derivation lives on this side of the wire
//! (`study_domain::signal_for_block`, beside the calibration numbers) so no
//! applet has to know the domain's thresholds. The route does two things:
//!
//! 1. writes the `activity_outcome` row — durable, and what #289 reads;
//! 2. derives a signal from it, if the policy says there is one, and folds it
//!    in through `waker::observe`.
//!
//! **Idempotent in both halves.** `(session_id, block_index)` is the key
//! (`OutcomeRepository::record`): of any number of replays, exactly one inserts
//! the row, and only that one folds a signal. A replay therefore writes no
//! second row *and* drains no second time — the difference between a harmless
//! duplicate and a doubled charge that changes when someone is interrupted. The
//! two halves are ordered the same way the waker orders claim-before-send: the
//! row first, then the signal. A failure between them costs that one signal —
//! the retry is a replay and folds nothing — rather than ever doubling it.
//!
//! **Refused, never clamped.** A block index outside the session's activity
//! list, an activity that is not the one at that index, an `elapsedMs` beyond
//! the session's total, a score outside `[0, 1]`, a score on a block that did
//! not complete, an outcome outside the three-value vocabulary — each is a 422
//! naming the field. A replay that contradicts what is already recorded is a
//! 409: nothing is wrong with the body on its own, it just is not what
//! happened the first time.
//!
//! **Subject-scoped.** The session is looked up through `SubjectId`, so an
//! outcome for another subject's session is a 404 — the same answer as a
//! session that does not exist, which is the answer that leaks nothing.

use crate::nudge::waker;
use crate::subject::SubjectId;
use crate::{AppState, FileHostError};
use axum::{extract::State, Json};
use chrono::{DateTime, Utc};
use outcome_repo::{OutcomeKind, OutcomeRecord, OutcomeRepository, Recorded};
use serde::{Deserialize, Serialize};
use session_repo::SessionRepository;
use sqlx::SqlitePool;
use std::borrow::Cow;
use tracing::instrument;

/// What an applet posts when a block ends.
///
/// `outcome` arrives as a string rather than as [`OutcomeKind`] so that an
/// unknown value is a 422 naming the field, not a JSON-extractor rejection
/// with a body nobody can act on.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OutcomeRequest {
	pub session_id: String,
	pub block_index: i64,
	pub activity_id: String,
	pub started_at: String,
	pub ended_at: String,
	pub planned_ms: i64,
	pub elapsed_ms: i64,
	pub outcome: String,
	#[serde(default)]
	pub score: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutcomeAccepted {
	/// `true` when this block already had exactly this outcome recorded — a
	/// retry. Nothing was written and no signal was folded.
	pub replayed: bool,
	/// The `kind` of the signal derived and folded in, if the policy produced
	/// one (`scored-below-target` today). Always `None` on a replay.
	pub signal: Option<&'static str>,
	/// The subject's re-solved eligibility, when a signal moved it — the same
	/// field `/signals` returns, for the same reason: it makes the arithmetic
	/// observable without waiting for a notification.
	pub eligible_at: Option<String>,
}

/// `POST /outcomes`
///
/// # Errors
/// 422 for a body that fails validation, 404 for a session this subject does
/// not own, 409 for a replay that contradicts the recorded outcome, and 500 for
/// a storage failure.
#[axum::debug_handler]
#[instrument(name = "record_outcome", skip_all, fields(otel.kind = "server"))]
pub async fn record(State(state): State<AppState>, subject: SubjectId, Json(request): Json<OutcomeRequest>) -> Result<Json<OutcomeAccepted>, FileHostError> {
	record_outcome(&state.core.shared_db, subject.as_str(), request).await.map(Json)
}

/// The handler's body, over a pool rather than `AppState`, so it is testable
/// without the NATS connection `AppState::build` needs.
pub(crate) async fn record_outcome(db: &SqlitePool, subject_id: &str, request: OutcomeRequest) -> Result<OutcomeAccepted, FileHostError> {
	let record = validate_shape(request)?;
	let outcomes = OutcomeRepository::new(db.clone());

	// A replay is recognised against what was *recorded*, before anything
	// that depends on the session's current shape: `PATCH /sessions/:id` can
	// replace a session's activities or shorten it after the fact, and an
	// identical retry of a report that was valid when it arrived must still
	// answer `replayed: true`, not a 422 (a real `chatgpt-codex-connector`
	// finding on #360). Subject-scoped, so another subject's row is never
	// read here.
	if let Some(stored) = outcomes.get(subject_id, &record.session_id, record.block_index).await? {
		return replay(&stored, &record);
	}

	let session = SessionRepository::new(db.clone())
		.get(subject_id, &record.session_id)
		.await
		.map_err(|err| FileHostError::OperationError(err.to_string()))?
		.ok_or(FileHostError::NotFound)?;
	validate_against_session(&record, &session)?;

	match outcomes.record(subject_id, &record).await? {
		// A concurrent report of the same block won the insert between the
		// `get` above and this write.
		Recorded::Existing(stored) => replay(&stored, &record),
		Recorded::New => {
			let Some(signal) = study_domain::signal_for_block(&record.activity_id, record.outcome == OutcomeKind::Completed, record.score) else {
				return Ok(OutcomeAccepted {
					replayed: false,
					signal: None,
					eligible_at: None,
				});
			};
			let eligible_at = waker::observe(db, subject_id, &signal).await?;
			Ok(OutcomeAccepted {
				replayed: false,
				signal: Some(signal.kind()),
				eligible_at: Some(eligible_at.to_rfc3339()),
			})
		}
	}
}

/// The answer to a report of a block that already has an outcome: an
/// identical one is a harmless retry, a different one is refused.
fn replay(stored: &OutcomeRecord, record: &OutcomeRecord) -> Result<OutcomeAccepted, FileHostError> {
	if stored == record {
		Ok(OutcomeAccepted {
			replayed: true,
			signal: None,
			eligible_at: None,
		})
	} else {
		Err(FileHostError::Conflict("this block already has a different outcome recorded"))
	}
}

type FieldErrors = Vec<(&'static str, Cow<'static, str>)>;

/// Everything checkable from the body alone. Timestamps are normalised to UTC
/// RFC 3339 on the way in: `ended_at` is the retention sweep's key and is
/// compared as text, which is only an ordering if every row is written in one
/// format.
fn validate_shape(request: OutcomeRequest) -> Result<OutcomeRecord, FileHostError> {
	let mut errors: FieldErrors = Vec::new();

	let outcome = OutcomeKind::parse(&request.outcome);
	if outcome.is_none() {
		errors.push(("outcome", "must be one of completed, abandoned, skipped".into()));
	}
	if let Some(score) = request.score {
		if !(0.0..=1.0).contains(&score) {
			errors.push(("score", "must be between 0 and 1".into()));
		}
		if outcome.is_some_and(|kind| kind != OutcomeKind::Completed) {
			errors.push((
				"score",
				"only a completed block can carry a score; leave it out for a block that was abandoned or skipped".into(),
			));
		}
	}
	if request.block_index < 0 {
		errors.push(("blockIndex", "must not be negative".into()));
	}
	if request.planned_ms < 0 {
		errors.push(("plannedMs", "must not be negative".into()));
	}
	if request.elapsed_ms < 0 {
		errors.push(("elapsedMs", "must not be negative".into()));
	}
	let started_at = parse_instant(&request.started_at, "startedAt", &mut errors);
	let ended_at = parse_instant(&request.ended_at, "endedAt", &mut errors);
	if let (Some(started), Some(ended)) = (started_at, ended_at) {
		if ended < started {
			errors.push(("endedAt", "must not be before startedAt".into()));
		}
	}

	match (outcome, started_at, ended_at) {
		(Some(outcome), Some(started_at), Some(ended_at)) if errors.is_empty() => Ok(OutcomeRecord {
			session_id: request.session_id,
			activity_id: request.activity_id,
			block_index: request.block_index,
			started_at: started_at.to_rfc3339(),
			ended_at: ended_at.to_rfc3339(),
			planned_ms: request.planned_ms,
			elapsed_ms: request.elapsed_ms,
			outcome,
			score: request.score,
		}),
		_ => Err(FileHostError::unprocessable_entity(errors)),
	}
}

fn parse_instant(raw: &str, field: &'static str, errors: &mut FieldErrors) -> Option<DateTime<Utc>> {
	let parsed = DateTime::parse_from_rfc3339(raw).ok().map(|at| at.with_timezone(&Utc));
	if parsed.is_none() {
		errors.push((field, "must be an RFC 3339 timestamp".into()));
	}
	parsed
}

/// Everything that needs the session: the block exists, it is the activity the
/// body says it is, and the block did not outlast the whole session.
fn validate_against_session(record: &OutcomeRecord, session: &session_repo::SessionRecord) -> Result<(), FileHostError> {
	let mut errors: FieldErrors = Vec::new();

	let block = usize::try_from(record.block_index).ok().and_then(|index| session.activities.get(index));
	match block {
		None => errors.push(("blockIndex", "is outside this session's activity list".into())),
		Some(block) => {
			let scheduled = block.get("activityId").and_then(serde_json::Value::as_str);
			if scheduled != Some(record.activity_id.as_str()) {
				errors.push(("activityId", "is not the activity at this block index".into()));
			}
		}
	}
	if record.elapsed_ms > session.total_duration_ms {
		errors.push(("elapsedMs", "exceeds the session's total duration".into()));
	}

	if errors.is_empty() {
		Ok(())
	} else {
		Err(FileHostError::unprocessable_entity(errors))
	}
}

#[cfg(test)]
mod tests {
	use super::{record_outcome, OutcomeAccepted, OutcomeRequest};
	use crate::FileHostError;
	use engagement_repo::EngagementRepository;
	use intervention::{Charge, Selector};
	use session_repo::{LayoutMode, SessionOrigin, SessionRecord, SessionRepository, SessionStatus};
	use sqlx::sqlite::SqlitePoolOptions;
	use sqlx::SqlitePool;
	use study_domain::{EngagementClass, StudyAction, StudyCalibration, StudySelector, StudyV1};

	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");
	const SUBJECT: &str = "subject-local";
	const SESSION: &str = "session-graded";

	/// A migrated in-memory database holding one session for [`SUBJECT`]: a
	/// `honeycomb` block then a `leetype` block, ten minutes in total.
	async fn pool() -> SqlitePool {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();
		let now = "2026-09-24T10:00:00+00:00".to_owned();
		let session = SessionRecord {
			id: SESSION.to_owned(),
			name: "Honeycomb + LeetType".to_owned(),
			status: SessionStatus::Completed,
			origin: SessionOrigin::User,
			activities: vec![serde_json::json!({ "activityId": "honeycomb" }), serde_json::json!({ "activityId": "leetype" })],
			scenes: Vec::new(),
			layout_mode: LayoutMode::Basic,
			layout: None,
			total_duration_ms: 600_000,
			created_at: now.clone(),
			updated_at: now,
			started_at: None,
			completed_at: None,
			final_elapsed_ms: None,
		};
		SessionRepository::new(pool.clone()).upsert(SUBJECT, &session).await.unwrap();
		pool
	}

	fn request(score: Option<f64>) -> OutcomeRequest {
		OutcomeRequest {
			session_id: SESSION.to_owned(),
			block_index: 0,
			activity_id: "honeycomb".to_owned(),
			started_at: "2026-09-24T10:00:00Z".to_owned(),
			ended_at: "2026-09-24T10:05:00Z".to_owned(),
			planned_ms: 300_000,
			elapsed_ms: 300_000,
			outcome: "completed".to_owned(),
			score,
		}
	}

	async fn stored_levels(pool: &SqlitePool) -> Vec<(i64, f64)> {
		let mut levels: Vec<(i64, f64)> = EngagementRepository::new(pool.clone())
			.charge(SUBJECT)
			.await
			.unwrap()
			.into_iter()
			.map(|row| (row.class, row.level))
			.collect();
		levels.sort_by_key(|(class, _)| *class);
		levels
	}

	async fn rows(pool: &SqlitePool) -> i64 {
		sqlx::query_scalar!(r#"SELECT COUNT(*) AS "count!: i64" FROM activity_outcome"#)
			.fetch_one(pool)
			.await
			.unwrap()
	}

	fn field_errors(err: &FileHostError) -> Vec<String> {
		let FileHostError::UnprocessableEntity { errors } = err else {
			panic!("expected a 422, got {err:?}");
		};
		let mut fields: Vec<String> = errors.keys().map(ToString::to_string).collect();
		fields.sort();
		fields
	}

	/// End to end: a poor score is written, derives `ScoredBelowTarget`,
	/// drains `Mastery`, and a subject with a session prepared is then offered
	/// review — the case that should produce *review*, not *more*.
	#[tokio::test]
	async fn a_poor_score_drains_mastery_and_selects_review() {
		let pool = pool().await;
		let accepted = record_outcome(&pool, SUBJECT, request(Some(0.1))).await.unwrap();
		assert!(!accepted.replayed);
		assert_eq!(accepted.signal, Some("scored-below-target"));
		assert!(accepted.eligible_at.is_some());
		assert_eq!(rows(&pool).await, 1);

		let stored = EngagementRepository::new(pool.clone()).charge(SUBJECT).await.unwrap();
		#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
		let levels: Vec<(u16, f64)> = stored.iter().map(|row| (row.class as u16, row.level)).collect();
		let as_of = chrono::DateTime::parse_from_rfc3339(&stored[0].as_of).unwrap().with_timezone(&chrono::Utc);
		let charge = Charge::<StudyV1>::from_storage::<StudyCalibration>(&levels, as_of);
		let deficits = charge.deficits::<StudyCalibration>(as_of);
		assert_eq!(
			deficits.first().map(|deficit| deficit.class),
			Some(EngagementClass::Mastery),
			"mastery is what a poor score drains"
		);

		let selector = StudySelector {
			prepared_session: Some("session-next".to_owned()),
		};
		assert_eq!(
			selector.select(&deficits),
			Some(StudyAction::SuggestReview {
				session_id: "session-next".to_owned()
			})
		);
	}

	/// A replay writes no second row and folds no second signal: the charge
	/// after two identical posts equals the charge after one.
	#[tokio::test]
	async fn a_replay_writes_nothing_and_drains_nothing() {
		let pool = pool().await;
		record_outcome(&pool, SUBJECT, request(Some(0.1))).await.unwrap();
		let after_one = stored_levels(&pool).await;

		let replay = record_outcome(&pool, SUBJECT, request(Some(0.1))).await.unwrap();
		assert_eq!(
			replay,
			OutcomeAccepted {
				replayed: true,
				signal: None,
				eligible_at: None
			}
		);
		assert_eq!(rows(&pool).await, 1, "no second row");
		assert_eq!(stored_levels(&pool).await, after_one, "no second drain");
	}

	/// An identical retry is recognised against what was recorded, even after
	/// the session was edited so the original report would no longer validate
	/// (from a `chatgpt-codex-connector` finding on #360).
	#[tokio::test]
	async fn a_replay_is_recognised_even_after_the_session_changed_shape() {
		let pool = pool().await;
		record_outcome(&pool, SUBJECT, request(Some(0.1))).await.unwrap();

		let sessions = SessionRepository::new(pool.clone());
		let mut edited = sessions.get(SUBJECT, SESSION).await.unwrap().unwrap();
		edited.activities = vec![serde_json::json!({ "activityId": "leetype" })];
		edited.total_duration_ms = 1_000;
		sessions.upsert(SUBJECT, &edited).await.unwrap();

		let replay = record_outcome(&pool, SUBJECT, request(Some(0.1))).await.unwrap();
		assert!(replay.replayed, "still the same report of the same block");
		assert_eq!(rows(&pool).await, 1);
	}

	/// A replay that disagrees with what was recorded is refused, and changes
	/// nothing.
	#[tokio::test]
	async fn a_contradictory_replay_is_a_conflict() {
		let pool = pool().await;
		record_outcome(&pool, SUBJECT, request(Some(0.1))).await.unwrap();
		let after_one = stored_levels(&pool).await;

		let err = record_outcome(&pool, SUBJECT, request(Some(0.9))).await.unwrap_err();
		assert!(matches!(err, FileHostError::Conflict(_)), "got {err:?}");
		assert_eq!(stored_levels(&pool).await, after_one);
	}

	/// At or above target, or not assessed at all: the row is written and
	/// nothing is folded — finishing is already `SessionCompleted`'s to credit.
	#[tokio::test]
	async fn a_good_or_unassessed_block_is_recorded_without_a_signal() {
		let pool = pool().await;
		let good = record_outcome(&pool, SUBJECT, request(Some(0.9))).await.unwrap();
		assert_eq!(good.signal, None);

		let mut unassessed = request(None);
		unassessed.block_index = 1;
		unassessed.activity_id = "leetype".to_owned();
		assert_eq!(record_outcome(&pool, SUBJECT, unassessed).await.unwrap().signal, None);

		assert_eq!(rows(&pool).await, 2);
		assert!(stored_levels(&pool).await.is_empty(), "no signal, so no charge row was ever written");
	}

	/// Every refusal names its field, and none of them writes anything.
	#[tokio::test]
	async fn invalid_outcomes_are_refused_by_field_not_clamped() {
		let pool = pool().await;

		let mut outside = request(Some(0.5));
		outside.block_index = 2;
		assert_eq!(field_errors(&record_outcome(&pool, SUBJECT, outside).await.unwrap_err()), ["blockIndex"]);

		let mut wrong_activity = request(Some(0.5));
		wrong_activity.activity_id = "leetype".to_owned();
		assert_eq!(field_errors(&record_outcome(&pool, SUBJECT, wrong_activity).await.unwrap_err()), ["activityId"]);

		let mut too_long = request(Some(0.5));
		too_long.elapsed_ms = 600_001;
		assert_eq!(field_errors(&record_outcome(&pool, SUBJECT, too_long).await.unwrap_err()), ["elapsedMs"]);

		assert_eq!(field_errors(&record_outcome(&pool, SUBJECT, request(Some(1.5))).await.unwrap_err()), ["score"]);
		assert_eq!(field_errors(&record_outcome(&pool, SUBJECT, request(Some(f64::NAN))).await.unwrap_err()), ["score"]);

		let mut scored_abandon = request(Some(0.2));
		scored_abandon.outcome = "abandoned".to_owned();
		assert_eq!(field_errors(&record_outcome(&pool, SUBJECT, scored_abandon).await.unwrap_err()), ["score"]);

		let mut unknown = request(None);
		unknown.outcome = "finished".to_owned();
		assert_eq!(field_errors(&record_outcome(&pool, SUBJECT, unknown).await.unwrap_err()), ["outcome"]);

		let mut backwards = request(None);
		backwards.ended_at = "2026-09-24T09:00:00Z".to_owned();
		assert_eq!(field_errors(&record_outcome(&pool, SUBJECT, backwards).await.unwrap_err()), ["endedAt"]);

		assert_eq!(rows(&pool).await, 0, "nothing refused was written");
	}

	/// Another subject's session is indistinguishable from no session.
	#[tokio::test]
	async fn an_outcome_for_another_subjects_session_is_not_found() {
		let pool = pool().await;
		let err = record_outcome(&pool, "subject-someone-else", request(Some(0.1))).await.unwrap_err();
		assert!(matches!(err, FileHostError::NotFound), "got {err:?}");
		assert_eq!(rows(&pool).await, 0);
	}
}

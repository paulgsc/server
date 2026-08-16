//! The waker — which discovers, and does not decide.
//!
//! The distinction from a cron is the whole design. A scheduler wakes up and
//! asks "should anyone be nudged?", which means the clock is the thing
//! producing work, which means every guard in the resulting policy is about
//! *time* and none of them is about *why*.
//!
//! This loop asks a much smaller question:
//!
//! ```sql
//! SELECT subject_id FROM engagement_gate WHERE eligible_at <= now
//! ```
//!
//! `eligible_at` was **solved** when the last signal arrived — the instant that
//! subject's engagement will decay to the threshold, computed once, by
//! arithmetic, and written down. So on a day when nobody has drifted, this
//! returns nothing and the pass costs one index probe. Nothing is polled toward
//! and no charge is ticked.
//!
//! The interval below is therefore not a schedule. It is the resolution at
//! which already-decided work is picked up, and shortening it makes
//! interventions land closer to their solved instant without changing which
//! ones happen.

use crate::nudge::constraints::StudyConstraints;
use crate::nudge::payload::NudgePayload;
use crate::nudge::presence;
use crate::websocket::WebSocketFsm;
use crate::{AppState, NudgeContext};
use chrono::Utc;
use engagement_repo::EngagementRepository;
use intervention::{Charge, Engine, Verdict};
use push_kit::SendOutcome;
use push_repo::{PushSubscriptionRepository, Topic};
use session_repo::{LayoutMode, SessionRecord, SessionRepository, SessionStatus};
use sqlx::SqlitePool;
use std::time::Duration;
use study_domain::{StudyAction, StudyCalibration, StudySelector, StudyV1};
use tracing::{debug, error, info, warn};

/// How many due subjects one pass will handle. A bound rather than a page: if
/// more are due than this, the rest are still due on the next pass, and a burst
/// that would notify a whole userbase at once is worth rate-limiting into.
const BATCH: i64 = 32;

/// Spawn the waker, cancelled through the shared token.
///
/// Takes `&AppState` — only the pieces this needs are cloned out for the
/// spawned task to own, so the caller isn't made to clone the whole state
/// just to start a background loop.
///
/// Guards `state.nudge` exactly once, here, rather than inside the pass that
/// runs every tick. `main.rs` already only calls `spawn` once a nudge is
/// configured, so this arm is unreachable in practice; keeping the check at
/// this single boundary means everything downstream can take a `&NudgeContext`
/// outright instead of re-deriving "is this even configured?" on every pass.
pub fn spawn(state: &AppState, interval: Duration) {
	let Some(nudge) = state.nudge.clone() else {
		warn!("waker spawn called without a configured nudge; not starting");
		return;
	};
	let cancel = state.core.cancel_token.clone();
	let db = state.core.shared_db.clone();
	let ws = state.realtime.ws.clone();
	crate::metrics::waker::record_interval(interval);

	tokio::spawn(async move {
		info!(interval_secs = interval.as_secs(), "engagement waker started");
		let mut ticker = tokio::time::interval(interval);
		ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

		loop {
			tokio::select! {
				() = cancel.cancelled() => {
					info!("engagement waker cancelled");
					return;
				}
				_ = ticker.tick() => {
					match run_once(&db, &ws, &nudge).await {
						Ok(_) => crate::metrics::waker::record_successful_pass(),
						Err(err) => error!(error = %err, "waker pass failed"),
					}
				}
			}
		}
	});
}

/// One pass. Public so a debug endpoint can force it without waiting.
///
/// Takes the database pool, the websocket layer, and a proven-present
/// `&NudgeContext` — the projection of `AppState` this pass actually reads —
/// rather than `&AppState` itself. `spawn` is the only caller and it borrows
/// these once per interval, not once per due subject, so the loop below
/// passes the same three references through without re-deriving them.
///
/// # Errors
/// Any storage failure. Per-subject failures are logged and skipped rather than
/// aborting the pass — one bad row must not stop everyone else's.
pub async fn run_once(db: &SqlitePool, ws: &WebSocketFsm, nudge: &NudgeContext) -> anyhow::Result<usize> {
	let engagement = EngagementRepository::new(db.clone());
	let now = Utc::now();
	let due = engagement.due(&now.to_rfc3339(), BATCH).await?;

	if due.is_empty() {
		return Ok(0);
	}

	debug!(count = due.len(), "subjects the arithmetic marked eligible");
	let mut intervened = 0;

	for gate in due {
		match consider(db, ws, nudge, &engagement, &gate.subject_id).await {
			Ok(true) => intervened += 1,
			Ok(false) => {}
			Err(err) => error!(subject = %gate.subject_id, error = %err, "could not consider a due subject"),
		}
	}

	Ok(intervened)
}

/// Evaluate one subject, and act if the engine says so.
async fn consider(db: &SqlitePool, ws: &WebSocketFsm, nudge: &NudgeContext, engagement: &EngagementRepository, subject_id: &str) -> anyhow::Result<bool> {
	let now = Utc::now();

	let stored = engagement.charge(subject_id).await?;
	#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
	let levels: Vec<(u16, f64)> = stored.iter().map(|row| (row.class as u16, row.level)).collect();
	let as_of = stored.first().map_or(now, |row| crate::nudge::clock::parse_timestamp(&row.as_of).unwrap_or(now));
	let mut charge = Charge::<StudyV1>::from_storage::<StudyCalibration>(&levels, as_of);

	let subscriptions = PushSubscriptionRepository::new(db.clone()).for_subject(subject_id).await?;
	let consented_topics: Vec<Topic> = Topic::ALL.iter().copied().filter(|topic| subscriptions.iter().any(|sub| sub.accepts(*topic))).collect();

	let constraints = StudyConstraints {
		clock: nudge.clock.clone(),
		enabled: nudge.enabled,
		quiet_hours_start: nudge.quiet_hours_start,
		quiet_hours_end: nudge.quiet_hours_end,
		presence: presence::observe(ws, nudge.presence_freshness).await,
		consented_topics,
	};

	// Selection needs a target in order to choose the message, but provisioning
	// must not happen until constraints admit the intervention (quiet hours or
	// missing consent must not create sessions). Replace this sentinel with the
	// persisted session before claiming or sending.
	let engine = Engine::<StudyV1, StudyCalibration, _, _>::new(
		constraints,
		StudySelector {
			prepared_session: Some(String::new()),
		},
	);
	let gate = engagement.gate(subject_id).await?;
	let last_intervened_at = gate
		.and_then(|row| row.last_intervened_at)
		.and_then(|raw| crate::nudge::clock::parse_timestamp(raw.as_str()));

	let action = match engine.evaluate(&charge, now, last_intervened_at) {
		Verdict::Intervene(action) => {
			// Provision before announcing. Inactivity commonly means that the
			// person never created a draft; requiring one made this path a
			// cyclical `NothingToSay` no-op.
			let session_id = prepare_session(&SessionRepository::new(db.clone()), now).await?;
			action.with_session(session_id)
		}
		Verdict::Wait { until } => {
			// Push the gate out so this subject stops being returned by `due`.
			// Without it the waker would re-read the same row every pass.
			let (levels, as_of) = charge.to_storage();
			engagement.save(subject_id, &levels, &as_of.to_rfc3339(), &until.to_rfc3339()).await?;
			return Ok(false);
		}
		Verdict::Suppressed { reason, retry_at } => {
			info!(subject = %subject_id, reason = reason.as_str(), "warranted but not admissible");
			let (levels, as_of) = charge.to_storage();
			engagement.save(subject_id, &levels, &as_of.to_rfc3339(), &retry_at.to_rfc3339()).await?;
			return Ok(false);
		}
		Verdict::NothingToSay => {
			// Depleted, admissible, and no action fits — which almost always
			// means nothing is prepared. A gap in the domain, worth saying so.
			warn!(subject = %subject_id, "engagement is low but there is nothing to point them at");
			let retry = now + chrono::Duration::hours(6);
			let (levels, as_of) = charge.to_storage();
			engagement.save(subject_id, &levels, &as_of.to_rfc3339(), &retry.to_rfc3339()).await?;
			return Ok(false);
		}
	};

	// Claim before sending. A crash between the two costs this intervention
	// rather than duplicating it, which is the right way round.
	let next_eligible = engine.intervened(&mut charge, now);
	// Disallowed for tracing; this is the stored history column.
	#[allow(clippy::disallowed_methods)]
	let serialized = serde_json::to_string(&action).unwrap_or_default();
	let Some(log_id) = engagement
		.claim(subject_id, &now.to_rfc3339(), &next_eligible.to_rfc3339(), action.kind(), &serialized)
		.await?
	else {
		info!(subject = %subject_id, "another pass claimed this subject first");
		return Ok(false);
	};

	let (levels, as_of) = charge.to_storage();
	engagement.save(subject_id, &levels, &as_of.to_rfc3339(), &next_eligible.to_rfc3339()).await?;

	let accepted = actuate(db, nudge, &action, subject_id).await?;
	if accepted == 0 {
		warn!(subject = %subject_id, "no device accepted the intervention; releasing the claim");
		engagement.release(subject_id, log_id, &now.to_rfc3339()).await?;
		return Ok(false);
	}

	engagement.mark_actuated(log_id, &Utc::now().to_rfc3339()).await?;
	info!(subject = %subject_id, action = action.kind(), devices = accepted, "intervened");
	Ok(true)
}

trait WithSession {
	fn with_session(self, session_id: String) -> Self;
}

impl WithSession for StudyAction {
	fn with_session(self, session_id: String) -> Self {
		match self {
			Self::LessonReady { .. } => Self::LessonReady { session_id },
			Self::ResumeAbandoned { .. } => Self::ResumeAbandoned { session_id },
			Self::SuggestReview { .. } => Self::SuggestReview { session_id },
			Self::NewMaterial { .. } => Self::NewMaterial { session_id },
		}
	}
}

/// Return a session the notification can open, scheduling one when the user
/// has no work waiting. Provisioning precedes the engagement claim and push so
/// a notification is never accepted for a session that does not exist yet.
async fn prepare_session(sessions: &SessionRepository, now: chrono::DateTime<Utc>) -> Result<String, session_repo::repository::SessionRepoError> {
	if let Some(mut session) = sessions
		.list()
		.await?
		.into_iter()
		.find(|session| matches!(session.status, SessionStatus::Scheduled | SessionStatus::Paused | SessionStatus::Draft))
	{
		if session.status == SessionStatus::Draft {
			session.status = SessionStatus::Scheduled;
			session.updated_at = now.to_rfc3339();
			sessions.upsert(&session).await?;
		}
		return Ok(session.id);
	}

	let stamp = now.to_rfc3339();
	let session = SessionRecord {
		id: format!("session-{}", uuid::Uuid::new_v4()),
		name: "Today's session".to_owned(),
		status: SessionStatus::Scheduled,
		activities: Vec::new(),
		scenes: Vec::new(),
		layout_mode: LayoutMode::Basic,
		layout: None,
		total_duration_ms: 0,
		created_at: stamp.clone(),
		updated_at: stamp,
		started_at: None,
		completed_at: None,
		final_elapsed_ms: None,
	};

	sessions.upsert(&session).await?;
	Ok(session.id)
}

/// Put an action on the wire, to every device that consented to its topic.
///
/// Returns how many were **accepted** — which, as `push_kit` is at pains to
/// say, is not how many were delivered.
async fn actuate(db: &SqlitePool, nudge: &NudgeContext, action: &StudyAction, subject_id: &str) -> Result<usize, sqlx::Error> {
	let subscriptions_repo = PushSubscriptionRepository::new(db.clone());
	let subscriptions = subscriptions_repo.for_subject(subject_id).await?;

	let payload = NudgePayload::for_action(&nudge.base_url, action);
	let encoded = match payload.to_bytes() {
		Ok(bytes) => bytes,
		Err(err) => {
			error!(error = %err, "could not serialize the notification payload");
			return Ok(0);
		}
	};

	let topic = crate::nudge::payload::topic_for(action);
	let mut accepted = 0;

	for stored in subscriptions {
		if !stored.accepts(topic) {
			continue;
		}

		let outcome = nudge.sender.deliver(&stored.subscription, &encoded).await;
		let stamp = Utc::now().to_rfc3339();
		let endpoint = &stored.subscription.endpoint;

		if outcome.should_prune() {
			info!(%endpoint, "subscription is gone; pruning");
			subscriptions_repo.delete_by_endpoint(endpoint).await?;
			continue;
		}

		if outcome.is_failure() {
			warn!(%endpoint, outcome = outcome.label(), detail = ?outcome, "push was not accepted");
			subscriptions_repo.record_failure(endpoint, &stamp).await?;
			continue;
		}

		accepted += 1;
		// Deliberately not "delivered": the push service accepted it, and
		// whether anyone ever sees it is not observable from here.
		debug_assert_eq!(outcome, SendOutcome::Accepted);
		subscriptions_repo.record_success(endpoint, &stamp).await?;
	}

	Ok(accepted)
}

/// Fold a signal into a subject's charge and re-solve their eligibility.
///
/// This is the *only* place work is created. Everything the waker later does
/// was decided here, by arithmetic, at the moment something actually happened.
///
/// # Errors
/// Propagates any storage failure.
pub async fn observe(db: &SqlitePool, subject_id: &str, signal: &study_domain::StudySignal) -> Result<chrono::DateTime<Utc>, sqlx::Error> {
	let engagement = EngagementRepository::new(db.clone());
	let now = Utc::now();

	let stored = engagement.charge(subject_id).await?;
	#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
	let levels: Vec<(u16, f64)> = stored.iter().map(|row| (row.class as u16, row.level)).collect();
	let as_of = stored.first().map_or(now, |row| crate::nudge::clock::parse_timestamp(&row.as_of).unwrap_or(now));

	// No rows means never seen, and `from_storage` starts such a subject
	// **full** rather than empty — an empty charge is instantly eligible, so
	// the alternative would nudge a brand-new account before it did anything.
	let mut charge = Charge::<StudyV1>::from_storage::<StudyCalibration>(&levels, as_of);

	charge.apply::<StudyCalibration>(signal, now);
	let eligible_at = charge.eligible_at::<StudyCalibration>(now);

	let (levels, stamp) = charge.to_storage();
	engagement.save(subject_id, &levels, &stamp.to_rfc3339(), &eligible_at.to_rfc3339()).await?;

	debug!(subject = %subject_id, signal = signal.kind(), eligible_at = %eligible_at, "signal folded in");
	Ok(eligible_at)
}

#[cfg(test)]
mod tests {
	use super::prepare_session;
	use chrono::{TimeZone as _, Utc};
	use session_repo::{LayoutMode, SessionRecord, SessionRepository, SessionStatus};
	use sqlx::sqlite::SqlitePoolOptions;

	async fn sessions() -> SessionRepository {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		sqlx::query(
			r#"CREATE TABLE sessions (
				id TEXT PRIMARY KEY, name TEXT NOT NULL, status TEXT NOT NULL,
				layout_mode TEXT NOT NULL, total_duration_ms INTEGER NOT NULL,
				created_at TEXT NOT NULL, updated_at TEXT NOT NULL,
				started_at TEXT, completed_at TEXT, final_elapsed_ms INTEGER,
				activities TEXT NOT NULL, scenes TEXT NOT NULL, layout TEXT
			)"#,
		)
		.execute(&pool)
		.await
		.unwrap();
		SessionRepository::new(pool)
	}

	fn at_noon() -> chrono::DateTime<Utc> {
		Utc.with_ymd_and_hms(2026, 8, 16, 12, 0, 0).unwrap()
	}

	#[tokio::test]
	async fn inactivity_provisions_a_scheduled_session_before_the_nudge() {
		let sessions = sessions().await;

		let id = prepare_session(&sessions, at_noon()).await.unwrap();
		let stored = sessions.get(&id).await.unwrap().unwrap();

		assert_eq!(stored.status, SessionStatus::Scheduled);
		assert_eq!(stored.name, "Today's session");
		assert!(stored.activities.is_empty());
	}

	#[tokio::test]
	async fn a_draft_is_scheduled_instead_of_creating_a_duplicate() {
		let sessions = sessions().await;
		let stamp = at_noon().to_rfc3339();
		let draft = SessionRecord {
			id: "session-draft".to_owned(),
			name: "Prepared lesson".to_owned(),
			status: SessionStatus::Draft,
			activities: Vec::new(),
			scenes: Vec::new(),
			layout_mode: LayoutMode::Basic,
			layout: None,
			total_duration_ms: 0,
			created_at: stamp.clone(),
			updated_at: stamp,
			started_at: None,
			completed_at: None,
			final_elapsed_ms: None,
		};
		sessions.upsert(&draft).await.unwrap();

		let id = prepare_session(&sessions, at_noon()).await.unwrap();

		assert_eq!(id, draft.id);
		assert_eq!(sessions.get(&id).await.unwrap().unwrap().status, SessionStatus::Scheduled);
		assert_eq!(sessions.list().await.unwrap().len(), 1);
	}
}

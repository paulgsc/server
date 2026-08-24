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

use crate::handlers::db::session::new_id;
use crate::nudge::constraints::StudyConstraints;
use crate::nudge::payload::NudgePayload;
use crate::nudge::presence;
use crate::websocket::WebSocketFsm;
use crate::{AppState, NudgeContext};
use chrono::{DateTime, Utc};
use engagement_repo::EngagementRepository;
use intervention::{Admissibility, Calibration, Charge, Engine, Selector, Verdict};
use push_kit::SendOutcome;
use push_repo::{PushSubscriptionRepository, Topic};
use session_repo::{LayoutMode, SessionRecord, SessionRepository, SessionStatus};
use sqlx::SqlitePool;
use std::time::Duration;
use study_domain::{StudyAction, StudyCalibration, StudySelector, StudySignal, StudyV1};
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
pub async fn run_once(db: &SqlitePool, ws: &WebSocketFsm, nudge: &NudgeContext) -> Result<usize, sqlx::Error> {
	let engagement = EngagementRepository::new(db.clone());
	let now = Utc::now();
	let due = engagement.due(&now.to_rfc3339(), BATCH).await?;
	crate::metrics::waker::record_due(due.len());

	if due.is_empty() {
		return Ok(0);
	}

	debug!(count = due.len(), "subjects the arithmetic marked eligible");
	let mut intervened = 0;

	for gate in due {
		match consider(db, ws, nudge, &engagement, &gate.subject_id).await {
			Ok(true) => intervened += 1,
			Ok(false) => {}
			Err(err) => {
				error!(subject = %gate.subject_id, error = %err, "could not consider a due subject");
				crate::metrics::waker::record_verdict("storage_error", "n/a");
			}
		}
	}

	Ok(intervened)
}

/// Evaluate one subject, and act if the engine says so.
async fn consider(db: &SqlitePool, ws: &WebSocketFsm, nudge: &NudgeContext, engagement: &EngagementRepository, subject_id: &str) -> Result<bool, sqlx::Error> {
	let now = Utc::now();

	let stored = engagement.charge(subject_id).await?;
	#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
	let levels: Vec<(u16, f64)> = stored.iter().map(|row| (row.class as u16, row.level)).collect();
	let as_of = stored.first().map_or(now, |row| crate::nudge::clock::parse_timestamp(&row.as_of).unwrap_or(now));
	let mut charge = Charge::<StudyV1>::from_storage::<StudyCalibration>(&levels, as_of);

	let subscriptions = PushSubscriptionRepository::new(db.clone()).for_subject(subject_id).await?;
	let consented_topics: Vec<Topic> = Topic::ALL.iter().copied().filter(|topic| subscriptions.iter().any(|sub| sub.accepts(*topic))).collect();

	// What is prepared and untouched. Without one there is nothing to point at,
	// and the selector returns `None` (or, for plain absence, `GetStarted`)
	// rather than inventing a reminder that opens nothing. A failed read is
	// deliberately *not* defaulted to empty: since `GetStarted` now fires on
	// exactly that emptiness, silently swallowing the error here would be
	// indistinguishable from a genuinely empty catalogue and could invite
	// someone who already has a session waiting. Logged and skipped instead —
	// `SessionRepoError` doesn't convert to this function's `sqlx::Error`, and
	// widening the signature is a bigger change than a rare read failure
	// warrants.
	//
	// The repository handle is hoisted to a local rather than constructed
	// inline: the `NothingToSay` arm below (#279/RCM2) needs the same one
	// to write a provisioned session, and it is a cheap handle over the
	// shared pool, not a connection of its own.
	let sessions = SessionRepository::new(db.clone());
	let prepared_session = match sessions.first_prepared(subject_id).await {
		Ok(prepared_session) => prepared_session,
		Err(err) => {
			error!(subject = %subject_id, error = %err, "could not read sessions; skipping this subject rather than guessing whether one is prepared");
			crate::metrics::waker::record_verdict("storage_error", "n/a");
			return Ok(false);
		}
	};
	// #263 (SLI2): `first_prepared`'s `LIMIT 1` reads at most one row for
	// this subject, replacing the #262 (SLI1) measurement of `list()`'s
	// unbounded, per-subject read — see `SessionRepository::first_prepared`'s
	// own doc comment for the query and the ordering decisions behind it.
	// The counter's meaning changes with it: previously a climbing total,
	// now it should stay at 0 or 1 per subject per pass regardless of table
	// size — the collapse `nudge::waker`'s rewritten characterisation test
	// now asserts.
	crate::metrics::waker::record_session_rows_read(usize::from(prepared_session.is_some()));

	let constraints = StudyConstraints {
		clock: nudge.clock.clone(),
		enabled: nudge.enabled,
		quiet_hours_start: nudge.quiet_hours_start,
		quiet_hours_end: nudge.quiet_hours_end,
		presence: presence::observe(ws, nudge.presence_freshness).await,
		consented_topics,
	};

	let engine = Engine::<StudyV1, StudyCalibration, _, _>::new(constraints, StudySelector { prepared_session });
	let gate = engagement.gate(subject_id).await?;
	let last_intervened_at = gate
		.and_then(|row| row.last_intervened_at)
		.and_then(|raw| crate::nudge::clock::parse_timestamp(raw.as_str()));

	let action = match engine.evaluate(&charge, now, last_intervened_at) {
		Verdict::Intervene(action) => action,
		Verdict::Wait { until } => {
			crate::metrics::waker::record_verdict("wait", "n/a");
			// Push the gate out so this subject stops being returned by `due`.
			// Without it the waker would re-read the same row every pass.
			let (levels, as_of) = charge.to_storage();
			engagement.save(subject_id, &levels, &as_of.to_rfc3339(), &until.to_rfc3339()).await?;
			return Ok(false);
		}
		Verdict::Suppressed { reason, retry_at } => {
			info!(subject = %subject_id, reason = reason.as_str(), "warranted but not admissible");
			crate::metrics::waker::record_verdict("suppressed", reason.as_str());
			let (levels, as_of) = charge.to_storage();
			engagement.save(subject_id, &levels, &as_of.to_rfc3339(), &retry_at.to_rfc3339()).await?;
			return Ok(false);
		}
		Verdict::NothingToSay => {
			// Depleted and past refractory/eligibility — `evaluate` only
			// reaches this arm once both have already passed — but nothing
			// fits. Plain absence always has `GetStarted` (see
			// `StudySelector::select`), so reaching here means the dominant
			// deficit is Momentum, Mastery, or Freshness with nothing
			// prepared to resume, review, or announce.
			//
			// #279 (RCM2)'s decision, written down per its acceptance
			// criteria: provision a session here rather than widen the
			// vocabulary. Two other shapes were weighed and rejected:
			//
			// - A fifth `StudyAction` variant (`ProposeSession`) breaks
			//   `StudyAction::session_id()`'s totality — every existing
			//   variant already carries a real id — and forces
			//   `payload::topic_for`/`NudgePayload::for_action` to grow a
			//   case for "the same message, before a session exists."
			// - A new `Verdict` arm in `intervention` puts "the domain
			//   wants something created" into the generic engine, which
			//   `intervention`'s own docs are explicit about keeping free
			//   of study vocabulary: "every user story adds a variant [to
			//   `study_domain`]; none of them should touch `intervention`."
			//
			// So `intervention` and `StudySelector` both stay untouched:
			// provisioning happens here, `prepared_session` becomes
			// `Some`, and the *existing* selector maps the same dominant
			// deficit to `ResumeAbandoned`/`SuggestReview`/`NewMaterial`
			// exactly as it would for a session that already existed.
			// RCM3 (#280) and RCM5 (#282) decide what actually goes in
			// it; this one is deliberately empty until they land — see
			// `provisioned_session`'s own doc comment.
			//
			// Crash safety without extra bookkeeping: the write below is
			// a `Draft` row `SessionRepository::first_prepared` will find
			// on any later pass. A crash between this write and `claim`
			// below costs this pass's notification, not a second session
			// — the next pass reads the row this one already wrote and
			// reaches `Verdict::Intervene` directly, skipping this arm
			// entirely.
			let session_id = new_id();
			let provisioned = provisioned_session(session_id.clone(), now);
			if let Err(err) = sessions.upsert(subject_id, &provisioned).await {
				error!(subject = %subject_id, error = %err, "could not write a provisioned session; skipping this subject rather than notifying about one that doesn't exist");
				crate::metrics::waker::record_verdict("storage_error", "n/a");
				return Ok(false);
			}

			// Neutral to engagement (delta 0.0, filed under Freshness): it
			// is the opportunity a later `LessonReady`/`ResumeAbandoned`/
			// `SuggestReview`/`NewMaterial` needs to be sayable, not a
			// sign of engagement itself. See `StudySignal::
			// SessionProvisioned`'s own doc comment — this is the first
			// thing in the codebase to actually apply it.
			charge.apply::<StudyCalibration>(&StudySignal::SessionProvisioned { session_id: session_id.clone() }, now);

			let deficits = charge.deficits::<StudyCalibration>(now);
			let Some(reselected) = StudySelector {
				prepared_session: Some(session_id),
			}
			.select(&deficits) else {
				// Cannot happen by construction: reaching `NothingToSay`
				// already proved the dominant deficit is not `Presence`,
				// and `prepared_session` is now `Some`, so `select`'s
				// `Some(_)` arm is exhaustive over the remaining three
				// classes. Guarded rather than `.expect`ed anyway — a
				// class added to `EngagementClass` without a matching
				// `StudySelector` arm should cost one skipped pass for
				// this subject, not a panicked waker.
				error!(subject = %subject_id, "provisioned a session but the selector still found nothing to say; this should be unreachable");
				crate::metrics::waker::record_verdict("nothing_to_say", "n/a");
				let retry = now + chrono::Duration::hours(6);
				let (levels, as_of) = charge.to_storage();
				engagement.save(subject_id, &levels, &as_of.to_rfc3339(), &retry.to_rfc3339()).await?;
				return Ok(false);
			};

			// The provisioned session is real and written, but the
			// intervention itself still has to clear the same admission
			// gate any other action would — quiet hours and consent do
			// not stop applying just because this action came from
			// provisioning rather than an existing session.
			match engine.admissibility().admit(now, &reselected) {
				Ok(()) => reselected,
				Err(reason) => {
					info!(subject = %subject_id, reason = reason.as_str(), "provisioned a session, but the intervention is not admissible yet");
					crate::metrics::waker::record_verdict("suppressed", reason.as_str());
					let retry = now + StudyCalibration::REFRACTORY.min(chrono::Duration::hours(1));
					let (levels, as_of) = charge.to_storage();
					engagement.save(subject_id, &levels, &as_of.to_rfc3339(), &retry.to_rfc3339()).await?;
					return Ok(false);
				}
			}
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
		crate::metrics::waker::record_verdict("claim_lost", "n/a");
		return Ok(false);
	};

	let (levels, as_of) = charge.to_storage();
	engagement.save(subject_id, &levels, &as_of.to_rfc3339(), &next_eligible.to_rfc3339()).await?;

	let accepted = actuate(db, nudge, &action, subject_id).await?;
	if accepted == 0 {
		warn!(subject = %subject_id, "no device accepted the intervention; releasing the claim");
		crate::metrics::waker::record_verdict("no_device_accepted", "n/a");
		engagement.release(subject_id, log_id, &now.to_rfc3339()).await?;
		return Ok(false);
	}

	engagement.mark_actuated(log_id, &Utc::now().to_rfc3339()).await?;
	info!(subject = %subject_id, action = action.kind(), devices = accepted, "intervened");
	crate::metrics::waker::record_verdict("sent", "n/a");
	Ok(true)
}

/// A minimal, valid session for a subject who has nothing prepared but is
/// about to be intervened on. Not the recommender (`#280`, RCM3) and not the
/// materialiser (`#282`, RCM5) — deliberately out of scope for #279, exactly
/// as its issue body draws the line. This session has no activities and no
/// scenes; it exists so `first_prepared` has a real row to find and
/// `GET /sessions/:id` has a real one to answer, not so a person has
/// something to actually do yet. RCM3 and RCM5 are what fill it in once they
/// land; until then `name` is the only signal that this was proposed rather
/// than authored — `#283` (RCM6) is what gives that signal a real field.
fn provisioned_session(id: String, now: DateTime<Utc>) -> SessionRecord {
	let stamp = now.to_rfc3339();
	SessionRecord {
		id,
		name: "Suggested for you".to_owned(),
		// Every session starts as a draft, provisioned ones included — see
		// `handlers::db::session::create_session`'s identical reasoning: a
		// session born anything but `Draft` could be offered before
		// anyone, including the recommender that will fill this one in,
		// had finished composing it.
		status: SessionStatus::Draft,
		activities: Vec::new(),
		scenes: Vec::new(),
		layout_mode: LayoutMode::Basic,
		layout: None,
		// Zero rather than computed from `scenes`: `scenes` is empty here
		// by construction, and `total_duration_of(&[])` is zero anyway —
		// see its own doc comment.
		total_duration_ms: 0,
		created_at: stamp.clone(),
		updated_at: stamp,
		started_at: None,
		completed_at: None,
		final_elapsed_ms: None,
	}
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

/// First contact: give a subject who has never been observed a gate row,
/// seeded full, so `due` can eventually find them without a signal ever
/// having arrived.
///
/// Before this, the chain was: no session created ⇒ no signal ⇒ no gate row
/// ⇒ never in `due` ⇒ never nudged. Not late — never. The only caller is
/// `POST /push/subscriptions`: it is a person explicitly saying "you may
/// interrupt me", the strongest statement of intent this deployment has, and
/// (unlike a page load, which is too broad, or a dedicated "hello" route,
/// which would just re-say what this one already implies) it happens exactly
/// once per device before any study behaviour exists. The alternative of
/// seeding lazily when the waker runs is not available at all: the waker can
/// only see rows that already exist, which is the circularity this function
/// closes.
///
/// Seeded **full**, by the same `Charge::full`/`eligible_at` arithmetic
/// `observe` falls back to for an unseen subject — see the comment there.
/// Starting empty would make a brand-new account instantly eligible; someone
/// who installs the app at 9am must not be interrupted at 9:05.
///
/// Idempotent: [`EngagementRepository::seed_if_absent`] is a no-op for a
/// subject who already has a row, whether from a prior signal or from a
/// previous device subscribing.
///
/// Returns whether this call actually seeded the row, so [`backfill_first_contact`]
/// can report how much of its reconciliation pass was real work.
///
/// # Errors
/// Propagates any storage failure.
pub async fn first_contact(db: &SqlitePool, subject_id: &str) -> Result<bool, sqlx::Error> {
	let engagement = EngagementRepository::new(db.clone());
	let now = Utc::now();

	let charge = Charge::<StudyV1>::full::<StudyCalibration>(now);
	let eligible_at = charge.eligible_at::<StudyCalibration>(now);
	let (levels, as_of) = charge.to_storage();

	let created = engagement.seed_if_absent(subject_id, &levels, &as_of.to_rfc3339(), &eligible_at.to_rfc3339()).await?;

	if created {
		debug!(subject = %subject_id, eligible_at = %eligible_at, "first contact: gate row seeded full");
	}

	Ok(created)
}

/// Reconcile pre-existing subscriptions against `engagement_gate`, once, at
/// boot.
///
/// `first_contact` only runs on a live `POST /push/subscriptions` call, and a
/// browser that already holds a subscription from before this landed has no
/// reason to send it again — the Push API does not re-announce an unchanged
/// subscription on its own. Without this pass, every subject who subscribed
/// before this release stays exactly the silence #1 victim this feature
/// exists to fix: a `push_subscriptions` row with no path into
/// `engagement_gate`, forever, unless they happen to unsubscribe and
/// resubscribe.
///
/// Run once at startup rather than folded into the waker's per-tick loop: this
/// is a one-time reconciliation against rows that predate the fix, not
/// ongoing work, and `first_contact`'s own idempotency makes repeating it on
/// every restart a cheap no-op rather than a hazard.
///
/// # Errors
/// Propagates a failure to read `push_subscriptions` itself. A failure to
/// seed one particular subject is logged and skipped rather than aborting the
/// pass — one bad row must not leave everyone else ungated, and the next boot
/// tries again.
pub async fn backfill_first_contact(db: &SqlitePool) -> Result<usize, sqlx::Error> {
	let subject_ids = PushSubscriptionRepository::new(db.clone()).distinct_subject_ids().await?;

	let mut seeded = 0;
	for subject_id in subject_ids {
		match first_contact(db, &subject_id).await {
			Ok(true) => seeded += 1,
			Ok(false) => {}
			Err(err) => error!(subject = %subject_id, error = %err, "could not backfill a gate row for an existing subscription; will retry next boot"),
		}
	}

	if seeded > 0 {
		info!(seeded, "backfilled gate rows for subscriptions that predate first contact");
	}

	Ok(seeded)
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::nudge::clock::NudgeClock;
	use crate::nudge::constraints::{StudyConstraints, Suppressed};
	use crate::nudge::presence::Presence;
	use chrono::{Duration, TimeZone as _};

	fn t0() -> chrono::DateTime<Utc> {
		Utc.with_ymd_and_hms(2026, 8, 5, 12, 0, 0).unwrap()
	}

	/// `format!` is on `clippy.toml`'s disallowed-macros list (eager
	/// allocation ahead of tracing); `write!` into an owned `String` is the
	/// same workaround #299 used for the identical lint.
	fn numbered(prefix: &str, i: impl std::fmt::Display) -> String {
		use std::fmt::Write as _;
		let mut s = String::from(prefix);
		let _ = write!(s, "{i}");
		s
	}

	/// #278's named edge case: someone subscribes, then unsubscribes every
	/// device before ever becoming due. `actuate` already degrades correctly
	/// once admission fails; the risk the issue calls out is landing in
	/// `Verdict::NothingToSay` instead, which `consider` logs at `warn!` on
	/// every pass. Presence — fastest half-life, highest weight — stays the
	/// dominant deficit for a subject who has never received a single signal,
	/// so `GetStarted` (#294) keeps firing and the engine explains the silence
	/// as `NotConsented` at `info!` rather than falling through to a warn loop.
	#[test]
	fn a_first_contact_subject_who_unsubscribed_everywhere_is_suppressed_not_stuck_with_nothing_to_say() {
		let charge = Charge::<StudyV1>::full::<StudyCalibration>(t0());
		let far_future = t0() + Duration::days(365);

		let constraints = StudyConstraints {
			clock: NudgeClock::resolve(Some("UTC")).0,
			enabled: true,
			quiet_hours_start: 22,
			quiet_hours_end: 8,
			presence: Presence { connected: 0, live: 0 },
			consented_topics: Vec::new(),
		};
		let engine = Engine::<StudyV1, StudyCalibration, _, _>::new(constraints, StudySelector { prepared_session: None });

		let verdict = engine.evaluate(&charge, far_future, None);
		assert!(
			matches!(
				verdict,
				Verdict::Suppressed {
					reason: Suppressed::NotConsented,
					..
				}
			),
			"got {verdict:?}, expected NotConsented suppression rather than NothingToSay"
		);
	}

	/// #263 (SLI2): the fix, not just the measurement. This is #262 (SLI1)'s
	/// own characterisation test, inverted rather than deleted, exactly as
	/// #262's doc comment called for: *"When #263 lands, this assertion
	/// should invert to a small constant — not be deleted, since 'the
	/// waker's read cost stays flat as the table grows' is exactly the
	/// invariant worth keeping under test forever after."*
	///
	/// Each due subject still owns its own `SESSIONS_PER_SUBJECT` (5)
	/// sessions, unchanged from #262's setup — the whole point is that this
	/// count no longer appears in the expected total below.
	/// `SessionRepository::first_prepared`'s `LIMIT 1` reads at most one row
	/// per due subject regardless of how many it owns, so growing
	/// `SESSIONS_PER_SUBJECT` from `5` to `500` could not move this number:
	/// that is what "flat as the table grows" means, made concrete.
	#[test]
	fn one_pass_reads_at_most_one_row_per_due_subject() {
		use metrics_util::debugging::{DebugValue, DebuggingRecorder};
		use metrics_util::CompositeKey;
		use push_kit::{ReqwestTransport, Sender, VapidIdentity};
		use session_repo::{LayoutMode, SessionRecord, SessionStatus};
		use sqlx::sqlite::SqlitePoolOptions;

		const DUE_SUBJECTS: i64 = 40; // > BATCH (32), so the cap itself is exercised
		const SESSIONS_PER_SUBJECT: usize = 5;

		// The `web-push` crate's own test vector — also what `push_kit`'s own
		// suite signs with (`crates/push_kit/src/identity.rs`). Fixed rather
		// than generated, so this test needs neither a random source nor a
		// `web_push`/`base64` dev-dependency just to hand `NudgeContext` a
		// keypair that validates against itself.
		const VAPID_PRIVATE: &str = "IQ9Ur0ykXoHS9gzfYX0aBjy9lvdrjx_PFUXmie9YRcY";
		const VAPID_PUBLIC: &str = "BMjQIp55pdbU8pfCBKyXcZjlmER_mXt5LqNrN1hrXbdBS5EnhIbMu3Au-RV53iIpztzNXkGI56BFB1udQ8Bq_H4";

		// Embedded and run against a fresh in-memory database rather than the
		// externally-applied `DATABASE_URL` scratch database `cargo test`
		// already requires for `sqlx::query!` to compile: that database is
		// one file shared by every crate's tests in one `rust_ci` run, and a
		// row-count characterisation needs a table only this test has written
		// to. `sqlx::migrate!` embeds the same `.up.sql`/`.down.sql` pairs at
		// compile time, so no second migration story exists to keep in step.
		static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");

		let recorder = DebuggingRecorder::new();
		let snapshotter = recorder.snapshotter();

		metrics::with_local_recorder(&recorder, || {
			let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("build a current-thread runtime");

			rt.block_on(async {
				// A single connection: SQLite's `:memory:` database is private
				// to the connection that opened it, so a pool free to open a
				// second one would silently hand some queries an empty,
				// unmigrated schema. This test has no concurrency to justify
				// more than one connection.
				let pool = SqlitePoolOptions::new()
					.max_connections(1)
					.connect("sqlite::memory:")
					.await
					.expect("open an in-memory sqlite database");
				MIGRATOR.run(&pool).await.expect("run the workspace migration history");

				let now = Utc::now();
				let past = (now - Duration::hours(1)).to_rfc3339();
				let now_str = now.to_rfc3339();

				let sessions_repo = SessionRepository::new(pool.clone());
				for i in 0..DUE_SUBJECTS {
					let subject_id = numbered("subject-", i);
					EngagementRepository::new(pool.clone())
						.seed_if_absent(&subject_id, &[], &now_str, &past)
						.await
						.expect("seed a due subject's gate row");

					for j in 0..SESSIONS_PER_SUBJECT {
						use std::fmt::Write as _;
						let mut id = String::from("session-");
						let _ = write!(id, "{i}-{j}");
						let record = SessionRecord {
							name: id.clone(),
							id,
							status: SessionStatus::Draft,
							activities: Vec::new(),
							scenes: Vec::new(),
							layout_mode: LayoutMode::Basic,
							layout: None,
							total_duration_ms: 0,
							created_at: now_str.clone(),
							updated_at: now_str.clone(),
							started_at: None,
							completed_at: None,
							final_elapsed_ms: None,
						};
						sessions_repo.upsert(&subject_id, &record).await.expect("seed a session row owned by this due subject");
					}
				}

				let ws = WebSocketFsm::new();
				let vapid = VapidIdentity::from_config(Some(VAPID_PRIVATE), Some(VAPID_PUBLIC), "mailto:test@example.com").expect("the fixed test keypair validates");
				let nudge = NudgeContext {
					clock: NudgeClock::resolve(Some("UTC")).0,
					sender: std::sync::Arc::new(Sender::new(vapid.clone(), ReqwestTransport::default())),
					vapid,
					enabled: true,
					quiet_hours_start: 22,
					quiet_hours_end: 8,
					presence_freshness: std::time::Duration::from_secs(120),
					base_url: "https://example.com".to_owned(),
				};

				run_once(&pool, &ws, &nudge)
					.await
					.expect("a pass over freshly-drafted sessions and unconsented subjects should not error");
			});
		});

		let snapshot: Vec<(CompositeKey, Option<metrics::Unit>, Option<metrics::SharedString>, DebugValue)> = snapshotter.snapshot().into_vec();
		let rows_read = snapshot.iter().find_map(|(key, _, _, value)| {
			(key.key().name() == "nudge_waker_session_rows_read_total").then_some(match value {
				DebugValue::Counter(n) => *n,
				_ => 0,
			})
		});

		#[allow(clippy::cast_sign_loss)] // BATCH and DUE_SUBJECTS are both small positive constants
		let expected = BATCH.min(DUE_SUBJECTS) as u64;
		assert_eq!(
			rows_read,
			Some(expected),
			"first_prepared's LIMIT 1 should cap each due subject's read at one row — \
			 BATCH.min(DUE_SUBJECTS) = {expected} rows total, independent of \
			 SESSIONS_PER_SUBJECT. If this now fails because the count is *higher*, something \
			 reintroduced an unbounded read on this path."
		);
	}

	/// #279 (RCM2)'s core acceptance criterion, end to end against a real
	/// database: a subject who is due, has nothing prepared, and whose
	/// dominant deficit is not `Presence` — `Verdict::NothingToSay`'s exact
	/// precondition — gets a real, findable session instead of a `warn!` and
	/// a six-hour retry.
	///
	/// Push subscriptions are deliberately not seeded. With none,
	/// `StudyConstraints::admit` refuses on `NotConsented` before it ever
	/// checks quiet hours (consent is the precondition checked first — see
	/// `constraints::admit`), which keeps this test's outcome independent of
	/// wall-clock time and lets it isolate the *provisioning* half of #279
	/// without needing a working push transport to assert "sent".
	#[test]
	fn nothing_prepared_and_a_non_presence_dominant_deficit_gets_a_provisioned_session_not_a_warn_loop() {
		use metrics_util::debugging::{DebugValue, DebuggingRecorder};
		use metrics_util::CompositeKey;
		use push_kit::{ReqwestTransport, Sender, VapidIdentity};
		use sqlx::sqlite::SqlitePoolOptions;

		const VAPID_PRIVATE: &str = "IQ9Ur0ykXoHS9gzfYX0aBjy9lvdrjx_PFUXmie9YRcY";
		const VAPID_PUBLIC: &str = "BMjQIp55pdbU8pfCBKyXcZjlmER_mXt5LqNrN1hrXbdBS5EnhIbMu3Au-RV53iIpztzNXkGI56BFB1udQ8Bq_H4";

		static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");

		let recorder = DebuggingRecorder::new();
		let snapshotter = recorder.snapshotter();
		let subject_id = "subject-nothing-prepared";

		metrics::with_local_recorder(&recorder, || {
			let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("build a current-thread runtime");

			rt.block_on(async {
				let pool = SqlitePoolOptions::new()
					.max_connections(1)
					.connect("sqlite::memory:")
					.await
					.expect("open an in-memory sqlite database");
				MIGRATOR.run(&pool).await.expect("run the workspace migration history");

				let now = Utc::now();
				let now_str = now.to_rfc3339();

				// Presence full (level 100, weight 1.0 → shortfall 0);
				// Momentum, Mastery, and Freshness all fully drained (level
				// 0). Momentum's shortfall (0.7 * 100 = 70) beats every
				// other class's, so Momentum is dominant — the "not
				// Presence" precondition `NothingToSay` requires — while
				// the weighted aggregate (100, Presence's contribution
				// alone) sits under `StudyCalibration::THRESHOLD` (110), so
				// this subject is due right now rather than waiting.
				let levels: Vec<(u16, f64)> = vec![(1, 100.0), (2, 0.0), (3, 0.0), (4, 0.0)];
				let engagement = EngagementRepository::new(pool.clone());
				engagement
					.save(subject_id, &levels, &now_str, &now_str)
					.await
					.expect("seed a due, nothing-prepared subject");

				let ws = WebSocketFsm::new();
				let vapid = VapidIdentity::from_config(Some(VAPID_PRIVATE), Some(VAPID_PUBLIC), "mailto:test@example.com").expect("the fixed test keypair validates");
				let nudge = NudgeContext {
					clock: NudgeClock::resolve(Some("UTC")).0,
					sender: std::sync::Arc::new(Sender::new(vapid.clone(), ReqwestTransport::default())),
					vapid,
					enabled: true,
					quiet_hours_start: 22,
					quiet_hours_end: 8,
					presence_freshness: std::time::Duration::from_secs(120),
					base_url: "https://example.com".to_owned(),
				};

				let intervened = consider(&pool, &ws, &nudge, &engagement, subject_id)
					.await
					.expect("a pass over a freshly-seeded subject should not error");
				assert!(
					!intervened,
					"no push subscription exists to consent to it, so this pass should be Suppressed rather than Sent"
				);

				// The core assertion: provisioning happened anyway.
				let sessions = SessionRepository::new(pool.clone());
				let provisioned_id = sessions
					.first_prepared(subject_id)
					.await
					.expect("read back the provisioned session")
					.expect("consider should have written a findable session even though admission later refused");

				let record = sessions
					.get(subject_id, &provisioned_id)
					.await
					.expect("read the provisioned record")
					.expect("the id first_prepared returned should resolve to a real row");
				assert_eq!(record.status, SessionStatus::Draft);
				assert!(record.activities.is_empty(), "RCM3/RCM5 fill activities in; RCM2 only establishes that a session exists");

				// Idempotency: a second pass — standing in for "the first
				// pass crashed between provisioning and claiming, and the
				// waker tried again" — must not create a second session.
				let second_pass = consider(&pool, &ws, &nudge, &engagement, subject_id).await.expect("a second pass should not error");
				assert!(!second_pass, "still nothing consented to receive it");
				let all_sessions = sessions.list(subject_id).await.expect("list this subject's sessions");
				assert_eq!(all_sessions.len(), 1, "provisioning must not run twice for the same subject");
				let still_the_same = sessions.first_prepared(subject_id).await.expect("read back again").expect("still findable");
				assert_eq!(
					still_the_same, provisioned_id,
					"the second pass should reuse the session the first pass wrote, not provision another"
				);
			});
		});

		let snapshot: Vec<(CompositeKey, Option<metrics::Unit>, Option<metrics::SharedString>, DebugValue)> = snapshotter.snapshot().into_vec();
		let find = |name: &str, label_value: &str| -> u64 {
			snapshot
				.iter()
				.find_map(|(key, _, _, value)| {
					let k = key.key();
					let matches = k.name() == name && k.labels().any(|l| l.value() == label_value);
					matches.then_some(match value {
						DebugValue::Counter(n) => *n,
						_ => 0,
					})
				})
				.unwrap_or(0)
		};

		assert_eq!(
			find("nudge_waker_verdicts_total", "nothing_to_say"),
			0,
			"the warn!-and-retry NothingToSay path is gone once provisioning handles it — reaching it here would mean the guard in `consider` regressed"
		);
		assert_eq!(
			find("nudge_waker_verdicts_total", "suppressed"),
			2,
			"both passes should have reached admission and been refused on NotConsented, not stalled on NothingToSay"
		);
	}
}

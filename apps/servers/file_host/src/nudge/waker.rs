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
use crate::{AppState, NudgeContext};
use activity_repo::{default_session_name, provision, recommend, total_duration_ms, ActivityRecord, ActivityRepository, DEFAULT_RECOMMENDATION_COUNT};
use chrono::{DateTime, Utc};
use engagement_repo::EngagementRepository;
use intervention::{Admissibility, Calibration, Charge, Engine, Selector, Verdict};
use push_kit::SendOutcome;
use push_repo::{PushSubscriptionRepository, Topic};
use session_repo::{LayoutMode, SessionOrigin, SessionRecord, SessionRepository, SessionStatus};
use sqlx::SqlitePool;
use std::collections::HashSet;
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
					match run_once(&db, &nudge).await {
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
/// Takes the database pool and a proven-present `&NudgeContext` — the
/// projection of `AppState` this pass actually reads — rather than
/// `&AppState` itself. `spawn` is the only caller and it borrows these once
/// per interval, not once per due subject, so the loop below passes the same
/// two references through without re-deriving them. No `&WebSocketFsm`
/// anymore: presence is a DB-backed lease now, not a WebSocket connection
/// count, so the waker has no reason to know the WS layer exists at all —
/// see `nudge::presence`.
///
/// # Errors
/// Any storage failure. Per-subject failures are logged and skipped rather than
/// aborting the pass — one bad row must not stop everyone else's.
pub async fn run_once(db: &SqlitePool, nudge: &NudgeContext) -> Result<usize, sqlx::Error> {
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
		match consider(db, nudge, &engagement, &gate.subject_id).await {
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
async fn consider(db: &SqlitePool, nudge: &NudgeContext, engagement: &EngagementRepository, subject_id: &str) -> Result<bool, sqlx::Error> {
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
		presence: presence::observe(db, subject_id, nudge.presence_lease_ttl).await,
		consented_topics,
	};

	let engine = Engine::<StudyV1, StudyCalibration, _, _>::new(constraints, StudySelector { prepared_session });
	let gate = engagement.gate(subject_id).await?;
	let last_intervened_at = gate
		.and_then(|row| row.last_intervened_at)
		.and_then(|raw| crate::nudge::clock::parse_timestamp(raw.as_str()));

	let action = match engine.evaluate(&charge, now, last_intervened_at) {
		Verdict::Intervene(action) => {
			// #284 (RCM7): if what's about to be pointed at is an un-started
			// `system` proposal, its content may be stale — refresh it here,
			// now that `evaluate` has already confirmed admission (including
			// presence). A real `chatgpt-codex-connector` finding on `#335`
			// caught an earlier version of this refresh running *before*
			// admission was checked at all: it could rewrite a proposal's
			// name/activities/duration while the subject was actively
			// viewing that exact session, only for admission to then
			// suppress the notification on `Present` anyway — mutating a
			// session out from under someone looking at it for a
			// notification that was never going to send. Gating on
			// `Verdict::Intervene` means this only ever runs immediately
			// before an intervention that will actually go out, matching
			// `first_prepared`'s own read at the top of this function to
			// whatever the engine actually decided to do with it. See
			// `refresh_stale_proposal`'s own doc comment for why this is
			// best-effort rather than fatal.
			if let Some(session_id) = action.session_id() {
				refresh_stale_proposal(db, &sessions, subject_id, session_id, now).await;
			}
			action
		}
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
			// RCM3 (#280), RCM4 (#281), and RCM5 (#282) are what actually
			// fill it in — see `materialize_provisioned_session`'s own
			// doc comment for what it writes and why.
			//
			// Crash safety without extra bookkeeping: the write below is
			// a `Scheduled` row `SessionRepository::first_prepared` will
			// find on any later pass (RCM5's own choice — see
			// `materialize_provisioned_session`'s doc comment for why
			// `Scheduled` is now safe where RCM2 originally chose
			// `Draft`). A crash between this write and `claim` below
			// costs this pass's notification, not a second session — the
			// next pass reads the row this one already wrote and reaches
			// `Verdict::Intervene` directly, skipping this arm entirely.
			//
			// Race safety is a separate concern from crash safety, and
			// needs its own guard: `prepared_session` above was read at
			// the *top* of `consider`, and a concurrent waker pass or the
			// subject's own `POST /sessions` call can create a prepared
			// session in the window between that read and this write.
			// Because the provisioned row's id is always freshly
			// generated, an unconditional `upsert` would not collide with
			// whatever won that race — it would just add a second, blank
			// draft beside it. `provision_or_refresh` closes the window
			// atomically (one statement, targeting #284's own partial
			// unique index — `origin = 'system' AND started_at IS NULL`,
			// not the three statuses `first_prepared` treats as prepared)
			// rather than trusting the read that already happened; see its
			// own doc comment for the mechanism and the #313 review that
			// caught the original race. #284 (RCM7) is also what turns a
			// race-losing write into a *refresh* rather than a no-op: the
			// losing side's freshly recommended content still wins, in
			// place, over whatever stale proposal the subject has been
			// ignoring — see `docs/study-nudge.md`'s "Never stack
			// proposals" section. The `first_prepared` re-read after it is
			// what makes this correct either way: whichever side of the
			// race actually landed (or whichever call's content the
			// refresh applied) is what gets used, not necessarily the row
			// built here.
			let catalogue = match ActivityRepository::new(db.clone()).list().await {
				Ok(catalogue) => catalogue,
				Err(err) => {
					error!(subject = %subject_id, error = %err, "could not read the activity catalogue; skipping this subject rather than provisioning an empty session");
					crate::metrics::waker::record_verdict("storage_error", "n/a");
					// A real Codex review finding on server#322 (P2): unlike
					// `first_prepared`'s per-subject read a few lines above,
					// a catalogue read is global -- if it is failing, it
					// fails identically for every subject reaching this arm
					// in the same pass. Leaving `eligible_at` where it was
					// (as the other `storage_error` branches in this
					// function do, for a genuinely per-subject failure)
					// would let `EngagementRepository::due`'s oldest-32
					// query keep re-selecting exactly these subjects every
					// subsequent pass, crowding the batch and starving
					// subjects who need no catalogue read at all -- one
					// with an existing prepared session, say. Advance the
					// gate instead of leaving it stuck.
					let retry = now + chrono::Duration::hours(1);
					let (levels, as_of) = charge.to_storage();
					engagement.save(subject_id, &levels, &as_of.to_rfc3339(), &retry.to_rfc3339()).await?;
					return Ok(false);
				}
			};
			let provisioned = materialize_provisioned_session(new_id(), subject_id, &catalogue, now);
			if provisioned.activities.is_empty() {
				// A real Codex review finding on server#322 (P2): every one
				// of `recommend()`'s picks was dropped by `provision()` --
				// a `NULL` floor on every eligible candidate, or a cap so
				// tight nothing fits. Persisting this anyway would write a
				// `Scheduled` row `first_prepared`/`provision_if_absent`
				// then treats as "already prepared" forever: nothing in
				// this codebase re-provisions once a prepared session
				// exists, so the subject would be stuck pointing at a
				// permanently empty session even after the catalogue is
				// fixed. Retry later instead of writing a session with
				// nothing in it -- a longer backoff than the storage-error
				// case above, since a catalogue that produces nothing
				// timeable needs a content fix, not a quick retry.
				warn!(subject = %subject_id, "recommend()+provision() produced no timeable activities; not persisting an empty session");
				crate::metrics::waker::record_verdict("nothing_to_provision", "n/a");
				let retry = now + chrono::Duration::hours(6);
				let (levels, as_of) = charge.to_storage();
				engagement.save(subject_id, &levels, &as_of.to_rfc3339(), &retry.to_rfc3339()).await?;
				return Ok(false);
			}
			if let Err(err) = sessions.provision_or_refresh(subject_id, &provisioned).await {
				error!(subject = %subject_id, error = %err, "could not write a provisioned session; skipping this subject rather than notifying about one that doesn't exist");
				crate::metrics::waker::record_verdict("storage_error", "n/a");
				return Ok(false);
			}
			let session_id = match sessions.first_prepared(subject_id).await {
				Ok(Some(id)) => id,
				Ok(None) => {
					// Unreachable in practice: `provision_or_refresh` just
					// proved a prepared session exists for this subject,
					// either the one built above or a concurrent writer's.
					// Guarded rather than trusted, per this codebase's
					// refuse-rather-than-guess convention.
					error!(subject = %subject_id, "provisioned a session but none is findable immediately after; skipping this pass");
					crate::metrics::waker::record_verdict("storage_error", "n/a");
					return Ok(false);
				}
				Err(err) => {
					error!(subject = %subject_id, error = %err, "could not re-read the provisioned session; skipping this subject");
					crate::metrics::waker::record_verdict("storage_error", "n/a");
					return Ok(false);
				}
			};

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
			// provisioning rather than an existing session. Presence is
			// re-checked against `reselected`'s own context here too,
			// automatically: `engine`'s constraints hold one fixed
			// `PresenceLeases` snapshot fetched once at the top of
			// `consider`, and `admit` asks it about whatever action it is
			// given, so this call site needed no changes of its own to
			// become context-aware alongside the one above.
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

/// #284 (RCM7): refresh `prepared`'s content in place if it is an un-started
/// `system` proposal — same id, freshly recommended contents — so an
/// ordinary pass over someone who keeps ignoring the same proposal does not
/// keep pointing at whatever `recommend()` produced the day it was first
/// written.
///
/// **Called only once `consider` has already decided to intervene using
/// `prepared`**, from the `Verdict::Intervene` arm of `engine.evaluate` — not
/// from the moment `first_prepared` resolves. A real `chatgpt-codex-
/// connector` finding on `#335` caught an earlier version that ran this
/// before admission was ever checked: it could rewrite a proposal's content
/// while the subject held a fresh presence lease on that exact session —
/// actively viewing it — only for `evaluate`'s own `admit` call to then
/// suppress the notification on `Present` anyway, mutating a session out
/// from under someone looking at it for nothing. `evaluate` already calls
/// `Admissibility::admit` internally before ever returning `Intervene`, so
/// gating this call on that verdict is sufficient — no separate presence
/// check is needed here.
///
/// **Best-effort, not fatal.** Unlike `Verdict::NothingToSay`'s own
/// provisioning (where a catalogue-read failure or an empty candidate set
/// means genuinely nothing to offer this pass, so the whole pass backs off),
/// a failure here still has a session to fall back to — the existing,
/// unrefreshed proposal `prepared` already names. So every failure path
/// below just logs and leaves that proposal exactly as it was, rather than
/// failing `consider` or advancing any gate: the person still gets notified
/// about *something* real, only not this pass's freshest recommendation.
///
/// **Reads before it writes.** `prepared` is the id `evaluate` is about to
/// act on — which, because `first_prepared`'s own priority order is
/// `paused`, then `scheduled`, then `draft`, is not always the `system`/
/// un-started proposal even when one exists (a person's own `paused`
/// session outranks it). A single indexed lookup by id decides whether this
/// is the row to refresh at all; nothing here assumes `prepared` already is
/// it.
///
/// **`created_at == updated_at` is the second gate, and it is load-bearing —
/// a real `chatgpt-codex-connector` finding on `#335`.** `origin = 'system'`
/// alone is not proof nobody has touched this row: `docs/study-nudge.md`'s
/// own "Origin" section already documents that, before `paulgsc/some-ui`
/// PRO1 ships, the live client never sends an `origin` field on an edit, so
/// `update_session` never promotes it — a person can rename this exact
/// proposal, or replace its activities, and it still reads as `system` and
/// `started_at IS NULL` afterwards. Before this refresh existed, that gap's
/// only cost was a `Momentum` misclassification if the row was later
/// abandoned; refreshing turns the same gap into silent data loss, since it
/// would overwrite the person's own edit with a fresh recommendation.
/// `update_session` advances `updated_at` unconditionally on every write
/// (`upsert`'s own doc comment), so a person's edit — even an unpromoted one
/// — always moves it away from `created_at`. `provision_or_refresh`'s own
/// `ON CONFLICT` branch deliberately never touches `updated_at`, which is
/// what makes `created_at == updated_at` survive any number of machine
/// refreshes and still mean exactly one thing: nothing but the waker has
/// ever written to this row. See that method's own doc comment
/// ("What refreshing touches") for the SQL side of this argument.
async fn refresh_stale_proposal(db: &SqlitePool, sessions: &SessionRepository, subject_id: &str, prepared: &str, now: DateTime<Utc>) {
	let record = match sessions.get(subject_id, prepared).await {
		Ok(Some(record)) => record,
		Ok(None) => return,
		Err(err) => {
			error!(subject = %subject_id, error = %err, "could not read the prepared session; leaving it as-is rather than guessing whether it needs refreshing");
			return;
		}
	};
	if !(matches!(record.origin, SessionOrigin::System) && record.started_at.is_none()) {
		return;
	}
	if record.created_at != record.updated_at {
		// A person edited this proposal without it ever being promoted to
		// `user` origin (the documented pre-PRO1 gap) -- refreshing would
		// silently overwrite their own edit. Leave it alone; this is their
		// session now in every way that matters here, whatever the column
		// still says.
		return;
	}

	let catalogue = match ActivityRepository::new(db.clone()).list().await {
		Ok(catalogue) => catalogue,
		Err(err) => {
			error!(subject = %subject_id, error = %err, "could not read the activity catalogue; leaving the existing proposal stale rather than failing this pass");
			return;
		}
	};
	let refreshed = materialize_provisioned_session(new_id(), subject_id, &catalogue, now);
	if refreshed.activities.is_empty() {
		// Same trap #322 (P2) named for the original provisioning path: a
		// catalogue with nothing currently timeable must not clobber a real
		// proposal with an empty one. Leaving the stale row in place is
		// strictly better than that — it is still a real, playable session.
		warn!(subject = %subject_id, "recommend()+provision() currently produces nothing timeable; leaving the existing proposal as-is");
		return;
	}
	if let Err(err) = sessions.provision_or_refresh(subject_id, &refreshed).await {
		error!(subject = %subject_id, error = %err, "could not refresh the existing proposal; leaving it as-is");
	}
}

/// A provisioned session, fully materialised — #282 (RCM5), the story that
/// closes out what #279 (RCM2) deliberately left empty. `recommend()` (#280,
/// RCM3) picks the activities; `provision()` (#281, RCM4) sets each one's
/// floor duration; this function's own naming and duration logic
/// (`activity_repo::naming`, `activity_repo::provisioning::total_duration_ms`)
/// fill in everything else #282's issue table asks for.
///
/// **`status: Scheduled`, not `Draft`.** A deliberate change from what this
/// function wrote before RCM3/RCM4/RCM5 existed to fill a session in. RCM2's
/// original `Draft` choice defended itself on crash-safety grounds specific
/// to a session with nothing in it yet — see `handlers::db::session::
/// create_session`'s identical reasoning for why a session born anything but
/// `Draft` could be offered before it was finished being composed. That
/// concern does not apply here: `recommend()`, `provision()`, and this
/// function's naming/duration all run synchronously inside one call, before
/// the row is ever written, so there is no partially-composed state a crash
/// could expose between "written" and "filled in." `Scheduled` is what
/// #282's own table asks for, so `first_prepared` and `StudySelector` treat
/// a provisioned session exactly like one a person scheduled themselves.
///
/// **`scenes: Vec::new()`.** #282's own "hard" decision — see
/// `docs/study-nudge.md`'s "Materialising a session: the `scenes` decision"
/// section for the full argument. In short: #272 already decided this
/// server cannot compute a scene's `props` (the closure stays client-side),
/// so the only honest options are an empty `scenes` array or a shape that
/// merely *looks* complete. This writes `[]`. `total_duration_ms` is
/// computed directly from the provisioned activities
/// (`activity_repo::total_duration_ms`) rather than derived from `scenes` —
/// `session_repo::total_duration_of(&[])` would read this session as
/// zero-length, which is wrong; it has real, timed blocks, only not yet
/// materialised into playable scenes. See `total_duration_ms`'s own doc
/// comment for why the two numbers are provably equal for the `basic`
/// layout this always produces.
///
/// **`layout: None`.** SQL `NULL`, not the JSON string `"null"` —
/// `deserialize_explicit_null`/the double-`Option` treatment on
/// `SessionRecord::layout` exists for exactly this distinction. Absent means
/// the client's naive default layout applies, which is right for a proposal
/// the server has no business asserting a layout for.
///
/// `origin: SessionOrigin::System` (`#283`, RCM6) is the real field, landed
/// after RCM5 kept this diff readable on its own. Before it existed, `name`
/// was the only signal that this was proposed rather than authored; now
/// `Momentum` reads `origin` directly (see `session_repo::model::
/// session_abandonment_is_real`) rather than inferring intent from a string.
///
/// `pub` rather than crate-private: `dump-provisioned-session` (the bin
/// this crate ships next to `dump-proposed-session`) calls this directly so
/// the fixture it emits is the exact same code path `consider` runs in
/// production, not a re-implementation of it — the same reason
/// `dump_proposed_session.rs` calls `activity_repo::provision` rather than
/// re-deriving its output.
pub fn materialize_provisioned_session(id: String, subject_id: &str, catalogue: &[ActivityRecord], now: DateTime<Utc>) -> SessionRecord {
	let stamp = now.to_rfc3339();
	let picks = recommend(subject_id, DEFAULT_RECOMMENDATION_COUNT, catalogue, &[], None, now);
	let provisioned = provision(&picks);

	// `name` is built from whichever picks actually survived `provision` —
	// a `min_duration_ms: NULL` activity, or one that would have pushed the
	// session over `CLIENT_MAX_TOTAL_DURATION_MS`, has no business in the
	// name of a session it is not in. `provision` preserves `recommend`'s
	// order (see its own `provision_preserves_the_recommenders_order`
	// test), so filtering `picks` down to the surviving ids, in `picks`'
	// own order, reconstructs exactly what `provisioned` contains.
	let provisioned_ids: HashSet<&str> = provisioned.iter().map(|activity| activity.activity_id.as_str()).collect();
	let named: Vec<ActivityRecord> = picks.into_iter().filter(|activity| provisioned_ids.contains(activity.id.as_str())).collect();

	SessionRecord {
		id,
		name: default_session_name(&named),
		status: SessionStatus::Scheduled,
		origin: SessionOrigin::System,
		activities: provisioned
			.iter()
			.map(|activity| serde_json::to_value(activity).unwrap_or(serde_json::Value::Null))
			.collect(),
		scenes: Vec::new(),
		layout_mode: LayoutMode::Basic,
		layout: None,
		total_duration_ms: total_duration_ms(&provisioned),
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
	use crate::nudge::presence::PresenceLeases;
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
			presence: PresenceLeases::empty(std::time::Duration::from_secs(75)),
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
							origin: SessionOrigin::User,
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

				let vapid = VapidIdentity::from_config(Some(VAPID_PRIVATE), Some(VAPID_PUBLIC), "mailto:test@example.com").expect("the fixed test keypair validates");
				let nudge = NudgeContext {
					clock: NudgeClock::resolve(Some("UTC")).0,
					sender: std::sync::Arc::new(Sender::new(vapid.clone(), ReqwestTransport::default())),
					vapid,
					enabled: true,
					quiet_hours_start: 22,
					quiet_hours_end: 8,
					presence_lease_ttl: std::time::Duration::from_secs(75),
					base_url: "https://example.com".to_owned(),
				};

				run_once(&pool, &nudge)
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
	/// a six-hour retry. Extended by #282 (RCM5) to assert the session it
	/// gets is fully materialised — real activities, a real name, a real
	/// duration — not just the placeholder row #279 originally wrote.
	///
	/// Also #282's own `SessionProvisioned` acceptance criterion, satisfied
	/// without any new production code: the `charge.apply::<StudyCalibration>
	/// (&StudySignal::SessionProvisioned { .. })` call a few lines below this
	/// arm's provisioning step was already unconditional as of #279, so it
	/// fires for a materialised session exactly as it did for the placeholder
	/// — `study_domain::signal`'s own `provisioning_is_an_opportunity_not_engagement`
	/// test is what pins the signal's shape (`Freshness`, delta `0.0`); this
	/// test is what pins that the call site is still reached.
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

				let vapid = VapidIdentity::from_config(Some(VAPID_PRIVATE), Some(VAPID_PUBLIC), "mailto:test@example.com").expect("the fixed test keypair validates");
				let nudge = NudgeContext {
					clock: NudgeClock::resolve(Some("UTC")).0,
					sender: std::sync::Arc::new(Sender::new(vapid.clone(), ReqwestTransport::default())),
					vapid,
					enabled: true,
					quiet_hours_start: 22,
					quiet_hours_end: 8,
					presence_lease_ttl: std::time::Duration::from_secs(75),
					base_url: "https://example.com".to_owned(),
				};

				let intervened = consider(&pool, &nudge, &engagement, subject_id)
					.await
					.expect("a pass over a freshly-seeded subject should not error");
				assert!(
					!intervened,
					"no push subscription exists to consent to it, so this pass should be Suppressed rather than Sent"
				);

				// The core assertion: provisioning happened, and it is the
				// real thing #280/#281/#282 build together, not just a
				// placeholder row. Recomputed independently here — same
				// catalogue, same subject, a fresh `Utc::now()` close
				// enough behind `consider`'s own call that the day-based
				// shuffle in `recommend` cannot have moved — rather than
				// hard-coded, so this assertion does not silently go
				// stale if the seeded catalogue or
				// `DEFAULT_RECOMMENDATION_COUNT` ever changes.
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
				assert_eq!(
					record.status,
					SessionStatus::Scheduled,
					"RCM5's own choice — see materialize_provisioned_session's doc comment for why Draft's original crash-safety concern no longer applies"
				);
				assert!(
					record.scenes.is_empty(),
					"RCM5's own scenes decision — see docs/study-nudge.md's 'Materialising a session' section"
				);

				let catalogue = ActivityRepository::new(pool.clone()).list().await.unwrap();
				let expected_picks = recommend(subject_id, DEFAULT_RECOMMENDATION_COUNT, &catalogue, &[], None, Utc::now());
				let expected_provisioned = provision(&expected_picks);
				let expected_ids: HashSet<&str> = expected_provisioned.iter().map(|activity| activity.activity_id.as_str()).collect();
				let expected_named: Vec<ActivityRecord> = expected_picks.into_iter().filter(|activity| expected_ids.contains(activity.id.as_str())).collect();

				assert!(!record.activities.is_empty(), "the seeded catalogue always has at least one activity with a real duration");
				assert_eq!(
					record.name,
					default_session_name(&expected_named),
					"name must match what the same recommend()+provision() run would name it"
				);
				assert_eq!(
					record.total_duration_ms,
					total_duration_ms(&expected_provisioned),
					"total_duration_ms must match the same run's summed floors"
				);
				assert!(record.total_duration_ms > 0, "a real provisioned session must have a nonzero duration");

				// Idempotency: a second pass — standing in for "the first
				// pass crashed between provisioning and claiming, and the
				// waker tried again" — must not create a second session.
				let second_pass = consider(&pool, &nudge, &engagement, subject_id).await.expect("a second pass should not error");
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

	/// #284 (RCM7)'s named "interaction between two stories" — RCM6's
	/// `origin`/`started_at` and RCM7's own "at most one" rule — run through
	/// `consider` end to end, not just `SessionRepository` directly. A
	/// provisioned session the subject actually **started** stops matching
	/// `first_prepared`'s three "prepared" statuses (`paused`, `scheduled`,
	/// `draft` — `active` is deliberately not one of them) the moment it goes
	/// `active`, so a subject who is somehow re-selected as due while still
	/// mid-session reaches `Verdict::NothingToSay` a second time even though
	/// their first proposal is far from abandoned. This pins that the second
	/// pass does the right thing anyway: `provision_or_refresh`'s partial
	/// index no longer covers the started row (`started_at` is no longer
	/// `NULL`), so a second, independent proposal is provisioned rather than
	/// the started session being silently refreshed out from under whoever
	/// is mid-session on it — the exact corruption #284's own issue text
	/// calls out as the one thing this rule must never do.
	#[test]
	fn a_started_provisioned_session_does_not_block_or_get_clobbered_by_a_second_provisioning_pass() {
		use push_kit::{ReqwestTransport, Sender, VapidIdentity};
		use session_repo::SessionRecord;
		use sqlx::sqlite::SqlitePoolOptions;

		const VAPID_PRIVATE: &str = "IQ9Ur0ykXoHS9gzfYX0aBjy9lvdrjx_PFUXmie9YRcY";
		const VAPID_PUBLIC: &str = "BMjQIp55pdbU8pfCBKyXcZjlmER_mXt5LqNrN1hrXbdBS5EnhIbMu3Au-RV53iIpztzNXkGI56BFB1udQ8Bq_H4";

		static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");

		let subject_id = "subject-mid-session";

		let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
		rt.block_on(async {
			let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
			MIGRATOR.run(&pool).await.unwrap();

			let now = Utc::now();
			let now_str = now.to_rfc3339();
			let levels: Vec<(u16, f64)> = vec![(1, 100.0), (2, 0.0), (3, 0.0), (4, 0.0)];
			let engagement = EngagementRepository::new(pool.clone());
			engagement.save(subject_id, &levels, &now_str, &now_str).await.unwrap();

			let vapid = VapidIdentity::from_config(Some(VAPID_PRIVATE), Some(VAPID_PUBLIC), "mailto:test@example.com").unwrap();
			let nudge = NudgeContext {
				clock: NudgeClock::resolve(Some("UTC")).0,
				sender: std::sync::Arc::new(Sender::new(vapid.clone(), ReqwestTransport::default())),
				vapid,
				enabled: true,
				quiet_hours_start: 22,
				quiet_hours_end: 8,
				presence_lease_ttl: std::time::Duration::from_secs(75),
				base_url: "https://example.com".to_owned(),
			};

			// First pass: nothing prepared, provisions a real session.
			consider(&pool, &nudge, &engagement, subject_id).await.unwrap();
			let sessions = SessionRepository::new(pool.clone());
			let first_id = sessions.first_prepared(subject_id).await.unwrap().unwrap();

			// The subject actually opens it: status -> active, started_at set.
			// This is exactly what `update_session` does on a real `Start`
			// PATCH — origin stays `system`, only the lifecycle fields move.
			let mut started = sessions.get(subject_id, &first_id).await.unwrap().unwrap();
			started.status = session_repo::SessionStatus::Active;
			started.started_at = Some(now_str.clone());
			sessions.upsert(subject_id, &started).await.unwrap();

			// Re-arm the gate as if this subject drifted due again while
			// still mid-session (an independent deficit crossing threshold,
			// say) -- `due` only needs `eligible_at <= now`, and the charge
			// levels above still keep Momentum dominant over Presence.
			engagement.save(subject_id, &levels, &now_str, &now_str).await.unwrap();

			// Second pass: `first_prepared` no longer finds the started
			// session (`active` isn't a "prepared" status), so this must
			// reach `NothingToSay` again and provision independently rather
			// than erroring or silently refreshing the started row.
			consider(&pool, &nudge, &engagement, subject_id).await.unwrap();

			let all: Vec<SessionRecord> = sessions.list(subject_id).await.unwrap();
			assert_eq!(all.len(), 2, "the started session and a fresh proposal must coexist, not collapse into one");

			let started_after = all.iter().find(|s| s.id == first_id).unwrap();
			assert!(
				matches!(started_after.status, session_repo::SessionStatus::Active),
				"a started system session must never be refreshed back to Scheduled"
			);
			assert_eq!(
				started_after.started_at.as_deref(),
				Some(now_str.as_str()),
				"a started system session's started_at must never be touched by a later provisioning pass"
			);

			let second_prepared = sessions.first_prepared(subject_id).await.unwrap().unwrap();
			assert_ne!(
				second_prepared, first_id,
				"the second pass must provision a genuinely new session, not point back at the started one"
			);
		});
	}

	/// A real `chatgpt-codex-connector` finding on `#335`: once an ignored
	/// proposal exists, `first_prepared` resolves to it on every later pass,
	/// so `Verdict::NothingToSay` — and therefore `provision_or_refresh` —
	/// is never reached again. Refreshing had to move to a path reached
	/// whenever such a proposal already exists, not only the one reached
	/// when nothing is prepared at all. This pins that the *ordinary* path
	/// (a `paused`/`scheduled` proposal already found, an intervention sent)
	/// actually refreshes stale content, not just that it avoids stacking a
	/// duplicate — `id` stays put, but the content a second day's pass sees
	/// must reflect that day's catalogue, not the day the proposal was
	/// first written.
	#[test]
	fn an_ordinary_second_pass_over_an_ignored_proposal_refreshes_its_content_not_just_its_existence() {
		use push_kit::{PushSubscription, ReqwestTransport, Sender, SubscriptionKeys, VapidIdentity};
		use push_repo::{Consent, PushSubscriptionRepository, Topic};
		use sqlx::sqlite::SqlitePoolOptions;

		const VAPID_PRIVATE: &str = "IQ9Ur0ykXoHS9gzfYX0aBjy9lvdrjx_PFUXmie9YRcY";
		const VAPID_PUBLIC: &str = "BMjQIp55pdbU8pfCBKyXcZjlmER_mXt5LqNrN1hrXbdBS5EnhIbMu3Au-RV53iIpztzNXkGI56BFB1udQ8Bq_H4";

		static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");

		let subject_id = "subject-ignoring-proposal";

		let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
		rt.block_on(async {
			let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
			MIGRATOR.run(&pool).await.unwrap();

			let now = Utc::now();
			let now_str = now.to_rfc3339();
			let levels: Vec<(u16, f64)> = vec![(1, 100.0), (2, 0.0), (3, 0.0), (4, 0.0)];
			let engagement = EngagementRepository::new(pool.clone());
			engagement.save(subject_id, &levels, &now_str, &now_str).await.unwrap();

			// Consented on every topic, so the second pass's `ResumeAbandoned`
			// (Momentum's dominant-deficit action) clears `admit`'s consent
			// check and actually reaches `Verdict::Intervene` — the whole
			// point of this test is to exercise the refresh gated on that
			// verdict, not on `NotConsented`/`Suppressed` short-circuiting
			// before it. Keys are inert placeholders: `admit` only reads the
			// topic list, and the refresh under test happens before `actuate`
			// ever tries to encrypt or send anything.
			PushSubscriptionRepository::new(pool.clone())
				.upsert(
					&PushSubscription {
						endpoint: "https://push.example.com/ignoring-proposal".to_owned(),
						keys: SubscriptionKeys {
							p256dh: "p256dh".to_owned(),
							auth: "auth".to_owned(),
						},
					},
					&Consent {
						subject_id: subject_id.to_owned(),
						topics: Topic::ALL.to_vec(),
						consented_at: now_str.clone(),
					},
					&now_str,
				)
				.await
				.unwrap();

			let vapid = VapidIdentity::from_config(Some(VAPID_PRIVATE), Some(VAPID_PUBLIC), "mailto:test@example.com").unwrap();
			let nudge = NudgeContext {
				clock: NudgeClock::resolve(Some("UTC")).0,
				sender: std::sync::Arc::new(Sender::new(vapid.clone(), ReqwestTransport::default())),
				vapid,
				enabled: true,
				// Disabled outright (start == end never matches, per
				// `is_within_quiet_hours`) rather than a fixed window: this
				// test must reach `Verdict::Intervene` regardless of the
				// wall-clock hour it happens to run at.
				quiet_hours_start: 0,
				quiet_hours_end: 0,
				presence_lease_ttl: std::time::Duration::from_secs(75),
				base_url: "https://example.com".to_owned(),
			};

			// First pass: nothing prepared, provisions a real proposal.
			consider(&pool, &nudge, &engagement, subject_id).await.unwrap();
			let sessions = SessionRepository::new(pool.clone());
			let first_id = sessions.first_prepared(subject_id).await.unwrap().unwrap();
			let first_record = sessions.get(subject_id, &first_id).await.unwrap().unwrap();
			let first_duration = first_record.total_duration_ms;
			assert!(first_duration > 0);

			// The catalogue changes before the subject is reconsidered — every
			// activity's floor duration grows, so any refreshed proposal's
			// total must strictly increase, regardless of which activities
			// `recommend()` happens to pick.
			sqlx::query!("UPDATE activities SET min_duration_ms = min_duration_ms * 10").execute(&pool).await.unwrap();

			// The subject ignored it and is due again (re-armed the same way
			// a real REFRACTORY-later pass would find them); Momentum stays
			// dominant. `first_prepared` now finds the existing proposal, so
			// this pass takes the *ordinary* Intervene path, not NothingToSay.
			// The refresh under test happens inside that arm, before the
			// claim/actuate steps that follow it — this call's own `bool`
			// return isn't asserted, since the placeholder subscription keys
			// above are not valid EC public keys and `Sender::prepare` fails
			// encryption locally (no network involved), the same way any
			// other real encryption failure would; what matters here is that
			// the refresh already ran by that point regardless.
			engagement.save(subject_id, &levels, &now_str, &now_str).await.unwrap();
			consider(&pool, &nudge, &engagement, subject_id).await.unwrap();

			let all = sessions.list(subject_id).await.unwrap();
			assert_eq!(all.len(), 1, "an ordinary ignored-proposal pass must refresh in place, not add a second session");

			let second_record = sessions.get(subject_id, &first_id).await.unwrap().unwrap();
			assert_eq!(second_record.id, first_id, "the id must stay stable across the refresh");
			assert!(
				second_record.total_duration_ms > first_duration,
				"the content must reflect the catalogue at refresh time ({}), not the day the proposal was first written ({first_duration})",
				second_record.total_duration_ms
			);
		});
	}

	/// A real `chatgpt-codex-connector` finding on `#335`: an earlier version
	/// of the refresh ran before `evaluate` ever checked admission, so it
	/// could rewrite a proposal's content while the subject held a fresh
	/// presence lease on that exact session — actively viewing it — only for
	/// `evaluate` to then suppress the notification on `Present` anyway.
	/// This pins the fix directly: with a fresh lease in place, the pass
	/// must be `Suppressed::Present` (not silently something else) *and*
	/// the proposal's content must be completely untouched, proving the
	/// refresh call gated on `Verdict::Intervene` correctly never runs.
	#[test]
	fn a_fresh_presence_lease_on_the_proposal_suppresses_admission_and_leaves_its_content_untouched() {
		use metrics_util::debugging::{DebugValue, DebuggingRecorder};
		use metrics_util::CompositeKey;
		use presence_repo::PresenceLeaseRepository;
		use push_kit::{PushSubscription, ReqwestTransport, Sender, SubscriptionKeys, VapidIdentity};
		use push_repo::{Consent, PushSubscriptionRepository, Topic};
		use sqlx::sqlite::SqlitePoolOptions;

		const VAPID_PRIVATE: &str = "IQ9Ur0ykXoHS9gzfYX0aBjy9lvdrjx_PFUXmie9YRcY";
		const VAPID_PUBLIC: &str = "BMjQIp55pdbU8pfCBKyXcZjlmER_mXt5LqNrN1hrXbdBS5EnhIbMu3Au-RV53iIpztzNXkGI56BFB1udQ8Bq_H4";

		static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");

		let subject_id = "subject-viewing-proposal";
		let recorder = DebuggingRecorder::new();
		let snapshotter = recorder.snapshotter();

		metrics::with_local_recorder(&recorder, || {
			let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
			rt.block_on(async {
				let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
				MIGRATOR.run(&pool).await.unwrap();

				let now = Utc::now();
				let now_str = now.to_rfc3339();
				let levels: Vec<(u16, f64)> = vec![(1, 100.0), (2, 0.0), (3, 0.0), (4, 0.0)];
				let engagement = EngagementRepository::new(pool.clone());
				engagement.save(subject_id, &levels, &now_str, &now_str).await.unwrap();

				PushSubscriptionRepository::new(pool.clone())
					.upsert(
						&PushSubscription {
							endpoint: "https://push.example.com/viewing-proposal".to_owned(),
							keys: SubscriptionKeys {
								p256dh: "p256dh".to_owned(),
								auth: "auth".to_owned(),
							},
						},
						&Consent {
							subject_id: subject_id.to_owned(),
							topics: Topic::ALL.to_vec(),
							consented_at: now_str.clone(),
						},
						&now_str,
					)
					.await
					.unwrap();

				let vapid = VapidIdentity::from_config(Some(VAPID_PRIVATE), Some(VAPID_PUBLIC), "mailto:test@example.com").unwrap();
				let nudge = NudgeContext {
					clock: NudgeClock::resolve(Some("UTC")).0,
					sender: std::sync::Arc::new(Sender::new(vapid.clone(), ReqwestTransport::default())),
					vapid,
					enabled: true,
					quiet_hours_start: 0,
					quiet_hours_end: 0,
					presence_lease_ttl: std::time::Duration::from_secs(75),
					base_url: "https://example.com".to_owned(),
				};

				// First pass: provisions the proposal.
				consider(&pool, &nudge, &engagement, subject_id).await.unwrap();
				let sessions = SessionRepository::new(pool.clone());
				let first_id = sessions.first_prepared(subject_id).await.unwrap().unwrap();
				let first_record = sessions.get(subject_id, &first_id).await.unwrap().unwrap();

				// The catalogue changes, exactly as the sibling test does --
				// if the refresh incorrectly ran anyway, this is what would
				// prove it by changing `total_duration_ms`.
				sqlx::query!("UPDATE activities SET min_duration_ms = min_duration_ms * 10").execute(&pool).await.unwrap();

				// The subject is actively looking at exactly this proposal
				// right now -- a fresh presence lease on its own id, the same
				// `context_key` `StudyAction::session_id()` would report.
				PresenceLeaseRepository::new(pool.clone()).observe(subject_id, &first_id, &now_str).await.unwrap();

				engagement.save(subject_id, &levels, &now_str, &now_str).await.unwrap();
				let intervened = consider(&pool, &nudge, &engagement, subject_id).await.unwrap();
				assert!(!intervened, "a subject actively viewing the proposal must not be notified about it");

				let all = sessions.list(subject_id).await.unwrap();
				assert_eq!(all.len(), 1, "presence suppression must not itself cause a second session to appear");

				let second_record = sessions.get(subject_id, &first_id).await.unwrap().unwrap();
				assert_eq!(
					second_record.name, first_record.name,
					"content must be completely untouched while the proposal is being viewed"
				);
				assert_eq!(
					second_record.total_duration_ms, first_record.total_duration_ms,
					"a fresh presence lease must prevent the refresh from running at all, not merely prevent the notification"
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
			find("nudge_waker_verdicts_total", "present"),
			1,
			"the second pass must be suppressed specifically on Present, not some other reason that would also leave content untouched"
		);
	}

	/// A real `chatgpt-codex-connector` finding on `#335`, P1: before PRO1
	/// (`paulgsc/some-ui#1052`) ships, the live client never sends an
	/// `origin` field on an edit, so a person renaming or re-composing this
	/// exact proposal through today's client leaves it reading as `origin =
	/// 'system' AND started_at IS NULL` — indistinguishable from an
	/// untouched one by those two columns alone. Before this story, that gap
	/// only cost a `Momentum` misclassification if the row was later
	/// abandoned; a refresh mechanism turns the same gap into silent data
	/// loss, since it would overwrite the person's own edit with a fresh
	/// recommendation. This pins the fix: `created_at != updated_at` (which
	/// a real edit always produces, since `update_session` advances
	/// `updated_at` unconditionally even without an `origin` field) must
	/// stop the refresh outright, leaving the person's edited content
	/// completely untouched.
	#[test]
	fn an_edited_but_unpromoted_proposal_survives_a_refresh_pass_untouched() {
		use push_kit::{PushSubscription, ReqwestTransport, Sender, SubscriptionKeys, VapidIdentity};
		use push_repo::{Consent, PushSubscriptionRepository, Topic};
		use session_repo::SessionRecord;
		use sqlx::sqlite::SqlitePoolOptions;

		const VAPID_PRIVATE: &str = "IQ9Ur0ykXoHS9gzfYX0aBjy9lvdrjx_PFUXmie9YRcY";
		const VAPID_PUBLIC: &str = "BMjQIp55pdbU8pfCBKyXcZjlmER_mXt5LqNrN1hrXbdBS5EnhIbMu3Au-RV53iIpztzNXkGI56BFB1udQ8Bq_H4";

		static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");

		let subject_id = "subject-edited-proposal";

		let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
		rt.block_on(async {
			let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
			MIGRATOR.run(&pool).await.unwrap();

			let now = Utc::now();
			let now_str = now.to_rfc3339();
			let levels: Vec<(u16, f64)> = vec![(1, 100.0), (2, 0.0), (3, 0.0), (4, 0.0)];
			let engagement = EngagementRepository::new(pool.clone());
			engagement.save(subject_id, &levels, &now_str, &now_str).await.unwrap();

			PushSubscriptionRepository::new(pool.clone())
				.upsert(
					&PushSubscription {
						endpoint: "https://push.example.com/edited-proposal".to_owned(),
						keys: SubscriptionKeys {
							p256dh: "p256dh".to_owned(),
							auth: "auth".to_owned(),
						},
					},
					&Consent {
						subject_id: subject_id.to_owned(),
						topics: Topic::ALL.to_vec(),
						consented_at: now_str.clone(),
					},
					&now_str,
				)
				.await
				.unwrap();

			let vapid = VapidIdentity::from_config(Some(VAPID_PRIVATE), Some(VAPID_PUBLIC), "mailto:test@example.com").unwrap();
			let nudge = NudgeContext {
				clock: NudgeClock::resolve(Some("UTC")).0,
				sender: std::sync::Arc::new(Sender::new(vapid.clone(), ReqwestTransport::default())),
				vapid,
				enabled: true,
				quiet_hours_start: 0,
				quiet_hours_end: 0,
				presence_lease_ttl: std::time::Duration::from_secs(75),
				base_url: "https://example.com".to_owned(),
			};

			// First pass: provisions the proposal.
			consider(&pool, &nudge, &engagement, subject_id).await.unwrap();
			let sessions = SessionRepository::new(pool.clone());
			let first_id = sessions.first_prepared(subject_id).await.unwrap().unwrap();

			// The person renames it (and only renames it) through today's
			// client -- an `UpdateSession` PATCH with no `origin` field,
			// exactly as `update_session` receives one before PRO1 ships.
			// `origin` stays `system` and `started_at` stays `None`; only
			// `updated_at` moves, since `update_session` advances it on
			// every write regardless of which fields changed.
			let mut edited = sessions.get(subject_id, &first_id).await.unwrap().unwrap();
			edited.name = "My own renamed session".to_owned();
			edited.updated_at = (now + Duration::minutes(1)).to_rfc3339();
			sessions.upsert(subject_id, &edited).await.unwrap();
			assert!(
				matches!(edited.origin, session_repo::SessionOrigin::System),
				"sanity: the live client does not promote origin on this PATCH"
			);

			// The catalogue changes, exactly as the sibling tests do -- if
			// the refresh incorrectly ran anyway, this is what would prove
			// it by changing total_duration_ms and clobbering the rename.
			sqlx::query!("UPDATE activities SET min_duration_ms = min_duration_ms * 10").execute(&pool).await.unwrap();

			engagement.save(subject_id, &levels, &now_str, &now_str).await.unwrap();
			consider(&pool, &nudge, &engagement, subject_id).await.unwrap();

			let all: Vec<SessionRecord> = sessions.list(subject_id).await.unwrap();
			assert_eq!(all.len(), 1, "an edited-but-unpromoted proposal must not be duplicated either");

			let after = sessions.get(subject_id, &first_id).await.unwrap().unwrap();
			assert_eq!(
				after.name, "My own renamed session",
				"the person's own edit must survive a refresh pass completely untouched"
			);
			assert_eq!(
				after.total_duration_ms, edited.total_duration_ms,
				"an unpromoted-but-edited proposal's duration must not be silently recomputed either"
			);
		});
	}

	/// A real Codex review finding on `server#322` (P2): if `recommend()` +
	/// `provision()` produce nothing timeable — every eligible candidate has
	/// a `NULL` `min_duration_ms` here — persisting an empty `Scheduled`
	/// session would be a permanent trap. `first_prepared`/`provision_or_refresh`
	/// would treat it as "already prepared" forever, so nothing in this
	/// codebase would ever provision this subject again, even after the
	/// catalogue is fixed. Confirms both halves of the fix: nothing is
	/// written, and the gate is pushed out rather than left where `due`
	/// would immediately re-select this subject.
	#[test]
	fn an_empty_provisioned_session_is_never_persisted_and_the_gate_is_pushed_out() {
		use push_kit::{ReqwestTransport, Sender, VapidIdentity};
		use sqlx::sqlite::SqlitePoolOptions;

		const VAPID_PRIVATE: &str = "IQ9Ur0ykXoHS9gzfYX0aBjy9lvdrjx_PFUXmie9YRcY";
		const VAPID_PUBLIC: &str = "BMjQIp55pdbU8pfCBKyXcZjlmER_mXt5LqNrN1hrXbdBS5EnhIbMu3Au-RV53iIpztzNXkGI56BFB1udQ8Bq_H4";

		static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");

		let subject_id = "subject-nothing-timeable";

		let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
		rt.block_on(async {
			let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
			MIGRATOR.run(&pool).await.unwrap();

			// Every seeded activity loses its floor -- `provision()` omits
			// all of them (see `provisioning.rs`'s own NULL-floor case),
			// regardless of which two `recommend()` would otherwise pick.
			sqlx::query!("UPDATE activities SET min_duration_ms = NULL").execute(&pool).await.unwrap();

			let now = Utc::now();
			let now_str = now.to_rfc3339();
			let levels: Vec<(u16, f64)> = vec![(1, 100.0), (2, 0.0), (3, 0.0), (4, 0.0)];
			let engagement = EngagementRepository::new(pool.clone());
			engagement.save(subject_id, &levels, &now_str, &now_str).await.unwrap();

			let vapid = VapidIdentity::from_config(Some(VAPID_PRIVATE), Some(VAPID_PUBLIC), "mailto:test@example.com").unwrap();
			let nudge = NudgeContext {
				clock: NudgeClock::resolve(Some("UTC")).0,
				sender: std::sync::Arc::new(Sender::new(vapid.clone(), ReqwestTransport::default())),
				vapid,
				enabled: true,
				quiet_hours_start: 22,
				quiet_hours_end: 8,
				presence_lease_ttl: std::time::Duration::from_secs(75),
				base_url: "https://example.com".to_owned(),
			};

			let intervened = consider(&pool, &nudge, &engagement, subject_id).await.unwrap();
			assert!(!intervened, "nothing was provisioned, so there is nothing to send");

			let sessions = SessionRepository::new(pool.clone());
			assert_eq!(
				sessions.first_prepared(subject_id).await.unwrap(),
				None,
				"an empty session must never be persisted as Scheduled — that would trap the subject permanently"
			);

			let still_due = engagement.due(&now_str, BATCH).await.unwrap();
			assert!(
				!still_due.iter().any(|gate| gate.subject_id == subject_id),
				"the gate must be pushed out, not left at `now` — otherwise this subject would be re-selected and re-attempted every single pass"
			);
		});
	}

	/// A real Codex review finding on `server#322` (P2): `ActivityRepository::
	/// list()` is a *global* read, unlike `first_prepared`'s per-subject one a
	/// few lines above it in `consider`. A malformed catalogue row fails it
	/// identically for every subject reaching this arm in the same pass — if
	/// the gate were left where it was, `due`'s oldest-32 query would keep
	/// re-selecting exactly those subjects every subsequent pass, starving
	/// everyone else, including a subject with an existing prepared session
	/// who needs no catalogue read at all.
	#[test]
	fn a_catalogue_read_failure_pushes_the_gate_out_rather_than_leaving_the_subject_stuck() {
		use push_kit::{ReqwestTransport, Sender, VapidIdentity};
		use sqlx::sqlite::SqlitePoolOptions;

		const VAPID_PRIVATE: &str = "IQ9Ur0ykXoHS9gzfYX0aBjy9lvdrjx_PFUXmie9YRcY";
		const VAPID_PUBLIC: &str = "BMjQIp55pdbU8pfCBKyXcZjlmER_mXt5LqNrN1hrXbdBS5EnhIbMu3Au-RV53iIpztzNXkGI56BFB1udQ8Bq_H4";

		static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");

		let subject_id = "subject-broken-catalogue";

		let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
		rt.block_on(async {
			let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
			MIGRATOR.run(&pool).await.unwrap();

			// A row this schema did not write: `ActivityMaturity::parse`
			// refuses "bogus", so `ActivityRepository::list()` returns
			// `Err` for the whole catalogue, not just this row.
			sqlx::query!(
				r#"
				INSERT INTO activities (
				    id, name, description, icon, registry_key, layout_tree, maturity,
				    min_duration_ms, published_at, version, fields, default_config, audio
				) VALUES ('broken', 'n', 'd', 'i', 'broken-key', 'study', 'bogus', NULL, '2026-08-01T00:00:00Z', 1, '[]', '{}', NULL)
				"#
			)
			.execute(&pool)
			.await
			.unwrap();

			let now = Utc::now();
			let now_str = now.to_rfc3339();
			let levels: Vec<(u16, f64)> = vec![(1, 100.0), (2, 0.0), (3, 0.0), (4, 0.0)];
			let engagement = EngagementRepository::new(pool.clone());
			engagement.save(subject_id, &levels, &now_str, &now_str).await.unwrap();

			let vapid = VapidIdentity::from_config(Some(VAPID_PRIVATE), Some(VAPID_PUBLIC), "mailto:test@example.com").unwrap();
			let nudge = NudgeContext {
				clock: NudgeClock::resolve(Some("UTC")).0,
				sender: std::sync::Arc::new(Sender::new(vapid.clone(), ReqwestTransport::default())),
				vapid,
				enabled: true,
				quiet_hours_start: 22,
				quiet_hours_end: 8,
				presence_lease_ttl: std::time::Duration::from_secs(75),
				base_url: "https://example.com".to_owned(),
			};

			let intervened = consider(&pool, &nudge, &engagement, subject_id).await.unwrap();
			assert!(!intervened, "the catalogue is unreadable, so nothing can be provisioned or sent");

			let sessions = SessionRepository::new(pool.clone());
			assert_eq!(sessions.first_prepared(subject_id).await.unwrap(), None);

			let still_due = engagement.due(&now_str, BATCH).await.unwrap();
			assert!(
				!still_due.iter().any(|gate| gate.subject_id == subject_id),
				"a global catalogue failure must not leave this subject re-selected every pass, crowding out subjects that need no catalogue read at all"
			);
		});
	}
}

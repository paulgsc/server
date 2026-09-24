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
use crate::nudge::constraints::{StudyConstraints, Suppressed};
use crate::nudge::payload::NudgePayload;
use crate::nudge::presence;
use crate::{AppState, NudgeContext};
use activity_repo::{
	default_session_name, provision, recommend, total_duration_ms, ActivityHistory, ActivityOutcome, ActivityRecord, ActivityRepository, DEFAULT_RECOMMENDATION_COUNT,
};
use chrono::{DateTime, Utc};
use engagement_repo::{EngagementRepository, INTERVENTION_LOG_RETENTION_DAYS, RETENTION_SWEEP_LIMIT};
use intervention::{Admissibility, Calibration, Charge, Engine, Selector, Verdict};
use outcome_repo::{ActivityStats, OutcomeRepository, ACTIVITY_OUTCOME_RETENTION_DAYS};
use publication_repo::PublicationRepository;
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
	crate::metrics::waker::record_pass_deadline(nudge.pass_deadline);

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
					crate::metrics::waker::record_pass_started();
					let started = tokio::time::Instant::now();
					let result = run_once(&db, &nudge).await;
					crate::metrics::waker::record_pass_duration(started.elapsed());
					match result {
						Ok(_) => crate::metrics::waker::record_successful_pass(),
						Err(err) => {
							let class = classify_pass_error(&err);
							error!(error = %err, class, "waker pass failed");
							crate::metrics::waker::record_failed_pass(class);
						}
					}
				}
			}
		}
	});
}

/// Which `metrics::waker::PASS_ERROR_CLASSES` bucket a failed pass lands in
/// — the word LOOPS shows after "FAILING ·", so it names the thing to go and
/// fix rather than the error type.
///
/// - `schema`: the query names a table or column the database doesn't have.
///   Nearly always a local database nobody ran `sqlx migrate run` on;
///   `/ready`'s `schema` dependency names which migrations.
/// - `locked`: `SQLITE_BUSY`/`SQLITE_LOCKED` outlasted `busy_timeout`.
/// - `io`: the file itself — can't open, read-only, full, corrupt, not a
///   database. Check the volume before the code.
/// - `pool`: no connection was handed out (timed out or closed).
/// - `other`: everything else; the log line has the detail.
///
/// Classified on `SQLite`'s primary result code (the low byte of the extended
/// code sqlx reports) plus the message text for `schema`, because a missing
/// table is plain `SQLITE_ERROR` — only the message says which kind.
pub(crate) fn classify_pass_error(err: &sqlx::Error) -> &'static str {
	match err {
		sqlx::Error::Database(db) => {
			let message = db.message();
			if message.starts_with("no such table") || message.starts_with("no such column") {
				return "schema";
			}
			match db.code().and_then(|code| code.parse::<i32>().ok()).map(|code| code & 0xff) {
				// SQLITE_BUSY, SQLITE_LOCKED
				Some(5 | 6) => "locked",
				// SQLITE_READONLY, SQLITE_IOERR, SQLITE_CORRUPT, SQLITE_FULL, SQLITE_CANTOPEN, SQLITE_NOTADB
				Some(8 | 10 | 11 | 13 | 14 | 26) => "io",
				// SQLITE_SCHEMA
				Some(17) => "schema",
				_ => "other",
			}
		}
		sqlx::Error::ColumnNotFound(_) => "schema",
		sqlx::Error::Io(_) => "io",
		sqlx::Error::PoolTimedOut | sqlx::Error::PoolClosed => "pool",
		_ => "other",
	}
}

/// What one pass got through — see [`run_once`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PassReport {
	/// Subjects `consider` was run for, successfully or not.
	pub considered: usize,
	/// Of those, how many ended with a notification accepted somewhere.
	pub intervened: usize,
	/// Due subjects this pass fetched and never reached, because
	/// `NudgeContext::pass_deadline` ran out first. Untouched: their gate rows
	/// are exactly as `due` found them, so the next pass picks them up.
	pub deferred: usize,
	/// Whether the pass ran out of time — before a subject, or inside one's
	/// deliveries. Exactly the passes `nudge_waker_pass_deadline_exceeded_total`
	/// counts.
	pub deadline_exceeded: bool,
	/// Subjects a `CurriculumUpdated` was applied to this pass (#273, CAT5) —
	/// see [`announce_publications`].
	pub announced: usize,
	/// History rows this pass's retention sweep deleted — `intervention_log`
	/// (#265, SLI4) and `activity_outcome` (#286) together — see
	/// [`sweep_history`].
	pub pruned: u64,
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
/// **Bounded by `NudgeContext::pass_deadline` (#264, SLI3).** The loop below
/// is serial on purpose — a burst that would notify a whole userbase at once
/// is worth rate-limiting into — and nothing above it is a `tower` layer, so
/// before this a pass took exactly as long as its slowest push providers
/// did. The deadline is enforced at two points, and `consider` is never
/// cancelled part-way — it claims before it sends, and cancelling it would
/// put a crash-shaped event on an arbitrary side of that line instead of the
/// one the claim was designed for:
///
/// - *between* subjects: once it has passed, no further subject is started;
/// - *between* deliveries, inside `actuate`: each delivery's timeout is the
///   lesser of `NudgeContext::delivery_timeout` and the time the pass has
///   left, and a device reached after the deadline is not tried at all. A
///   subject's device list is not bounded, so without this one subject with
///   enough stalled devices could hold a pass for `devices ×
///   delivery_timeout` — a real `chatgpt-codex-connector` finding on #357.
///
/// What remains past the deadline is storage work, which `SQLite`'s
/// `busy_timeout` bounds. Subjects not reached are left exactly as `due`
/// returned them and are still due next pass — the same property `BATCH`
/// already relies on.
///
/// # Errors
/// Any storage failure. Per-subject failures are logged and skipped rather than
/// aborting the pass — one bad row must not stop everyone else's.
pub async fn run_once(db: &SqlitePool, nudge: &NudgeContext) -> Result<PassReport, sqlx::Error> {
	let deadline = tokio::time::Instant::now() + nudge.pass_deadline;
	let engagement = EngagementRepository::new(db.clone());
	let now = Utc::now();
	let due = engagement.due(&now.to_rfc3339(), BATCH).await?;
	crate::metrics::waker::record_due(due.len());

	let mut report = PassReport::default();
	debug!(count = due.len(), "subjects the arithmetic marked eligible");

	for gate in &due {
		if tokio::time::Instant::now() >= deadline {
			report.deferred = due.len() - report.considered;
			warn!(
				considered = report.considered,
				deferred = report.deferred,
				deadline_ms = nudge.pass_deadline.as_millis(),
				"waker pass reached its deadline; the remaining due subjects are left for the next pass"
			);
			break;
		}

		report.considered += 1;
		match consider(db, nudge, &engagement, &gate.subject_id, deadline).await {
			Ok(true) => report.intervened += 1,
			Ok(false) => {}
			Err(err) => {
				error!(subject = %gate.subject_id, error = %err, "could not consider a due subject");
				crate::metrics::waker::record_verdict("storage_error", "n/a");
			}
		}
	}

	// After the loop, not only at the pre-subject check above: a deadline
	// that runs out inside the last (or only) subject's deliveries truncates
	// the pass just as much, and would otherwise go uncounted — a real
	// `chatgpt-codex-connector` finding on #357.
	if report.deferred > 0 || tokio::time::Instant::now() >= deadline {
		report.deadline_exceeded = true;
		crate::metrics::waker::record_deadline_exceeded();
	} else {
		// Housekeeping after the work people are waiting on, and only if the
		// pass still has time: a pass that ran out has already left due
		// subjects for the next one, and the sweep can wait with them.
		let announced = announce_publications(db, deadline).await;
		report.announced = announced.applied;
		if announced.deadline_exceeded {
			// The fan-out ran the pass out of time: count it like any other
			// deadline, and leave the sweep for a pass that has room.
			report.deadline_exceeded = true;
			crate::metrics::waker::record_deadline_exceeded();
		} else {
			report.pruned = sweep_history(&engagement, &OutcomeRepository::new(db.clone()), now).await;
		}
	}

	Ok(report)
}

/// How many subjects one pass applies `CurriculumUpdated` to (#273, CAT5).
///
/// A publish reaches everyone it applies to — one read-modify-write of their
/// charge each — so it is bounded like everything else reachable from the
/// waker (#253): a pass applies at most this many, and the rest are picked up
/// on the next pass from `curriculum_delivery`, which is what makes the
/// fan-out resumable.
pub(crate) const ANNOUNCE_PER_PASS: i64 = 64;

/// `CurriculumUpdated`'s producer: detect newly published catalogue entries,
/// and apply the signal to each subject they apply to, at most once each
/// (#273, CAT5).
///
/// A publication is a catalogue `(id, version)` never seen before — a new row
/// or a version bump; the catalogue that existed when this landed is recorded
/// as the baseline by its migration, so deploying this announces nothing. The
/// audience is `study_domain::CURRICULUM_AUDIENCE`. Each subject's delivery is
/// claimed in `curriculum_delivery` *before* the signal is folded, so
/// re-running a publish — or resuming one a crashed pass left half done —
/// never drains anyone twice; a crash between the two costs that subject that
/// one drain instead.
///
/// Runs after the pass's subjects and inside its deadline. Returns how many
/// subjects the signal was applied to. Failures are logged, not propagated:
/// like the retention sweep, this must not fail a pass whose subjects were
/// already handled, and everything it skips is still pending next pass.
async fn announce_publications(db: &SqlitePool, deadline: tokio::time::Instant) -> Announced {
	let mut announced = fan_out_publications(db, deadline).await;
	// After the work, not only before each recipient: a claim or fold that
	// starts just inside the deadline can finish past it, and the pass has
	// overrun all the same — a real `chatgpt-codex-connector` finding on #362,
	// the fan-out's twin of the one on #357.
	if tokio::time::Instant::now() >= deadline {
		announced.deadline_exceeded = true;
	}
	announced
}

/// [`announce_publications`]' body; it checks the deadline before each
/// recipient, and its caller once more after everything it awaited.
async fn fan_out_publications(db: &SqlitePool, deadline: tokio::time::Instant) -> Announced {
	let publications = PublicationRepository::new(db.clone());
	let stamp = Utc::now().to_rfc3339();
	let mut announced = Announced::default();

	match publications.detect_activity_publications(&stamp).await {
		Ok(0) => {}
		Ok(found) => info!(found, "new catalogue material detected; announcing it"),
		Err(err) => error!(error = %err, "could not detect catalogue publications"),
	}
	let pending = match publications.pending(ANNOUNCE_PER_PASS).await {
		Ok(pending) => pending,
		Err(err) => {
			error!(error = %err, "could not read pending publications");
			return announced;
		}
	};

	'publications: for publication in pending {
		// Before each publication and again after its audience read, not only
		// per recipient: an empty or failed batch never enters the recipient
		// loop, and would otherwise run on past the deadline — and mark itself
		// complete — (a real `chatgpt-codex-connector` finding on #362). A
		// failed read `continue`s into this same check.
		if tokio::time::Instant::now() >= deadline {
			announced.deadline_exceeded = true;
			break;
		}
		#[allow(clippy::cast_possible_wrap)] // bounded by ANNOUNCE_PER_PASS
		let budget = ANNOUNCE_PER_PASS - announced.applied as i64;
		if budget <= 0 {
			break;
		}
		let audience = match publications.audience(&publication, budget).await {
			Ok(audience) => audience,
			Err(err) => {
				error!(publication = publication.id, error = %err, "could not read a publication's audience");
				continue;
			}
		};
		if tokio::time::Instant::now() >= deadline {
			announced.deadline_exceeded = true;
			break;
		}
		#[allow(clippy::cast_possible_wrap)] // at most `budget`
		let last_batch = (audience.len() as i64) < budget;

		let signal = StudySignal::CurriculumUpdated {
			curriculum_id: publication.curriculum_id.clone(),
		};
		// Every subject up to here has been claimed; the cursor only ever
		// moves over a contiguous claimed prefix, so a transient failure
		// leaves that subject (and everyone after it) for the next pass.
		let mut claimed_through: Option<&str> = None;
		let mut complete = true;
		for subject_id in &audience {
			// The deadline between recipients, not only before the first:
			// each claim and fold can wait out SQLite's busy timeout (a real
			// `chatgpt-codex-connector` finding on #362).
			if tokio::time::Instant::now() >= deadline {
				announced.deadline_exceeded = true;
				complete = false;
				break;
			}
			match publications.claim_delivery(publication.id, subject_id, &stamp).await {
				Ok(true) => match observe(db, subject_id, &signal).await {
					Ok(_) => announced.applied += 1,
					Err(err) => {
						error!(publication = publication.id, subject = %subject_id, error = %err, "claimed a delivery but could not apply it; this subject will not be drained for this publication");
					}
				},
				Ok(false) => {}
				Err(err) => {
					error!(publication = publication.id, subject = %subject_id, error = %err, "could not claim a delivery; leaving it, and the rest of this batch, for the next pass");
					complete = false;
					break;
				}
			}
			claimed_through = Some(subject_id.as_str());
		}

		if let Some(subject_id) = claimed_through {
			if let Err(err) = publications.advance_cursor(publication.id, subject_id).await {
				error!(publication = publication.id, error = %err, "could not advance a publication's cursor; the next pass re-reads, and the delivery claims keep it idempotent");
			}
		}
		// Done only when the audience came back short *and* every subject in
		// it was claimed — never on a batch a failure or the deadline cut off
		// (a real `chatgpt-codex-connector` finding on #362).
		if last_batch && complete {
			if let Err(err) = publications.mark_fanned_out(publication.id, &stamp).await {
				error!(publication = publication.id, error = %err, "could not mark a publication fanned out; the next pass will find its audience empty and try again");
			} else {
				info!(publication = publication.id, curriculum_id = %publication.curriculum_id, version = publication.version, "publication announced to everyone it applies to");
			}
		}
		if announced.deadline_exceeded {
			break 'publications;
		}
	}
	announced
}

/// What [`announce_publications`] got through.
#[derive(Debug, Default, Clone, Copy)]
struct Announced {
	/// Subjects the signal was applied to.
	applied: usize,
	/// Whether the pass deadline cut the fan-out short.
	deadline_exceeded: bool,
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
async fn sweep_history(engagement: &EngagementRepository, outcomes: &OutcomeRepository, now: DateTime<Utc>) -> u64 {
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

/// Evaluate one subject, and act if the engine says so.
async fn consider(db: &SqlitePool, nudge: &NudgeContext, engagement: &EngagementRepository, subject_id: &str, deadline: tokio::time::Instant) -> Result<bool, sqlx::Error> {
	let now = Utc::now();

	let stored = engagement.charge(subject_id).await?;
	// The version every save below is conditional on — see
	// `EngagementRepository::save_if_unchanged`.
	let read_as_of = stored.first().map(|row| row.as_of.clone());
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
	// inline: `propose_a_session` (#279/RCM2, widened by #285/RCM8) needs
	// the same one to write a provisioned session, and it is a cheap handle
	// over the shared pool, not a connection of its own.
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

	// Read before the selector is built, because #285 (RCM8) below needs to
	// know whether this subject had anything prepared *going in* — after the
	// provisioning block runs, `Some` no longer distinguishes "they already
	// had one" from "the waker just made one."
	let nothing_prepared = prepared_session.is_none();
	let engine = Engine::<StudyV1, StudyCalibration, _, _>::new(constraints, StudySelector { prepared_session });
	let gate = engagement.gate(subject_id).await?;
	let last_intervened_at = gate
		.and_then(|row| row.last_intervened_at)
		.and_then(|raw| crate::nudge::clock::parse_timestamp(raw.as_str()));

	let mut verdict = engine.evaluate(&charge, now, last_intervened_at);

	// #285 (RCM8), the epic's closing move: **provisioning happens before
	// selection is final, for every warranted subject with nothing
	// prepared** — not only for the three deficits whose selector arm
	// returns `None`.
	//
	// #279 (RCM2) hung provisioning off `Verdict::NothingToSay`, which is
	// exactly the set of subjects `StudySelector::select` had no answer for:
	// dominant deficit `Momentum`, `Mastery`, or `Freshness` with nothing to
	// resume, review, or announce. That left the fourth class out. A
	// `Presence`-dominant subject with nothing prepared never reaches
	// `NothingToSay` at all, because #294 gave plain absence its one
	// sessionless answer, `StudyAction::GetStarted` — so the person the
	// whole cold-start epic (`#257`) is named for, someone who has done
	// nothing but subscribe, got an invitation to go find something rather
	// than the session RCM3/RCM4/RCM5 can now actually compose for them.
	// `#285`'s own acceptance scenario states the fix as its first clause:
	// one subscription, no sessions, no signals, clock advanced ⇒ *a session
	// exists*, owned by that subject, `origin = 'system'`.
	//
	// So the condition here is "warranted, with nothing prepared", not any
	// particular verdict. `Wait` is the one verdict that means *not*
	// warranted — `evaluate` returns it before selection is ever reached,
	// for a subject still inside refractory or still above threshold — and
	// it is the only one excluded. Everything else has already proven
	// eligibility and refractory, which is exactly the point at which
	// composing a proposal is worth the catalogue read.
	//
	// Provisioning before a `Suppressed` verdict is deliberate and not new:
	// #279's arm already wrote the session first and checked admission
	// after, so a quiet-hours or not-yet-consented subject ends the pass
	// with a real proposal waiting for the next admissible one. This only
	// makes the `Presence` path behave the same way as the other three.
	//
	// `GetStarted` survives as exactly what `docs/study-nudge.md` predicted
	// it would become here: the fallback for a catalogue that cannot produce
	// anything. When `propose_a_session` comes back `Unavailable`, the
	// sessionless verdict computed above is still standing, and for plain
	// absence it is still honest — an invitation opens the app, which needs
	// no session to exist.
	if nothing_prepared && !matches!(verdict, Verdict::Wait { .. }) {
		match propose_a_session(db, nudge, &sessions, subject_id, now).await {
			Proposal::Prepared(session_id) => {
				// Neutral to engagement (delta 0.0, filed under Freshness):
				// it is the opportunity a later `LessonReady`/
				// `ResumeAbandoned`/`SuggestReview`/`NewMaterial` needs to be
				// sayable, not a sign of engagement itself. See
				// `StudySignal::SessionProvisioned`'s own doc comment — this
				// is the only thing in the codebase that applies it.
				charge.apply::<StudyCalibration>(&StudySignal::SessionProvisioned { session_id: session_id.clone() }, now);
				verdict = decide_with_a_proposal(engine.admissibility(), &charge, now, session_id);
			}
			Proposal::Unavailable { label, retry_in } => {
				if matches!(verdict, Verdict::NothingToSay) {
					// The one condition `#285` makes alertable, and the only
					// way a pass still ends with nothing to say: this subject
					// is due, past refractory, and their dominant deficit has
					// no sessionless answer — while the catalogue could not
					// produce a proposal either. `propose_a_session` has
					// already logged *which* way the catalogue failed; this
					// line is the separate fact that there was no fallback.
					//
					// `error!`, not the `warn!` this condition carried before
					// the epic, and a different sentence: "there is nothing to
					// point them at" described an ordinary state of the world
					// in RCM2's day. What is wrong now is the catalogue, and a
					// log line describing a condition that no longer exists is
					// worse than no log line at all.
					error!(
						subject = %subject_id,
						reason = label,
						"warranted, but the catalogue produced nothing to propose and this subject's dominant deficit has no sessionless answer; a study deployment in this state can never nudge anyone whose deficit is not Presence"
					);
					crate::metrics::waker::record_nothing_to_say();
					crate::metrics::waker::record_verdict(label, "n/a");
					if let Some(retry_in) = retry_in {
						let retry = now + retry_in;
						let (levels, as_of) = charge.to_storage();
						save_unless_superseded(engagement, subject_id, read_as_of.as_deref(), &levels, as_of, retry).await?;
					}
					return Ok(false);
				}
				// Otherwise the sessionless verdict computed above — plain
				// absence's `GetStarted`, admitted or suppressed — still
				// stands, and it is the pass's real terminal outcome. Nothing
				// is recorded here on purpose: `record_verdict` names *the*
				// outcome one due subject reached this pass, so counting a
				// failed proposal attempt alongside the notification that
				// went out anyway would double-count the pass in the
				// breakdown panel.
			}
		}
	}

	let action = match verdict {
		Verdict::Intervene(action) => action,
		Verdict::Wait { until } => {
			crate::metrics::waker::record_verdict("wait", "n/a");
			// Push the gate out so this subject stops being returned by `due`.
			// Without it the waker would re-read the same row every pass.
			let (levels, as_of) = charge.to_storage();
			save_unless_superseded(engagement, subject_id, read_as_of.as_deref(), &levels, as_of, until).await?;
			return Ok(false);
		}
		Verdict::Suppressed { reason, retry_at } => {
			info!(subject = %subject_id, reason = reason.as_str(), "warranted but not admissible");
			crate::metrics::waker::record_verdict("suppressed", reason.as_str());
			let (levels, as_of) = charge.to_storage();
			save_unless_superseded(engagement, subject_id, read_as_of.as_deref(), &levels, as_of, retry_at).await?;
			return Ok(false);
		}
		Verdict::NothingToSay => {
			// **Unreachable by construction as of #285 (RCM8)**, and kept
			// only because `Verdict` is a closed enum this `match` has to be
			// total over — defensive, not expected. Two facts rule it out.
			// `evaluate` reaches its own `NothingToSay` arm only when
			// `StudySelector::select` returns `None`, which it only does when
			// `prepared_session` is `None`; and every warranted subject with
			// nothing prepared has just been through the block above, which
			// either handed selection a `Some` — exhaustive over all four
			// classes — or returned early. `decide_with_a_proposal` can
			// technically produce this arm, but only by the same `None`, from
			// a selector that was just given a `Some`.
			//
			// So the only way here is a class added to `EngagementClass`
			// without a matching `StudySelector` arm: a build-time mistake,
			// not a state the running system can drift into. That costs this
			// subject one skipped pass rather than a panicked waker, the same
			// refuse-rather-than-guess convention the rest of this function
			// follows.
			error!(subject = %subject_id, "a prepared session still selected nothing to say; unreachable by construction — an EngagementClass with no StudySelector arm is the only way here");
			crate::metrics::waker::record_nothing_to_say();
			crate::metrics::waker::record_verdict("nothing_to_say", "n/a");
			let retry = now + chrono::Duration::hours(6);
			let (levels, as_of) = charge.to_storage();
			save_unless_superseded(engagement, subject_id, read_as_of.as_deref(), &levels, as_of, retry).await?;
			return Ok(false);
		}
	};

	// Claim before sending. A crash between the two costs this intervention
	// rather than duplicating it, which is the right way round.
	let next_eligible = engine.intervened(&mut charge, now);
	// Disallowed for tracing; this is the stored history column.
	#[allow(clippy::disallowed_methods)]
	let serialized = serde_json::to_string(&action).unwrap_or_default();
	// The recharge is written by the claim itself, version-checked against
	// `read_as_of`: a signal folded in since this pass read the charge means
	// `action` was chosen from deficits that no longer exist, so the claim is
	// refused and nothing stale is sent — see `EngagementRepository::claim`.
	let (levels, as_of) = charge.to_storage();
	let Some(log_id) = engagement
		.claim(
			subject_id,
			read_as_of.as_deref(),
			&now.to_rfc3339(),
			&next_eligible.to_rfc3339(),
			action.kind(),
			&serialized,
			&levels,
			&as_of.to_rfc3339(),
		)
		.await?
	else {
		info!(subject = %subject_id, "another pass claimed this subject first, or a signal changed their charge while this pass was deciding");
		crate::metrics::waker::record_verdict("claim_lost", "n/a");
		return Ok(false);
	};

	// #284 (RCM7): refresh only after winning the claim above, not merely on
	// `Verdict::Intervene` — a real `chatgpt-codex-connector` finding on
	// `#335` caught the earlier ordering, where a concurrent pass over the
	// same subject that ultimately *loses* the claim below still ran this
	// refresh beforehand. Because a machine refresh deliberately never moves
	// `updated_at` (`refresh_if_untouched`'s own doc comment), that losing
	// pass's write still lands as "untouched" even after the winning pass
	// has already claimed, saved, and — by the time the losing pass's own
	// catalogue read and write finish — actuated: the recipient can open the
	// proposal the winner just sent while the loser is still silently
	// rewriting it underneath them, for a notification the loser never
	// sends. Only the pass that actually holds `log_id` reaches here, so at
	// most one pass per intervention ever performs this write, immediately
	// before the one send it corresponds to — not one per pass that merely
	// evaluated to `Intervene`.
	if let Some(session_id) = action.session_id() {
		refresh_stale_proposal(db, nudge, &sessions, subject_id, session_id, now).await;
	}

	let Delivery { accepted, timed_out } = actuate(db, nudge, &action, subject_id, deadline).await?;
	if accepted == 0 && timed_out > 0 {
		// #264 (SLI3): a timeout is crash-shaped, and lands on the same side
		// of the claim-before-send line a crash does. Unlike every other
		// failure `actuate` sees, it cannot say whether the push service
		// took the message — a provider that received the request and never
		// answered may still deliver it — so handing the claim back here
		// would let the next pass send a second notification for one
		// intervention. Kept instead: this intervention is spent, the gate
		// already sits at `next_eligible`, and `actuated_at` stays `NULL`
		// because nothing confirmed acceptance.
		warn!(subject = %subject_id, timed_out, "no device confirmed the intervention and at least one delivery timed out; keeping the claim rather than risking a duplicate");
		crate::metrics::waker::record_verdict("delivery_timed_out", "n/a");
		return Ok(false);
	}
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

/// Write the waker's verdict for a subject unless a signal folded in since it
/// read their charge (#360).
///
/// If one did, that signal has already written a newer charge and re-solved
/// `eligible_at` from it, and this pass's verdict — computed from the older
/// read — is the stale one: dropping it leaves the subject exactly where the
/// signal put them, to be reconsidered on a later pass. The intervention path
/// does not come through here: its recharge is written by the version-checked
/// claim itself (`EngagementRepository::claim`), so a signal that lands first
/// refuses the claim rather than letting a stale action be sent.
async fn save_unless_superseded(
	engagement: &EngagementRepository,
	subject_id: &str,
	read_as_of: Option<&str>,
	levels: &[(u16, f64)],
	as_of: DateTime<Utc>,
	eligible_at: DateTime<Utc>,
) -> Result<(), sqlx::Error> {
	if !engagement
		.save_if_unchanged(subject_id, read_as_of, levels, &as_of.to_rfc3339(), &eligible_at.to_rfc3339())
		.await?
	{
		// Not a verdict of its own: the caller has already recorded this
		// subject's terminal outcome for the pass, and the breakdown counts
		// exactly one per subject.
		debug!(subject = %subject_id, "a signal folded in while this pass was deciding; keeping its charge and eligibility rather than this pass's");
	}
	Ok(())
}

/// What `recommend()` ranks a subject's proposal with, beyond the catalogue
/// (#289, TEL4).
///
/// `Default` is the cold-start input every provisioned session used before
/// `activity_outcome` existed: no history, so every activity reads as never
/// played. It is still what every subject gets unless
/// `NudgeContext::recommender_uses_outcomes` is on — and what a subject with
/// no outcomes gets even when it is, which is the "zero-data path is
/// unchanged" property #289 requires.
#[derive(Debug, Clone, Default)]
pub struct RankingInputs {
	pub history: Vec<ActivityHistory>,
	pub last_session_at: Option<DateTime<Utc>>,
}

impl RankingInputs {
	/// Turn one subject's per-activity stats into the recommender's history.
	///
	/// - never played (only skipped, or absent) → no entry: still new on
	///   axis 1, full lift on axis 2;
	/// - an assessed mean → `Completed { score: mean }`, so a poorly-scored
	///   activity lifts and a well-scored one does not;
	/// - otherwise, abandoned and never completed → `Abandoned`;
	/// - otherwise, completed but never assessed → `Unassessed`.
	///
	/// `last_session_at` is the latest play across every activity, which is
	/// what axis 1's "republished since their last session" compares against.
	///
	/// Computed once per decision from one query, so the recommender's seeded
	/// shuffle stays deterministic: the same subject on the same day, with the
	/// same outcomes, gets the same proposal.
	#[must_use]
	pub fn from_stats(stats: &[ActivityStats]) -> Self {
		let history = stats
			.iter()
			.filter(|activity| activity.plays > 0)
			.map(|activity| ActivityHistory {
				activity_id: activity.activity_id.clone(),
				outcome: match activity.mean_score {
					Some(score) => ActivityOutcome::Completed { score },
					None if activity.completed == 0 => ActivityOutcome::Abandoned,
					None => ActivityOutcome::Unassessed,
				},
			})
			.collect();
		let last_session_at = stats
			.iter()
			.filter_map(|activity| activity.last_played_at.as_deref())
			.filter_map(crate::nudge::clock::parse_timestamp)
			.max();
		Self { history, last_session_at }
	}
}

/// This subject's ranking inputs, behind the #289 flag.
///
/// With the flag on, a stats read that fails — a storage error, or a subject
/// whose outcomes span more activities than `STATS_CEILING` — is `None`, and
/// the caller composes nothing rather than falling back to the cold-start
/// input: ranking a subject *with* history as though they had none would
/// persist a proposal that looks informed and is not (a real
/// `chatgpt-codex-connector` finding on #361). With the flag off, the
/// cold-start input is the policy, not a fallback.
async fn ranking_inputs(db: &SqlitePool, nudge: &NudgeContext, subject_id: &str) -> Option<RankingInputs> {
	if !nudge.recommender_uses_outcomes {
		return Some(RankingInputs::default());
	}
	match OutcomeRepository::new(db.clone()).stats(subject_id).await {
		Ok(stats) => Some(RankingInputs::from_stats(&stats)),
		Err(err) => {
			error!(subject = %subject_id, error = %err, "could not read outcome stats; composing nothing rather than ranking this subject as though nothing had been played");
			None
		}
	}
}

/// What one attempt to compose something to point at produced — see
/// [`propose_a_session`].
enum Proposal {
	/// A prepared session now exists for this subject: the one this pass
	/// wrote, or a concurrent writer's that `first_prepared` resolved to
	/// instead.
	Prepared(String),
	/// Nothing could be composed this pass. Both fields are what the caller
	/// needs *only if* it has no sessionless action to fall back on; when it
	/// does (plain absence's `GetStarted`), both are correctly ignored,
	/// because the pass's terminal outcome is then that notification rather
	/// than this failure.
	Unavailable {
		/// The `record_verdict` label for this failure, recorded by the
		/// caller so exactly one verdict is counted per pass.
		label: &'static str,
		/// How far to push `eligible_at` out, or `None` to leave it exactly
		/// where it is so the next pass retries this subject immediately.
		retry_in: Option<chrono::Duration>,
	},
}

/// Compose a session for a subject who has nothing prepared, and write it.
///
/// #279 (RCM2)'s decision, written down per its acceptance criteria:
/// provision a session rather than widen the vocabulary. Two other shapes
/// were weighed and rejected:
///
/// - A fifth `StudyAction` variant (`ProposeSession`) breaks
///   `StudyAction::session_id()`'s totality — every existing variant already
///   carries a real id — and forces `payload::topic_for`/
///   `NudgePayload::for_action` to grow a case for "the same message, before
///   a session exists."
/// - A new `Verdict` arm in `intervention` puts "the domain wants something
///   created" into the generic engine, which `intervention`'s own docs are
///   explicit about keeping free of study vocabulary: "every user story adds
///   a variant [to `study_domain`]; none of them should touch
///   `intervention`."
///
/// So `intervention` and `StudySelector` both stay untouched: provisioning
/// happens here, `prepared_session` becomes `Some`, and the *existing*
/// selector maps the same dominant deficit to `LessonReady`/
/// `ResumeAbandoned`/`SuggestReview`/`NewMaterial` exactly as it would for a
/// session that already existed. RCM3 (#280), RCM4 (#281), and RCM5 (#282)
/// are what actually fill it in — see `materialize_provisioned_session`'s own
/// doc comment for what it writes and why.
///
/// **Crash safety without extra bookkeeping.** The write is a `Scheduled` row
/// `SessionRepository::first_prepared` will find on any later pass (RCM5's own
/// choice — see `materialize_provisioned_session`'s doc comment for why
/// `Scheduled` is now safe where RCM2 originally chose `Draft`). A crash
/// between this write and `consider`'s `claim` costs that pass's
/// notification, not a second session — the next pass reads the row this one
/// already wrote and never calls this function at all.
///
/// **Race safety is a separate concern, and needs its own guard.**
/// `consider`'s `first_prepared` read happened at the *top* of the pass, and
/// a concurrent waker pass or the subject's own `POST /sessions` call can
/// create a prepared session in the window between that read and this write.
/// Because the provisioned row's id is always freshly generated, an
/// unconditional `upsert` would not collide with whatever won that race — it
/// would just add a second, blank draft beside it. `provision_if_absent`
/// closes the window atomically (one statement, targeting #284's own partial
/// unique index — `origin = 'system' AND started_at IS NULL`, not the three
/// statuses `first_prepared` treats as prepared) rather than trusting the
/// read that already happened; see its own doc comment for the mechanism and
/// the #313 review that caught the original race. A race-losing write is a
/// no-op rather than a refresh (#345): the row it would otherwise rewrite is,
/// by construction, another concurrent pass's brand-new proposal — never the
/// stale one #284 wanted refreshed, since that row would have been found by
/// `first_prepared` at the top of `consider` — and the winner may already
/// have claimed and sent a notification pointing at it. Staleness is
/// `refresh_stale_proposal`'s job, after a claim is won; see
/// `docs/study-nudge.md`'s "Never stack proposals" section. The
/// `first_prepared` re-read below is what makes this correct either way:
/// whichever side of the race actually landed is what gets returned, not
/// necessarily the row built here.
///
/// **Logs, but never records a verdict.** Every failure below says in the log
/// what the catalogue could not do, which is true regardless of what the
/// caller does next. Whether that failure is also the *pass's* outcome is the
/// caller's question, not this function's — a `Presence`-dominant subject
/// still gets `GetStarted` out of it — and `record_verdict` counts one
/// terminal outcome per due subject per pass.
async fn propose_a_session(db: &SqlitePool, nudge: &NudgeContext, sessions: &SessionRepository, subject_id: &str, now: DateTime<Utc>) -> Proposal {
	let catalogue = match ActivityRepository::new(db.clone()).list().await {
		Ok(catalogue) => catalogue,
		Err(err) => {
			error!(subject = %subject_id, error = %err, "could not read the activity catalogue; composing nothing rather than an empty session");
			// A real Codex review finding on server#322 (P2): unlike
			// `first_prepared`'s per-subject read, a catalogue read is
			// global -- if it is failing, it fails identically for every
			// subject reaching this function in the same pass. Leaving
			// `eligible_at` where it was would let
			// `EngagementRepository::due`'s oldest-32 query keep
			// re-selecting exactly these subjects every subsequent pass,
			// crowding the batch and starving subjects who need no
			// catalogue read at all -- one with an existing prepared
			// session, say. Ask for the gate to be advanced instead.
			return Proposal::Unavailable {
				label: "storage_error",
				retry_in: Some(chrono::Duration::hours(1)),
			};
		}
	};

	let Some(inputs) = ranking_inputs(db, nudge, subject_id).await else {
		// Per-subject, like a failed session write below: the gate is pushed
		// out an hour rather than left where it is, since an over-ceiling
		// history will not fix itself by the next pass.
		return Proposal::Unavailable {
			label: "storage_error",
			retry_in: Some(chrono::Duration::hours(1)),
		};
	};
	let provisioned = materialize_provisioned_session(new_id(), subject_id, &catalogue, &inputs, now);
	if provisioned.activities.is_empty() {
		// A real Codex review finding on server#322 (P2): every one of
		// `recommend()`'s picks was dropped by `provision()` -- a `NULL`
		// floor on every eligible candidate, or a cap so tight nothing fits.
		// Persisting this anyway would write a `Scheduled` row
		// `first_prepared`/`provision_if_absent` then treats as "already
		// prepared" forever: nothing in this codebase re-provisions once a
		// prepared session exists, so the subject would be stuck pointing at
		// a permanently empty session even after the catalogue is fixed.
		// Retry later instead of writing a session with nothing in it -- a
		// longer backoff than the storage-error case above, since a
		// catalogue that produces nothing timeable needs a content fix, not
		// a quick retry.
		warn!(subject = %subject_id, "recommend()+provision() produced no timeable activities; not persisting an empty session");
		return Proposal::Unavailable {
			label: "nothing_to_provision",
			retry_in: Some(chrono::Duration::hours(6)),
		};
	}

	if let Err(err) = sessions.provision_if_absent(subject_id, &provisioned).await {
		error!(subject = %subject_id, error = %err, "could not write a provisioned session; skipping this subject rather than notifying about one that doesn't exist");
		// Per-subject, unlike the catalogue failures above: the gate stays
		// where it is so the very next pass tries this subject again.
		return Proposal::Unavailable {
			label: "storage_error",
			retry_in: None,
		};
	}

	match sessions.first_prepared(subject_id).await {
		Ok(Some(session_id)) => Proposal::Prepared(session_id),
		Ok(None) => {
			// Unreachable in practice: `provision_if_absent` just proved a
			// prepared session exists for this subject, either the one built
			// above or a concurrent writer's. Guarded rather than trusted,
			// per this codebase's refuse-rather-than-guess convention.
			error!(subject = %subject_id, "provisioned a session but none is findable immediately after; skipping this pass");
			Proposal::Unavailable {
				label: "storage_error",
				retry_in: None,
			}
		}
		Err(err) => {
			error!(subject = %subject_id, error = %err, "could not re-read the provisioned session; skipping this subject");
			Proposal::Unavailable {
				label: "storage_error",
				retry_in: None,
			}
		}
	}
}

/// Re-run the half of [`intervention::Engine::evaluate`] that happens *after*
/// warrant, now that there is a session to point at.
///
/// Deliberately not a second `evaluate` call: warrant (refractory, then the
/// solved eligibility instant) was settled before anything was written, and
/// nothing this pass does afterwards can change it — `StudySignal::
/// SessionProvisioned` carries a delta of `0.0` precisely so that composing a
/// proposal is not itself evidence of engagement. What *does* change is the
/// selector's input, so only selection and admission are redone, in the same
/// order and with the same `retry_at` arithmetic `evaluate` uses.
///
/// The provisioned action still has to clear the same admission gate any
/// other action would — quiet hours and consent do not stop applying just
/// because this action came from provisioning rather than an existing
/// session. Presence is re-checked against the new action's own context
/// automatically: `consider`'s constraints hold one fixed `PresenceLeases`
/// snapshot, fetched once at the top of the pass, and `admit` asks it about
/// whatever action it is given.
fn decide_with_a_proposal(admissibility: &StudyConstraints, charge: &Charge<StudyV1>, now: DateTime<Utc>, session_id: String) -> Verdict<StudyAction, Suppressed> {
	let deficits = charge.deficits::<StudyCalibration>(now);
	let Some(action) = (StudySelector {
		prepared_session: Some(session_id),
	})
	.select(&deficits) else {
		// Cannot happen: `select` is exhaustive over all four classes once
		// `prepared_session` is `Some`. Handed back as a verdict rather than
		// `.expect`ed so `consider`'s own defensive arm is the single place
		// that decides what an impossible state costs.
		return Verdict::NothingToSay;
	};

	match admissibility.admit(now, &action) {
		Ok(()) => Verdict::Intervene(action),
		Err(reason) => Verdict::Suppressed {
			reason,
			retry_at: now + StudyCalibration::REFRACTORY.min(chrono::Duration::hours(1)),
		},
	}
}

/// #284 (RCM7): refresh `prepared`'s content in place if it is an un-started
/// `system` proposal — same id, freshly recommended contents — so an
/// ordinary pass over someone who keeps ignoring the same proposal does not
/// keep pointing at whatever `recommend()` produced the day it was first
/// written.
///
/// **Called only after `consider` has already won the claim on the
/// intervention it is about to send** — not from the moment `first_prepared`
/// resolves, and not merely on `Verdict::Intervene` either. Both were real
/// `chatgpt-codex-connector` findings on `#335`. The first caught an earlier
/// version that ran this before admission was ever checked: it could rewrite
/// a proposal's content while the subject held a fresh presence lease on
/// that exact session — actively viewing it — only for `evaluate`'s own
/// `admit` call to then suppress the notification on `Present` anyway,
/// mutating a session out from under someone looking at it for nothing.
/// Moving the call to gate on `Verdict::Intervene` fixed that, but not a
/// second race one level up: two concurrent passes over the same subject can
/// both reach `Intervene`, and gating on that verdict alone meant *both* ran
/// this refresh — including the one that goes on to lose the claim below
/// and therefore never sends anything. Because a machine refresh
/// deliberately never moves `updated_at`, the loser's write still lands as
/// "untouched" after the winner has already claimed and sent, letting the
/// loser silently rewrite the exact session the recipient just opened from
/// the winner's notification. Gating on a successfully claimed `log_id`
/// instead closes that: at most one pass per intervention ever reaches this
/// call, and it is always the one that goes on to actuate.
///
/// **Best-effort, not fatal.** Unlike `propose_a_session` (where a
/// catalogue-read failure or an empty candidate set means there may be
/// nothing to offer this pass at all, and only plain absence's `GetStarted`
/// saves it), a failure here still has a session to fall back to — the existing,
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
/// — always moves it away from `created_at`. `SessionRepository::
/// refresh_if_untouched`'s own `UPDATE` deliberately never touches
/// `updated_at` either, which is what makes `created_at == updated_at`
/// survive any number of machine refreshes and still mean exactly one thing:
/// nothing but the waker has ever written to this row.
///
/// **The checks below are an optimisation, not the safety boundary — a real
/// `chatgpt-codex-connector` finding on `#335` caught an earlier version
/// treating them as if they were.** `record` is read here, then `evaluate`,
/// admission, a catalogue read, and `recommend()`/`provision()` all run
/// before anything is written — a real window for a person's own
/// `PATCH`/`DELETE` to land in. Re-deciding from a now-stale `record` and
/// writing unconditionally would silently overwrite whatever changed in
/// that window, or orphan a new row while `consider`'s already-selected
/// `StudyAction` keeps pointing at the old id. So the actual enforcement
/// lives one level down, in `refresh_if_untouched`'s own `WHERE` clause,
/// which re-checks the identical predicate atomically at write time; see
/// its own doc comment for the three ways the row can have changed and why
/// each one safely no-ops instead. Everything here only decides whether
/// it is worth doing the catalogue read and `recommend()` call at all.
async fn refresh_stale_proposal(db: &SqlitePool, nudge: &NudgeContext, sessions: &SessionRepository, subject_id: &str, prepared: &str, now: DateTime<Utc>) {
	let record = match sessions.get(subject_id, prepared).await {
		Ok(Some(record)) => record,
		Ok(None) => return,
		Err(err) => {
			error!(subject = %subject_id, error = %err, "could not read the prepared session; leaving it as-is rather than guessing whether it needs refreshing");
			return;
		}
	};
	if !(matches!(record.origin, SessionOrigin::System) && record.started_at.is_none() && record.created_at == record.updated_at) {
		return;
	}

	let catalogue = match ActivityRepository::new(db.clone()).list().await {
		Ok(catalogue) => catalogue,
		Err(err) => {
			error!(subject = %subject_id, error = %err, "could not read the activity catalogue; leaving the existing proposal stale rather than failing this pass");
			return;
		}
	};
	let Some(inputs) = ranking_inputs(db, nudge, subject_id).await else {
		// The existing proposal is a real, playable session; leave it rather
		// than replace it with one ranked on no history.
		return;
	};
	let refreshed = materialize_provisioned_session(new_id(), subject_id, &catalogue, &inputs, now);
	if refreshed.activities.is_empty() {
		// Same trap #322 (P2) named for the original provisioning path: a
		// catalogue with nothing currently timeable must not clobber a real
		// proposal with an empty one. Leaving the stale row in place is
		// strictly better than that — it is still a real, playable session.
		warn!(subject = %subject_id, "recommend()+provision() currently produces nothing timeable; leaving the existing proposal as-is");
		return;
	}
	match sessions.refresh_if_untouched(subject_id, prepared, &refreshed).await {
		// `false` means the row changed state (edited, started, promoted,
		// or deleted) in the window since `record` was read above — not an
		// error, just nothing left to do; the row is already exactly as
		// whatever touched it last left it.
		Ok(_) => {}
		Err(err) => error!(subject = %subject_id, error = %err, "could not refresh the existing proposal; leaving it as-is"),
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
pub fn materialize_provisioned_session(id: String, subject_id: &str, catalogue: &[ActivityRecord], inputs: &RankingInputs, now: DateTime<Utc>) -> SessionRecord {
	let stamp = now.to_rfc3339();
	let picks = recommend(subject_id, DEFAULT_RECOMMENDATION_COUNT, catalogue, &inputs.history, inputs.last_session_at, now);
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
/// say, is not how many were delivered — and how many timed out.
///
/// Each delivery is bounded by `NudgeContext::delivery_timeout`, or by what
/// is left of the pass's `deadline` if that is sooner (#264, SLI3). A
/// provider that has not answered by then is recorded exactly like any other
/// non-acceptance — `record_failure`, never a prune, never `Accepted` — but
/// counted separately, because what the caller may do next differs: see
/// `consider`. A device reached after the deadline is not tried and not
/// recorded against: nothing was sent to it, so there is nothing ambiguous to
/// protect and nothing to count as its failure.
async fn actuate(db: &SqlitePool, nudge: &NudgeContext, action: &StudyAction, subject_id: &str, deadline: tokio::time::Instant) -> Result<Delivery, sqlx::Error> {
	let subscriptions_repo = PushSubscriptionRepository::new(db.clone());
	let subscriptions = subscriptions_repo.for_subject(subject_id).await?;

	let payload = NudgePayload::for_action(&nudge.base_url, action);
	let encoded = match payload.to_bytes() {
		Ok(bytes) => bytes,
		Err(err) => {
			error!(error = %err, "could not serialize the notification payload");
			return Ok(Delivery::default());
		}
	};

	let topic = crate::nudge::payload::topic_for(action);
	let mut delivery = Delivery::default();

	for stored in subscriptions {
		if !stored.accepts(topic) {
			continue;
		}

		let endpoint = &stored.subscription.endpoint;
		let budget = nudge.delivery_timeout.min(deadline.saturating_duration_since(tokio::time::Instant::now()));
		if budget.is_zero() {
			warn!(%endpoint, "waker pass deadline reached; not trying this subject's remaining devices");
			break;
		}
		let Ok(outcome) = tokio::time::timeout(budget, nudge.sender.deliver(&stored.subscription, &encoded)).await else {
			warn!(%endpoint, timeout_ms = budget.as_millis(), "push service did not answer in time");
			subscriptions_repo.record_failure(endpoint, &Utc::now().to_rfc3339()).await?;
			delivery.timed_out += 1;
			continue;
		};
		let stamp = Utc::now().to_rfc3339();

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

		delivery.accepted += 1;
		// Deliberately not "delivered": the push service accepted it, and
		// whether anyone ever sees it is not observable from here.
		debug_assert_eq!(outcome, SendOutcome::Accepted);
		subscriptions_repo.record_success(endpoint, &stamp).await?;
	}

	Ok(delivery)
}

/// What [`actuate`] reports back for one intervention.
#[derive(Debug, Default, Clone, Copy)]
struct Delivery {
	/// Push services that answered `Accepted`.
	accepted: usize,
	/// Push services that did not answer within their budget — the delivery
	/// timeout, or what was left of the pass.
	timed_out: usize,
}

/// Fold a signal into a subject's charge and re-solve their eligibility.
///
/// This is the *only* place work is created. Everything the waker later does
/// was decided here, by arithmetic, at the moment something actually happened.
///
/// # Errors
/// Propagates any storage failure.
pub async fn observe(db: &SqlitePool, subject_id: &str, signal: &study_domain::StudySignal) -> Result<chrono::DateTime<Utc>, sqlx::Error> {
	let now = Utc::now();

	// One write transaction for the read, the fold, and the save — see
	// `EngagementRepository::fold`. Two signals for the same subject arriving
	// together (a poor score from `POST /outcomes` beside a
	// `session-completed` from `/signals`, say) used to be able to read the
	// same stored charge and have the second save erase the first; a real
	// `chatgpt-codex-connector` finding on #360.
	let eligible_at = EngagementRepository::new(db.clone())
		.fold(subject_id, |stored| {
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
			(levels, stamp.to_rfc3339(), eligible_at.to_rfc3339(), eligible_at)
		})
		.await?;

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

	/// A deadline no test that calls `consider` directly will reach.
	fn far_deadline() -> tokio::time::Instant {
		tokio::time::Instant::now() + std::time::Duration::from_secs(3600)
	}

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
	/// `Verdict::NothingToSay` instead, which `consider` logged at `warn!`
	/// on every pass at the time (`error!`, and only on a bug, since #285). Presence — fastest half-life, highest weight — stays the
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
					delivery_timeout: std::time::Duration::from_secs(10),
					pass_deadline: std::time::Duration::from_secs(120),
					recommender_uses_outcomes: false,
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
					delivery_timeout: std::time::Duration::from_secs(10),
					pass_deadline: std::time::Duration::from_secs(120),
					recommender_uses_outcomes: false,
				};

				let intervened = consider(&pool, &nudge, &engagement, subject_id, far_deadline())
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
				// Same `(subject, day)` shuffle hazard the sibling acceptance
				// test's helper documents, and the same fix: seed from the
				// instant the waker recorded, not from a fresh clock read.
				let provisioned_at = crate::nudge::clock::parse_timestamp(&record.created_at).unwrap();
				let expected_picks = recommend(subject_id, DEFAULT_RECOMMENDATION_COUNT, &catalogue, &[], None, provisioned_at);
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
				let second_pass = consider(&pool, &nudge, &engagement, subject_id, far_deadline())
					.await
					.expect("a second pass should not error");
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
	/// pass does the right thing anyway: `provision_if_absent`'s partial
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
				delivery_timeout: std::time::Duration::from_secs(10),
				pass_deadline: std::time::Duration::from_secs(120),
				recommender_uses_outcomes: false,
			};

			// First pass: nothing prepared, provisions a real session.
			consider(&pool, &nudge, &engagement, subject_id, far_deadline()).await.unwrap();
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
			consider(&pool, &nudge, &engagement, subject_id, far_deadline()).await.unwrap();

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
	/// so `Verdict::NothingToSay` — and therefore `provision_if_absent` —
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
				delivery_timeout: std::time::Duration::from_secs(10),
				pass_deadline: std::time::Duration::from_secs(120),
				recommender_uses_outcomes: false,
			};

			// First pass: nothing prepared, provisions a real proposal.
			consider(&pool, &nudge, &engagement, subject_id, far_deadline()).await.unwrap();
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
			// The refresh under test happens after this pass wins the claim,
			// just before `actuate` — this call's own `bool` return isn't
			// asserted, since the placeholder subscription keys above are
			// not valid EC public keys and `Sender::prepare` fails
			// encryption locally (no network involved), the same way any
			// other real encryption failure would; what matters here is that
			// the refresh already ran by that point regardless.
			engagement.save(subject_id, &levels, &now_str, &now_str).await.unwrap();
			consider(&pool, &nudge, &engagement, subject_id, far_deadline()).await.unwrap();

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
	/// the proposal's content must be completely untouched, proving that a
	/// verdict short of `Intervene` — which never reaches the claim the
	/// refresh is now gated on either — correctly never runs it.
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
					delivery_timeout: std::time::Duration::from_secs(10),
					pass_deadline: std::time::Duration::from_secs(120),
					recommender_uses_outcomes: false,
				};

				// First pass: provisions the proposal.
				consider(&pool, &nudge, &engagement, subject_id, far_deadline()).await.unwrap();
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
				let intervened = consider(&pool, &nudge, &engagement, subject_id, far_deadline()).await.unwrap();
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

	/// A real `chatgpt-codex-connector` finding on `#335`'s closing review:
	/// an earlier version of the fix still gated the refresh on
	/// `Verdict::Intervene` alone, so a pass that reaches `Intervene` but
	/// then *loses* the claim below — a concurrent pass over the same
	/// subject claimed it first — still performed the write, racing against
	/// the winning pass's own already-sent notification. This pins the fix:
	/// refresh must be gated on actually winning the claim, not merely on
	/// the verdict. Simulated here by advancing `eligible_at` into the
	/// future before the pass under test runs, exactly as a concurrent
	/// winner's own `claim` call would have — `evaluate` itself never reads
	/// `eligible_at`, only `EngagementRepository::claim`'s own `WHERE
	/// eligible_at <= now` does, so the pass under test still reaches
	/// `Verdict::Intervene` exactly as the ordinary sibling test does, and
	/// only differs at the claim step.
	#[test]
	fn a_pass_that_loses_the_claim_never_refreshes_the_proposal_it_was_about_to_send() {
		use push_kit::{PushSubscription, ReqwestTransport, Sender, SubscriptionKeys, VapidIdentity};
		use push_repo::{Consent, PushSubscriptionRepository, Topic};
		use sqlx::sqlite::SqlitePoolOptions;

		const VAPID_PRIVATE: &str = "IQ9Ur0ykXoHS9gzfYX0aBjy9lvdrjx_PFUXmie9YRcY";
		const VAPID_PUBLIC: &str = "BMjQIp55pdbU8pfCBKyXcZjlmER_mXt5LqNrN1hrXbdBS5EnhIbMu3Au-RV53iIpztzNXkGI56BFB1udQ8Bq_H4";

		static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");

		let subject_id = "subject-losing-the-claim";

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
						endpoint: "https://push.example.com/losing-the-claim".to_owned(),
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
				delivery_timeout: std::time::Duration::from_secs(10),
				pass_deadline: std::time::Duration::from_secs(120),
				recommender_uses_outcomes: false,
			};

			// First pass: provisions a real proposal, exactly as the sibling
			// tests do.
			consider(&pool, &nudge, &engagement, subject_id, far_deadline()).await.unwrap();
			let sessions = SessionRepository::new(pool.clone());
			let first_id = sessions.first_prepared(subject_id).await.unwrap().unwrap();
			let first_record = sessions.get(subject_id, &first_id).await.unwrap().unwrap();

			// The catalogue changes -- if the refresh incorrectly ran despite
			// losing the claim below, this is what would prove it.
			sqlx::query!("UPDATE activities SET min_duration_ms = min_duration_ms * 10").execute(&pool).await.unwrap();

			// Simulate a concurrent winning pass: it already advanced
			// `eligible_at` into the future via its own `claim` call,
			// moments before this pass's own claim attempt.
			let future = now + Duration::hours(1);
			engagement.save(subject_id, &levels, &now_str, &future.to_rfc3339()).await.unwrap();

			let intervened = consider(&pool, &nudge, &engagement, subject_id, far_deadline()).await.unwrap();
			assert!(!intervened, "a pass that loses the claim must not report having intervened");

			let after = sessions.get(subject_id, &first_id).await.unwrap().unwrap();
			assert_eq!(
				after.total_duration_ms, first_record.total_duration_ms,
				"a pass that loses the claim must never have refreshed the proposal it was about to send -- only the winning pass may"
			);
		});
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
				delivery_timeout: std::time::Duration::from_secs(10),
				pass_deadline: std::time::Duration::from_secs(120),
				recommender_uses_outcomes: false,
			};

			// First pass: provisions the proposal.
			consider(&pool, &nudge, &engagement, subject_id, far_deadline()).await.unwrap();
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
			consider(&pool, &nudge, &engagement, subject_id, far_deadline()).await.unwrap();

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
	/// session would be a permanent trap. `first_prepared`/`provision_if_absent`
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
				delivery_timeout: std::time::Duration::from_secs(10),
				pass_deadline: std::time::Duration::from_secs(120),
				recommender_uses_outcomes: false,
			};

			let intervened = consider(&pool, &nudge, &engagement, subject_id, far_deadline()).await.unwrap();
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
				delivery_timeout: std::time::Duration::from_secs(10),
				pass_deadline: std::time::Duration::from_secs(120),
				recommender_uses_outcomes: false,
			};

			let intervened = consider(&pool, &nudge, &engagement, subject_id, far_deadline()).await.unwrap();
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

	/// **`#285` (RCM8), the `#257` epic's closing argument, as one scenario.**
	///
	/// Every preceding RCM story fixed a mechanism; this asserts the property
	/// they were all for, in the exact shape `#285`'s own issue text states
	/// it: a fresh database, one push subscription for one subject, no
	/// sessions, no signals, no engagement rows; the clock advances past the
	/// solved eligibility instant; the waker runs; and *then* a session
	/// exists, owned by that subject, its activities selected by RCM3, each
	/// block at its floor per RCM4, `origin = 'system'`, exactly one
	/// notification accepted — and a second pass immediately after produces
	/// no second session and no second notification.
	///
	/// Both of `#257`'s silences are in the setup rather than asserted
	/// separately. **Silence #1** (no signal ⇒ no gate row ⇒ never due) is
	/// why the only thing this subject ever does is subscribe: without
	/// `first_contact` (#278, RCM1) the `due` query below could never return
	/// them at all. **Silence #2** (nothing prepared ⇒ nothing to say) is why
	/// the `sessions` table starts empty: before RCM2–RCM8 a subject in this
	/// state got an invitation at best, and a `warn!` and a six-hour retry at
	/// worst. Every clause below failed on `main` before this epic, starting
	/// with the first.
	///
	/// Deliberately driven through `run_once`, not `consider`: `due`'s own
	/// `WHERE eligible_at <= now` is the scheduler, and a test that called
	/// `consider` directly would assert the decision while skipping the
	/// discovery — exactly the half of silence #1 that was broken.
	#[test]
	fn a_subject_who_has_only_ever_subscribed_gets_one_proposed_session_and_exactly_one_notification() {
		use metrics_util::debugging::DebuggingRecorder;
		use push_kit::{PushSubscription, SubscriptionKeys};
		use push_repo::{Consent, PushSubscriptionRepository, Topic};
		use sqlx::sqlite::SqlitePoolOptions;
		use std::sync::atomic::Ordering;
		// `push_kit`'s own fixture (`crates/push_kit/src/sender.rs`): a real
		// P-256 point and a real 16-byte auth secret, because "exactly one
		// notification was accepted" is a clause about what `Sender::deliver`
		// actually put on a socket. The inert placeholders the sibling tests
		// in this module use are fine when the assertion is about what
		// happened *before* the send — RFC 8291 encryption fails on them
		// locally, no network involved — and are exactly wrong here.
		const P256DH: &str = "BLMbF9ffKBiWQLCKvTHb6LO8Nb6dcUh6TItC455vu2kElga6PQvUmaFyCdykxY2nOSSL3yKgfbmFLRTUaGv4yV8";
		const AUTH: &str = "xS03Fi5ErfTNH_l9WHE9Ig";

		static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");

		let subject_id = "subject-cold-start";

		let (endpoint, notifications) = loopback_push_service();

		let recorder = DebuggingRecorder::new();
		let snapshotter = recorder.snapshotter();

		metrics::with_local_recorder(&recorder, || {
			let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();

			rt.block_on(async {
				let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
				MIGRATOR.run(&pool).await.unwrap();

				let now = Utc::now();
				let now_str = now.to_rfc3339();
				let engagement = EngagementRepository::new(pool.clone());
				let sessions = SessionRepository::new(pool.clone());

				// GIVEN one push subscription for one subject, and nothing
				// else this subject has ever done.
				PushSubscriptionRepository::new(pool.clone())
					.upsert(
						&PushSubscription {
							endpoint: endpoint.clone(),
							keys: SubscriptionKeys {
								p256dh: P256DH.to_owned(),
								auth: AUTH.to_owned(),
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
				assert!(
					first_contact(&pool, subject_id).await.unwrap(),
					"RCM1: subscribing is what puts a subject in the gate at all — without this row `due` can never return them, however long they stay away"
				);
				assert!(sessions.list(subject_id).await.unwrap().is_empty(), "no sessions: the state the whole epic is about");
				assert!(
					engagement.due(&now_str, BATCH).await.unwrap().is_empty(),
					"and not due yet either: first contact seeds the charge full, so someone who installs the app at 9am is not interrupted at 9:05"
				);

				// WHEN the clock advances past the solved eligibility instant.
				rewind_first_contact_past_the_solved_instant(&engagement, subject_id, now).await;
				assert!(
					engagement.due(&now_str, BATCH).await.unwrap().iter().any(|gate| gate.subject_id == subject_id),
					"the solved instant has passed, so the arithmetic — not a schedule — is what makes this subject due"
				);

				// AND the waker runs.
				let intervened = run_once(&pool, &nudge_context()).await.unwrap().intervened;
				assert_eq!(intervened, 1, "one due subject, one intervention");

				// THEN a session exists, owned by that subject.
				let all = sessions.list(subject_id).await.unwrap();
				assert_eq!(all.len(), 1, "exactly one, and it is the one the waker composed");
				let record = &all[0];
				assert!(matches!(record.origin, SessionOrigin::System), "origin = 'system' (#283, RCM6): proposed, not authored");
				assert_eq!(record.status, SessionStatus::Scheduled, "#282 (RCM5)'s own choice of status");
				assert!(record.started_at.is_none(), "nobody has opened it yet");
				assert!(record.total_duration_ms > 0, "a real, timed session — not the placeholder row RCM2 originally wrote");

				// AND its activities were selected by RCM3, and each block is
				// at its floor per RCM4.
				let catalogue = ActivityRepository::new(pool.clone()).list().await.unwrap();
				assert_composed_by_rcm3_at_rcm4_floors(subject_id, record, &catalogue);

				// AND exactly one notification was accepted.
				assert_eq!(notifications.load(Ordering::SeqCst), 1, "one subject, one device, one push");
				let sent = sqlx::query_scalar!(
					r#"SELECT COUNT(*) AS "count: i64" FROM intervention_log WHERE subject_id = ? AND actuated_at IS NOT NULL"#,
					subject_id
				)
				.fetch_one(&pool)
				.await
				.unwrap();
				assert_eq!(sent, 1, "and the history says so too — a claimed-but-unsent row would be NULL here");

				// AND running the waker again produces no second session and
				// no second notification. This is the refractory floor and
				// the claim doing their jobs together, not a special case:
				// `intervened` wrote `eligible_at` 20 hours out, so `due`
				// does not return this subject at all.
				let again = run_once(&pool, &nudge_context()).await.unwrap().intervened;
				assert_eq!(again, 0, "nothing is due; the second pass costs one index probe");
				assert_eq!(sessions.list(subject_id).await.unwrap().len(), 1, "no second session");
				assert_eq!(notifications.load(Ordering::SeqCst), 1, "no second notification");
			});
		});

		assert_exactly_one_send_and_no_silence(&snapshotter);
	}

	/// The metric half of the scenario above, kept out of its body so the
	/// narrative stays one readable pass.
	fn assert_exactly_one_send_and_no_silence(snapshotter: &metrics_util::debugging::Snapshotter) {
		use metrics_util::debugging::DebugValue;
		use metrics_util::CompositeKey;

		let snapshot: Vec<(CompositeKey, Option<metrics::Unit>, Option<metrics::SharedString>, DebugValue)> = snapshotter.snapshot().into_vec();
		let counter = |name: &str, label_value: Option<&str>| -> u64 {
			snapshot
				.iter()
				.find_map(|(key, _, _, value)| {
					let key = key.key();
					let matches = key.name() == name && label_value.is_none_or(|wanted| key.labels().any(|label| label.value() == wanted));
					matches.then_some(match value {
						DebugValue::Counter(count) => *count,
						_ => 0,
					})
				})
				.unwrap_or(0)
		};

		assert_eq!(counter("nudge_waker_verdicts_total", Some("sent")), 1, "one pass landed a notification, and only one did");
		assert_eq!(
			counter("nudge_waker_nothing_to_say_total", None),
			0,
			"#285's own metric: a subject who has only subscribed is exactly who this epic exists for, so a deployment with something to say to nobody else must still have something to say to them"
		);
		assert_eq!(
			counter("nudge_waker_verdicts_total", Some("nothing_to_say")),
			0,
			"and the verdict breakdown agrees — reaching the defensive arm at all would mean an EngagementClass has no StudySelector arm"
		);
	}

	/// "The clock advances past the solved eligibility instant," for a
	/// scenario that cannot advance it: `consider` reads `Utc::now()`
	/// directly, so the gate row is rewritten instead to be exactly what
	/// `first_contact` would have written had it happened far enough in the
	/// past — the solved instant for a full charge, plus an hour.
	///
	/// The levels are that same full charge, untouched; only `as_of` moves.
	/// Decay is a closed-form function of it (`engagement_charge`'s own
	/// schema comment), so this is the same subject, later — not a hand-tuned
	/// charge picked to produce the deficit the scenario wants.
	async fn rewind_first_contact_past_the_solved_instant(engagement: &EngagementRepository, subject_id: &str, now: DateTime<Utc>) {
		let shift = Charge::<StudyV1>::full::<StudyCalibration>(now).eligible_at::<StudyCalibration>(now) - now + Duration::hours(1);
		let contacted_at = now - shift;
		let seeded = Charge::<StudyV1>::full::<StudyCalibration>(contacted_at);
		let (levels, as_of) = seeded.to_storage();

		engagement
			.save(subject_id, &levels, &as_of.to_rfc3339(), &seeded.eligible_at::<StudyCalibration>(contacted_at).to_rfc3339())
			.await
			.unwrap();
	}

	/// Two clauses of the scenario above: the session's activities are what
	/// `recommend()` (#280, RCM3) picked, and every block sits at its floor
	/// per `provision()` (#281, RCM4).
	///
	/// The first is recomputed from the same catalogue rather than
	/// hard-coded, so it does not go stale if the seeded catalogue or
	/// `DEFAULT_RECOMMENDATION_COUNT` changes. The second is deliberately
	/// *not* recomputed: it restates RCM4's rule — `max(this activity's own
	/// floor, the client's minimum)`, never the catalogue's `defaultMinutes`
	/// — against the catalogue rows directly, so both failing together means
	/// the picks changed and the second failing alone means the flooring did.
	fn assert_composed_by_rcm3_at_rcm4_floors(subject_id: &str, record: &SessionRecord, catalogue: &[ActivityRecord]) {
		use activity_repo::CLIENT_MIN_ACTIVITY_DURATION_MS;

		// Seeded with the instant the waker itself used, recovered from the
		// row it wrote, rather than a fresh `Utc::now()`. A real
		// `chatgpt-codex-connector` finding on `#342`: `recommend`'s third
		// axis is a deterministic shuffle keyed by `(subject, day)`
		// (`shuffle_ranks` hashes a `NaiveDate`), so a pass that provisions
		// just before UTC midnight and an assertion that recomputes just
		// after would disagree about the expected order for a run that
		// behaved perfectly. `materialize_provisioned_session` writes
		// `created_at` from the very `now` it passes to `recommend`, so
		// reading it back makes this exact instead of almost always right.
		let provisioned_at = crate::nudge::clock::parse_timestamp(&record.created_at).unwrap();
		let expected = provision(&recommend(subject_id, DEFAULT_RECOMMENDATION_COUNT, catalogue, &[], None, provisioned_at));
		assert!(!expected.is_empty(), "the seeded catalogue always has at least one timeable activity");
		assert_eq!(
			record.activities,
			expected.iter().map(|activity| serde_json::to_value(activity).unwrap()).collect::<Vec<_>>(),
			"the stored activities must be exactly what recommend() picked and provision() floored"
		);

		for activity in &record.activities {
			let id = activity.get("activityId").and_then(serde_json::Value::as_str).unwrap();
			let minutes = activity
				.get("config")
				.and_then(|config| config.get("durationMinutes"))
				.and_then(serde_json::Value::as_i64)
				.unwrap();
			let catalogued = catalogue.iter().find(|row| row.id == id).unwrap();
			assert_eq!(
				minutes,
				catalogued.min_duration_ms.unwrap().max(CLIENT_MIN_ACTIVITY_DURATION_MS) / 60_000,
				"every block must be at its floor, not at the catalogue's defaultMinutes"
			);
		}
	}

	/// A push service the scenario above can actually be delivered to:
	/// loopback, `std::net`, no HTTP crate. Returns the endpoint to subscribe
	/// with and a count of the notifications it has accepted.
	///
	/// `push_kit`'s `PushTransport` seam is narrow enough that a fake would be
	/// six lines (its own doc comment says so, and `push_kit`'s tests use
	/// one), but `NudgeContext::sender` is a concrete
	/// `Sender<ReqwestTransport>` — so the seam a `file_host` test can reach
	/// is the socket, not the trait. Answering `201` here is what makes
	/// "exactly one notification was accepted" a claim about
	/// `SendOutcome::Accepted` rather than about a mock.
	fn loopback_push_service() -> (String, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
		use std::io::{Read as _, Write as _};
		use std::sync::atomic::{AtomicUsize, Ordering};

		let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
		let mut endpoint = numbered("http://", listener.local_addr().unwrap());
		endpoint.push_str("/wpush/v2/cold-start");

		let notifications = std::sync::Arc::new(AtomicUsize::new(0));
		let accepted = std::sync::Arc::clone(&notifications);

		// Detached rather than joined: the accept loop has no way to know the
		// test is finished, and the whole point of the second pass is that no
		// further connection arrives. The count is read only after both passes
		// have awaited their sends, so every increment that will ever happen
		// has already happened by then.
		std::thread::spawn(move || {
			for stream in listener.incoming() {
				let Ok(mut stream) = stream else { continue };

				// The whole request — headers, then exactly `Content-Length`
				// bytes — before answering, so the client sees a complete
				// exchange rather than a reset mid-body that `SendOutcome`
				// would read as a transport failure.
				let mut request: Vec<u8> = Vec::new();
				let mut chunk = [0_u8; 1024];
				while let Ok(read) = stream.read(&mut chunk) {
					if read == 0 {
						break;
					}
					request.extend_from_slice(&chunk[..read]);
					let Some(head) = request.windows(4).position(|window| window == b"\r\n\r\n") else {
						continue;
					};
					let headers = String::from_utf8_lossy(&request[..head]).to_lowercase();
					let length = headers
						.lines()
						.find_map(|line| line.strip_prefix("content-length:"))
						.and_then(|value| value.trim().parse::<usize>().ok())
						.unwrap_or(0);
					if request.len() >= head + 4 + length {
						break;
					}
				}

				accepted.fetch_add(1, Ordering::SeqCst);
				// `Connection: close`, so a second notification can never be
				// smuggled down a kept-alive socket and go uncounted — one
				// connection is one notification.
				let _ = stream.write_all(b"HTTP/1.1 201 Created\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
				let _ = stream.flush();
			}
		});

		(endpoint, notifications)
	}

	/// The `NudgeContext` the scenario above runs both of its passes with.
	///
	/// A real `ReqwestTransport`, because `NudgeContext::sender` is concrete
	/// — but over a client built with `no_proxy()`, so a machine with
	/// `HTTP_PROXY` set in its environment (this repo's own CI containers
	/// among them) cannot silently route a loopback request through a proxy
	/// that is not there. Quiet hours are disabled outright (`start == end`
	/// never matches, per `is_within_quiet_hours`) rather than set to a fixed
	/// window: the scenario must reach `Verdict::Intervene` regardless of the
	/// wall-clock hour CI happens to run it at.
	fn nudge_context() -> NudgeContext {
		use push_kit::{ReqwestTransport, Sender, VapidIdentity};

		let vapid = VapidIdentity::from_config(
			Some("IQ9Ur0ykXoHS9gzfYX0aBjy9lvdrjx_PFUXmie9YRcY"),
			Some("BMjQIp55pdbU8pfCBKyXcZjlmER_mXt5LqNrN1hrXbdBS5EnhIbMu3Au-RV53iIpztzNXkGI56BFB1udQ8Bq_H4"),
			"mailto:test@example.com",
		)
		.unwrap();
		let transport = ReqwestTransport::new(reqwest::Client::builder().no_proxy().build().unwrap());

		NudgeContext {
			clock: NudgeClock::resolve(Some("UTC")).0,
			sender: std::sync::Arc::new(Sender::new(vapid.clone(), transport)),
			vapid,
			enabled: true,
			quiet_hours_start: 0,
			quiet_hours_end: 0,
			presence_lease_ttl: std::time::Duration::from_secs(75),
			base_url: "https://example.com".to_owned(),
			delivery_timeout: std::time::Duration::from_secs(10),
			pass_deadline: std::time::Duration::from_secs(120),
			recommender_uses_outcomes: false,
		}
	}

	/// A push service that accepts every connection and never answers — the
	/// provider #264 (SLI3) is about. Connections are held open for the life
	/// of the test rather than dropped, because a dropped socket is a reset
	/// the client sees at once, which is a transport error and not a stall.
	/// Returns the endpoint and how many connections it has accepted.
	fn stalling_push_service() -> (String, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
		use std::sync::atomic::{AtomicUsize, Ordering};

		let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
		let mut endpoint = numbered("http://", listener.local_addr().unwrap());
		endpoint.push_str("/wpush/v2/stalled");

		let connections = std::sync::Arc::new(AtomicUsize::new(0));
		let seen = std::sync::Arc::clone(&connections);
		// `collect` never returns — `incoming` never ends — so every accepted
		// stream stays owned, and open, for as long as the thread lives.
		std::thread::spawn(move || {
			let _held: Vec<std::net::TcpStream> = listener
				.incoming()
				.flatten()
				.inspect(|_| {
					seen.fetch_add(1, Ordering::SeqCst);
				})
				.collect();
		});

		(endpoint, connections)
	}

	/// One consenting device for `subject_id`, keyed with `push_kit`'s real
	/// fixture so encryption succeeds and the request actually reaches the
	/// socket — see the cold-start scenario above for why the inert
	/// placeholders are wrong for any test about what a send did.
	async fn subscribe(pool: &SqlitePool, subject_id: &str, endpoint: &str, now: &str) {
		use push_kit::{PushSubscription, SubscriptionKeys};
		use push_repo::Consent;

		PushSubscriptionRepository::new(pool.clone())
			.upsert(
				&PushSubscription {
					endpoint: endpoint.to_owned(),
					keys: SubscriptionKeys {
						p256dh: "BLMbF9ffKBiWQLCKvTHb6LO8Nb6dcUh6TItC455vu2kElga6PQvUmaFyCdykxY2nOSSL3yKgfbmFLRTUaGv4yV8".to_owned(),
						auth: "xS03Fi5ErfTNH_l9WHE9Ig".to_owned(),
					},
				},
				&Consent {
					subject_id: subject_id.to_owned(),
					topics: Topic::ALL.to_vec(),
					consented_at: now.to_owned(),
				},
				now,
			)
			.await
			.unwrap();
	}

	async fn intervention_rows(pool: &SqlitePool, subject_id: &str) -> (i64, i64) {
		let total = sqlx::query_scalar!(r#"SELECT COUNT(*) AS "count: i64" FROM intervention_log WHERE subject_id = ?"#, subject_id)
			.fetch_one(pool)
			.await
			.unwrap();
		let actuated = sqlx::query_scalar!(
			r#"SELECT COUNT(*) AS "count: i64" FROM intervention_log WHERE subject_id = ? AND actuated_at IS NOT NULL"#,
			subject_id
		)
		.fetch_one(pool)
		.await
		.unwrap();
		(total, actuated)
	}

	/// #264 (SLI3): a pass stops at its deadline, between subjects, and the
	/// subjects it never reached are untouched and still due.
	///
	/// Six due subjects, every one with a single device on a push service
	/// that never answers. Each delivery is cut off at 200ms and the pass at
	/// 300ms, so the pass reaches the second subject before the deadline and
	/// the third after it — but the assertions below do not depend on that
	/// arithmetic landing exactly: they hold for any split in which at least
	/// one subject was reached and at least one was not, and they check each
	/// subject against the side of the split it actually landed on.
	#[test]
	fn a_pass_that_reaches_its_deadline_leaves_the_subjects_it_never_reached_untouched_and_still_due() {
		use sqlx::sqlite::SqlitePoolOptions;

		const SUBJECTS: usize = 6;
		static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");

		let (endpoint, connections) = stalling_push_service();
		let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();

		rt.block_on(async {
			let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
			MIGRATOR.run(&pool).await.unwrap();

			let now = Utc::now();
			let now_str = now.to_rfc3339();
			let engagement = EngagementRepository::new(pool.clone());
			let subjects: Vec<String> = (0..SUBJECTS).map(|i| numbered("subject-stalled-", i)).collect();
			for (i, subject_id) in subjects.iter().enumerate() {
				// One endpoint per subject: `push_subscriptions` is keyed by
				// endpoint, and the stall is a property of the service, not of
				// the path.
				subscribe(&pool, subject_id, &numbered(&endpoint, i), &now_str).await;
				first_contact(&pool, subject_id).await.unwrap();
				rewind_first_contact_past_the_solved_instant(&engagement, subject_id, now).await;
			}
			assert_eq!(engagement.due(&now_str, BATCH).await.unwrap().len(), SUBJECTS, "every subject starts due");

			let nudge = NudgeContext {
				delivery_timeout: std::time::Duration::from_millis(200),
				pass_deadline: std::time::Duration::from_millis(300),
				..nudge_context()
			};
			let started = std::time::Instant::now();
			let report = run_once(&pool, &nudge).await.unwrap();
			let elapsed = started.elapsed();

			assert!(
				report.considered >= 1,
				"the deadline is checked between subjects, so the first is always reached: {report:?}"
			);
			assert!(report.deferred >= 1, "six subjects at 200ms each cannot all fit in a 300ms pass: {report:?}");
			assert_eq!(
				report.considered + report.deferred,
				SUBJECTS,
				"every due subject is either considered or deferred: {report:?}"
			);
			assert_eq!(report.intervened, 0, "nothing was accepted, so nothing counts as an intervention: {report:?}");
			assert!(report.deadline_exceeded, "{report:?}");
			assert!(
				elapsed < std::time::Duration::from_secs(5),
				"a pass over a provider that never answers must still end — it took {elapsed:?}"
			);
			assert_eq!(
				connections.load(std::sync::atomic::Ordering::SeqCst),
				report.considered,
				"one delivery attempt per considered subject, none for the rest"
			);

			let still_due: Vec<String> = engagement.due(&now_str, BATCH).await.unwrap().into_iter().map(|gate| gate.subject_id).collect();
			let sessions = SessionRepository::new(pool.clone());
			let mut reached = 0;
			for subject_id in &subjects {
				let (total, actuated) = intervention_rows(&pool, subject_id).await;
				if total == 0 {
					// Deferred: not claimed, not proposed to, and still due.
					assert!(still_due.contains(subject_id), "{subject_id} was never reached, so the next pass must still find it");
					assert!(
						sessions.list(subject_id).await.unwrap().is_empty(),
						"{subject_id} was never reached, so nothing was proposed to it"
					);
				} else {
					// Reached: the timeout kept the claim (see the next test).
					reached += 1;
					assert_eq!((total, actuated), (1, 0), "{subject_id}: claimed once, never confirmed");
					assert!(!still_due.contains(subject_id), "{subject_id}'s claim stands, so it is not retried into a duplicate");
				}
			}
			assert_eq!(reached, report.considered, "the subjects with a claim are exactly the ones the report says were considered");
		});
	}

	/// #264 (SLI3): a timed-out delivery is a failure — never a prune, never
	/// `Accepted` — and no sequence of passes turns it into a second
	/// notification.
	///
	/// One subject, two devices: one on a push service that never answers,
	/// one on a service that accepts. The first pass sends exactly once and
	/// records the stall against the stalled device only; the second pass
	/// sends nothing at all. Exactly one `intervention_log` row, and it is
	/// the one that reached `actuated_at`.
	///
	/// The single-device case — every device stalls — is the other half of
	/// "no duplicate", and is asserted per subject in the deadline test
	/// above: the claim is kept rather than released, so the next pass never
	/// finds that subject due.
	#[test]
	fn a_timed_out_delivery_is_a_failure_not_a_prune_and_is_never_retried_into_a_duplicate() {
		use sqlx::sqlite::SqlitePoolOptions;
		use std::sync::atomic::Ordering;

		static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");

		let subject_id = "subject-one-slow-device";
		let (stalled, stalled_connections) = stalling_push_service();
		let (answering, accepted) = loopback_push_service();
		let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();

		rt.block_on(async {
			let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
			MIGRATOR.run(&pool).await.unwrap();

			let now = Utc::now();
			let now_str = now.to_rfc3339();
			let engagement = EngagementRepository::new(pool.clone());
			subscribe(&pool, subject_id, &stalled, &now_str).await;
			subscribe(&pool, subject_id, &answering, &now_str).await;
			first_contact(&pool, subject_id).await.unwrap();
			rewind_first_contact_past_the_solved_instant(&engagement, subject_id, now).await;

			let nudge = NudgeContext {
				delivery_timeout: std::time::Duration::from_millis(200),
				..nudge_context()
			};

			let first = run_once(&pool, &nudge).await.unwrap();
			assert_eq!(first.intervened, 1, "the answering device accepted, so this is an intervention: {first:?}");
			assert_eq!(stalled_connections.load(Ordering::SeqCst), 1, "the stalled device was tried");
			assert_eq!(accepted.load(Ordering::SeqCst), 1, "and the answering one accepted, once");

			let devices = PushSubscriptionRepository::new(pool.clone()).for_subject(subject_id).await.unwrap();
			assert_eq!(devices.len(), 2, "a timeout is not a prune: both devices are still subscribed");
			let stalled_device = devices.iter().find(|device| device.subscription.endpoint == stalled).unwrap();
			let answering_device = devices.iter().find(|device| device.subscription.endpoint == answering).unwrap();
			assert_eq!(stalled_device.failure_count, 1, "the timeout is recorded against the device that timed out");
			assert_eq!(answering_device.failure_count, 0, "and not against the one that answered");

			let second = run_once(&pool, &nudge).await.unwrap();
			assert_eq!(second, PassReport::default(), "nothing is due: the claim from the first pass stands");
			assert_eq!(accepted.load(Ordering::SeqCst), 1, "no second notification");
			assert_eq!(stalled_connections.load(Ordering::SeqCst), 1, "and no second attempt at the stalled device either");
			assert_eq!(
				intervention_rows(&pool, subject_id).await,
				(1, 1),
				"exactly one intervention_log row, and it reached actuated_at"
			);
		});
	}

	/// #264 (SLI3), from a `chatgpt-codex-connector` finding on #357: the pass
	/// deadline also bounds the deliveries *inside* one subject.
	///
	/// A subject's device list is unbounded, so checking the deadline only
	/// between subjects let one subject with many stalled devices hold a pass
	/// for `devices × delivery_timeout`. Here one subject has five devices on a
	/// push service that never answers, a ten-second delivery timeout, and a
	/// 300ms pass. The first delivery is cut off at what is left of the pass,
	/// the other four are never tried, and the pass ends in well under one
	/// delivery timeout — with the claim kept, because the one delivery that
	/// was tried is ambiguous.
	#[test]
	fn the_pass_deadline_cuts_short_one_subjects_deliveries_not_only_the_batch() {
		use sqlx::sqlite::SqlitePoolOptions;
		use std::sync::atomic::Ordering;

		const DEVICES: usize = 5;
		static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");

		let subject_id = "subject-many-stalled-devices";
		let (endpoint, connections) = stalling_push_service();
		let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();

		rt.block_on(async {
			let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
			MIGRATOR.run(&pool).await.unwrap();

			let now = Utc::now();
			let now_str = now.to_rfc3339();
			let engagement = EngagementRepository::new(pool.clone());
			for device in 0..DEVICES {
				subscribe(&pool, subject_id, &numbered(&endpoint, device), &now_str).await;
			}
			first_contact(&pool, subject_id).await.unwrap();
			rewind_first_contact_past_the_solved_instant(&engagement, subject_id, now).await;

			let nudge = NudgeContext {
				delivery_timeout: std::time::Duration::from_secs(10),
				pass_deadline: std::time::Duration::from_millis(300),
				..nudge_context()
			};
			let started = std::time::Instant::now();
			let report = run_once(&pool, &nudge).await.unwrap();
			let elapsed = started.elapsed();

			assert!(
				elapsed < std::time::Duration::from_secs(3),
				"the pass must end near its deadline, not after 5 × 10s — it took {elapsed:?}"
			);
			assert_eq!(report.considered, 1, "{report:?}");
			assert!(report.deadline_exceeded, "a deadline reached inside the only subject still counts: {report:?}");
			assert_eq!(connections.load(Ordering::SeqCst), 1, "only the first device was tried before the pass ran out");
			assert_eq!(
				intervention_rows(&pool, subject_id).await,
				(1, 0),
				"the one tried delivery is ambiguous, so the claim stands unconfirmed"
			);

			let devices = PushSubscriptionRepository::new(pool.clone()).for_subject(subject_id).await.unwrap();
			let failures: i64 = devices.iter().map(|device| device.failure_count).sum();
			assert_eq!(failures, 1, "a device that was never tried is not recorded as failing");
		});
	}

	/// #265 (SLI4): a pass deletes exactly the `intervention_log` rows decided
	/// before the ninety-day horizon — sent or not — and nothing newer.
	///
	/// Driven through `run_once` with nothing due, because the quiet pass is
	/// the one that runs most: the sweep must happen on a day with no work,
	/// not only on a day with some.
	#[tokio::test]
	async fn a_pass_deletes_exactly_the_intervention_history_past_the_retention_horizon() {
		use sqlx::sqlite::SqlitePoolOptions;

		static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();

		let now = Utc::now();
		let horizon = Duration::days(INTERVENTION_LOG_RETENTION_DAYS);
		// (label, decided_at, actuated_at)
		let rows = [
			("sent, a year ago", now - Duration::days(365), Some(now - Duration::days(365))),
			("sent, a day past the horizon", now - horizon - Duration::days(1), Some(now - horizon - Duration::days(1))),
			("claimed but never confirmed, past the horizon", now - horizon - Duration::days(1), None),
			("sent, a day inside the horizon", now - horizon + Duration::days(1), Some(now - horizon + Duration::days(1))),
			("claimed but never confirmed, today", now, None),
		];
		for (label, decided_at, actuated_at) in &rows {
			let decided_at = decided_at.to_rfc3339();
			let actuated_at = actuated_at.map(|at| at.to_rfc3339());
			sqlx::query!(
				"INSERT INTO intervention_log (subject_id, action_kind, action, decided_at, actuated_at) VALUES ('subject-history', ?, '{}', ?, ?)",
				label,
				decided_at,
				actuated_at
			)
			.execute(&pool)
			.await
			.unwrap();
		}

		// #286: `activity_outcome` is swept by the same pass, against its own
		// (longer) horizon — one row just past it, one just inside.
		for (session_id, days_ago) in [
			("session-past-horizon", ACTIVITY_OUTCOME_RETENTION_DAYS + 1),
			("session-inside-horizon", ACTIVITY_OUTCOME_RETENTION_DAYS - 1),
		] {
			let ended_at = (now - Duration::days(days_ago)).to_rfc3339();
			sqlx::query!(
				"INSERT INTO activity_outcome (subject_id, session_id, activity_id, block_index, started_at, ended_at, planned_ms, elapsed_ms, outcome, score) VALUES ('subject-history', ?, 'honeycomb', 0, ?, ?, 60000, 60000, 'completed', NULL)",
				session_id,
				ended_at,
				ended_at
			)
			.execute(&pool)
			.await
			.unwrap();
		}

		let report = run_once(&pool, &nudge_context()).await.unwrap();
		assert_eq!(
			report.pruned, 4,
			"the three intervention_log rows decided before its horizon and the one outcome past its own, and only those: {report:?}"
		);
		let outcomes: Vec<String> = sqlx::query_scalar!(r#"SELECT session_id AS "session_id!" FROM activity_outcome"#)
			.fetch_all(&pool)
			.await
			.unwrap();
		assert_eq!(outcomes, vec!["session-inside-horizon".to_owned()], "activity_outcome keeps what is inside its horizon");

		let survivors: Vec<String> = sqlx::query_scalar!(r#"SELECT action_kind AS "action_kind!" FROM intervention_log ORDER BY decided_at"#)
			.fetch_all(&pool)
			.await
			.unwrap();
		assert_eq!(
			survivors,
			vec!["sent, a day inside the horizon".to_owned(), "claimed but never confirmed, today".to_owned()],
			"a claimed-but-unconfirmed row is not exempt from the horizon, and is not deleted before it either"
		);
	}

	/// #265 (SLI4): one sweep deletes at most `RETENTION_SWEEP_LIMIT` rows,
	/// oldest first, so a first run against a table that has grown since
	/// launch drains over several passes rather than becoming the long pole in
	/// one.
	#[tokio::test]
	async fn one_retention_sweep_is_bounded_and_takes_the_oldest_rows_first() {
		use sqlx::sqlite::SqlitePoolOptions;

		const EXTRA: i64 = 5;
		static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();

		// `LIMIT + EXTRA` rows past the horizon, one minute apart, oldest first
		// — so "which survive the first sweep" has exactly one right answer.
		let now = Utc::now();
		let oldest = now - Duration::days(INTERVENTION_LOG_RETENTION_DAYS * 2);
		let engagement = EngagementRepository::new(pool.clone());
		for minute in 0..RETENTION_SWEEP_LIMIT + EXTRA {
			let decided_at = (oldest + Duration::minutes(minute)).to_rfc3339();
			let label = numbered("row-", minute);
			sqlx::query!(
				"INSERT INTO intervention_log (subject_id, action_kind, action, decided_at, actuated_at) VALUES ('subject-backlog', ?, '{}', ?, ?)",
				label,
				decided_at,
				decided_at
			)
			.execute(&pool)
			.await
			.unwrap();
		}

		let first = sweep_history(&engagement, &OutcomeRepository::new(pool.clone()), now).await;
		#[allow(clippy::cast_sign_loss)] // both are small positive constants
		let limit = RETENTION_SWEEP_LIMIT as u64;
		assert_eq!(first, limit, "a sweep never deletes more than its limit");

		let survivors: Vec<String> = sqlx::query_scalar!(r#"SELECT action_kind AS "action_kind!" FROM intervention_log ORDER BY decided_at"#)
			.fetch_all(&pool)
			.await
			.unwrap();
		let newest: Vec<String> = (RETENTION_SWEEP_LIMIT..RETENTION_SWEEP_LIMIT + EXTRA).map(|minute| numbered("row-", minute)).collect();
		assert_eq!(survivors, newest, "the oldest rows go first, so what is left is the newest of the backlog");

		#[allow(clippy::cast_sign_loss)]
		let extra = EXTRA as u64;
		assert_eq!(
			sweep_history(&engagement, &OutcomeRepository::new(pool.clone()), now).await,
			extra,
			"the next sweep takes the remainder"
		);
		assert_eq!(
			sweep_history(&engagement, &OutcomeRepository::new(pool.clone()), now).await,
			0,
			"and after that there is nothing past the horizon"
		);
	}

	/// Concurrent signals for one subject all land (from a
	/// `chatgpt-codex-connector` finding on #360): `observe` reads, folds, and
	/// saves in one `BEGIN IMMEDIATE` transaction, so no fold can start from a
	/// charge another is about to overwrite. Before, three concurrent poor
	/// scores could each read a full charge and leave it drained by one.
	#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
	async fn concurrent_signals_for_one_subject_are_all_folded_in() {
		use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
		use std::str::FromStr as _;

		const SIGNALS: u32 = 6;
		static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");
		let dir = tempfile::tempdir().unwrap();
		let url = "sqlite://".to_owned() + &dir.path().join("fold.db").to_string_lossy();
		let options = SqliteConnectOptions::from_str(&url)
			.unwrap()
			.create_if_missing(true)
			.journal_mode(SqliteJournalMode::Wal)
			.busy_timeout(std::time::Duration::from_secs(10));
		let pool = SqlitePoolOptions::new().max_connections(SIGNALS).connect_with(options).await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();

		let signal = StudySignal::ScoredBelowTarget {
			activity_id: "honeycomb".to_owned(),
			score: 0.0,
		};
		let tasks: Vec<_> = (0..SIGNALS)
			.map(|_| {
				let pool = pool.clone();
				let signal = signal.clone();
				tokio::spawn(async move { observe(&pool, "subject-busy", &signal).await.unwrap() })
			})
			.collect();
		for task in tasks {
			task.await.unwrap();
		}

		let mastery = EngagementRepository::new(pool.clone())
			.charge("subject-busy")
			.await
			.unwrap()
			.into_iter()
			.find(|row| row.class == 3)
			.unwrap()
			.level;
		// Full (100) less 20 per poor score, six times: 0 if every fold
		// landed (decay over the test's milliseconds is negligible); 80 if
		// they had all read the same starting charge.
		assert!(mastery < 5.0, "every one of {SIGNALS} concurrent folds must land: mastery is {mastery}");
	}

	/// #289 (TEL4): how per-activity stats become the recommender's history.
	#[test]
	fn ranking_history_maps_assessed_abandoned_and_unassessed_play_distinctly() {
		fn stats(id: &str, completed: i64, abandoned: i64, skipped: i64, mean_score: Option<f64>, last: Option<&str>) -> ActivityStats {
			ActivityStats {
				activity_id: id.to_owned(),
				plays: completed + abandoned,
				completed,
				abandoned,
				skipped,
				mean_score,
				last_played_at: last.map(ToOwned::to_owned),
			}
		}

		let inputs = RankingInputs::from_stats(&[
			stats("assessed", 2, 0, 0, Some(0.3), Some("2026-09-20T10:00:00+00:00")),
			stats("abandoned", 0, 1, 0, None, Some("2026-09-22T10:00:00+00:00")),
			stats("unassessed", 1, 1, 0, None, Some("2026-09-21T10:00:00+00:00")),
			stats("only-skipped", 0, 0, 3, None, None),
		]);
		let outcome = |id: &str| inputs.history.iter().find(|entry| entry.activity_id == id).map(|entry| entry.outcome);
		assert_eq!(outcome("assessed"), Some(ActivityOutcome::Completed { score: 0.3 }));
		assert_eq!(outcome("abandoned"), Some(ActivityOutcome::Abandoned));
		assert_eq!(outcome("unassessed"), Some(ActivityOutcome::Unassessed), "finished without a score is not a score");
		assert_eq!(outcome("only-skipped"), None, "a block passed over was never played, so it is still new");
		assert_eq!(inputs.last_session_at, crate::nudge::clock::parse_timestamp("2026-09-22T10:00:00+00:00"));

		let empty = RankingInputs::from_stats(&[]);
		assert!(empty.history.is_empty() && empty.last_session_at.is_none(), "no outcomes is exactly the cold-start input");
	}

	/// #289 (TEL4): the flag, both ways. Off, a subject's outcomes are not
	/// read at all; on, they are. A subject with no outcomes gets the same
	/// proposal either way, and the same subject on the same day gets the
	/// same proposal twice — stats are read once per decision, not sampled.
	#[tokio::test]
	async fn the_outcome_flag_gates_ranking_and_changes_nothing_for_a_subject_with_no_outcomes() {
		use sqlx::sqlite::SqlitePoolOptions;

		static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();

		sqlx::query!(
			"INSERT INTO activity_outcome (subject_id, session_id, activity_id, block_index, started_at, ended_at, planned_ms, elapsed_ms, outcome, score) VALUES ('subject-with-history', 'session-1', 'honeycomb', 0, '2026-09-20T10:00:00+00:00', '2026-09-20T10:05:00+00:00', 300000, 300000, 'completed', 0.95)"
		)
		.execute(&pool)
		.await
		.unwrap();

		let off = nudge_context();
		let on = NudgeContext {
			recommender_uses_outcomes: true,
			..nudge_context()
		};

		let with_history_off = ranking_inputs(&pool, &off, "subject-with-history").await.unwrap();
		assert!(with_history_off.history.is_empty(), "flag off: outcomes are not read");
		let with_history_on = ranking_inputs(&pool, &on, "subject-with-history").await.unwrap();
		assert_eq!(with_history_on.history.len(), 1, "flag on: they are");

		let catalogue = ActivityRepository::new(pool.clone()).list().await.unwrap();
		let now = t0();
		let fresh_off = materialize_provisioned_session(
			"a".to_owned(),
			"subject-fresh",
			&catalogue,
			&ranking_inputs(&pool, &off, "subject-fresh").await.unwrap(),
			now,
		);
		let fresh_on = materialize_provisioned_session(
			"a".to_owned(),
			"subject-fresh",
			&catalogue,
			&ranking_inputs(&pool, &on, "subject-fresh").await.unwrap(),
			now,
		);
		assert_eq!(fresh_off.activities, fresh_on.activities, "the zero-data path is unchanged by the flag");
		assert_eq!(fresh_off.name, fresh_on.name);

		let first = materialize_provisioned_session("a".to_owned(), "subject-with-history", &catalogue, &with_history_on, now);
		let again = materialize_provisioned_session(
			"a".to_owned(),
			"subject-with-history",
			&catalogue,
			&ranking_inputs(&pool, &on, "subject-with-history").await.unwrap(),
			now,
		);
		assert_eq!(first.activities, again.activities, "determinism survives the flag");

		// A subject whose history cannot be read in full is not ranked as
		// though they had none (from a `chatgpt-codex-connector` finding on
		// #361): over the stats ceiling, there are no inputs at all.
		for i in 0..=outcome_repo::STATS_CEILING {
			let mut activity = String::from("bulk-");
			activity.push_str(&i.to_string());
			let mut session = String::from("session-bulk-");
			session.push_str(&i.to_string());
			sqlx::query!(
				"INSERT INTO activity_outcome (subject_id, session_id, activity_id, block_index, started_at, ended_at, planned_ms, elapsed_ms, outcome, score) VALUES ('subject-sprawling', ?, ?, 0, '2026-09-20T10:00:00+00:00', '2026-09-20T10:05:00+00:00', 1, 1, 'completed', NULL)",
				session,
				activity
			)
			.execute(&pool)
			.await
			.unwrap();
		}
		assert!(ranking_inputs(&pool, &on, "subject-sprawling").await.is_none(), "flag on: refused, not cold-started");
		assert!(ranking_inputs(&pool, &off, "subject-sprawling").await.is_some(), "flag off: stats are never read");
	}

	/// A session this subject has actually started — what puts them in
	/// `CURRICULUM_AUDIENCE` (#273).
	async fn studied(pool: &SqlitePool, subject_id: &str) {
		let now = Utc::now().to_rfc3339();
		let mut id = String::from("session-studied-");
		id.push_str(subject_id);
		let record = SessionRecord {
			name: id.clone(),
			id,
			status: SessionStatus::Completed,
			origin: SessionOrigin::User,
			activities: Vec::new(),
			scenes: Vec::new(),
			layout_mode: LayoutMode::Basic,
			layout: None,
			total_duration_ms: 0,
			created_at: now.clone(),
			updated_at: now.clone(),
			started_at: Some(now.clone()),
			completed_at: Some(now),
			final_elapsed_ms: None,
		};
		SessionRepository::new(pool.clone()).upsert(subject_id, &record).await.unwrap();
	}

	async fn publish(pool: &SqlitePool, id: &str, version: i64) {
		let published_at = Utc::now().to_rfc3339();
		sqlx::query!(
			r#"
			INSERT INTO activities (id, name, description, icon, registry_key, layout_tree, maturity, min_duration_ms, published_at, version, fields, default_config, audio)
			VALUES (?1, ?1, 'new material', 'hexagon', ?1, 'study', 'ready', 300000, ?2, ?3, '[]', '{}', NULL)
			ON CONFLICT (id) DO UPDATE SET version = excluded.version, published_at = excluded.published_at
			"#,
			id,
			published_at,
			version
		)
		.execute(pool)
		.await
		.unwrap();
	}

	async fn freshness(pool: &SqlitePool, subject_id: &str) -> Option<f64> {
		EngagementRepository::new(pool.clone())
			.charge(subject_id)
			.await
			.unwrap()
			.into_iter()
			.find(|row| row.class == 4)
			.map(|row| row.level)
	}

	/// #273 (CAT5): the catalogue that exists at deploy is a baseline, a new
	/// activity drains freshness once for subjects who have studied — and only
	/// them — a re-run drains nobody twice, and a version bump is news again.
	#[tokio::test]
	async fn publishing_an_activity_drains_freshness_once_for_subjects_who_have_studied() {
		use sqlx::sqlite::SqlitePoolOptions;

		static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();

		studied(&pool, "subject-studied").await;
		first_contact(&pool, "subject-studied").await.unwrap();
		first_contact(&pool, "subject-only-subscribed").await.unwrap();
		let nudge = nudge_context();

		assert_eq!(run_once(&pool, &nudge).await.unwrap().announced, 0, "the seeded catalogue is not new to anyone");
		let before = freshness(&pool, "subject-studied").await.unwrap();

		publish(&pool, "new-thing", 1).await;
		assert_eq!(run_once(&pool, &nudge).await.unwrap().announced, 1, "one subject has studied");
		let after_one = freshness(&pool, "subject-studied").await.unwrap();
		assert!(before - after_one > 30.0, "CurriculumUpdated drains freshness by 35: {before} -> {after_one}");
		assert_eq!(
			freshness(&pool, "subject-only-subscribed").await,
			Some(100.0),
			"someone who has never studied starts full and stays full — everything is new to them"
		);

		assert_eq!(run_once(&pool, &nudge).await.unwrap().announced, 0, "re-running the publish announces nothing");
		let after_rerun = freshness(&pool, "subject-studied").await.unwrap();
		assert!(
			(after_one - after_rerun).abs() < 0.01,
			"two applications drain the same as one: {after_one} vs {after_rerun}"
		);

		publish(&pool, "new-thing", 2).await;
		assert_eq!(run_once(&pool, &nudge).await.unwrap().announced, 1, "a version bump is new material again");
	}

	/// #273 (CAT5): the fan-out is bounded per pass and resumes where it left
	/// off, without re-announcing to anyone it already reached.
	#[tokio::test]
	async fn the_announcement_fan_out_is_bounded_per_pass_and_resumable() {
		use sqlx::sqlite::SqlitePoolOptions;

		const EXTRA: i64 = 5;
		static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();

		for i in 0..ANNOUNCE_PER_PASS + EXTRA {
			studied(&pool, &numbered("subject-", i)).await;
		}
		publish(&pool, "new-thing", 1).await;
		let nudge = nudge_context();

		#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
		let (cap, extra) = (ANNOUNCE_PER_PASS as usize, EXTRA as usize);
		assert_eq!(run_once(&pool, &nudge).await.unwrap().announced, cap, "the first pass stops at the cap");
		let pending = PublicationRepository::new(pool.clone()).pending(10).await.unwrap();
		assert_eq!(pending.len(), 1, "and the publication is still in progress");
		let past_cursor: i64 = sqlx::query_scalar!(
			r#"SELECT COUNT(*) AS "count!: i64" FROM curriculum_delivery WHERE subject_id > ?"#,
			pending[0].cursor_subject
		)
		.fetch_one(&pool)
		.await
		.unwrap();
		assert!(pending[0].cursor_subject.is_some(), "its cursor has moved");
		assert_eq!(past_cursor, 0, "everyone reached is at or before the cursor, so the next pass seeks past them");
		assert_eq!(run_once(&pool, &nudge).await.unwrap().announced, extra, "the next pass reaches the rest, and only the rest");
		assert!(PublicationRepository::new(pool.clone()).pending(10).await.unwrap().is_empty(), "then it is done");
		assert_eq!(run_once(&pool, &nudge).await.unwrap().announced, 0);
	}

	/// #273 (CAT5): the fan-out honours the pass deadline between recipients
	/// and reports it, and a batch the deadline cut short is never marked
	/// complete (from `chatgpt-codex-connector` findings on #362).
	#[tokio::test]
	async fn an_announcement_the_deadline_cuts_short_stays_pending_and_is_reported() {
		use sqlx::sqlite::SqlitePoolOptions;

		static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();
		for i in 0..3 {
			studied(&pool, &numbered("subject-", i)).await;
		}
		publish(&pool, "new-thing", 1).await;

		let out_of_time = announce_publications(&pool, tokio::time::Instant::now()).await;
		assert!(out_of_time.deadline_exceeded, "{out_of_time:?}");
		assert_eq!(out_of_time.applied, 0);
		assert_eq!(
			PublicationRepository::new(pool.clone()).pending(10).await.unwrap().len(),
			1,
			"a cut-short batch is not marked complete"
		);

		let with_time = announce_publications(&pool, far_deadline()).await;
		assert!(!with_time.deadline_exceeded);
		assert_eq!(with_time.applied, 3, "the next pass with time reaches everyone");
		assert!(PublicationRepository::new(pool.clone()).pending(10).await.unwrap().is_empty());
	}

	/// #273 (CAT5): the audience is fixed when the publication is detected —
	/// a subject whose first session started afterwards has not missed it,
	/// wherever their id falls relative to the cursor (from a
	/// `chatgpt-codex-connector` finding on #362).
	#[tokio::test]
	async fn a_subject_who_first_studies_after_a_publication_is_not_its_audience() {
		use sqlx::sqlite::SqlitePoolOptions;

		static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();
		studied(&pool, "subject-a-before").await;
		studied(&pool, "subject-z-after").await;
		// In another offset, to hold the comparison to instants, not text.
		sqlx::query!("UPDATE sessions SET started_at = '2999-01-01T02:00:00+02:00' WHERE subject_id = 'subject-z-after'")
			.execute(&pool)
			.await
			.unwrap();
		publish(&pool, "new-thing", 1).await;

		let announced = announce_publications(&pool, far_deadline()).await;
		assert_eq!(announced.applied, 1, "{announced:?}");
		let reached: Vec<String> = sqlx::query_scalar!("SELECT subject_id FROM curriculum_delivery").fetch_all(&pool).await.unwrap();
		assert_eq!(reached, ["subject-a-before"]);
	}

	/// #273 (CAT5): a publication with nobody to reach is not marked complete
	/// by a pass already past its deadline — the empty batch never enters the
	/// per-recipient check (from a `chatgpt-codex-connector` finding on #362).
	#[tokio::test]
	async fn an_empty_audience_past_the_deadline_is_left_pending() {
		use sqlx::sqlite::SqlitePoolOptions;

		static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();
		publish(&pool, "new-thing", 1).await;

		let out_of_time = announce_publications(&pool, tokio::time::Instant::now()).await;
		assert!(out_of_time.deadline_exceeded, "{out_of_time:?}");
		assert_eq!(
			PublicationRepository::new(pool.clone()).pending(10).await.unwrap().len(),
			1,
			"no work past the deadline, completion included"
		);
		announce_publications(&pool, far_deadline()).await;
		assert!(
			PublicationRepository::new(pool.clone()).pending(10).await.unwrap().is_empty(),
			"the next pass with time completes it"
		);
	}

	/// #273 (CAT5): a deadline that runs out after the last recipient — here,
	/// with nothing left to announce, after the only reads — is still reported,
	/// so `run_once` counts it and skips the sweep (from a
	/// `chatgpt-codex-connector` finding on #362).
	#[tokio::test]
	async fn an_announcement_that_ends_past_the_deadline_reports_it() {
		use sqlx::sqlite::SqlitePoolOptions;

		static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();

		let announced = announce_publications(&pool, tokio::time::Instant::now()).await;
		assert_eq!(announced.applied, 0);
		assert!(announced.deadline_exceeded, "no recipient ran, yet the pass is past its deadline: {announced:?}");
	}

	/// #273 (CAT5), end to end through the engine: a subject whose freshness is
	/// the thinnest margin, with a session prepared, is told about new material
	/// once a publication drains it — `StudyAction::NewMaterial`, on
	/// `Topic::NewMaterial`.
	#[tokio::test]
	async fn a_publication_makes_new_material_the_thing_to_say() {
		use sqlx::sqlite::SqlitePoolOptions;

		static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();

		let subject_id = "subject-almost-drifted";
		studied(&pool, subject_id).await;
		// Weighted shortfalls 39 / 39.2 / 39 / 26 and an aggregate of 116.8 —
		// just above `THRESHOLD` (110), so not yet eligible. Draining
		// freshness by 35 takes it to 0: its shortfall becomes 40, the
		// largest, and the aggregate falls to 102.8, under the threshold.
		// Only freshness moved, so it is what is said.
		let now = Utc::now();
		let levels: Vec<(u16, f64)> = vec![(1, 61.0), (2, 44.0), (3, 22.0), (4, 35.0)];
		EngagementRepository::new(pool.clone())
			.save(subject_id, &levels, &now.to_rfc3339(), &(now + Duration::days(1)).to_rfc3339())
			.await
			.unwrap();

		publish(&pool, "new-thing", 1).await;
		assert_eq!(announce_publications(&pool, far_deadline()).await.applied, 1);

		let stored = EngagementRepository::new(pool.clone()).charge(subject_id).await.unwrap();
		#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
		let levels: Vec<(u16, f64)> = stored.iter().map(|row| (row.class as u16, row.level)).collect();
		let as_of = crate::nudge::clock::parse_timestamp(&stored[0].as_of).unwrap();
		let charge = Charge::<StudyV1>::from_storage::<StudyCalibration>(&levels, as_of);

		let constraints = StudyConstraints {
			clock: NudgeClock::resolve(Some("UTC")).0,
			enabled: true,
			quiet_hours_start: 0,
			quiet_hours_end: 0,
			presence: PresenceLeases::empty(std::time::Duration::from_secs(75)),
			consented_topics: Topic::ALL.to_vec(),
		};
		let engine = Engine::<StudyV1, StudyCalibration, _, _>::new(
			constraints,
			StudySelector {
				prepared_session: Some("session-next".to_owned()),
			},
		);
		let verdict = engine.evaluate(&charge, as_of, None);
		let Verdict::Intervene(action) = verdict else {
			panic!("expected an intervention once freshness drained, got {verdict:?}");
		};
		assert_eq!(
			action,
			StudyAction::NewMaterial {
				session_id: "session-next".to_owned()
			}
		);
		assert_eq!(crate::nudge::payload::topic_for(&action), Topic::NewMaterial);
	}

	/// A signal folded in while the waker was deciding is not overwritten by
	/// the waker's stale verdict (from a `chatgpt-codex-connector` finding on
	/// #360): the waker reads, a signal folds and re-solves, and the waker's
	/// later save — computed from the older read — is dropped.
	#[tokio::test]
	async fn a_waker_save_never_overwrites_a_signal_folded_in_after_its_read() {
		use sqlx::sqlite::SqlitePoolOptions;

		static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();
		let engagement = EngagementRepository::new(pool.clone());
		let subject_id = "subject-racing";
		first_contact(&pool, subject_id).await.unwrap();

		// The waker's read, at the top of `consider`.
		let stored = engagement.charge(subject_id).await.unwrap();
		let read_as_of = stored.first().map(|row| row.as_of.clone());

		// A poor score lands while the waker is still deciding.
		let signal_eligible_at = observe(
			&pool,
			subject_id,
			&StudySignal::ScoredBelowTarget {
				activity_id: "honeycomb".to_owned(),
				score: 0.0,
			},
		)
		.await
		.unwrap();

		// The waker's verdict, from its stale read: "full, wait a year".
		let stale = Charge::<StudyV1>::full::<StudyCalibration>(Utc::now());
		let (levels, as_of) = stale.to_storage();
		save_unless_superseded(&engagement, subject_id, read_as_of.as_deref(), &levels, as_of, Utc::now() + Duration::days(365))
			.await
			.unwrap();

		let mastery = engagement.charge(subject_id).await.unwrap().into_iter().find(|row| row.class == 3).unwrap().level;
		assert!(mastery < 90.0, "the poor score's drain survives: mastery is {mastery}");
		let gate = engagement.gate(subject_id).await.unwrap().unwrap();
		assert_eq!(gate.eligible_at, signal_eligible_at.to_rfc3339(), "and so does the eligibility it re-solved");

		// With nothing folded in since its read, the waker's save still lands.
		let fresh = engagement.charge(subject_id).await.unwrap().first().map(|row| row.as_of.clone());
		let later = Utc::now() + Duration::days(2);
		save_unless_superseded(&engagement, subject_id, fresh.as_deref(), &levels, as_of, later).await.unwrap();
		assert_eq!(engagement.gate(subject_id).await.unwrap().unwrap().eligible_at, later.to_rfc3339());
	}

	/// A signal folded in after the waker read the charge refuses the waker's
	/// claim — nothing chosen from the stale deficits is sent — and the claim
	/// writes the intervention's recharge in the same transaction when it does
	/// win (from a `chatgpt-codex-connector` finding on #360).
	#[tokio::test]
	async fn a_claim_is_refused_when_a_signal_changed_the_charge_since_the_read() {
		use sqlx::sqlite::SqlitePoolOptions;

		static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();
		let engagement = EngagementRepository::new(pool.clone());
		let subject_id = "subject-claim-race";
		first_contact(&pool, subject_id).await.unwrap();
		let past = (Utc::now() - Duration::hours(1)).to_rfc3339();
		let stored = engagement.charge(subject_id).await.unwrap();
		#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
		let levels: Vec<(u16, f64)> = stored.iter().map(|row| (row.class as u16, row.level)).collect();
		engagement.save(subject_id, &levels, &stored[0].as_of, &past).await.unwrap();

		let read_as_of = engagement.charge(subject_id).await.unwrap().first().map(|row| row.as_of.clone());
		observe(
			&pool,
			subject_id,
			&StudySignal::ScoredBelowTarget {
				activity_id: "honeycomb".to_owned(),
				score: 0.0,
			},
		)
		.await
		.unwrap();
		// `observe` re-solved eligibility from a still-mostly-full charge; put
		// the gate back in the past so only the version check can refuse.
		let after = engagement.charge(subject_id).await.unwrap();
		#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
		let after_levels: Vec<(u16, f64)> = after.iter().map(|row| (row.class as u16, row.level)).collect();
		let fold_as_of = after[0].as_of.clone();
		sqlx::query!("UPDATE engagement_gate SET eligible_at = ? WHERE subject_id = ?", past, subject_id)
			.execute(&pool)
			.await
			.unwrap();

		let now = Utc::now().to_rfc3339();
		let later = (Utc::now() + Duration::hours(20)).to_rfc3339();
		let stale = engagement
			.claim(subject_id, read_as_of.as_deref(), &now, &later, "lesson-ready", "{}", &levels, &now)
			.await
			.unwrap();
		assert_eq!(stale, None, "a claim decided from the pre-signal charge is refused");
		assert_eq!(intervention_rows(&pool, subject_id).await, (0, 0), "and nothing was logged");
		assert_eq!(engagement.charge(subject_id).await.unwrap()[0].as_of, fold_as_of, "the signal's charge stands");

		let won = engagement
			.claim(subject_id, Some(&fold_as_of), &now, &later, "lesson-ready", "{}", &after_levels, &now)
			.await
			.unwrap();
		assert!(won.is_some(), "a claim decided from the current charge wins");
		assert_eq!(
			engagement.charge(subject_id).await.unwrap()[0].as_of,
			now,
			"and writes its recharge in the same transaction"
		);
		assert_eq!(engagement.gate(subject_id).await.unwrap().unwrap().eligible_at, later);
	}

	/// Each class `classify_pass_error` can return is a real `sqlx::Error` a
	/// pass can hit, produced the way a pass would hit it — not a
	/// hand-built error value whose code might not match what `SQLite`
	/// actually reports.
	#[tokio::test]
	async fn pass_errors_are_classified_by_what_to_go_and_fix() {
		use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
		use std::str::FromStr as _;

		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();

		// The motivating case: the waker's first query against a database
		// that never had `engagement_gate` created.
		let missing_table = EngagementRepository::new(pool.clone()).due(&t0().to_rfc3339(), BATCH).await.unwrap_err();
		assert_eq!(classify_pass_error(&missing_table), "schema");

		sqlx::query("CREATE TABLE t (a INTEGER)").execute(&pool).await.unwrap();
		let missing_column = sqlx::query("SELECT b FROM t").execute(&pool).await.unwrap_err();
		assert_eq!(classify_pass_error(&missing_column), "schema");

		let syntax = sqlx::query("SELEKT 1").execute(&pool).await.unwrap_err();
		assert_eq!(classify_pass_error(&syntax), "other");

		// Two connections to one file: the first holds a write lock, the
		// second has no busy_timeout to wait it out.
		let dir = tempfile::tempdir().unwrap();
		let url = "sqlite://".to_owned() + &dir.path().join("locked.db").to_string_lossy();
		let holder = SqlitePoolOptions::new()
			.max_connections(1)
			.connect_with(SqliteConnectOptions::from_str(&url).unwrap().create_if_missing(true))
			.await
			.unwrap();
		sqlx::query("CREATE TABLE t (a INTEGER)").execute(&holder).await.unwrap();
		let mut lock = holder.acquire().await.unwrap();
		sqlx::query("BEGIN IMMEDIATE").execute(&mut *lock).await.unwrap();
		let contender = SqlitePoolOptions::new()
			.max_connections(1)
			.connect_with(SqliteConnectOptions::from_str(&url).unwrap().busy_timeout(std::time::Duration::ZERO))
			.await
			.unwrap();
		let busy = sqlx::query("INSERT INTO t VALUES (1)").execute(&contender).await.unwrap_err();
		assert_eq!(classify_pass_error(&busy), "locked");
		drop(lock);

		assert_eq!(classify_pass_error(&sqlx::Error::Io(std::io::Error::other("disk gone"))), "io");

		pool.close().await;
		let closed = sqlx::query("SELECT 1").execute(&pool).await.unwrap_err();
		assert_eq!(classify_pass_error(&closed), "pool");

		for err in [&missing_table, &busy, &closed, &syntax] {
			assert!(crate::metrics::waker::PASS_ERROR_CLASSES.contains(&classify_pass_error(err)));
		}
	}

	/// A failed pass sets exactly its own class to 1 and every other class
	/// to 0, and the next success clears it — the state LOOPS reads.
	#[test]
	fn a_failed_pass_is_visible_until_the_next_success() {
		use metrics_util::debugging::{DebugValue, DebuggingRecorder};

		let recorder = DebuggingRecorder::new();
		let snapshotter = recorder.snapshotter();
		let failing = |class: &str| {
			snapshotter
				.snapshot()
				.into_vec()
				.into_iter()
				.find(|(key, ..)| key.key().name() == "nudge_waker_pass_failing" && key.key().labels().any(|l| l.value() == class))
				.map(|(.., value)| value)
		};

		metrics::with_local_recorder(&recorder, || {
			crate::metrics::waker::record_interval(std::time::Duration::from_secs(300));
			for class in crate::metrics::waker::PASS_ERROR_CLASSES {
				assert_eq!(failing(class), Some(DebugValue::Gauge(0.0.into())), "{class} exists at 0 from spawn");
			}

			crate::metrics::waker::record_failed_pass("schema");
			assert_eq!(failing("schema"), Some(DebugValue::Gauge(1.0.into())));
			assert_eq!(failing("locked"), Some(DebugValue::Gauge(0.0.into())));

			crate::metrics::waker::record_successful_pass();
			assert_eq!(failing("schema"), Some(DebugValue::Gauge(0.0.into())));
		});
	}
}

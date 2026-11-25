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

use super::actuate::{save_unless_superseded, send, Claim};
use super::deadline::{Deadline, PassClock, TokioClock};
use super::propose::{decide_with_a_proposal, propose_a_session, Proposal};
use super::sweep::{catch_up, sweep_history, CatchUp};
use crate::nudge::constraints::StudyConstraints;
use crate::nudge::presence;
use crate::{AppState, NudgeContext};
use chrono::{DateTime, Utc};
use engagement_repo::EngagementRepository;
use intervention::{Charge, Engine, Verdict};
use outcome_repo::OutcomeRepository;
use publication_repo::{Publication, PublicationRepository};
use push_repo::{PushSubscriptionRepository, Topic};
use session_repo::SessionRepository;
use sqlx::SqlitePool;
use std::time::Duration;
use study_domain::{StudyCalibration, StudySelector, StudySignal, StudyV1};
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
	/// Subjects `consider` was run for, successfully or not, and saw through
	/// to its decision: whatever it decided, a subject it decided to notify
	/// had at least one delivery begin. A subject the deadline reached
	/// between its claim and its first delivery is not counted here but in
	/// `deferred` (#575).
	pub considered: usize,
	/// Of those, how many ended with a notification accepted somewhere.
	pub intervened: usize,
	/// Due subjects this pass fetched and never considered, because
	/// `NudgeContext::pass_deadline` ran out first. Still due — at most one was
	/// caught up to the curriculum epoch before the deadline struck, and at
	/// most one was claimed and handed its claim back because the deadline
	/// came before its first delivery — so the next pass picks them up.
	pub deferred: usize,
	/// Whether the pass ran out of time — before a subject, or inside one's
	/// deliveries. Exactly the passes `nudge_waker_pass_deadline_exceeded_total`
	/// counts.
	pub deadline_exceeded: bool,
	/// Subjects behind the curriculum epoch this pass caught up — one
	/// `CurriculumUpdated` each (#273, CAT5); see [`catch_up`].
	pub caught_up: usize,
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
/// A subject is therefore never cut in half (#575). If the deadline reaches
/// one after its claim and before its first delivery, nothing was sent, so
/// `consider` hands the claim back and the subject counts as `deferred`
/// with the rest the pass didn't reach — never as `considered` with no
/// attempt behind it. That is the same deadline check, read once more; not a
/// second deadline (`docs/study-nudge.md`, "Scaling invariants", 4).
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
	run_pass(db, nudge, &Deadline::new(TokioClock::start(), nudge.pass_deadline)).await
}

/// [`run_once`] against `deadline`'s clock.
pub(crate) async fn run_pass<C: PassClock>(db: &SqlitePool, nudge: &NudgeContext, deadline: &Deadline<C>) -> Result<PassReport, sqlx::Error> {
	let engagement = EngagementRepository::new(db.clone());
	let now = Utc::now();
	// New material first, so this pass's `due` already reads the epoch it
	// moved to: one append per publication, whatever the number of subjects.
	let newest = newest_publication(db, now, deadline).await;
	let mut report = PassReport::default();
	// Detection is storage work before the first subject; a pass it ran out
	// of time starts nothing more (a real `chatgpt-codex-connector` finding on
	// #362). Nothing was read, so nothing is deferred: the next pass reads it.
	if deadline.passed() {
		warn!(deadline_ms = nudge.pass_deadline.as_millis(), "waker pass reached its deadline before reading due subjects");
		report.deadline_exceeded = true;
		crate::metrics::waker::record_deadline_exceeded();
		return Ok(report);
	}
	let epoch = newest.as_ref().map_or(0, |publication| publication.id);
	let due = engagement.due(&now.to_rfc3339(), epoch, BATCH).await?;
	crate::metrics::waker::record_due(due.len());

	debug!(count = due.len(), epoch, "subjects the arithmetic marked eligible, or behind the curriculum epoch");

	for (reached, gate) in due.iter().enumerate() {
		if deadline.passed() {
			report.deferred += due.len() - reached;
			warn!(
				considered = report.considered,
				deferred = report.deferred,
				deadline_ms = nudge.pass_deadline.as_millis(),
				"waker pass reached its deadline; the remaining due subjects are left for the next pass"
			);
			break;
		}

		// Behind the epoch (#273, CAT5): fold the new material in first, so
		// `consider` decides on the drained charge. A subject the drain leaves
		// ineligible is done for this pass — the fold re-solved `eligible_at`,
		// and the arithmetic, not this loop, decides when they are next due.
		if let Some(publication) = newest.as_ref().filter(|_| gate.curriculum_epoch < epoch) {
			let due_by_time = match catch_up(db, &gate.subject_id, gate.curriculum_epoch, publication).await {
				// Against the clock now, not the pass's `now`: a drain that
				// makes them eligible at once solves `eligible_at` to the
				// catch-up's own instant, which is later than the pass began
				// (a real `chatgpt-codex-connector` finding on #362).
				Ok(CatchUp::Drained(eligible_at)) => {
					report.caught_up += 1;
					eligible_at <= Utc::now()
				}
				// Nothing they missed applies to them, or someone else caught
				// them up between `due` and here: due only if they already were.
				Ok(CatchUp::NothingApplies | CatchUp::AlreadyCaughtUp) => crate::nudge::clock::parse_timestamp(&gate.eligible_at).is_none_or(|at| at <= now),
				Err(err) => {
					error!(subject = %gate.subject_id, error = %err, "could not catch a subject up to the curriculum epoch; they stay behind for the next pass");
					crate::metrics::waker::record_verdict("storage_error", "n/a");
					false
				}
			};
			if !due_by_time {
				continue;
			}
			// The catch-up is this subject's storage work; `consider` is more
			// of it and may provision a session. Neither is started past the
			// deadline — the subject stays due and the next pass considers
			// them (a real `chatgpt-codex-connector` finding on #362).
			if deadline.passed() {
				report.deferred += due.len() - reached;
				warn!(
					considered = report.considered,
					deferred = report.deferred,
					deadline_ms = nudge.pass_deadline.as_millis(),
					"waker pass reached its deadline catching a subject up; they and the rest are left for the next pass"
				);
				break;
			}
		}

		match consider(db, nudge, &engagement, &gate.subject_id, deadline).await {
			Ok(Considered::Sent) => {
				report.considered += 1;
				report.intervened += 1;
			}
			Ok(Considered::NotSent) => report.considered += 1,
			// Handed back before anything was sent: as if never reached, and
			// the loop's next check ends the pass.
			Ok(Considered::Deferred) => report.deferred += 1,
			Err(err) => {
				report.considered += 1;
				error!(subject = %gate.subject_id, error = %err, "could not consider a due subject");
				crate::metrics::waker::record_verdict("storage_error", "n/a");
			}
		}
	}

	// Strikes held through the pass are taken now, if home was heard (#1180).
	// A write that fails is logged and the pass goes on: a strike is
	// bookkeeping, and the pass's own report and sweep come first.
	if let Err(err) = crate::nudge::actuate::settle_strikes(db, deadline).await {
		error!(error = %err, "could not take the strikes this pass held; the browsers are struck again next time they fail");
	}

	// After the loop, not only at the pre-subject check above: a deadline
	// that runs out inside the last (or only) subject's deliveries truncates
	// the pass just as much, and would otherwise go uncounted — a real
	// `chatgpt-codex-connector` finding on #357.
	if report.deferred > 0 || deadline.passed() {
		report.deadline_exceeded = true;
		crate::metrics::waker::record_deadline_exceeded();
	} else {
		// Housekeeping after the work people are waiting on, and only if the
		// pass still has time: a pass that ran out has already left due
		// subjects for the next one, and the sweep can wait with them.
		report.pruned = sweep_history(&engagement, &OutcomeRepository::new(db.clone()), now).await;
	}

	Ok(report)
}

/// Append newly published catalogue material to the log and return its newest
/// entry, whose `id` is the curriculum epoch (#273, CAT5).
///
/// This is the whole of publishing: O(catalogue) — bounded by
/// `ACTIVITY_DETECTION_CEILING`, and refused over it — never O(subjects). Who
/// the material reaches is each subject's watermark falling behind the epoch,
/// which `due` reads and [`catch_up`] closes; see `docs/study-nudge.md`,
/// "Scaling invariants".
///
/// Failures are logged and read as "nothing new", never propagated: a pass
/// that cannot read the log still serves the subjects the arithmetic marked
/// due, and the new material waits for a pass that can. A detection that runs
/// the pass out of time does not start the epoch read; `run_once` then ends
/// the pass.
async fn newest_publication<C: PassClock>(db: &SqlitePool, now: DateTime<Utc>, deadline: &Deadline<C>) -> Option<Publication> {
	let publications = PublicationRepository::new(db.clone());
	match publications.detect_activity_publications(&now.to_rfc3339()).await {
		Ok(0) => {}
		Ok(found) => info!(found, "new catalogue material detected"),
		Err(err) => error!(error = %err, "could not detect catalogue publications"),
	}
	if deadline.passed() {
		return None;
	}
	// #277 (CUR4): lessons append to the same log — a second producer, not a
	// second mechanism.
	match publications.detect_curriculum_publications(&now.to_rfc3339()).await {
		Ok(0) => {}
		Ok(found) => info!(found, "new lesson material detected"),
		Err(err) => error!(error = %err, "could not detect lesson publications"),
	}
	if deadline.passed() {
		return None;
	}
	match publications.newest().await {
		Ok(newest) => newest,
		Err(err) => {
			error!(error = %err, "could not read the curriculum epoch; nobody is caught up this pass");
			None
		}
	}
}

/// How [`consider`] left a subject.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Considered {
	/// A notification was accepted somewhere.
	Sent,
	/// Decided and done for this pass, with nothing accepted: not eligible,
	/// suppressed, the claim lost, or every delivery tried and refused or
	/// timed out.
	NotSent,
	/// Claimed, then the pass deadline came before any device was tried
	/// (#575): nothing was sent, the claim is handed back, and the subject is
	/// due next pass exactly as if this one never reached it.
	Deferred,
}

/// Evaluate one subject, and act if the engine says so.
async fn consider<C: PassClock>(
	db: &SqlitePool,
	nudge: &NudgeContext,
	engagement: &EngagementRepository,
	subject_id: &str,
	deadline: &Deadline<C>,
) -> Result<Considered, sqlx::Error> {
	let now = Utc::now();

	let stored = engagement.charge(subject_id).await?;
	// The version every save below is conditional on — see
	// `EngagementRepository::save_if_unchanged`.
	let read_as_of = stored.first().map(|row| row.as_of.clone());
	#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
	let levels: Vec<(u16, f64)> = stored.iter().map(|row| (row.class as u16, row.level)).collect();
	let as_of = stored.first().map_or(now, |row| crate::nudge::clock::parse_timestamp(&row.as_of).unwrap_or(now));
	let mut charge = Charge::<StudyV1>::from_storage::<StudyCalibration>(&levels, as_of);

	// A browser set aside and probed within the day is not sent to, so it
	// gives no consent this pass: a subject with only such browsers is not
	// claimed (#1180), and its verdict reads `not-consented`. One whose daily
	// probe is due is claimed, and the nudge that is due anyway is its probe.
	let mut subscriptions = PushSubscriptionRepository::new(db.clone()).for_subject(subject_id).await?;
	subscriptions.retain(|sub| crate::strikes::turn(sub.set_aside_at.as_deref(), sub.last_probe_at.as_deref(), now) != crate::strikes::Turn::Skip);
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
			return Ok(Considered::NotSent);
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
						save_unless_superseded(&nudge.deletion, engagement, subject_id, read_as_of.as_deref(), &levels, as_of, retry).await?;
					}
					return Ok(Considered::NotSent);
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
			save_unless_superseded(&nudge.deletion, engagement, subject_id, read_as_of.as_deref(), &levels, as_of, until).await?;
			return Ok(Considered::NotSent);
		}
		Verdict::Suppressed { reason, retry_at } => {
			info!(subject = %subject_id, reason = reason.as_str(), "warranted but not admissible");
			crate::metrics::waker::record_verdict("suppressed", reason.as_str());
			let (levels, as_of) = charge.to_storage();
			save_unless_superseded(&nudge.deletion, engagement, subject_id, read_as_of.as_deref(), &levels, as_of, retry_at).await?;
			return Ok(Considered::NotSent);
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
			save_unless_superseded(&nudge.deletion, engagement, subject_id, read_as_of.as_deref(), &levels, as_of, retry).await?;
			return Ok(Considered::NotSent);
		}
	};

	// Claim before sending. A crash between the two costs this intervention
	// rather than duplicating it, which is the right way round.
	let next_eligible = engine.intervened(&mut charge, now);
	let (levels, as_of) = charge.to_storage();
	let claim = Claim {
		action,
		now,
		next_eligible,
		read_as_of,
		levels,
		as_of,
	};
	send(db, nudge, engagement, subject_id, deadline, claim).await
}

#[cfg(test)]
mod tests;

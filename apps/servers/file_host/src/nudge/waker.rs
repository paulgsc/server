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
use session_repo::{SessionRepository, SessionStatus};
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
pub async fn run_once(db: &SqlitePool, ws: &WebSocketFsm, nudge: &NudgeContext) -> Result<usize, sqlx::Error> {
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
	let sessions = match SessionRepository::new(db.clone()).list(subject_id).await {
		Ok(sessions) => sessions,
		Err(err) => {
			error!(subject = %subject_id, error = %err, "could not read sessions; skipping this subject rather than guessing whether one is prepared");
			return Ok(false);
		}
	};
	// #262 (SLI1): the measurement, not the fix. `list()` is scoped to this
	// subject as of #260 (SUB2) — it can no longer return another subject's
	// rows — but it is still unbounded *within* that scope, so this is still
	// this subject's full session count, not the one row `consider` actually
	// needs. See `metrics::waker::record_session_rows_read`. #263 (SLI2)
	// replaces this call with a purpose-built `first_prepared` lookup.
	crate::metrics::waker::record_session_rows_read(sessions.len());
	let prepared_session = sessions
		.iter()
		.find(|session| matches!(session.status, SessionStatus::Scheduled | SessionStatus::Paused | SessionStatus::Draft))
		.map(|session| session.id.clone());

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
			// Depleted, admissible, and no action fits. Plain absence — the
			// one deficit with a sessionless answer — now gets `GetStarted`
			// even with nothing prepared, so reaching this arm means the
			// dominant deficit is Momentum, Mastery, or Freshness with
			// nothing to resume, review, or announce. A gap in the domain,
			// worth saying so.
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
			matches!(verdict, Verdict::Suppressed { reason: Suppressed::NotConsented, .. }),
			"got {verdict:?}, expected NotConsented suppression rather than NothingToSay"
		);
	}

	/// #262 (SLI1): the measurement, not the fix (that's #263/SLI2). This is a
	/// **characterisation test**, not an `#[ignore]`d assertion of a
	/// not-yet-decided target bound — #262's acceptance criteria offer either,
	/// and this is the choice, for two reasons: #263 has not picked its bound
	/// yet, so a target number here would be invented; and a test that
	/// actually runs in CI is a stronger regression guard between now and
	/// #263 than one nobody runs. When #263 lands, this assertion should
	/// invert to a small constant — not be deleted, since "the waker's read
	/// cost stays flat as the table grows" is exactly the invariant worth
	/// keeping under test forever after.
	///
	/// Seeds more due subjects than `BATCH` so the assertion exercises the
	/// cap, not just proportionality: a fix that read once per due subject
	/// (rather than once per subject *and* per row it owns) would already
	/// land far below this number, and this test would need rewriting —
	/// which is the point of committing it now.
	///
	/// Each due subject owns its own `SESSIONS_PER_SUBJECT` sessions, seeded
	/// through `SessionRepository::upsert` rather than a shared pool of rows
	/// under one id. Before #260 (SUB2), `list()` had no `WHERE subject_id`
	/// clause at all, so every subject's pass genuinely re-read the same
	/// whole table; #260 scoped that query, so a table shared across subjects
	/// would no longer demonstrate the defect this test exists to catch — a
	/// query that provably cannot see another subject's rows is not
	/// "unbounded" in the sense SLI1 means. What's still true, and still
	/// worth a test until #263, is that each subject's *own* read stays
	/// unbounded within that scope.
	#[test]
	fn one_pass_reads_each_due_subjects_entire_session_set_once() {
		use metrics_util::debugging::{DebugValue, DebuggingRecorder};
		use metrics_util::CompositeKey;
		use push_kit::{ReqwestTransport, Sender, VapidIdentity};
		use session_repo::{LayoutMode, SessionRecord};
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
		let expected = BATCH.min(DUE_SUBJECTS) as u64 * SESSIONS_PER_SUBJECT as u64;
		assert_eq!(
			rows_read,
			Some(expected),
			"today's waker reads a due subject's entire owned session set once per pass — \
			 BATCH.min(DUE_SUBJECTS) × SESSIONS_PER_SUBJECT = {expected} rows. If this now fails \
			 because the count is *lower*, #263 (SLI2) landed: rewrite this assertion to the new \
			 bound instead of deleting the test, since a flat read cost is the invariant worth \
			 keeping."
		);
	}
}

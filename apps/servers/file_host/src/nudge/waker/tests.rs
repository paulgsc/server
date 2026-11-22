use super::*;
use crate::nudge::actuate::unless_deleted;
use crate::nudge::clock::NudgeClock;
use crate::nudge::constraints::{StudyConstraints, Suppressed};
use crate::nudge::observe::{first_contact, observe};
use crate::nudge::presence::PresenceLeases;
use crate::nudge::propose::{materialize_provisioned_session, ranking_inputs, RankingInputs};
use activity_repo::{default_session_name, provision, recommend, total_duration_ms, ActivityOutcome, ActivityRecord, ActivityRepository, DEFAULT_RECOMMENDATION_COUNT};
use chrono::{Duration, TimeZone as _};
use engagement_repo::{INTERVENTION_LOG_RETENTION_DAYS, RETENTION_SWEEP_LIMIT};
use outcome_repo::{ActivityStats, ACTIVITY_OUTCOME_RETENTION_DAYS};
use session_repo::{SessionOrigin, SessionRecord, SessionStatus};
use std::collections::HashSet;
use study_domain::StudyAction;

/// A deadline no test that calls `consider` directly will reach.
fn far_deadline() -> Deadline<TokioClock> {
	Deadline::new(TokioClock::start(), std::time::Duration::from_secs(3600))
}

/// A pass clock that moves only when a delivery's budget runs out, by
/// exactly that budget (#575): every push service in a test that uses it
/// is one that never answers, so where the pass deadline falls is decided
/// by arithmetic, not by how fast the runner is. The delivery is still
/// started, so it is polled once before its budget is spent.
#[derive(Default)]
struct SteppedClock {
	elapsed_ms: std::sync::atomic::AtomicU64,
}

impl PassClock for SteppedClock {
	fn elapsed(&self) -> std::time::Duration {
		std::time::Duration::from_millis(self.elapsed_ms.load(std::sync::atomic::Ordering::SeqCst))
	}

	async fn timeout<F: std::future::Future + Send>(&self, budget: std::time::Duration, work: F) -> Option<F::Output> {
		tokio::select! {
			biased;
			out = work => return Some(out),
			() = std::future::ready(()) => {}
		}
		self.elapsed_ms.fetch_add(u64::try_from(budget.as_millis()).unwrap(), std::sync::atomic::Ordering::SeqCst);
		None
	}
}

/// A pass over `nudge` on a stepped clock; the report and the time it took.
async fn stepped_pass(pool: &SqlitePool, nudge: &NudgeContext) -> (PassReport, std::time::Duration) {
	let deadline = Deadline::new(SteppedClock::default(), nudge.pass_deadline);
	let report = run_pass(pool, nudge, &deadline).await.unwrap();
	(report, deadline.clock.elapsed())
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
		let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();

		rt.block_on(async {
			// A single connection: SQLite's `:memory:` database is private
			// to the connection that opened it, so a pool free to open a
			// second one would silently hand some queries an empty,
			// unmigrated schema. This test has no concurrency to justify
			// more than one connection.
			let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
			MIGRATOR.run(&pool).await.unwrap();

			let now = Utc::now();
			let past = (now - Duration::hours(1)).to_rfc3339();
			let now_str = now.to_rfc3339();

			let sessions_repo = SessionRepository::new(pool.clone());
			for i in 0..DUE_SUBJECTS {
				let subject_id = numbered("subject-", i);
				EngagementRepository::new(pool.clone()).seed_if_absent(&subject_id, &[], &now_str, &past).await.unwrap();

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
					sessions_repo.upsert(&subject_id, &record).await.unwrap();
				}
			}

			let vapid = VapidIdentity::from_config(Some(VAPID_PRIVATE), Some(VAPID_PUBLIC), "mailto:test@example.com").unwrap();
			let nudge = NudgeContext {
				clock: NudgeClock::resolve(Some("UTC")).0,
				sender: std::sync::Arc::new(Sender::new(
					vapid.clone(),
					ReqwestTransport::new(outbound::client(outbound::Caller::Nudge).build().unwrap()),
				)),
				vapid,
				enabled: true,
				quiet_hours_start: 22,
				quiet_hours_end: 8,
				presence_lease_ttl: std::time::Duration::from_secs(75),
				base_url: "https://example.com".to_owned(),
				delivery_timeout: std::time::Duration::from_secs(10),
				pass_deadline: std::time::Duration::from_secs(120),
				recommender_uses_outcomes: false,
				deletion: crate::auth::DeletionLock::default(),
			};

			run_once(&pool, &nudge).await.unwrap();
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
#[allow(clippy::too_many_lines)] // one scenario, read top to bottom: seed, pass, then each thing it must have written
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
		let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();

		rt.block_on(async {
			let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
			MIGRATOR.run(&pool).await.unwrap();

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
			engagement.save(subject_id, &levels, &now_str, &now_str).await.unwrap();

			let vapid = VapidIdentity::from_config(Some(VAPID_PRIVATE), Some(VAPID_PUBLIC), "mailto:test@example.com").unwrap();
			let nudge = NudgeContext {
				clock: NudgeClock::resolve(Some("UTC")).0,
				sender: std::sync::Arc::new(Sender::new(
					vapid.clone(),
					ReqwestTransport::new(outbound::client(outbound::Caller::Nudge).build().unwrap()),
				)),
				vapid,
				enabled: true,
				quiet_hours_start: 22,
				quiet_hours_end: 8,
				presence_lease_ttl: std::time::Duration::from_secs(75),
				base_url: "https://example.com".to_owned(),
				delivery_timeout: std::time::Duration::from_secs(10),
				pass_deadline: std::time::Duration::from_secs(120),
				recommender_uses_outcomes: false,
				deletion: crate::auth::DeletionLock::default(),
			};

			let intervened = consider(&pool, &nudge, &engagement, subject_id, &far_deadline()).await.unwrap();
			assert_eq!(
				intervened,
				Considered::NotSent,
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
			let provisioned_id = sessions.first_prepared(subject_id).await.unwrap().unwrap();

			let record = sessions.get(subject_id, &provisioned_id).await.unwrap().unwrap();
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
			let second_pass = consider(&pool, &nudge, &engagement, subject_id, &far_deadline()).await.unwrap();
			assert_eq!(second_pass, Considered::NotSent, "still nothing consented to receive it");
			let all_sessions = sessions.list(subject_id).await.unwrap();
			assert_eq!(all_sessions.len(), 1, "provisioning must not run twice for the same subject");
			let still_the_same = sessions.first_prepared(subject_id).await.unwrap().unwrap();
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
			sender: std::sync::Arc::new(Sender::new(
				vapid.clone(),
				ReqwestTransport::new(outbound::client(outbound::Caller::Nudge).build().unwrap()),
			)),
			vapid,
			enabled: true,
			quiet_hours_start: 22,
			quiet_hours_end: 8,
			presence_lease_ttl: std::time::Duration::from_secs(75),
			base_url: "https://example.com".to_owned(),
			delivery_timeout: std::time::Duration::from_secs(10),
			pass_deadline: std::time::Duration::from_secs(120),
			recommender_uses_outcomes: false,
			deletion: crate::auth::DeletionLock::default(),
		};

		// First pass: nothing prepared, provisions a real session.
		consider(&pool, &nudge, &engagement, subject_id, &far_deadline()).await.unwrap();
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
		consider(&pool, &nudge, &engagement, subject_id, &far_deadline()).await.unwrap();

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
			sender: std::sync::Arc::new(Sender::new(
				vapid.clone(),
				ReqwestTransport::new(outbound::client(outbound::Caller::Nudge).build().unwrap()),
			)),
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
			deletion: crate::auth::DeletionLock::default(),
		};

		// First pass: nothing prepared, provisions a real proposal.
		consider(&pool, &nudge, &engagement, subject_id, &far_deadline()).await.unwrap();
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
		consider(&pool, &nudge, &engagement, subject_id, &far_deadline()).await.unwrap();

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
				sender: std::sync::Arc::new(Sender::new(
					vapid.clone(),
					ReqwestTransport::new(outbound::client(outbound::Caller::Nudge).build().unwrap()),
				)),
				vapid,
				enabled: true,
				quiet_hours_start: 0,
				quiet_hours_end: 0,
				presence_lease_ttl: std::time::Duration::from_secs(75),
				base_url: "https://example.com".to_owned(),
				delivery_timeout: std::time::Duration::from_secs(10),
				pass_deadline: std::time::Duration::from_secs(120),
				recommender_uses_outcomes: false,
				deletion: crate::auth::DeletionLock::default(),
			};

			// First pass: provisions the proposal.
			consider(&pool, &nudge, &engagement, subject_id, &far_deadline()).await.unwrap();
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
			let intervened = consider(&pool, &nudge, &engagement, subject_id, &far_deadline()).await.unwrap();
			assert_eq!(intervened, Considered::NotSent, "a subject actively viewing the proposal must not be notified about it");

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
			sender: std::sync::Arc::new(Sender::new(
				vapid.clone(),
				ReqwestTransport::new(outbound::client(outbound::Caller::Nudge).build().unwrap()),
			)),
			vapid,
			enabled: true,
			quiet_hours_start: 0,
			quiet_hours_end: 0,
			presence_lease_ttl: std::time::Duration::from_secs(75),
			base_url: "https://example.com".to_owned(),
			delivery_timeout: std::time::Duration::from_secs(10),
			pass_deadline: std::time::Duration::from_secs(120),
			recommender_uses_outcomes: false,
			deletion: crate::auth::DeletionLock::default(),
		};

		// First pass: provisions a real proposal, exactly as the sibling
		// tests do.
		consider(&pool, &nudge, &engagement, subject_id, &far_deadline()).await.unwrap();
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

		let intervened = consider(&pool, &nudge, &engagement, subject_id, &far_deadline()).await.unwrap();
		assert_eq!(intervened, Considered::NotSent, "a pass that loses the claim must not report having intervened");

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
			sender: std::sync::Arc::new(Sender::new(
				vapid.clone(),
				ReqwestTransport::new(outbound::client(outbound::Caller::Nudge).build().unwrap()),
			)),
			vapid,
			enabled: true,
			quiet_hours_start: 0,
			quiet_hours_end: 0,
			presence_lease_ttl: std::time::Duration::from_secs(75),
			base_url: "https://example.com".to_owned(),
			delivery_timeout: std::time::Duration::from_secs(10),
			pass_deadline: std::time::Duration::from_secs(120),
			recommender_uses_outcomes: false,
			deletion: crate::auth::DeletionLock::default(),
		};

		// First pass: provisions the proposal.
		consider(&pool, &nudge, &engagement, subject_id, &far_deadline()).await.unwrap();
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
		consider(&pool, &nudge, &engagement, subject_id, &far_deadline()).await.unwrap();

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
			sender: std::sync::Arc::new(Sender::new(
				vapid.clone(),
				ReqwestTransport::new(outbound::client(outbound::Caller::Nudge).build().unwrap()),
			)),
			vapid,
			enabled: true,
			quiet_hours_start: 22,
			quiet_hours_end: 8,
			presence_lease_ttl: std::time::Duration::from_secs(75),
			base_url: "https://example.com".to_owned(),
			delivery_timeout: std::time::Duration::from_secs(10),
			pass_deadline: std::time::Duration::from_secs(120),
			recommender_uses_outcomes: false,
			deletion: crate::auth::DeletionLock::default(),
		};

		let intervened = consider(&pool, &nudge, &engagement, subject_id, &far_deadline()).await.unwrap();
		assert_eq!(intervened, Considered::NotSent, "nothing was provisioned, so there is nothing to send");

		let sessions = SessionRepository::new(pool.clone());
		assert_eq!(
			sessions.first_prepared(subject_id).await.unwrap(),
			None,
			"an empty session must never be persisted as Scheduled — that would trap the subject permanently"
		);

		let still_due = engagement.due(&now_str, 0, BATCH).await.unwrap();
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
			sender: std::sync::Arc::new(Sender::new(
				vapid.clone(),
				ReqwestTransport::new(outbound::client(outbound::Caller::Nudge).build().unwrap()),
			)),
			vapid,
			enabled: true,
			quiet_hours_start: 22,
			quiet_hours_end: 8,
			presence_lease_ttl: std::time::Duration::from_secs(75),
			base_url: "https://example.com".to_owned(),
			delivery_timeout: std::time::Duration::from_secs(10),
			pass_deadline: std::time::Duration::from_secs(120),
			recommender_uses_outcomes: false,
			deletion: crate::auth::DeletionLock::default(),
		};

		let intervened = consider(&pool, &nudge, &engagement, subject_id, &far_deadline()).await.unwrap();
		assert_eq!(intervened, Considered::NotSent, "the catalogue is unreadable, so nothing can be provisioned or sent");

		let sessions = SessionRepository::new(pool.clone());
		assert_eq!(sessions.first_prepared(subject_id).await.unwrap(), None);

		let still_due = engagement.due(&now_str, 0, BATCH).await.unwrap();
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
				engagement.due(&now_str, 0, BATCH).await.unwrap().is_empty(),
				"and not due yet either: first contact seeds the charge full, so someone who installs the app at 9am is not interrupted at 9:05"
			);

			// WHEN the clock advances past the solved eligibility instant.
			rewind_first_contact_past_the_solved_instant(&engagement, subject_id, now).await;
			assert!(
				engagement.due(&now_str, 0, BATCH).await.unwrap().iter().any(|gate| gate.subject_id == subject_id),
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
	let transport = ReqwestTransport::new(outbound::client(outbound::Caller::Nudge).no_proxy().build().unwrap());

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
		deletion: crate::auth::DeletionLock::default(),
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

/// A push service that answers every request with `status`, a body that
/// echoes the request line (its per-browser path included) and a
/// `Retry-After` that does the same: the shape of a provider that puts the
/// URI it was sent into its own error text. Returns the endpoint.
fn echoing_push_service(status: u16, token: &str) -> String {
	use std::io::{Read as _, Write as _};

	let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
	let mut endpoint = numbered("http://", listener.local_addr().unwrap());
	endpoint.push_str("/wpush/v2/");
	endpoint.push_str(token);

	std::thread::spawn(move || {
		for stream in listener.incoming() {
			let Ok(mut stream) = stream else { continue };
			// The whole request, as `loopback_push_service` reads it, so the
			// client sees a complete exchange and not a reset.
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
			let request_line = String::from_utf8_lossy(&request).lines().next().unwrap_or_default().to_owned();
			let body = "rejected: ".to_owned() + &request_line;
			let response = "HTTP/1.1 ".to_owned()
				+ &status.to_string()
				+ " X\r\nRetry-After: "
				+ &request_line
				+ "\r\nContent-Length: "
				+ &body.len().to_string()
				+ "\r\nConnection: close\r\n\r\n"
				+ &body;
			let _ = stream.write_all(response.as_bytes());
			let _ = stream.flush();
		}
	});

	endpoint
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

/// The failures recorded against each of `subject_id`'s devices.
async fn failures(pool: &SqlitePool, subject_id: &str) -> Vec<i64> {
	PushSubscriptionRepository::new(pool.clone())
		.for_subject(subject_id)
		.await
		.unwrap()
		.iter()
		.map(|device| device.failure_count)
		.collect()
}

/// #264 (SLI3), #575: a pass stops at its deadline, between subjects; the
/// subjects it reached each had a whole delivery attempt and keep their
/// claim, and the subjects it never reached are untouched and still due.
///
/// Six due subjects, every one with a single device on a push service
/// that never answers. Each delivery is cut off at 200ms and the pass at
/// 300ms, on a clock that moves only when a delivery's budget runs out
/// (`SteppedClock`), so the split is exact: the first subject's delivery
/// takes its whole 200ms, the second's takes the 100ms the pass has left,
/// and the third is reached at 300ms, which is the deadline. Before #575
/// this ran on the wall clock, and where 300ms fell depended on the
/// runner (#572); the assertions had to allow the last considered
/// subject to be cut before its delivery.
#[test]
fn a_pass_that_reaches_its_deadline_leaves_the_subjects_it_never_reached_untouched_and_still_due() {
	use sqlx::sqlite::SqlitePoolOptions;

	const SUBJECTS: usize = 6;
	const REACHED: usize = 2;
	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");

	let (endpoint, _) = stalling_push_service();
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
		let due: Vec<String> = engagement.due(&now_str, 0, BATCH).await.unwrap().into_iter().map(|gate| gate.subject_id).collect();
		assert_eq!(due.len(), SUBJECTS, "every subject starts due");

		let nudge = NudgeContext {
			delivery_timeout: std::time::Duration::from_millis(200),
			pass_deadline: std::time::Duration::from_millis(300),
			..nudge_context()
		};
		let (report, took) = stepped_pass(&pool, &nudge).await;

		assert_eq!(
			report,
			PassReport {
				considered: REACHED,
				intervened: 0,
				deferred: SUBJECTS - REACHED,
				deadline_exceeded: true,
				caught_up: 0,
				pruned: 0,
			},
			"two whole deliveries fit the pass, the rest are deferred, and nothing was accepted"
		);
		assert_eq!(took, std::time::Duration::from_millis(300), "the pass ran exactly to its deadline");

		let still_due: Vec<String> = engagement.due(&now_str, 0, BATCH).await.unwrap().into_iter().map(|gate| gate.subject_id).collect();
		let sessions = SessionRepository::new(pool.clone());
		let (reached, deferred) = due.split_at(REACHED);
		for subject_id in reached {
			// The timeout kept the claim (see the next test).
			assert_eq!(intervention_rows(&pool, subject_id).await, (1, 0), "{subject_id}: claimed once, never confirmed");
			assert_eq!(failures(&pool, subject_id).await, vec![1], "{subject_id}: its one delivery was tried, and timed out");
			assert!(!still_due.contains(subject_id), "{subject_id}'s claim stands, so it is not retried into a duplicate");
		}
		for subject_id in deferred {
			assert_eq!(intervention_rows(&pool, subject_id).await, (0, 0), "{subject_id} was never reached, so never claimed");
			assert_eq!(failures(&pool, subject_id).await, vec![0], "{subject_id}'s device was never tried");
			assert!(sessions.list(subject_id).await.unwrap().is_empty(), "{subject_id} was never proposed to");
			assert!(still_due.contains(subject_id), "{subject_id} holds no claim, so the next pass must still find it");
		}
	});
}

/// #575: a subject the deadline reaches after its claim and before its
/// first delivery is deferred, not considered: nothing was sent, so its
/// claim is handed back, no device is recorded as failing, and it is due
/// for the next pass.
///
/// On a pass this is the moment between the deadline check before a
/// subject and the first delivery, which only storage work fills; here
/// `consider` is given a deadline that has already passed.
#[test]
fn a_subject_the_deadline_reaches_before_its_first_delivery_is_handed_back_and_deferred() {
	use sqlx::sqlite::SqlitePoolOptions;

	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");

	let subject_id = "subject-cut-before-delivery";
	let (endpoint, _) = stalling_push_service();
	let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();

	rt.block_on(async {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();

		let now = Utc::now();
		let now_str = now.to_rfc3339();
		let engagement = EngagementRepository::new(pool.clone());
		subscribe(&pool, subject_id, &endpoint, &now_str).await;
		first_contact(&pool, subject_id).await.unwrap();
		rewind_first_contact_past_the_solved_instant(&engagement, subject_id, now).await;

		let passed = Deadline::new(SteppedClock::default(), std::time::Duration::ZERO);
		let considered = consider(&pool, &nudge_context(), &engagement, subject_id, &passed).await.unwrap();

		assert_eq!(considered, Considered::Deferred);
		assert_eq!(failures(&pool, subject_id).await, vec![0], "no device was tried, so none is recorded as failing");
		assert_eq!(intervention_rows(&pool, subject_id).await, (0, 0), "the claim was handed back");
		// Due from the moment the claim went back, which is after `now`.
		let still_due: Vec<String> = engagement
			.due(&Utc::now().to_rfc3339(), 0, BATCH)
			.await
			.unwrap()
			.into_iter()
			.map(|gate| gate.subject_id)
			.collect();
		assert_eq!(still_due, vec![subject_id.to_owned()], "so the next pass finds them");
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

/// docs/identity.md, invariant 14: a push endpoint is a per-browser
/// address, so no delivery outcome logs one.
///
/// Six devices, six outcomes that each log: a service that never answers
/// (a timeout), a port nothing listens on (a refused connection, whose
/// `reqwest` error names the URL it failed on unless the adapter strips
/// it), one that accepts, and three that refuse in their own words (a 400,
/// a 503 and a 429), each echoing the request line into its body and its
/// `Retry-After` the way a provider that quotes the URI would. Every
/// endpoint carries a path that is the per-browser part; none of it may
/// appear in anything captured.
#[test]
fn no_push_endpoint_reaches_a_log_line_whatever_the_delivery_outcome() {
	use sqlx::sqlite::SqlitePoolOptions;

	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");

	// What the production subscriber lets through, whatever `RUST_LOG` asks for
	// (`auth::loggable`), and not the HTTP client's own connection logs, which
	// the control below shows would name every endpoint's host.
	let (captured, _guard) = crate::privacy::capture_where(crate::auth::loggable);
	let subject_id = "subject-three-devices";
	let (stalled, _) = stalling_push_service();
	let (answering, _) = loopback_push_service();
	let refused = {
		let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
		let mut endpoint = numbered("http://", listener.local_addr().unwrap());
		endpoint.push_str("/wpush/v2/refused-device-token");
		drop(listener);
		endpoint
	};
	let quoting = [
		echoing_push_service(400, "bad-request-device-token"),
		echoing_push_service(503, "unavailable-device-token"),
		echoing_push_service(429, "rate-limited-device-token"),
	];
	let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();

	rt.block_on(async {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();

		let now = Utc::now();
		let now_str = now.to_rfc3339();
		let engagement = EngagementRepository::new(pool.clone());
		for endpoint in [&stalled, &refused, &answering].into_iter().chain(&quoting) {
			subscribe(&pool, subject_id, endpoint, &now_str).await;
		}
		first_contact(&pool, subject_id).await.unwrap();
		rewind_first_contact_past_the_solved_instant(&engagement, subject_id, now).await;

		let nudge = NudgeContext {
			delivery_timeout: std::time::Duration::from_millis(200),
			..nudge_context()
		};
		let pass = run_once(&pool, &nudge).await.unwrap();
		assert_eq!(pass.intervened, 1, "the accepting device made this an intervention: {pass:?}");
	});

	let lines = captured.lines();
	// Without these the test passes vacuously: each outcome must have
	// logged, and logged the service, for "no endpoint" to mean anything.
	for expected in ["did not answer in time", "push was not accepted", "service=other"] {
		assert!(lines.iter().any(|line| line.contains(expected)), "no line said {expected:?}: {lines:#?}");
	}
	for outcome in ["transport-error", "rejected", "service-error", "rate-limited"] {
		assert!(
			lines.iter().any(|line| line.contains(&("outcome=".to_owned() + outcome))),
			"no device reached the {outcome} path: {lines:#?}"
		);
	}
	for line in &lines {
		for endpoint in [&stalled, &refused, &answering].into_iter().chain(&quoting) {
			let per_browser_path = &endpoint[endpoint.find("/wpush").unwrap()..];
			assert!(!line.contains(endpoint.as_str()), "an endpoint reached a log line: {line}");
			assert!(!line.contains("127.0.0.1"), "a push endpoint's host reached a log line: {line}");
			assert!(!line.contains(per_browser_path), "a per-browser endpoint path reached a log line: {line}");
		}
	}
}

/// The control for `auth::loggable`'s HTTP-client entries: with no filter,
/// the client's own debug and trace logs name the host and port of every
/// connection (a push endpoint's host, for a delivery), and with the
/// production filter they do not. If the first half fails, the library
/// stopped logging the host and the filter entry may no longer be needed.
#[test]
fn the_http_clients_connection_logs_name_the_host_unless_the_production_filter_drops_them() {
	fn lines_for_one_connection(keep: fn(&tracing::Metadata<'_>) -> bool) -> Vec<String> {
		let (captured, _guard) = crate::privacy::capture_where(keep);
		let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
		let url = numbered("http://", listener.local_addr().unwrap());
		// Accepts and drops: a refused-after-connect request is all that is needed.
		std::thread::spawn(move || {
			let _ = listener.accept();
		});
		let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
		rt.block_on(async {
			let _ = outbound::client(outbound::Caller::Nudge).no_proxy().build().unwrap().get(&url).send().await;
		});
		captured.lines()
	}

	let everything = lines_for_one_connection(|_| true);
	assert!(
		everything.iter().any(|line| line.contains("127.0.0.1")),
		"the client no longer logs the host it connects to: {everything:#?}"
	);
	let production = lines_for_one_connection(crate::auth::loggable);
	assert!(
		!production.iter().any(|line| line.contains("127.0.0.1")),
		"the production filter let the client's connection log through: {production:#?}"
	);
}

/// #264 (SLI3), from a `chatgpt-codex-connector` finding on #357: the pass
/// deadline also bounds the deliveries *inside* one subject.
///
/// A subject's device list is unbounded, so checking the deadline only
/// between subjects let one subject with many stalled devices hold a pass
/// for `devices × delivery_timeout`. Here one subject has five devices on a
/// push service that never answers, a ten-second delivery timeout, and a
/// 300ms pass, on a clock that moves only when a delivery's budget runs out
/// (`SteppedClock`). The first delivery is cut off at what is left of the
/// pass, exactly 300ms, the other four are never tried, and the claim is
/// kept, because the one delivery that was tried is ambiguous.
#[test]
fn the_pass_deadline_cuts_short_one_subjects_deliveries_not_only_the_batch() {
	use sqlx::sqlite::SqlitePoolOptions;

	const DEVICES: usize = 5;
	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");

	let subject_id = "subject-many-stalled-devices";
	let (endpoint, _) = stalling_push_service();
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
		let (report, took) = stepped_pass(&pool, &nudge).await;

		assert_eq!(took, std::time::Duration::from_millis(300), "the pass ends at its deadline, not after 5 × 10s");
		assert_eq!(report.considered, 1, "{report:?}");
		assert!(report.deadline_exceeded, "a deadline reached inside the only subject still counts: {report:?}");
		assert_eq!(
			intervention_rows(&pool, subject_id).await,
			(1, 0),
			"the one tried delivery is ambiguous, so the claim stands unconfirmed"
		);
		let mut failed = failures(&pool, subject_id).await;
		failed.sort_unstable();
		assert_eq!(failed, vec![0, 0, 0, 0, 1], "only the first device was tried; one never tried is not recorded as failing");
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

/// A pass writes for subjects it read earlier, with push deliveries in
/// between (a provisioned `sessions` draft checks nothing about the
/// account). `unless_deleted` makes the write wait out a deletion in
/// progress and skips it for a subject the deletion removed.
/// `docs/identity.md` invariant 9.
#[tokio::test]
async fn a_write_for_a_deleted_subject_is_skipped_and_waits_out_a_deletion() {
	use crate::auth::DeletionLock;

	let pool = migrated_pool().await;
	let engagement = EngagementRepository::new(pool.clone());
	let deletion = DeletionLock::default();
	sqlx::query("INSERT INTO engagement_gate (subject_id, eligible_at, intervention_count, curriculum_epoch) VALUES ('subject-kept', 'now', 0, 0)")
		.execute(&pool)
		.await
		.unwrap();

	assert_eq!(unless_deleted(&engagement, &deletion, "subject-kept", async { 1 }).await.unwrap(), Some(1));
	assert_eq!(
		unless_deleted(&engagement, &deletion, "subject-deleted", async { 1 }).await.unwrap(),
		None,
		"no gate row: the account is gone"
	);

	let deleting = deletion.exclude().await;
	let waiting = tokio::spawn({
		let (engagement, deletion) = (EngagementRepository::new(pool.clone()), deletion.clone());
		async move { unless_deleted(&engagement, &deletion, "subject-kept", async { 1 }).await.unwrap() }
	});
	tokio::time::sleep(std::time::Duration::from_millis(100)).await;
	assert!(!waiting.is_finished(), "the write waits for the deletion to commit");
	// The deletion removes the gate row before it lets go.
	sqlx::query("DELETE FROM engagement_gate").execute(&pool).await.unwrap();
	drop(deleting);
	assert_eq!(waiting.await.unwrap(), None, "and then finds the subject gone");
}

async fn migrated_pool() -> SqlitePool {
	use sqlx::sqlite::SqlitePoolOptions;

	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");
	let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
	MIGRATOR.run(&pool).await.unwrap();
	pool
}

/// #273 (CAT5): the catalogue that exists at deploy is the baseline; a new
/// activity drains freshness once for every subject the nudge already
/// knew; a second pass drains nobody again, because the watermark moved
/// with the drain; and a version bump is news again.
#[tokio::test]
async fn a_publication_drains_everyone_known_before_it_once() {
	let pool = migrated_pool().await;
	first_contact(&pool, "subject-a").await.unwrap();
	first_contact(&pool, "subject-b").await.unwrap();
	let nudge = nudge_context();

	assert_eq!(run_once(&pool, &nudge).await.unwrap().caught_up, 0, "the seeded catalogue is not new to anyone");
	let before = freshness(&pool, "subject-a").await.unwrap();

	publish(&pool, "new-thing", 1).await;
	assert_eq!(run_once(&pool, &nudge).await.unwrap().caught_up, 2, "both were known before the publication");
	let after_one = freshness(&pool, "subject-a").await.unwrap();
	assert!(before - after_one > 30.0, "CurriculumUpdated drains freshness by 35: {before} -> {after_one}");

	assert_eq!(run_once(&pool, &nudge).await.unwrap().caught_up, 0, "caught up is caught up");
	let after_rerun = freshness(&pool, "subject-a").await.unwrap();
	assert!((after_one - after_rerun).abs() < 0.01, "a second pass drains nothing: {after_one} vs {after_rerun}");
	let newest = PublicationRepository::new(pool.clone()).newest().await.unwrap().unwrap();
	assert!(
		catch_up(&pool, "subject-a", 0, &newest).await.unwrap() == CatchUp::AlreadyCaughtUp,
		"and a direct catch-up at the same epoch writes nothing"
	);

	publish(&pool, "new-thing", 2).await;
	assert_eq!(run_once(&pool, &nudge).await.unwrap().caught_up, 2, "a version bump is new material again");
}

/// #273 (CAT5): the audience is fixed by each subject's own watermark — a
/// subject the nudge first learns of after a publication is stamped with
/// its epoch and is never behind it, while one known before is drained.
#[tokio::test]
async fn a_subject_known_only_after_a_publication_is_not_drained_by_it() {
	let pool = migrated_pool().await;
	let nudge = nudge_context();
	first_contact(&pool, "subject-early").await.unwrap();
	publish(&pool, "new-thing", 1).await;
	assert_eq!(run_once(&pool, &nudge).await.unwrap().caught_up, 1, "the early subject");

	first_contact(&pool, "subject-late").await.unwrap();
	observe(&pool, "subject-late-by-signal", &StudySignal::SessionStarted { session_id: "s".to_owned() })
		.await
		.unwrap();
	assert_eq!(run_once(&pool, &nudge).await.unwrap().caught_up, 0, "neither late subject is behind");
	assert_eq!(freshness(&pool, "subject-late").await, Some(100.0), "first contact starts full and stays full");
	let epoch = PublicationRepository::new(pool.clone()).newest().await.unwrap().unwrap().id;
	for late in ["subject-late", "subject-late-by-signal"] {
		let gate = EngagementRepository::new(pool.clone()).gate(late).await.unwrap().unwrap();
		assert_eq!(gate.curriculum_epoch, epoch, "{late} starts at the current epoch");
	}
}

/// #273 (CAT5): however many publications a subject missed, catching up is
/// one drain — the semantic signal is "there is new material".
#[tokio::test]
async fn several_missed_publications_cause_one_drain() {
	let pool = migrated_pool().await;
	let nudge = nudge_context();
	first_contact(&pool, "subject-away").await.unwrap();
	let before = freshness(&pool, "subject-away").await.unwrap();

	publish(&pool, "new-thing", 1).await;
	publish(&pool, "other-thing", 1).await;
	publish(&pool, "third-thing", 1).await;
	assert_eq!(run_once(&pool, &nudge).await.unwrap().caught_up, 1);
	let after = freshness(&pool, "subject-away").await.unwrap();
	assert!((before - after - 35.0).abs() < 1.0, "one drain of 35, not three: {before} -> {after}");
}

/// #273 (CAT5): behind subjects are bounded by what already bounds a pass —
/// `BATCH` — and later passes finish the rest, each exactly once. There is
/// no separate fan-out loop, cap, or cursor.
#[tokio::test]
async fn behind_subjects_beyond_a_batch_are_finished_by_later_passes() {
	const EXTRA: i64 = 5;
	let pool = migrated_pool().await;
	let nudge = nudge_context();
	for i in 0..BATCH + EXTRA {
		first_contact(&pool, &numbered("subject-", i)).await.unwrap();
	}
	publish(&pool, "new-thing", 1).await;

	#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
	let (batch, extra) = (BATCH as usize, EXTRA as usize);
	assert_eq!(run_once(&pool, &nudge).await.unwrap().caught_up, batch, "one pass reaches one batch");
	assert_eq!(run_once(&pool, &nudge).await.unwrap().caught_up, extra, "the next reaches the rest, and only the rest");
	assert_eq!(run_once(&pool, &nudge).await.unwrap().caught_up, 0, "then nobody is behind");
}

/// #273 (CAT5): a subject the drain makes eligible is considered in the
/// same pass that caught them up, not left for the next (from a
/// `chatgpt-codex-connector` finding on #362).
#[tokio::test]
async fn a_catch_up_that_makes_a_subject_eligible_is_considered_in_the_same_pass() {
	let pool = migrated_pool().await;
	let nudge = nudge_context();
	let subject_id = "subject-almost-drifted";
	// Just above `THRESHOLD`; draining freshness by 35 takes it under — see
	// `a_publication_makes_new_material_the_thing_to_say`.
	let now = Utc::now();
	let levels: Vec<(u16, f64)> = vec![(1, 61.0), (2, 44.0), (3, 22.0), (4, 35.0)];
	EngagementRepository::new(pool.clone())
		.save(subject_id, &levels, &now.to_rfc3339(), &(now + Duration::days(1)).to_rfc3339())
		.await
		.unwrap();
	publish(&pool, "new-thing", 1).await;

	let report = run_once(&pool, &nudge).await.unwrap();
	assert_eq!(report.caught_up, 1, "{report:?}");
	assert_eq!(report.considered, 1, "eligible after the drain, so considered now: {report:?}");
}

/// #273 (CAT5): detection is storage work before the first subject; a pass
/// already out of time after it reads nothing more — no epoch, no `due` —
/// and the next pass with time catches everyone up (from a
/// `chatgpt-codex-connector` finding on #362).
#[tokio::test]
async fn a_pass_out_of_time_after_detection_starts_no_more_queries() {
	let pool = migrated_pool().await;
	first_contact(&pool, "subject-a").await.unwrap();
	// Already due by time, so a `due` read would return them.
	let now = Utc::now();
	let full: Vec<(u16, f64)> = vec![(1, 100.0), (2, 100.0), (3, 100.0), (4, 100.0)];
	EngagementRepository::new(pool.clone())
		.save("subject-a", &full, &now.to_rfc3339(), &(now - Duration::days(1)).to_rfc3339())
		.await
		.unwrap();
	publish(&pool, "new-thing", 1).await;

	let out_of_time = NudgeContext {
		pass_deadline: std::time::Duration::ZERO,
		..nudge_context()
	};
	let report = run_once(&pool, &out_of_time).await.unwrap();
	assert!(report.deadline_exceeded, "{report:?}");
	assert_eq!(
		(report.caught_up, report.considered, report.deferred),
		(0, 0, 0),
		"`due` never ran, so there was nothing to reach or defer: {report:?}"
	);

	assert_eq!(run_once(&pool, &nudge_context()).await.unwrap().caught_up, 1, "the next pass with time");
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
	let newest = newest_publication(&pool, Utc::now(), &far_deadline()).await.unwrap();
	let watermark = EngagementRepository::new(pool.clone()).gate(subject_id).await.unwrap().unwrap().curriculum_epoch;
	assert!(
		matches!(catch_up(&pool, subject_id, watermark, &newest).await.unwrap(), CatchUp::Drained(_)),
		"known before the publication, so behind it"
	);

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

/// One `activity_outcome` row for `subject_id` on `activity_id` — what puts
/// them in `LESSON_AUDIENCE` for that activity's lessons (#277), unless the
/// block was skipped.
async fn played(pool: &SqlitePool, subject_id: &str, activity_id: &str, outcome: &str) {
	let mut session = String::from("session-played-");
	session.push_str(subject_id);
	session.push_str(activity_id);
	sqlx::query!(
		"INSERT INTO activity_outcome (subject_id, session_id, activity_id, block_index, started_at, ended_at, planned_ms, elapsed_ms, outcome, score) VALUES (?, ?, ?, 0, '2026-09-20T10:00:00+00:00', '2026-09-20T10:05:00+00:00', 300000, 300000, ?, NULL)",
		subject_id,
		session,
		activity_id,
		outcome
	)
	.execute(pool)
	.await
	.unwrap();
}

/// A `topiks`-shaped corpus directory with one lesson per `(key, body)`.
#[allow(clippy::disallowed_methods)] // a fixture file, not a tracing argument
fn write_topiks(dir: &std::path::Path, lessons: &[(&str, &str, &[u8])]) {
	let entries: Vec<serde_json::Value> = lessons
		.iter()
		.map(|(key, name, _)| serde_json::json!({ "key": key, "displayName": name, "description": "d", "batchCount": 1, "totalQuestions": 1, "totalMessages": 1 }))
		.collect();
	std::fs::write(
		dir.join("manifest.json"),
		serde_json::to_vec(&serde_json::json!({ "version": "1", "topiks": entries })).unwrap(),
	)
	.unwrap();
	for (key, _, body) in lessons {
		let mut file = (*key).to_owned();
		file.push_str(".json");
		std::fs::write(dir.join(file), body).unwrap();
	}
}

/// #277 (CUR4), in order: importing the existing corpus announces nothing
/// and moves no epoch; a new lesson drains exactly the known subjects who
/// had played its activity when it was published; re-importing unchanged
/// content, or a rename, announces nothing; changed content is news again —
/// and for a subject who is otherwise almost due, the news is what gets
/// said: `NewMaterial`, on `Topic::NewMaterial`.
#[tokio::test]
async fn a_new_lesson_drains_once_those_who_had_played_its_activity() {
	use curriculum_repo::import_dir;

	let pool = migrated_pool().await;
	let nudge = nudge_context();
	let learner = "subject-plays-topik";
	// Almost due, with freshness the thinnest margin — see
	// `a_publication_makes_new_material_the_thing_to_say`.
	let now = Utc::now();
	let levels: Vec<(u16, f64)> = vec![(1, 61.0), (2, 44.0), (3, 22.0), (4, 35.0)];
	EngagementRepository::new(pool.clone())
		.save(learner, &levels, &now.to_rfc3339(), &(now + Duration::days(1)).to_rfc3339())
		.await
		.unwrap();
	played(&pool, learner, "topik", "completed").await;
	for other in ["subject-plays-honeycomb", "subject-skipped-topik", "subject-plays-topik-later"] {
		first_contact(&pool, other).await.unwrap();
	}
	played(&pool, "subject-plays-honeycomb", "honeycomb", "completed").await;
	played(&pool, "subject-skipped-topik", "topik", "skipped").await;
	// Played, but only after the lesson is published: it applies to them no
	// more than to someone who never played.
	played(&pool, "subject-plays-topik-later", "topik", "completed").await;
	sqlx::query!("UPDATE activity_outcome SET ended_at = '2999-01-01T00:00:00+00:00' WHERE subject_id = 'subject-plays-topik-later'")
		.execute(&pool)
		.await
		.unwrap();

	let dir = tempfile::tempdir().unwrap();
	write_topiks(dir.path(), &[("beginner", "Beginner", b"{\"v\":1}")]);
	import_dir(&pool, dir.path(), "topik", &now.to_rfc3339(), false).await.unwrap();
	assert_eq!(run_once(&pool, &nudge).await.unwrap().caught_up, 0, "the existing corpus is not new to anyone");

	write_topiks(dir.path(), &[("beginner", "Beginner", b"{\"v\":1}"), ("intermediate", "Intermediate", b"{\"v\":1}")]);
	import_dir(&pool, dir.path(), "topik", &now.to_rfc3339(), false).await.unwrap();
	assert_eq!(
		run_once(&pool, &nudge).await.unwrap().caught_up,
		1,
		"one subject had played topik when it was published; honeycomb-only, skip-only and later players are not its audience"
	);
	let epoch = PublicationRepository::new(pool.clone()).newest().await.unwrap().unwrap().id;
	for other in ["subject-plays-honeycomb", "subject-skipped-topik", "subject-plays-topik-later"] {
		let gate = EngagementRepository::new(pool.clone()).gate(other).await.unwrap().unwrap();
		assert_eq!(gate.curriculum_epoch, epoch, "{other}: the watermark moves even when nothing applies");
		assert_eq!(freshness(&pool, other).await, Some(100.0), "{other}: and nothing is drained");
	}
	assert_eq!(run_once(&pool, &nudge).await.unwrap().caught_up, 0, "and never twice");

	let stored = EngagementRepository::new(pool.clone()).charge(learner).await.unwrap();
	#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
	let levels: Vec<(u16, f64)> = stored.iter().map(|row| (row.class as u16, row.level)).collect();
	let as_of = crate::nudge::clock::parse_timestamp(&stored[0].as_of).unwrap();
	let charge = Charge::<StudyV1>::from_storage::<StudyCalibration>(&levels, as_of);
	let engine = Engine::<StudyV1, StudyCalibration, _, _>::new(
		StudyConstraints {
			clock: NudgeClock::resolve(Some("UTC")).0,
			enabled: true,
			quiet_hours_start: 0,
			quiet_hours_end: 0,
			presence: PresenceLeases::empty(std::time::Duration::from_secs(75)),
			consented_topics: Topic::ALL.to_vec(),
		},
		StudySelector {
			prepared_session: Some("session-openable".to_owned()),
		},
	);
	let verdict = engine.evaluate(&charge, as_of, None);
	let Verdict::Intervene(action) = verdict else {
		panic!("expected the new lesson to make this subject due, got {verdict:?}");
	};
	assert_eq!(
		action,
		StudyAction::NewMaterial {
			session_id: "session-openable".to_owned()
		}
	);
	assert_eq!(crate::nudge::payload::topic_for(&action), Topic::NewMaterial);

	write_topiks(
		dir.path(),
		&[("beginner", "Beginner, renamed", b"{\"v\":1}"), ("intermediate", "Intermediate", b"{\"v\":1}")],
	);
	import_dir(&pool, dir.path(), "topik", &now.to_rfc3339(), false).await.unwrap();
	assert_eq!(run_once(&pool, &nudge).await.unwrap().caught_up, 0, "unchanged bytes, or a rename, are not new material");

	write_topiks(
		dir.path(),
		&[("beginner", "Beginner, renamed", b"{\"v\":2}"), ("intermediate", "Intermediate", b"{\"v\":1}")],
	);
	import_dir(&pool, dir.path(), "topik", &now.to_rfc3339(), false).await.unwrap();
	assert_eq!(run_once(&pool, &nudge).await.unwrap().caught_up, 1, "changed bytes are new material again");
}

/// The operator's weekly batch: a retired lesson is not in the manifest, so
/// it is news to nobody. Retiring and restoring move no version and
/// announce nothing; a version a lesson gained while retired is announced
/// when it is restored, because nobody has seen it; and a lesson retired
/// after it was detected but before a subject is caught up applies to them
/// no more than a deleted one would.
#[tokio::test]
async fn a_retired_lesson_is_news_to_nobody() {
	use curriculum_repo::{import_dir, CurriculumRepository, ManifestEntry};

	let pool = migrated_pool().await;
	let nudge = nudge_context();
	let learner = "subject-plays-topik";
	first_contact(&pool, learner).await.unwrap();
	played(&pool, learner, "topik", "completed").await;
	let now = Utc::now().to_rfc3339();
	let dir = tempfile::tempdir().unwrap();
	write_topiks(dir.path(), &[("beginner", "Beginner", b"{}")]);
	import_dir(&pool, dir.path(), "topik", &now, false).await.unwrap();
	assert_eq!(run_once(&pool, &nudge).await.unwrap().caught_up, 0, "the baseline");

	let entry = |key: &str| ManifestEntry {
		key: key.to_owned(),
		display_name: key.to_owned(),
		description: "d".to_owned(),
		batch_count: 1,
		total_questions: 1,
		total_messages: 1,
		difficulty: None,
		tags: None,
	};
	let mut conn = pool.acquire().await.unwrap();
	CurriculumRepository::retire(&mut conn, "beginner", &now).await.unwrap();
	drop(conn);
	assert_eq!(run_once(&pool, &nudge).await.unwrap().caught_up, 0, "retiring is not a publication");
	let mut conn = pool.acquire().await.unwrap();
	CurriculumRepository::restore(&mut conn, "beginner").await.unwrap();
	drop(conn);
	assert_eq!(run_once(&pool, &nudge).await.unwrap().caught_up, 0, "nor is restoring unchanged content");

	// Written and retired before any pass saw it, then edited while retired.
	let mut conn = pool.acquire().await.unwrap();
	CurriculumRepository::upsert(&mut conn, "topik", &entry("week-40"), b"{\"v\":1}", &now, false)
		.await
		.unwrap();
	CurriculumRepository::retire(&mut conn, "week-40", &now).await.unwrap();
	drop(conn);
	assert_eq!(run_once(&pool, &nudge).await.unwrap().caught_up, 0, "a retired lesson is not detected");
	let mut conn = pool.acquire().await.unwrap();
	CurriculumRepository::upsert(&mut conn, "topik", &entry("week-40"), b"{\"v\":2}", &now, false)
		.await
		.unwrap();
	drop(conn);
	assert_eq!(run_once(&pool, &nudge).await.unwrap().caught_up, 0, "nor is a version it gains while retired");
	let mut conn = pool.acquire().await.unwrap();
	CurriculumRepository::restore(&mut conn, "week-40").await.unwrap();
	drop(conn);
	assert_eq!(
		run_once(&pool, &nudge).await.unwrap().caught_up,
		1,
		"until it is listed: then that version is new to everyone"
	);

	// Detected, then retired before this subject is caught up.
	let mut conn = pool.acquire().await.unwrap();
	CurriculumRepository::upsert(&mut conn, "topik", &entry("week-41"), b"{}", &now, false).await.unwrap();
	drop(conn);
	PublicationRepository::new(pool.clone()).detect_curriculum_publications(&now).await.unwrap();
	let mut conn = pool.acquire().await.unwrap();
	CurriculumRepository::retire(&mut conn, "week-41", &now).await.unwrap();
	drop(conn);
	assert_eq!(
		run_once(&pool, &nudge).await.unwrap().caught_up,
		0,
		"a lesson retired since it was published applies to nobody"
	);
	let epoch = PublicationRepository::new(pool.clone()).newest().await.unwrap().unwrap().id;
	let gate = EngagementRepository::new(pool.clone()).gate(learner).await.unwrap().unwrap();
	assert_eq!(gate.curriculum_epoch, epoch, "and the watermark still moves past it");
}

/// #277 (CUR4): a catch-up that chose its publication from a stale
/// watermark — another pass drained that publication since — advances
/// without draining it again (from a `chatgpt-codex-connector` finding on
/// #367).
#[tokio::test]
async fn a_catch_up_from_a_stale_watermark_does_not_drain_twice() {
	use curriculum_repo::import_dir;

	let pool = migrated_pool().await;
	let subject_id = "subject-racing";
	first_contact(&pool, subject_id).await.unwrap();
	let stale = EngagementRepository::new(pool.clone()).gate(subject_id).await.unwrap().unwrap().curriculum_epoch;

	// Pass one: a new activity, which applies to them, is drained for.
	publish(&pool, "new-thing", 1).await;
	assert_eq!(run_once(&pool, &nudge_context()).await.unwrap().caught_up, 1);
	let drained_once = freshness(&pool, subject_id).await.unwrap();

	// Then a lesson for an activity they have never played moves the epoch.
	let dir = tempfile::tempdir().unwrap();
	write_topiks(dir.path(), &[("beginner", "Beginner", b"{}")]);
	import_dir(&pool, dir.path(), "honeycomb", &Utc::now().to_rfc3339(), false).await.unwrap();
	write_topiks(dir.path(), &[("beginner", "Beginner", b"{}"), ("second", "Second", b"{}")]);
	import_dir(&pool, dir.path(), "honeycomb", &Utc::now().to_rfc3339(), false).await.unwrap();
	let newest = newest_publication(&pool, Utc::now(), &far_deadline()).await.unwrap();

	// A racing pass that read the watermark before pass one picks the
	// activity publication again — and must not drain for it twice.
	assert_eq!(catch_up(&pool, subject_id, stale, &newest).await.unwrap(), CatchUp::NothingApplies);
	assert!((freshness(&pool, subject_id).await.unwrap() - drained_once).abs() < 0.01, "drained once, not twice");
	let gate = EngagementRepository::new(pool.clone()).gate(subject_id).await.unwrap().unwrap();
	assert_eq!(gate.curriculum_epoch, newest.id, "and the watermark still reaches the epoch");
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
	save_unless_superseded(
		&crate::auth::DeletionLock::default(),
		&engagement,
		subject_id,
		read_as_of.as_deref(),
		&levels,
		as_of,
		Utc::now() + Duration::days(365),
	)
	.await
	.unwrap();

	let mastery = engagement.charge(subject_id).await.unwrap().into_iter().find(|row| row.class == 3).unwrap().level;
	assert!(mastery < 90.0, "the poor score's drain survives: mastery is {mastery}");
	let gate = engagement.gate(subject_id).await.unwrap().unwrap();
	assert_eq!(gate.eligible_at, signal_eligible_at.to_rfc3339(), "and so does the eligibility it re-solved");

	// With nothing folded in since its read, the waker's save still lands.
	let fresh = engagement.charge(subject_id).await.unwrap().first().map(|row| row.as_of.clone());
	let later = Utc::now() + Duration::days(2);
	save_unless_superseded(&crate::auth::DeletionLock::default(), &engagement, subject_id, fresh.as_deref(), &levels, as_of, later)
		.await
		.unwrap();
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
	let missing_table = EngagementRepository::new(pool.clone()).due(&t0().to_rfc3339(), 0, BATCH).await.unwrap_err();
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

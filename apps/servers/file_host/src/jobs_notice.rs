//! The Jobs arrival notice (paulgsc/server#470, RUN4).
//!
//! Home tells each phone that asked, through Firebase Cloud Messaging, when a
//! morning run's record arrives, so the owner learns the run is in without
//! opening Jobs.
//!
//! **When.** After each mailbox pass ([`crate::mailbox`]), [`run_once`]
//! looks at the jobs routine's run records home holds
//! (`mailbox_repo::jobs_runs`, the newest [`RUNS_READ`]) that started within
//! [`WINDOW_HOURS`] and that `jobs_notice_sent` does not name yet. It tells
//! each phone in `jobs_notice_device` once per record, oldest record first,
//! and records what it said. The record is the trigger, not the digest:
//! every run leaves one, a stopped run included, and it comes last (paulgsc/some-ui
//! `packages/job-desk/ROUTINE.md` §9). With no phone asking, it does nothing.
//!
//! **A re-run says nothing.** A record that added no picks, on a morning
//! another record within [`WINDOW_HOURS`] before it added them, is recorded
//! `skipped` and sent to nobody: the record that added the picks speaks, as
//! on the phone's card.
//!
//! **What it says is fixed text** ([`words`]): whether the picks are at home
//! or the run couldn't finish, never counts, titles or reasons, since FCM
//! carries the text through Google (docs/identity.md, "What is still
//! exposed"). The phone reads the detail from home.
//!
//! **Delivery.** A phone FCM no longer knows ([`SendOutcome::Unregistered`])
//! is forgotten. A record is recorded once any phone took it, or every phone
//! answered for good; when none took it and one never got an answer, or FCM
//! said to try later (a `429` or a `5xx`), nothing is recorded and the next
//! pass tries again within the window. A token
//! never reaches a log line (docs/identity.md invariant 24): outcomes are
//! logged as counts.
//!
//! Each write holds account deletion's lock, and each repo write checks the
//! account in the statement (invariant 9). The lock is never held across a
//! request to FCM.

use crate::auth::DeletionLock;
use chrono::{DateTime, Duration, SecondsFormat, Utc};
use fcm_kit::{DeviceToken, FcmTransport, Notice, SendOutcome, Sender, ServiceAccount};
use notice_repo::Said;
use serde::{de::IgnoredAny, Deserialize};
use sqlx::SqlitePool;
use std::path::Path;

/// How long after a run starts home will still tell a phone of it.
///
/// A record home first reads later than this (home was off all day) tells no
/// one: by then the owner has opened Jobs, or it is tomorrow's news.
pub const WINDOW_HOURS: i64 = 12;

/// The run records one pass looks at: `GET /jobs/runs`' week.
pub const RUNS_READ: i64 = crate::handlers::jobs::RUNS_PER_ANSWER;

/// Replaces an earlier notice on the phone, so a morning shows one.
pub const TAG: &str = "jobs-run";

/// What the app opens from the notice.
const DATA: &[(&str, &str)] = &[("open", "/jobs")];

/// The notice for picks at home.
pub const PICKS: Notice<'static> = Notice {
	title: "This morning’s picks are at home",
	body: "Open Jobs on the home Wi‑Fi to see them.",
	tag: TAG,
	data: DATA,
};

/// The notice for a run that put its picks in, but couldn't finish.
pub const PARTLY: Notice<'static> = Notice {
	title: "This morning’s run couldn’t finish",
	body: "Its picks are at home. Open Jobs on the home Wi‑Fi to see why.",
	tag: TAG,
	data: DATA,
};

/// The notice for a run that stopped before its picks.
pub const STOPPED: Notice<'static> = Notice {
	title: "This morning’s run couldn’t finish",
	body: "Open Jobs on the home Wi‑Fi to see why.",
	tag: TAG,
	data: DATA,
};

/// What a run record tells: when it started, how it ended, whether it added
/// the picks. The rest of `JobsRunSchema` (paulgsc/some-ui
/// `packages/job-desk/src/run.ts`) is the phone's to read.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Run {
	started_at: String,
	outcome: String,
	added: Vec<IgnoredAny>,
}

/// What a pass did, as counts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Outcome {
	/// Records told to at least one phone, or answered for good by all.
	pub noticed: usize,
	/// Re-runs recorded and told to nobody.
	pub skipped: usize,
	/// Phones FCM no longer knows, forgotten.
	pub forgotten: usize,
	/// Records no phone took, left for the next pass.
	pub unsent: usize,
}

impl Outcome {
	/// Whether the pass did anything worth a log line.
	#[must_use]
	pub const fn busy(&self) -> bool {
		self.noticed + self.skipped + self.forgotten + self.unsent > 0
	}
}

/// Read the service account `FCM_SERVICE_ACCOUNT_FILE` names.
///
/// # Errors
/// The file can't be read or is not a service account's key; the error names
/// the variable and why, never the file's contents.
pub fn load(path: &Path) -> anyhow::Result<ServiceAccount> {
	let text = std::fs::read_to_string(path).map_err(|_| anyhow::anyhow!("FCM_SERVICE_ACCOUNT_FILE: the file can't be read"))?;
	ServiceAccount::from_json(&text).map_err(|why| anyhow::anyhow!("FCM_SERVICE_ACCOUNT_FILE: {why}"))
}

/// What home says of `run`, given the records before it in the window: what
/// it records, and the notice it sends (none for a re-run).
fn words(run: &Run, started: DateTime<Utc>, earlier: &[(DateTime<Utc>, bool)]) -> (Said, Option<Notice<'static>>) {
	if !run.added.is_empty() {
		return if run.outcome == "done" {
			(Said::Picks, Some(PICKS))
		} else {
			(Said::Trouble, Some(PARTLY))
		};
	}
	let picked_before = earlier.iter().any(|(at, added)| *added && *at <= started && started - *at < Duration::hours(WINDOW_HOURS));
	if picked_before {
		(Said::Skipped, None)
	} else {
		(Said::Trouble, Some(STOPPED))
	}
}

/// One pass: tell each phone of each run record not yet noticed.
///
/// # Errors
/// A storage failure, which ends the pass; what it recorded stays.
pub async fn run_once<T: FcmTransport>(db: &SqlitePool, deletion: &DeletionLock, subject: &str, sender: &Sender<T>, now: DateTime<Utc>) -> Result<Outcome, sqlx::Error> {
	let mut outcome = Outcome::default();
	let mut conn = db.acquire().await?;
	let mut devices = notice_repo::devices(&mut conn, subject).await?;
	if devices.is_empty() {
		return Ok(outcome);
	}
	let held = mailbox_repo::jobs_runs(&mut conn, subject, RUNS_READ).await.map_err(|err| match err {
		mailbox_repo::MailboxError::Storage(err) => err,
		mailbox_repo::MailboxError::Rewritten => sqlx::Error::Protocol(String::from("a read cannot rewrite")),
	})?;
	drop(conn);

	// A record that does not read as one is not this pass's to judge: the
	// phone's reader reports it.
	let mut runs: Vec<(String, DateTime<Utc>, Run)> = held
		.into_iter()
		.filter_map(|file| {
			let run: Run = serde_json::from_slice(&file.body).ok()?;
			let started = DateTime::parse_from_rfc3339(&run.started_at).ok()?.with_timezone(&Utc);
			Some((file.name, started, run))
		})
		.filter(|(_, started, _)| *started <= now && now - *started < Duration::hours(WINDOW_HOURS))
		.collect();
	runs.sort_by_key(|(_, started, _)| *started);
	let added: Vec<(DateTime<Utc>, bool)> = runs.iter().map(|(_, started, run)| (*started, !run.added.is_empty())).collect();

	let stamp = now.to_rfc3339_opts(SecondsFormat::Millis, true);
	let unix = u64::try_from(now.timestamp()).unwrap_or(0);
	for (index, (name, started, run)) in runs.iter().enumerate() {
		let mut conn = db.acquire().await?;
		if notice_repo::was_noticed(&mut conn, subject, name).await? {
			continue;
		}
		drop(conn);
		let (said, notice) = words(run, *started, &added[..index]);
		let Some(notice) = notice else {
			record(db, deletion, subject, name, said, 0, &stamp).await?;
			outcome.skipped += 1;
			continue;
		};
		let mut took = 0_i64;
		let mut unanswered = false;
		let mut gone = Vec::new();
		for token in &devices {
			match sender.send(&DeviceToken::new(token.clone()), &notice, unix).await {
				SendOutcome::Delivered => took += 1,
				SendOutcome::Unregistered => gone.push(token.clone()),
				// Throttling and FCM's own failures pass (FCM's error codes say
				// to retry them); any other refusal is final for this record.
				SendOutcome::Refused(status) if status == 429 || status >= 500 => {
					tracing::warn!(status, "FCM could not take a Jobs arrival notice now");
					unanswered = true;
				}
				SendOutcome::Refused(status) => tracing::warn!(status, "FCM refused a Jobs arrival notice"),
				SendOutcome::Transport(why) => {
					tracing::warn!(error = %why, "a Jobs arrival notice got no answer from FCM");
					unanswered = true;
				}
			}
		}
		for token in &gone {
			forget(db, deletion, subject, token).await?;
			outcome.forgotten += 1;
		}
		devices.retain(|token| !gone.contains(token));
		if took == 0 && unanswered {
			outcome.unsent += 1;
			continue;
		}
		record(db, deletion, subject, name, said, took, &stamp).await?;
		outcome.noticed += 1;
		if devices.is_empty() {
			break;
		}
	}
	Ok(outcome)
}

/// One write, under the deletion hold, as `mailbox::pass`'s.
async fn record(db: &SqlitePool, deletion: &DeletionLock, subject: &str, name: &str, said: Said, devices: i64, now: &str) -> Result<(), sqlx::Error> {
	let _hold = deletion.hold().await;
	let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
	notice_repo::record_sent(&mut tx, subject, name, said, devices, now).await?;
	tx.commit().await
}

/// As [`record`], for a phone FCM no longer knows.
async fn forget(db: &SqlitePool, deletion: &DeletionLock, subject: &str, token: &str) -> Result<(), sqlx::Error> {
	let _hold = deletion.hold().await;
	let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
	notice_repo::forget(&mut tx, subject, token).await?;
	tx.commit().await
}

#[cfg(test)]
mod tests {
	use super::{run_once, Outcome, PARTLY, PICKS, STOPPED};
	use crate::auth::DeletionLock;
	use chrono::{DateTime, Duration, Utc};
	use fcm_kit::{FcmTransport, Request, Response, Sender, ServiceAccount};
	use jwt_simple::prelude::RS256KeyPair;
	use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};
	use std::collections::HashMap;
	use std::sync::{Arc, Mutex};

	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");
	const A: &str = "subject-a";

	/// FCM, answering each device by its token: delivered unless told
	/// otherwise. It records each notice it took, as (token, title, body).
	#[derive(Clone, Default)]
	struct Fake {
		answers: Arc<Mutex<HashMap<String, Result<u16, String>>>>,
		took: Arc<Mutex<Vec<(String, String, String)>>>,
	}

	impl Fake {
		fn answer(&self, token: &str, answer: Result<u16, &str>) {
			self.answers.lock().unwrap().insert(token.to_owned(), answer.map_err(str::to_owned));
		}

		fn took(&self) -> Vec<(String, String, String)> {
			std::mem::take(&mut *self.took.lock().unwrap())
		}
	}

	impl FcmTransport for Fake {
		type Error = String;

		async fn send(&self, request: Request) -> Result<Response, Self::Error> {
			if request.url.ends_with("/token") {
				let body = br#"{"access_token":"ya29.test","expires_in":3600}"#.to_vec();
				return Ok(Response { status: 200, body });
			}
			let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
			let message = &body["message"];
			let token = message["token"].as_str().unwrap().to_owned();
			let answer = self.answers.lock().unwrap().get(&token).cloned().unwrap_or(Ok(200));
			let status = answer?;
			if status == 200 {
				let notification = &message["notification"];
				let token = token.clone();
				self
					.took
					.lock()
					.unwrap()
					.push((token, notification["title"].as_str().unwrap().to_owned(), notification["body"].as_str().unwrap().to_owned()));
			}
			// A refusal quotes the token back, as a provider might.
			let body = if status == 200 {
				b"{}".to_vec()
			} else {
				(String::from(r#"{"error":{"message":"not sent to "#) + &token + r#""}}"#).into_bytes()
			};
			Ok(Response { status, body })
		}
	}

	fn sender(fake: &Fake) -> Sender<Fake> {
		let key = RS256KeyPair::generate(2048).unwrap();
		let file = serde_json::json!({
			"type": "service_account",
			"project_id": "some-ui",
			"client_email": "fcm@some-ui.iam.gserviceaccount.com",
			"private_key": key.to_pem().unwrap(),
			"token_uri": "https://oauth2.googleapis.com/token",
		});
		Sender::new(ServiceAccount::from_json(&file.to_string()).unwrap(), fake.clone())
	}

	async fn pool() -> SqlitePool {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();
		sqlx::query("INSERT INTO account (subject_id, user_handle) VALUES (?1, ?1)")
			.bind(A)
			.execute(&pool)
			.await
			.unwrap();
		pool
	}

	async fn phone(pool: &SqlitePool, token: &str) {
		let mut conn = pool.acquire().await.unwrap();
		notice_repo::register(&mut conn, A, token, "2026-10-26T06:00:00.000Z").await.unwrap();
	}

	/// A run record the poller kept, under the name its start gives it.
	async fn run(pool: &SqlitePool, seq: i64, started: &str, outcome: &str, added: bool) -> String {
		let name = String::from("run-jobs-") + &started.replace(['-', ':'], "") + ".json";
		let added = if added {
			serde_json::json!([{ "folder": "brief", "name": "digest-2026-10-26.json", "title": "2 picks from 7 postings" }])
		} else {
			serde_json::json!([])
		};
		let body = serde_json::json!({
			"schema": "some-ui.routine-run/1", "routine": "jobs", "startedAt": started, "finishedAt": started,
			"outcome": outcome, "found": { "postings": 7, "scouts": 0, "decisions": 0 }, "added": added, "problems": [],
		});
		sqlx::query("INSERT INTO mailbox_item (subject_id, folder, name, seq, sha256, size, body, pulled_at) VALUES (?1, 'run', ?2, ?3, 's', 0, ?4, 'now')")
			.bind(A)
			.bind(&name)
			.bind(seq)
			.bind(body.to_string().into_bytes())
			.execute(pool)
			.await
			.unwrap();
		name
	}

	async fn sent(pool: &SqlitePool) -> Vec<(String, String, i64)> {
		sqlx::query_as("SELECT run_name, said, devices FROM jobs_notice_sent WHERE subject_id = ?1 ORDER BY run_name")
			.bind(A)
			.fetch_all(pool)
			.await
			.unwrap()
	}

	async fn devices(pool: &SqlitePool) -> Vec<String> {
		let mut conn = pool.acquire().await.unwrap();
		notice_repo::devices(&mut conn, A).await.unwrap()
	}

	/// 08:10 on the 26th in Los Angeles: eighteen minutes after the run.
	fn now() -> DateTime<Utc> {
		DateTime::parse_from_rfc3339("2026-10-26T15:10:00Z").unwrap().with_timezone(&Utc)
	}

	async fn pass(pool: &SqlitePool, sender: &Sender<Fake>, at: DateTime<Utc>) -> Outcome {
		run_once(pool, &DeletionLock::default(), A, sender, at).await.unwrap()
	}

	fn words(notice: fcm_kit::Notice<'_>) -> (String, String) {
		(notice.title.to_owned(), notice.body.to_owned())
	}

	#[tokio::test]
	async fn with_no_phone_asking_nothing_is_sent_or_recorded() {
		let pool = pool().await;
		let fake = Fake::default();
		run(&pool, 1, "2026-10-26T14:52:07Z", "done", true).await;
		assert_eq!(pass(&pool, &sender(&fake), now()).await, Outcome::default());
		assert!(fake.took().is_empty());
		assert!(sent(&pool).await.is_empty());
	}

	#[tokio::test]
	async fn a_morning_run_tells_each_phone_once() {
		let pool = pool().await;
		let fake = Fake::default();
		let sender = sender(&fake);
		phone(&pool, "phone-1").await;
		phone(&pool, "phone-2").await;
		let name = run(&pool, 1, "2026-10-26T14:52:07Z", "done", true).await;
		assert_eq!(pass(&pool, &sender, now()).await, Outcome { noticed: 1, ..Outcome::default() });
		let (title, body) = words(PICKS);
		assert_eq!(
			fake.took(),
			[(String::from("phone-1"), title.clone(), body.clone()), (String::from("phone-2"), title, body)]
		);
		assert_eq!(sent(&pool).await, [(name, String::from("picks"), 2)]);
		assert_eq!(
			pass(&pool, &sender, now() + Duration::minutes(15)).await,
			Outcome::default(),
			"the next pass tells no one again"
		);
		assert!(fake.took().is_empty());
	}

	#[tokio::test]
	async fn a_run_that_couldnt_finish_says_so_and_a_rerun_after_the_picks_says_nothing() {
		let pool = pool().await;
		let fake = Fake::default();
		let sender = sender(&fake);
		phone(&pool, "phone-1").await;
		run(&pool, 1, "2026-10-26T14:52:07Z", "partly", true).await;
		run(&pool, 2, "2026-10-26T15:05:00Z", "stopped", false).await;
		assert_eq!(
			pass(&pool, &sender, now()).await,
			Outcome {
				noticed: 1,
				skipped: 1,
				..Outcome::default()
			}
		);
		let (title, body) = words(PARTLY);
		assert_eq!(fake.took(), [(String::from("phone-1"), title, body)]);
		let said: Vec<String> = sent(&pool).await.into_iter().map(|(_, said, _)| said).collect();
		assert_eq!(said, ["trouble", "skipped"]);
	}

	#[tokio::test]
	async fn a_run_that_stopped_alone_says_it_couldnt_finish() {
		let pool = pool().await;
		let fake = Fake::default();
		phone(&pool, "phone-1").await;
		run(&pool, 1, "2026-10-26T14:52:07Z", "stopped", false).await;
		pass(&pool, &sender(&fake), now()).await;
		let (title, body) = words(STOPPED);
		assert_eq!(fake.took(), [(String::from("phone-1"), title, body)]);
	}

	#[tokio::test]
	async fn a_record_older_than_the_window_tells_no_one() {
		let pool = pool().await;
		let fake = Fake::default();
		phone(&pool, "phone-1").await;
		run(&pool, 1, "2026-10-26T14:52:07Z", "done", true).await;
		assert_eq!(pass(&pool, &sender(&fake), now() + Duration::hours(12)).await, Outcome::default());
		assert!(fake.took().is_empty());
	}

	#[tokio::test]
	async fn a_phone_fcm_forgot_is_forgotten_and_a_send_with_no_answer_is_tried_again() {
		let pool = pool().await;
		let fake = Fake::default();
		let sender = sender(&fake);
		phone(&pool, "gone").await;
		phone(&pool, "flaky").await;
		fake.answer("gone", Ok(404));
		fake.answer("flaky", Err("connection reset"));
		let name = run(&pool, 1, "2026-10-26T14:52:07Z", "done", true).await;
		assert_eq!(
			pass(&pool, &sender, now()).await,
			Outcome {
				forgotten: 1,
				unsent: 1,
				..Outcome::default()
			}
		);
		assert_eq!(devices(&pool).await, ["flaky"]);
		assert!(sent(&pool).await.is_empty(), "nobody took it, so it is not recorded");

		fake.answer("flaky", Ok(200));
		assert_eq!(pass(&pool, &sender, now() + Duration::minutes(15)).await, Outcome { noticed: 1, ..Outcome::default() });
		assert_eq!(fake.took().len(), 1);
		assert_eq!(sent(&pool).await, [(name, String::from("picks"), 1)]);
	}

	/// docs/identity.md invariant 24: a device token never reaches a log line,
	/// whatever FCM answers. One phone is refused, one gets no answer, one is
	/// gone and one takes it; FCM's refusal body quotes the token back.
	#[tokio::test]
	async fn no_device_token_reaches_a_log_line_whatever_fcm_answers() {
		let (captured, _guard) = crate::privacy::capture_where(crate::auth::loggable);
		let pool = pool().await;
		let fake = Fake::default();
		for (token, answer) in [
			("refused-token", Ok(400)),
			("unanswered-token", Err("connection reset")),
			("gone-token", Ok(404)),
			("taking-token", Ok(200)),
		] {
			phone(&pool, token).await;
			fake.answer(token, answer);
		}
		run(&pool, 1, "2026-10-26T14:52:07Z", "done", true).await;
		run(&pool, 2, "2026-10-26T14:59:00Z", "stopped", false).await;
		pass(&pool, &sender(&fake), now()).await;
		let lines = captured.lines();
		assert!(!lines.is_empty(), "the refusal and the missing answer are logged");
		for line in &lines {
			assert!(!line.contains("-token"), "{line}");
		}
	}

	#[tokio::test]
	async fn fcm_saying_try_later_leaves_the_record_for_the_next_pass() {
		let pool = pool().await;
		let fake = Fake::default();
		let sender = sender(&fake);
		phone(&pool, "phone-1").await;
		run(&pool, 1, "2026-10-26T14:52:07Z", "done", true).await;
		for status in [429, 500, 503] {
			fake.answer("phone-1", Ok(status));
			assert_eq!(pass(&pool, &sender, now()).await, Outcome { unsent: 1, ..Outcome::default() }, "{status}");
			assert!(sent(&pool).await.is_empty(), "{status} is not final");
		}
		fake.answer("phone-1", Ok(200));
		assert_eq!(pass(&pool, &sender, now()).await, Outcome { noticed: 1, ..Outcome::default() });
	}

	#[tokio::test]
	async fn a_refusal_is_final_for_that_record() {
		let pool = pool().await;
		let fake = Fake::default();
		phone(&pool, "phone-1").await;
		fake.answer("phone-1", Ok(400));
		let name = run(&pool, 1, "2026-10-26T14:52:07Z", "done", true).await;
		assert_eq!(pass(&pool, &sender(&fake), now()).await, Outcome { noticed: 1, ..Outcome::default() });
		assert_eq!(sent(&pool).await, [(name, String::from("picks"), 0)]);
		assert_eq!(devices(&pool).await, ["phone-1"], "a refusal keeps the phone");
	}
}

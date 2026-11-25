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
//! and records what it said. A record past the window that a phone was owed
//! and none was told of is recorded `expired` and counted once as lost
//! (#1176); one that a later run pushed out of the newest [`RUNS_READ`]
//! before its window closed is not looked at again, so not counted (the
//! bound). The record is the trigger, not the digest:
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
//! FCM still knows is gone; when none took it and one never got an answer,
//! FCM said to try later (a `429` or a `5xx`), or FCM refused it (a bad key,
//! a JWT home could not sign: fixed on home's side, #1206), nothing is
//! recorded and the next pass tries again within the window. While one
//! waits it is open (`jobs-notice`/`refused` or `unanswered`, a level each
//! pass sets afresh, docs/home-compute.md invariant 9), and one the window
//! outlives untold is a loss, stored `expired` and counted once (#1176). A token
//! never reaches a log line (docs/identity.md invariant 24): outcomes are
//! logged as counts.
//!
//! Each write holds account deletion's lock, and each repo write checks the
//! account in the statement (invariant 9). The lock is never held across a
//! request to FCM.

use crate::auth::DeletionLock;
use crate::failure::{Failure, Step};
use crate::strikes::Turn;
use chrono::{DateTime, Duration, SecondsFormat, Utc};
use fcm_kit::{DeviceToken, FcmTransport, Notice, SendOutcome, Sender, ServiceAccount};
use notice_repo::Said;
use serde::{de::IgnoredAny, Deserialize};
use sqlx::SqlitePool;
use std::path::Path;

/// How long after a run starts home will still tell a phone of it.
///
/// A record home first reads later than this (home was off all day) tells no
/// one: by then the owner has opened Jobs, or it is tomorrow's news. A record
/// a phone had asked to hear of that leaves the window untold, this way or
/// because every try was refused or unanswered, is a loss: stored `expired`
/// and counted once (`jobs-notice`/`untold`, #1176).
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
	/// Records told to at least one phone, or whose every phone FCM no
	/// longer knows.
	pub noticed: usize,
	/// Re-runs recorded and told to nobody.
	pub skipped: usize,
	/// Phones FCM no longer knows, forgotten.
	pub forgotten: usize,
	/// Records no phone took, left for the next pass.
	pub unsent: usize,
	/// Records that left the window before any phone took them, stored as
	/// `expired` and counted once (`jobs-notice`/`untold`, #1176).
	pub expired: usize,
}

impl Outcome {
	/// Whether the pass did anything worth a log line.
	#[must_use]
	pub const fn busy(&self) -> bool {
		self.noticed + self.skipped + self.forgotten + self.unsent + self.expired > 0
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

/// Nothing of the notice is open: no phone asked, or there is no account
/// (invariant 9: the pass that looks sets the level, here to 0).
pub fn nothing_open() {
	crate::failure::open(Step::JobsNoticeRefused, 0);
	crate::failure::open(Step::JobsNoticeUnanswered, 0);
}

/// One pass: tell each phone of each run record not yet noticed.
///
/// # Errors
/// A storage failure, which ends the pass; what it recorded stays.
pub async fn run_once<T: FcmTransport>(db: &SqlitePool, deletion: &DeletionLock, subject: &str, sender: &Sender<T>, now: DateTime<Utc>) -> Result<Outcome, sqlx::Error> {
	let mut outcome = Outcome::default();
	let mut conn = db.acquire().await?;
	let mut devices = notice_repo::phones(&mut conn, subject).await?;
	if devices.is_empty() {
		// No phone asked: nothing is owed, so nothing is open.
		nothing_open();
		return Ok(outcome);
	}
	// A record no phone is sent to (every one set aside, #1222) is owed to
	// none: setting each aside was counted once, and its records are not
	// counted lost again each morning.
	let asked_since = if devices.iter().any(|phone| phone.set_aside_at.is_none()) {
		notice_repo::first_registered(&mut conn, subject).await?
	} else {
		None
	};
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
		.collect();
	runs.retain(|(_, started, _)| *started <= now);
	runs.sort_by_key(|(_, started, _)| *started);
	// Every record read, in or past the window, so a re-run's silence reads
	// the record that added its morning's picks wherever it falls.
	let added: Vec<(DateTime<Utc>, bool)> = runs.iter().map(|(_, started, run)| (*started, !run.added.is_empty())).collect();
	let in_window = |started: DateTime<Utc>| now - started < Duration::hours(WINDOW_HOURS);

	let stamp = now.to_rfc3339_opts(SecondsFormat::Millis, true);
	outcome.expired = expire(db, deletion, subject, &runs, now, asked_since.as_deref(), &stamp).await?;
	let unix = u64::try_from(now.timestamp()).unwrap_or(0);
	let (mut refused_open, mut unanswered_open) = (0, 0);
	for (index, (name, started, run)) in runs.iter().enumerate() {
		if !in_window(*started) {
			continue;
		}
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
		let mut refused = false;
		let mut gone = Vec::new();
		// Phones that took it, and phones whose refusal may be their own
		// (#1222, `strikes::fcm`): which strikes needs every phone's answer.
		let (mut taking, mut striking) = (Vec::new(), Vec::new());
		for phone in &devices {
			// A phone set aside is included at most once a day (#1180).
			let turn = crate::strikes::turn(phone.set_aside_at.as_deref(), phone.last_probe_at.as_deref(), now);
			if turn == Turn::Skip {
				continue;
			}
			let token = &phone.token;
			if turn == Turn::Probe {
				probed(db, deletion, subject, token, &stamp).await?;
			}
			let answer = sender.send(&DeviceToken::new(token.clone()), &notice, unix).await;
			if turn == Turn::Send {
				striking.push((token.clone(), answer.clone()));
			}
			match answer {
				SendOutcome::Delivered => {
					took += 1;
					taking.push(token.clone());
				}
				SendOutcome::Unregistered => gone.push(token.clone()),
				// Throttling and FCM's own failures pass (FCM's error codes say
				// to retry them). Any other refusal (a key FCM won't take, a JWT
				// home could not sign, a token FCM calls invalid) is not a
				// delivery either: the record waits, and is tried again each
				// pass until a phone takes it or it leaves the window (#1206).
				// A phone whose own token FCM refuses for good is set aside
				// after `strikes::SET_ASIDE_AFTER` in a row (#1222).
				SendOutcome::Refused(status) if status == 429 || status >= 500 => {
					tracing::warn!(status, "FCM could not take a Jobs arrival notice now");
					unanswered = true;
				}
				SendOutcome::Refused(status) => {
					tracing::warn!(status, "FCM refused a Jobs arrival notice");
					refused = true;
				}
				SendOutcome::Mismatched => {
					tracing::warn!("FCM refused a Jobs arrival notice: the token is another sender's");
					refused = true;
				}
				SendOutcome::Transport(why) => {
					tracing::warn!(error = %why, "a Jobs arrival notice got no answer from FCM");
					unanswered = true;
				}
			}
		}
		for token in &taking {
			phone_took(db, deletion, subject, token).await?;
		}
		for (token, answer) in &striking {
			if let Some(why) = crate::strikes::fcm(answer, took > 0) {
				strike(db, deletion, subject, token, why, &stamp).await?;
			}
		}
		for token in &gone {
			forget(db, deletion, subject, token).await?;
			outcome.forgotten += 1;
		}
		devices.retain(|phone| !gone.contains(&phone.token));
		if took == 0 && (unanswered || refused) {
			outcome.unsent += 1;
			// Open until a phone takes it or it leaves the window: a final
			// refusal (a key home must fix) apart from a try-later.
			if refused {
				refused_open += 1;
			} else {
				unanswered_open += 1;
			}
			continue;
		}
		record(db, deletion, subject, name, said, took, &stamp).await?;
		outcome.noticed += 1;
		if devices.is_empty() {
			break;
		}
	}
	// Records waiting on a phone, as this pass found them (invariant 9: a
	// level, never a recount). A pass that ended early (every phone gone)
	// tried no more, so what it did not try is not open.
	crate::failure::open(Step::JobsNoticeRefused, refused_open);
	crate::failure::open(Step::JobsNoticeUnanswered, unanswered_open);
	Ok(outcome)
}

/// Stores each record of `runs` (sorted by start) that started before the
/// window at `at`, that a phone was owed and none was told of, as `expired`,
/// and counts it: a loss, the
/// window outlived every try. Stored so it is counted once, by the pass that
/// stores it (invariant 9: a condition that ends in a loss is an event at
/// the end). How many it stored.
async fn expire(
	db: &SqlitePool,
	deletion: &DeletionLock,
	subject: &str,
	runs: &[(String, DateTime<Utc>, Run)],
	at: DateTime<Utc>,
	asked_since: Option<&str>,
	now: &str,
) -> Result<usize, sqlx::Error> {
	let added: Vec<(DateTime<Utc>, bool)> = runs.iter().map(|(_, started, run)| (*started, !run.added.is_empty())).collect();
	let mut expired = 0;
	for (index, (name, started, run)) in runs.iter().enumerate() {
		if at - *started < Duration::hours(WINDOW_HOURS) || !owed(asked_since, *started) {
			continue;
		}
		let mut conn = db.acquire().await?;
		if notice_repo::was_noticed(&mut conn, subject, name).await? {
			continue;
		}
		drop(conn);
		// A re-run that added nothing, on a morning another record's picks
		// spoke for, is silent by design: never owed, so never lost (Codex
		// on #1230). It is recorded `skipped`, as it would have been in time.
		if words(run, *started, &added[..index]).0 == Said::Skipped {
			record(db, deletion, subject, name, Said::Skipped, 0, now).await?;
			continue;
		}
		if record(db, deletion, subject, name, Said::Expired, 0, now).await? == notice_repo::Change::Added {
			Failure::alone(Step::JobsNoticeUntold, &"no phone took a Jobs arrival notice before its window closed");
			expired += 1;
		}
	}
	Ok(expired)
}

/// Whether a record that started at `started` was owed a notice: a phone had
/// asked (`asked_since`, the first phone still asking) before its window
/// closed. A time that does not read is owed: counting a loss that wasn't is
/// the side to err on.
fn owed(asked_since: Option<&str>, started: DateTime<Utc>) -> bool {
	let Some(asked) = asked_since else {
		return false;
	};
	DateTime::parse_from_rfc3339(asked).map_or(true, |asked| asked.with_timezone(&Utc) < started + Duration::hours(WINDOW_HOURS))
}

/// One write, under the deletion hold, as `mailbox::pass`'s.
async fn record(db: &SqlitePool, deletion: &DeletionLock, subject: &str, name: &str, said: Said, devices: i64, now: &str) -> Result<notice_repo::Change, sqlx::Error> {
	let _hold = deletion.hold().await;
	let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
	let change = notice_repo::record_sent(&mut tx, subject, name, said, devices, now).await?;
	tx.commit().await?;
	Ok(change)
}

/// As [`record`], for a phone that took a notice: its run of failures ends
/// and, set aside, it is back (#1180).
async fn phone_took(db: &SqlitePool, deletion: &DeletionLock, subject: &str, token: &str) -> Result<(), sqlx::Error> {
	let _hold = deletion.hold().await;
	let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
	notice_repo::took(&mut tx, subject, token).await?;
	tx.commit().await
}

/// As [`record`], for a failure that was the phone's own (`why`, from
/// `strikes::fcm`): the one that reaches `strikes::SET_ASIDE_AFTER` sets it
/// aside, counted once (#1222).
async fn strike(db: &SqlitePool, deletion: &DeletionLock, subject: &str, token: &str, why: &'static str, now: &str) -> Result<(), sqlx::Error> {
	let _hold = deletion.hold().await;
	let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
	let after = notice_repo::strike(&mut tx, subject, token, why, crate::strikes::SET_ASIDE_AFTER, now).await?;
	tx.commit().await?;
	if after.is_some_and(|strikes| strikes >= crate::strikes::SET_ASIDE_AFTER) {
		tracing::warn!(why, "a phone failed on its own too many times in a row; set aside");
		Failure::alone(Step::JobsNoticePhoneSetAside, &why);
	}
	Ok(())
}

/// As [`record`], for a set-aside phone included in a real send.
async fn probed(db: &SqlitePool, deletion: &DeletionLock, subject: &str, token: &str, now: &str) -> Result<(), sqlx::Error> {
	let _hold = deletion.hold().await;
	let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
	notice_repo::probed(&mut tx, subject, token, now).await?;
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
	use crate::failure::{counted::failures, held, Step};
	use chrono::{DateTime, Duration, Utc};
	use fcm_kit::{FcmTransport, Request, Response, Sender, ServiceAccount};
	use jwt_simple::prelude::RS256KeyPair;
	use metrics_util::debugging::DebuggingRecorder;
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
		/// What Google's token endpoint answers, when not 200: a key it
		/// won't take.
		token: Arc<Mutex<Option<u16>>>,
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
				let refused = *self.token.lock().unwrap();
				if let Some(status) = refused {
					return Ok(Response { status, body: b"{}".to_vec() });
				}
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
			// A refusal quotes the token back, as a provider might; a 403 to a
			// token named `mismatch…` is FCM's SENDER_ID_MISMATCH.
			let body = if status == 200 {
				b"{}".to_vec()
			} else if status == 403 && token.starts_with("mismatch") {
				br#"{"error":{"details":[{"errorCode":"SENDER_ID_MISMATCH"}]}}"#.to_vec()
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

	/// #1176: a record older than the window tells no one, and since a phone
	/// had asked before it closed, it is a loss: stored `expired`, counted
	/// once, by the pass that stores it, and never again.
	#[tokio::test]
	async fn a_record_older_than_the_window_tells_no_one_and_is_counted_once_as_lost() {
		let recorder = DebuggingRecorder::new();
		let snapshotter = recorder.snapshotter();
		let _recording = metrics::set_default_local_recorder(&recorder);
		let pool = pool().await;
		let fake = Fake::default();
		phone(&pool, "phone-1").await;
		let name = run(&pool, 1, "2026-10-26T14:52:07Z", "done", true).await;
		assert_eq!(pass(&pool, &sender(&fake), now() + Duration::hours(12)).await, Outcome { expired: 1, ..Outcome::default() });
		assert!(fake.took().is_empty());
		assert_eq!(sent(&pool).await, [(name, String::from("expired"), 0)]);
		assert_eq!(failures(&snapshotter, Step::JobsNoticeUntold), Some(1));
		assert_eq!(pass(&pool, &sender(&fake), now() + Duration::hours(13)).await, Outcome::default());
		assert_eq!(failures(&snapshotter, Step::JobsNoticeUntold).unwrap_or(0), 0, "never again");
	}

	/// #1176: a record that started before any phone asked, and whose window
	/// closed before one did, was owed to no one: nothing is stored or
	/// #1176, Codex on #1230: a re-run that added nothing, on a morning
	/// another record's picks spoke for, is silent by design. First read past
	/// its window, it is recorded `skipped` as it would have been in time:
	/// never owed, never counted lost.
	#[tokio::test]
	async fn a_silent_rerun_read_past_its_window_is_skipped_not_lost() {
		let recorder = DebuggingRecorder::new();
		let snapshotter = recorder.snapshotter();
		let _recording = metrics::set_default_local_recorder(&recorder);
		let pool = pool().await;
		let fake = Fake::default();
		phone(&pool, "phone-1").await;
		let picks = run(&pool, 1, "2026-10-26T14:52:07Z", "done", true).await;
		assert_eq!(pass(&pool, &sender(&fake), now()).await, Outcome { noticed: 1, ..Outcome::default() });
		let rerun = run(&pool, 2, "2026-10-26T15:05:00Z", "done", false).await;
		assert_eq!(pass(&pool, &sender(&fake), now() + Duration::hours(13)).await, Outcome::default());
		assert_eq!(sent(&pool).await, [(picks, String::from("picks"), 1), (rerun, String::from("skipped"), 0)]);
		assert_eq!(failures(&snapshotter, Step::JobsNoticeUntold).unwrap_or(0), 0, "never owed, never lost");
	}

	/// counted.
	#[tokio::test]
	async fn a_record_no_phone_had_asked_for_is_not_a_loss() {
		let pool = pool().await;
		let fake = Fake::default();
		run(&pool, 1, "2026-10-20T14:52:07Z", "done", true).await;
		phone(&pool, "phone-1").await;
		assert_eq!(pass(&pool, &sender(&fake), now()).await, Outcome::default());
		assert!(sent(&pool).await.is_empty());
	}

	/// #1176, invariant 9: records every phone refused, or FCM never
	/// answered, are open while they wait, a level each pass sets afresh,
	/// never a count; a pass a phone takes them in clears it, and the one
	/// that finds them past the window counts the loss once.
	#[tokio::test]
	async fn a_record_waiting_on_a_phone_is_held_open_and_a_lost_one_counted() {
		let recorder = DebuggingRecorder::new();
		let snapshotter = recorder.snapshotter();
		let _recording = metrics::set_default_local_recorder(&recorder);
		let pool = pool().await;
		let fake = Fake::default();
		let sender = sender(&fake);
		phone(&pool, "phone-1").await;
		run(&pool, 1, "2026-10-26T14:52:07Z", "done", true).await;

		fake.answer("phone-1", Ok(401));
		crate::failure::open(Step::JobsNoticeRefused, 9);
		pass(&pool, &sender, now()).await;
		let seen = held::seen(&snapshotter);
		assert_eq!((seen.open(Step::JobsNoticeRefused), seen.open(Step::JobsNoticeUnanswered)), (Some(1.0), Some(0.0)));

		fake.answer("phone-1", Ok(503));
		crate::failure::open(Step::JobsNoticeUnanswered, 9);
		pass(&pool, &sender, now()).await;
		let seen = held::seen(&snapshotter);
		assert_eq!((seen.open(Step::JobsNoticeRefused), seen.open(Step::JobsNoticeUnanswered)), (Some(0.0), Some(1.0)));
		assert_eq!(seen.counted(Step::JobsNoticeUntold).unwrap_or(0), 0, "waiting is not lost");

		crate::failure::open(Step::JobsNoticeUnanswered, 9);
		assert_eq!(pass(&pool, &sender, now() + Duration::hours(12)).await, Outcome { expired: 1, ..Outcome::default() });
		let seen = held::seen(&snapshotter);
		assert_eq!(seen.open(Step::JobsNoticeUnanswered), Some(0.0), "past the window, not waiting");
		assert_eq!(seen.counted(Step::JobsNoticeUntold), Some(1));
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

	/// JN1 (#1206): a notice every phone refused was not delivered, so it is
	/// not recorded as told. It waits, and once the cause is fixed (a key FCM
	/// takes again) the next pass within the window tells the phone. Family:
	/// final refusals of the send (400, 401, 403) and of the token endpoint,
	/// with no phone, one phone gone, or one phone that took it. A JWT home
	/// could not sign (`Refused(0)`) takes the same arm, untested: a test key
	/// always signs.
	#[tokio::test]
	async fn a_notice_every_phone_refused_is_tried_again() {
		let pool = pool().await;
		let fake = Fake::default();
		let sender = sender(&fake);
		phone(&pool, "phone-1").await;
		let name = run(&pool, 1, "2026-10-26T14:52:07Z", "done", true).await;
		for status in [400, 401, 403] {
			fake.answer("phone-1", Ok(status));
			assert_eq!(pass(&pool, &sender, now()).await, Outcome { unsent: 1, ..Outcome::default() }, "{status}");
			assert!(sent(&pool).await.is_empty(), "{status} is not a delivery");
			assert_eq!(devices(&pool).await, ["phone-1"], "a refusal keeps the phone");
		}
		fake.answer("phone-1", Ok(200));
		assert_eq!(pass(&pool, &sender, now() + Duration::minutes(15)).await, Outcome { noticed: 1, ..Outcome::default() });
		assert_eq!(fake.took().len(), 1);
		assert_eq!(sent(&pool).await, [(name, String::from("picks"), 1)]);
	}

	/// JN1: a key Google won't take (its token endpoint refuses home's
	/// assertion) is fixed on home's side too, so the notice waits for it.
	#[tokio::test]
	async fn a_notice_home_could_not_get_a_token_for_is_tried_again() {
		let pool = pool().await;
		let fake = Fake::default();
		let sender = sender(&fake);
		phone(&pool, "phone-1").await;
		*fake.token.lock().unwrap() = Some(401);
		run(&pool, 1, "2026-10-26T14:52:07Z", "done", true).await;
		assert_eq!(pass(&pool, &sender, now()).await, Outcome { unsent: 1, ..Outcome::default() });
		assert!(sent(&pool).await.is_empty());

		*fake.token.lock().unwrap() = None;
		assert_eq!(pass(&pool, &sender, now() + Duration::minutes(15)).await, Outcome { noticed: 1, ..Outcome::default() });
		assert_eq!(fake.took().len(), 1);
	}

	/// JN1: a phone FCM forgot is forgotten, and the refusing one keeps the
	/// record waiting. Bound: the refusing phone is kept and sent to again
	/// next pass, however often it refuses (#1222).
	#[tokio::test]
	async fn a_gone_phone_beside_a_refusing_one_is_forgotten_and_the_notice_waits() {
		let pool = pool().await;
		let fake = Fake::default();
		phone(&pool, "gone").await;
		phone(&pool, "refusing").await;
		fake.answer("gone", Ok(404));
		fake.answer("refusing", Ok(403));
		run(&pool, 1, "2026-10-26T14:52:07Z", "done", true).await;
		assert_eq!(
			pass(&pool, &sender(&fake), now()).await,
			Outcome {
				forgotten: 1,
				unsent: 1,
				..Outcome::default()
			}
		);
		assert_eq!(devices(&pool).await, ["refusing"]);
		assert!(sent(&pool).await.is_empty());
	}

	/// JN1: one phone took it, so it was told: another phone's refusal does
	/// not hold the record back.
	#[tokio::test]
	async fn a_notice_one_phone_took_is_recorded_whatever_the_others_said() {
		let pool = pool().await;
		let fake = Fake::default();
		phone(&pool, "refusing").await;
		phone(&pool, "taking").await;
		fake.answer("refusing", Ok(401));
		let name = run(&pool, 1, "2026-10-26T14:52:07Z", "done", true).await;
		assert_eq!(pass(&pool, &sender(&fake), now()).await, Outcome { noticed: 1, ..Outcome::default() });
		assert_eq!(sent(&pool).await, [(name, String::from("picks"), 1)]);
	}

	/// The phones' strike state: (token, set aside, probed).
	async fn aside(pool: &SqlitePool) -> Vec<(String, bool, bool)> {
		let mut conn = pool.acquire().await.unwrap();
		notice_repo::phones(&mut conn, A)
			.await
			.unwrap()
			.into_iter()
			.map(|phone| (phone.token, phone.set_aside_at.is_some(), phone.last_probe_at.is_some()))
			.collect()
	}

	/// #1222, C1513: a phone whose token is another sender's is set aside at
	/// the third refusal in a row, counted once; it is then included once a
	/// day, in a send that happens anyway, and taking a notice brings it back.
	#[tokio::test]
	async fn a_phone_another_sender_owns_is_set_aside_once_and_comes_back_when_it_takes_one() {
		let recorder = DebuggingRecorder::new();
		let snapshotter = recorder.snapshotter();
		let _recording = metrics::set_default_local_recorder(&recorder);
		let pool = pool().await;
		let fake = Fake::default();
		let sender = sender(&fake);
		phone(&pool, "mismatch-1").await;
		fake.answer("mismatch-1", Ok(403));
		run(&pool, 1, "2026-10-26T14:52:07Z", "done", true).await;
		for minutes in [0, 15, 30] {
			pass(&pool, &sender, now() + Duration::minutes(minutes)).await;
		}
		assert_eq!(aside(&pool).await, [(String::from("mismatch-1"), true, false)]);
		assert_eq!(failures(&snapshotter, Step::JobsNoticePhoneSetAside), Some(1));

		// Its next send is the day's probe; the one after skips it.
		pass(&pool, &sender, now() + Duration::minutes(45)).await;
		assert_eq!(aside(&pool).await, [(String::from("mismatch-1"), true, true)]);
		fake.answer("mismatch-1", Ok(200));
		pass(&pool, &sender, now() + Duration::minutes(60)).await;
		assert!(fake.took().is_empty(), "skipped within the day");
		assert_eq!(failures(&snapshotter, Step::JobsNoticePhoneSetAside).unwrap_or(0), 0, "no new count");

		// A day on, the next morning's notice probes it, and it takes it: back.
		run(&pool, 2, "2026-10-27T15:00:00Z", "done", true).await;
		pass(&pool, &sender, now() + Duration::hours(25)).await;
		assert_eq!(fake.took().len(), 1, "the probe took the morning's notice");
		assert!(aside(&pool).await.iter().all(|(_, set_aside, _)| !set_aside), "back");
	}

	/// C1513: a lone phone's 400 never strikes (its token can't be told
	/// apart from our notice); beside a phone that took the notice, it does.
	#[tokio::test]
	async fn a_400_strikes_only_beside_a_phone_that_took_the_notice() {
		let pool = pool().await;
		let fake = Fake::default();
		let sender = sender(&fake);
		phone(&pool, "lone").await;
		fake.answer("lone", Ok(400));
		run(&pool, 1, "2026-10-26T14:52:07Z", "done", true).await;
		for minutes in [0, 15, 30, 45] {
			pass(&pool, &sender, now() + Duration::minutes(minutes)).await;
		}
		assert_eq!(aside(&pool).await, [(String::from("lone"), false, false)], "never struck");

		phone(&pool, "taking").await;
		for day in 1..=3 {
			run(&pool, day + 1, &format!("2026-10-{:02}T14:52:07Z", 26 + day), "done", true).await;
			pass(&pool, &sender, now() + Duration::days(day)).await;
		}
		assert!(aside(&pool).await.contains(&(String::from("lone"), true, false)), "{:?}", aside(&pool).await);
	}
}

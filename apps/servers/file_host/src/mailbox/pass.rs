//! One pass of the mailbox poller: read what `brief` holds after home's
//! cursor, and keep each file that checks out.
//!
//! A failure to reach or read the mailbox, or to write, ends the pass with
//! what it kept so far, and the next pass tries again from the cursor. A file
//! that does not check out (its bytes are not what the listing said, it is
//! not JSON, or its name already holds other bytes) is refused and counted,
//! and the pass goes on: one bad file never holds back the days after it.
//! A refused file that is the newest in `brief` is asked for again each pass
//! until a newer one is kept, since the cursor is the highest `seq` held.

use crate::auth::DeletionLock;
use chrono::{SecondsFormat, Utc};
use mailbox_kit::{Folder, Mailbox, MailboxError, MailboxTransport};
use mailbox_repo::{Change, ItemIn, BRIEF};
use serde::de::IgnoredAny;
use sqlx::SqlitePool;
use std::fmt;

/// The most files one pass takes. More waiting are taken by the next pass.
pub const ITEMS_PER_PASS: u32 = 20;

/// What a pass did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Outcome {
	/// Files kept this pass.
	pub kept: usize,
	/// Files refused this pass, each by why: `mismatch`, `bad_name`,
	/// `not_json` or `rewritten` (`metrics::mailbox::REFUSALS`).
	pub refused: Vec<&'static str>,
	/// The subject has no account: the pass stopped and kept nothing more.
	pub no_account: bool,
}

/// Why a pass ended early, and what it had done by then.
#[derive(Debug)]
pub struct PassError {
	pub outcome: Outcome,
	pub failure: Failure,
}

#[derive(Debug)]
pub enum Failure {
	Mailbox(MailboxError),
	Storage(sqlx::Error),
}

impl PassError {
	/// Which `metrics::mailbox::PASS_ERROR_CLASSES` bucket this lands in.
	#[must_use]
	pub const fn class(&self) -> &'static str {
		match &self.failure {
			Failure::Mailbox(err) => err.kind(),
			Failure::Storage(_) => "storage",
		}
	}
}

impl fmt::Display for PassError {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match &self.failure {
			Failure::Mailbox(err) => err.fmt(f),
			Failure::Storage(err) => write!(f, "database error: {err}"),
		}
	}
}

impl std::error::Error for PassError {}

/// Now, as ISO-8601 UTC to the millisecond.
#[must_use]
pub fn now() -> String {
	Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// Read `brief` after `subject`'s cursor and keep what checks out.
///
/// # Errors
/// A [`PassError`] when the mailbox cannot be listed or read (anything but a
/// file that does not check out), or the database fails.
pub async fn run_once<T: MailboxTransport>(
	db: &SqlitePool,
	mailbox: &Mailbox<T>,
	subject: &str,
	deletion: &DeletionLock,
	now: &(impl Fn() -> String + Sync),
) -> Result<Outcome, PassError> {
	let mut outcome = Outcome::default();
	let after = match cursor(db, subject).await {
		Ok(after) => after,
		Err(err) => {
			return Err(PassError {
				outcome,
				failure: Failure::Storage(err),
			})
		}
	};
	let items = match mailbox.list(Folder::Brief, after, ITEMS_PER_PASS).await {
		Ok(items) => items,
		Err(err) => {
			return Err(PassError {
				outcome,
				failure: Failure::Mailbox(err),
			})
		}
	};
	for item in items.iter().take(ITEMS_PER_PASS as usize) {
		let body = match mailbox.get(Folder::Brief, item).await {
			Ok(body) => body,
			Err(err @ (MailboxError::Mismatch | MailboxError::BadName)) => {
				outcome.refused.push(err.kind());
				continue;
			}
			Err(err) => {
				return Err(PassError {
					outcome,
					failure: Failure::Mailbox(err),
				})
			}
		};
		if serde_json::from_slice::<IgnoredAny>(&body).is_err() {
			outcome.refused.push("not_json");
			continue;
		}
		let kept = ItemIn {
			folder: BRIEF,
			name: &item.name,
			seq: item.seq,
			sha256: &item.sha256.to_ascii_lowercase(),
			body: &body,
		};
		match keep(db, deletion, subject, kept, &now()).await {
			Ok(Change::Added) => outcome.kept += 1,
			Ok(Change::Unchanged) => {}
			Ok(Change::NoAccount) => {
				outcome.no_account = true;
				return Ok(outcome);
			}
			Err(mailbox_repo::MailboxError::Rewritten) => outcome.refused.push("rewritten"),
			Err(mailbox_repo::MailboxError::Storage(err)) => {
				return Err(PassError {
					outcome,
					failure: Failure::Storage(err),
				})
			}
		}
	}
	Ok(outcome)
}

async fn cursor(db: &SqlitePool, subject: &str) -> Result<i64, sqlx::Error> {
	let mut conn = db.acquire().await?;
	mailbox_repo::cursor(&mut conn, subject, BRIEF).await.map_err(|err| match err {
		mailbox_repo::MailboxError::Storage(err) => err,
		mailbox_repo::MailboxError::Rewritten => sqlx::Error::Protocol(String::from("a cursor read cannot rewrite")),
	})
}

/// One write, under the deletion hold (docs/identity.md invariant 9):
/// `keep_item` checks the subject's account in the statement that writes, and
/// the hold keeps a deletion from landing between them. Held for this write
/// only, never across a request to the mailbox.
async fn keep(db: &SqlitePool, deletion: &DeletionLock, subject: &str, item: ItemIn<'_>, now: &str) -> Result<Change, mailbox_repo::MailboxError> {
	let _hold = deletion.hold().await;
	let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
	let change = mailbox_repo::keep_item(&mut tx, subject, item, now).await?;
	tx.commit().await?;
	Ok(change)
}

#[cfg(test)]
mod tests {
	use super::{run_once, Failure, Outcome, ITEMS_PER_PASS};
	use crate::auth::DeletionLock;
	use mailbox_kit::{Mailbox, MailboxError, MailboxTransport, Method, Request, Response, Token};
	use sha2::{Digest, Sha256};
	use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};
	use std::collections::HashMap;
	use std::sync::Mutex;

	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");
	const OWNER: &str = "subject-owner";
	const BASE: &str = "https://mailbox.example.test";

	/// A mailbox in memory: `brief`'s items in `seq` order, the listed SHA-256
	/// of each (which a test may make lie), and a switch that fails listings.
	#[derive(Default)]
	struct Fake {
		items: Mutex<Vec<(i64, String, String, String)>>,
		bodies: Mutex<HashMap<String, String>>,
		listing_fails: Mutex<bool>,
		gets: Mutex<Vec<String>>,
	}

	impl Fake {
		fn add(&self, seq: i64, name: &str, body: &str) {
			let sha = hex::encode(Sha256::digest(body.as_bytes()));
			self.items.lock().unwrap().push((seq, name.to_owned(), body.to_owned(), sha));
			self.bodies.lock().unwrap().insert(name.to_owned(), body.to_owned());
		}
	}

	fn answer(status: u16, body: &str) -> Response {
		Response {
			status,
			body: body.as_bytes().to_vec(),
		}
	}

	impl MailboxTransport for &Fake {
		type Error = &'static str;

		async fn send(&self, request: Request) -> Result<Response, Self::Error> {
			assert_eq!(request.method, Method::Get, "the inbound half only reads");
			let path = request.url.strip_prefix(BASE).unwrap().to_owned();
			if let Some(query) = path.strip_prefix("/v1/brief/items?") {
				if *self.listing_fails.lock().unwrap() {
					return Err("connection refused");
				}
				let after: i64 = query.split('&').find_map(|pair| pair.strip_prefix("after=")).unwrap().parse().unwrap();
				let limit: usize = query.split('&').find_map(|pair| pair.strip_prefix("limit=")).unwrap().parse().unwrap();
				let listed: Vec<serde_json::Value> = self
					.items
					.lock()
					.unwrap()
					.iter()
					.filter(|(seq, ..)| *seq > after)
					.take(limit)
					.map(|(seq, name, body, sha)| {
						serde_json::json!({
							"seq": seq, "name": name, "size": body.len(), "sha256": sha,
							"contentType": "application/json", "putAt": "2026-10-23T14:59:07.446Z"
						})
					})
					.collect();
				return Ok(answer(200, &serde_json::json!({ "items": listed }).to_string()));
			}
			let name = path.strip_prefix("/v1/brief/items/").unwrap().to_owned();
			self.gets.lock().unwrap().push(name.clone());
			Ok(self.bodies.lock().unwrap().get(&name).map_or_else(|| answer(404, "{}"), |body| answer(200, body)))
		}
	}

	async fn pool() -> SqlitePool {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();
		sqlx::query("INSERT INTO account (subject_id, user_handle) VALUES (?1, ?2)")
			.bind(OWNER)
			.bind(OWNER.as_bytes())
			.execute(&pool)
			.await
			.unwrap();
		pool
	}

	fn digest(day: &str) -> (String, String) {
		(
			String::from("digest-") + day + ".json",
			serde_json::json!({ "schema": "some-ui.job-digest/1", "day": day, "picks": [] }).to_string(),
		)
	}

	async fn pass(pool: &SqlitePool, fake: &Fake) -> Result<Outcome, super::PassError> {
		let mailbox = Mailbox::new(BASE, Token::new(String::from("home-token")), fake).unwrap();
		run_once(pool, &mailbox, OWNER, &DeletionLock::default(), &|| String::from("2026-10-23T15:00:00.000Z")).await
	}

	async fn held(pool: &SqlitePool) -> Vec<String> {
		sqlx::query_scalar("SELECT name FROM mailbox_item ORDER BY seq").fetch_all(pool).await.unwrap()
	}

	#[tokio::test]
	async fn a_new_digest_is_kept() {
		let (pool, fake) = (pool().await, Fake::default());
		let (name, body) = digest("2026-10-23");
		fake.add(10, &name, &body);
		assert_eq!(pass(&pool, &fake).await.unwrap().kept, 1);
		let (seq, stored): (i64, Vec<u8>) = sqlx::query_as("SELECT seq, body FROM mailbox_item WHERE subject_id = ?1 AND folder = 'brief' AND name = ?2")
			.bind(OWNER)
			.bind(&name)
			.fetch_one(&pool)
			.await
			.unwrap();
		assert_eq!((seq, stored), (10, body.into_bytes()), "the mailbox's seq and the bytes as it held them");
	}

	#[tokio::test]
	async fn a_held_item_is_not_fetched_again() {
		let (pool, fake) = (pool().await, Fake::default());
		let (first, body) = digest("2026-10-22");
		fake.add(9, &first, &body);
		pass(&pool, &fake).await.unwrap();
		let (second, body) = digest("2026-10-23");
		fake.add(10, &second, &body);
		assert_eq!(pass(&pool, &fake).await.unwrap().kept, 1);
		assert_eq!(pass(&pool, &fake).await.unwrap(), Outcome::default(), "an idle pass lists and fetches nothing");
		assert_eq!(*fake.gets.lock().unwrap(), [first, second.clone()], "each file fetched once");
		assert_eq!(held(&pool).await.last(), Some(&second));
	}

	#[tokio::test]
	async fn a_sha_mismatch_is_not_kept() {
		let (pool, fake) = (pool().await, Fake::default());
		let (bad, body) = digest("2026-10-22");
		fake.add(9, &bad, &body);
		fake
			.bodies
			.lock()
			.unwrap()
			.insert(bad, String::from(r#"{"schema":"some-ui.job-digest/1","tampered":true}"#));
		let (good, body) = digest("2026-10-23");
		fake.add(10, &good, &body);
		let outcome = pass(&pool, &fake).await.unwrap();
		assert_eq!((outcome.kept, outcome.refused), (1, vec!["mismatch"]), "refused, and the next day still kept");
		assert_eq!(held(&pool).await, [good]);
	}

	#[tokio::test]
	async fn a_non_json_body_is_not_kept() {
		let (pool, fake) = (pool().await, Fake::default());
		fake.add(9, "digest-2026-10-22.json", "not json");
		let outcome = pass(&pool, &fake).await.unwrap();
		assert_eq!((outcome.kept, outcome.refused), (0, vec!["not_json"]));
		assert!(held(&pool).await.is_empty());
	}

	#[tokio::test]
	async fn a_pass_stops_at_twenty() {
		let (pool, fake) = (pool().await, Fake::default());
		for n in 1..=25 {
			let day = String::from("2026-09-") + &format_day(n);
			let (name, body) = digest(&day);
			fake.add(i64::from(n), &name, &body);
		}
		assert_eq!(pass(&pool, &fake).await.unwrap().kept, ITEMS_PER_PASS as usize);
		assert_eq!(pass(&pool, &fake).await.unwrap().kept, 5, "the rest on the next pass");
		assert_eq!(held(&pool).await.len(), 25);
	}

	fn format_day(n: u8) -> String {
		if n < 10 {
			String::from("0") + &n.to_string()
		} else {
			n.to_string()
		}
	}

	#[tokio::test]
	async fn a_listing_error_keeps_nothing_half_written() {
		let (pool, fake) = (pool().await, Fake::default());
		let (name, body) = digest("2026-10-23");
		fake.add(10, &name, &body);
		*fake.listing_fails.lock().unwrap() = true;
		let err = pass(&pool, &fake).await.unwrap_err();
		assert!(matches!(err.failure, Failure::Mailbox(MailboxError::Transport(_))));
		assert_eq!((err.class(), err.outcome), ("transport", Outcome::default()));
		assert!(held(&pool).await.is_empty());
		*fake.listing_fails.lock().unwrap() = false;
		assert_eq!(pass(&pool, &fake).await.unwrap().kept, 1, "the next pass tries again");
	}

	#[tokio::test]
	async fn nothing_is_written_for_a_deleted_subject() {
		let (pool, fake) = (pool().await, Fake::default());
		sqlx::query("DELETE FROM account WHERE subject_id = ?1").bind(OWNER).execute(&pool).await.unwrap();
		let (name, body) = digest("2026-10-23");
		fake.add(10, &name, &body);
		let outcome = pass(&pool, &fake).await.unwrap();
		assert!(outcome.no_account);
		assert_eq!(outcome.kept, 0);
		assert!(held(&pool).await.is_empty());
	}

	/// A pass's write waits out a deletion that holds the lock, and then finds
	/// the account gone.
	#[tokio::test]
	async fn a_write_waits_out_a_deletion_in_flight() {
		let (pool, fake) = (pool().await, Fake::default());
		let (name, body) = digest("2026-10-23");
		fake.add(10, &name, &body);
		let deletion = DeletionLock::default();
		let exclusive = deletion.exclude().await;
		let mailbox = Mailbox::new(BASE, Token::new(String::from("home-token")), &fake).unwrap();
		let running = run_once(&pool, &mailbox, OWNER, &deletion, &|| String::from("2026-10-23T15:00:00.000Z"));
		let delete = async {
			tokio::task::yield_now().await;
			sqlx::query("DELETE FROM account WHERE subject_id = ?1").bind(OWNER).execute(&pool).await.unwrap();
			drop(exclusive);
		};
		let (outcome, ()) = tokio::join!(running, delete);
		assert!(outcome.unwrap().no_account);
		assert!(held(&pool).await.is_empty());
	}

	#[tokio::test]
	async fn a_disabled_mailbox_spawns_nothing() {
		use crate::Config;
		use clap::Parser;
		const REQUIRED: [&str; 9] = [
			"file_host",
			"--obs-host",
			"h",
			"--obs-password",
			"p",
			"--database-url",
			"sqlite::memory:",
			"--auth-enabled",
			"false",
		];
		let config = Config::try_parse_from(REQUIRED).unwrap();
		assert!(!config.mailbox_enabled, "off by default");
		assert!(super::super::MailboxContext::from_config(&config, DeletionLock::default()).unwrap().is_none());

		let on = |extra: &[&str]| {
			let args = REQUIRED.iter().chain(&["--mailbox-enabled", "true"]).chain(extra).copied();
			super::super::MailboxContext::from_config(&Config::try_parse_from(args).unwrap(), DeletionLock::default())
		};
		let url = ["--mailbox-url", BASE];
		let token = ["--mailbox-token", "home-token"];
		let subject = ["--mailbox-subject", OWNER];
		for (missing, args) in [
			("MAILBOX_URL", [&token[..], &subject[..]].concat()),
			("MAILBOX_TOKEN", [&url[..], &subject[..]].concat()),
			("MAILBOX_SUBJECT", [&url[..], &token[..]].concat()),
		] {
			let err = on(&args).err().unwrap().to_string();
			assert!(err.contains(missing), "{err}");
			assert!(!err.contains("home-token"), "the token is never in the error: {err}");
		}
		let all = [&url[..], &token[..], &subject[..]].concat();
		let insecure = [&["--mailbox-url", "http://mailbox.example.test"][..], &token[..], &subject[..]].concat();
		assert!(on(&insecure).is_err(), "only https");
		assert!(on(&[&all[..], &["--mailbox-seconds", "59"][..]].concat()).is_err(), "at least a minute");
		let context = on(&all).unwrap().unwrap();
		assert_eq!((context.subject.as_str(), context.interval.as_secs()), (OWNER, 900));
	}
}

//! Home renders each week's Rust Rounds essay (paulgsc/some-ui#2691, RR16,
//! epic paulgsc/some-ui#2231).
//!
//! After a week's deadline the routine writes its essay
//! (`some-ui.rust-essay/1`) and puts it in the mailbox's `brief` folder as
//! `rust-essay-<week>.json`, where the mailbox pass keeps it as any `brief`
//! file (owner, paulgsc/some-ui#2684, 2026-11-07: home renders from the
//! script it keeps; it never goes to `paulgsc/testing`). This worker gives
//! each essay home holds to home's render worker, which records the essay's
//! stage with the tutor's lines said by home's speaker and renders it
//! (`infra/render/render-calls.sh`, its `rust-essay` job).
//!
//! **Only from the week's final record, never a draft** (RR22,
//! paulgsc/some-ui#2832). Until a week's deadline its send is a draft that a
//! later send replaces, so an essay of a week whose deadline has not passed
//! waits. Past it, the pass makes the week final as any read of it does
//! (`rust_round_repo::sent::finalize`), and checks the essay against that
//! final (`rust_round_repo::essay::against`): an essay plays exactly the takes
//! the final names, and a ticket nothing answered is a silence. An essay that
//! contradicts the final is never given to the worker; it is refused, and the
//! refusal is kept in [`STATUS_FOLDER`] as `<week>.refused.log` (the fields
//! and why, nothing the essay says), for the `RustEssayRefused` alert.
//!
//! **What the worker is given** ([`export`]): `<week>/essay.json` in
//! [`ESSAYS_FOLDER`], byte for byte as the mailbox held it, written in a
//! temporary folder and renamed into place whole, once. An essay already
//! given is left alone: the mailbox never rewrites a kept file.
//!
//! **What it says.** Each minute, what the worker has made of the essays
//! given ([`observe`]), as Playoff Delta's (`pd_fills::render`): the CALL
//! dashboard and the `RustEssay*` alerts in `infra/prometheus/rules/home.yml`,
//! which reach the owner on Slack. A failed render is the worker's
//! `<week>.failed.log`, as every job's.
//!
//! **Whose.** The mailbox's subject: `MAILBOX_SUBJECT` when the deployment
//! set one, else the owner's account (`mailbox::pass_subject`). With no
//! mailbox configured, nothing is given.
//!
//! Cost: one read of the subject's essays in `brief` (one a week, each tens
//! of KiB) every [`INTERVAL`], and per essay not yet given, its week's file
//! and one `BEGIN IMMEDIATE` transaction; one `stat` per essay each minute.

use crate::auth::owner::Owner;
use crate::call_weeks::render::{tally, Held, Renders};
use crate::AppState;
use rust_round_repo::essay::{self, Played};
use rust_round_repo::round::Problem;
use sqlx::SqlitePool;
use std::path::Path;
use std::time::Duration;
use tracing::{error, info, warn};

/// Where each essay home gives the render worker is written.
///
/// Beside the database: on the host, `${STORAGE}/rust-essays`, mounted
/// read-only into the worker as `/essays` (`infra/compose/call-render.yml`).
pub const ESSAYS_FOLDER: &str = "/app/dbs/rust-essays";

/// Where the worker keeps each essay's video, as this server sees it.
pub const RENDERS_FOLDER: &str = "/app/dbs/renders/rust-essay";

/// Where the worker says how its passes went, and this server keeps each
/// essay it refused, as this server sees it.
pub const STATUS_FOLDER: &str = "/app/dbs/renders/rust-essay-worker";

/// Where the worker keeps a waiting essay's work and step cache between
/// attempts, as this server sees it.
pub const WORK_FOLDER: &str = "/app/dbs/renders/rust-essay-work";

/// The video's file in an essay's folder.
pub const VIDEO: &str = "essay.mp4";

/// The essay's file in its folder under [`ESSAYS_FOLDER`].
pub const ESSAY_FILE: &str = "essay.json";

/// How often home looks for an essay to give: one comes a week, after its
/// week's deadline.
pub const INTERVAL: Duration = Duration::from_secs(600);

/// How often the worker reads what the render worker has made.
pub const TICK: Duration = Duration::from_secs(60);

const _: () = assert!(TICK.as_secs() <= INTERVAL.as_secs());

/// What [`export`] did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Exported {
	/// Essays written for the worker.
	pub written: usize,
	/// Essays whose week's deadline has not passed, or whose week home does
	/// not hold yet.
	pub waiting: usize,
	/// Essays refused: unreadable, misnamed, or not the week's final record.
	pub refused: usize,
	/// Essays that could not be written.
	pub failed: usize,
}

/// Start the worker.
///
/// Its folders and the render worker's are made first, before the server
/// answers its health check, by this server's uid, which the worker runs as
/// too, so the worker never finds one missing or not its own.
pub fn start(state: &AppState) {
	for folder in [ESSAYS_FOLDER, RENDERS_FOLDER, WORK_FOLDER, STATUS_FOLDER] {
		if std::fs::create_dir_all(folder).is_err() {
			error!(error = "render folder", "rust essays worker cannot make the render worker's folders");
		}
	}
	let Some(configured) = state.mailbox.as_ref().map(|mailbox| mailbox.subject.clone()) else {
		info!("no mailbox, so no Rust Rounds essay reaches home: the rust essays worker is off");
		return;
	};
	let cancel = state.core.cancel_token.clone();
	let db = state.core.shared_db.clone();
	tokio::spawn(async move {
		info!(interval_secs = INTERVAL.as_secs(), "rust essays worker started");
		crate::metrics::rust_essays::record_started();
		let mut ticker = tokio::time::interval(TICK);
		ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
		let mut last: Option<tokio::time::Instant> = None;
		loop {
			tokio::select! {
				() = cancel.cancelled() => {
					info!("rust essays worker cancelled");
					return;
				}
				_ = ticker.tick() => {
					observe_renders().await;
					if last.is_some_and(|last| last.elapsed() < INTERVAL) {
						continue;
					}
					last = Some(tokio::time::Instant::now());
					let result = pass(&db, configured.as_deref()).await;
					crate::metrics::rust_essays::record_pass(&result);
					log_pass(&result);
				}
			}
		}
	});
}

/// One pass for the mailbox's subject; nothing when there is none.
async fn pass(db: &SqlitePool, configured: Option<&str>) -> Result<Exported, sqlx::Error> {
	match crate::mailbox::pass_subject(db, configured).await? {
		Owner::Subject(subject) => {
			let now = crate::handlers::rust_rounds::now();
			export(db, subject.as_str(), Path::new(ESSAYS_FOLDER), Path::new(STATUS_FOLDER), &now).await
		}
		Owner::NoAccount | Owner::Several => Ok(Exported::default()),
	}
}

/// Gives the render worker each essay `subject` holds that `folder` has
/// none of, once its week is final and the essay is that final's.
///
/// An essay refused is kept as `<week>.refused.log` in `status`, its fields
/// and why one a line, and that file is removed once the essay is given.
///
/// # Errors
/// A storage failure.
pub async fn export(db: &SqlitePool, subject: &str, folder: &Path, status: &Path, now: &str) -> Result<Exported, sqlx::Error> {
	let mut done = Exported::default();
	let held = {
		let mut conn = db.acquire().await?;
		mailbox_repo::rust_essays(&mut conn, subject).await.map_err(storage)?
	};
	for held in held {
		if folder.join(&held.id).exists() {
			continue;
		}
		match given(db, subject, &held, now).await? {
			Given::Waiting => done.waiting += 1,
			Given::Refused(problems) => {
				done.refused += 1;
				if keep_refusal(status, &held.id, &problems).await.is_err() {
					done.failed += 1;
				}
			}
			Given::Final(_played) => {
				let partial = folder.join(String::from(".") + &held.id + ".partial");
				if write_whole(&partial, &folder.join(&held.id), &held.body).await.is_ok() {
					done.written += 1;
					let _ = tokio::fs::remove_file(status.join(String::from(&held.id) + ".refused.log")).await;
				} else {
					done.failed += 1;
				}
			}
		}
	}
	Ok(done)
}

/// What an essay home holds is, for the worker.
enum Given {
	/// Its week's deadline has not passed, or home holds no such week yet.
	Waiting,
	/// Never to be rendered as it stands.
	Refused(Vec<Problem>),
	/// The week's final record's essay, with the takes it plays.
	Final(Vec<Played>),
}

async fn given(db: &SqlitePool, subject: &str, held: &mailbox_repo::HeldWeek, now: &str) -> Result<Given, sqlx::Error> {
	let essay = match essay::read(&held.body) {
		Ok(essay) if essay.week == held.id => essay,
		Ok(_) => return Ok(Given::Refused(vec![(String::from("week.id"), "is not the week the essay's name gives")])),
		Err(problems) => return Ok(Given::Refused(problems)),
	};
	let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
	let week_file = mailbox_repo::rust_week(&mut tx, subject, &rust_round_repo::week::mailbox_name(&essay.week))
		.await
		.map_err(storage)?;
	let Some(window) = week_file
		.and_then(|file| crate::handlers::rust_rounds::reads_as_named(&essay.week, &file.body))
		.and_then(|week| week.window)
	else {
		return Ok(Given::Waiting);
	};
	if rust_round_repo::instant_order(now, &window.due) != Some(std::cmp::Ordering::Greater) {
		return Ok(Given::Waiting);
	}
	rust_round_repo::sent::finalize(&mut tx, subject, &essay.week, &window.due, now).await?;
	let standing = rust_round_repo::sent::standing(&mut tx, subject, &essay.week).await?;
	tx.commit().await?;
	if standing.as_ref().is_some_and(|standing| standing.draft) {
		// A draft finalize could not make final (sent after the deadline):
		// left for a person to read, and never rendered from.
		return Ok(Given::Refused(vec![(String::from("week"), "has only a draft past its deadline, never a final")]));
	}
	Ok(match essay::against(&essay, standing.as_ref().map(|standing| &standing.sent)) {
		Ok(played) => Given::Final(played),
		Err(problems) => Given::Refused(problems),
	})
}

fn storage(err: mailbox_repo::MailboxError) -> sqlx::Error {
	match err {
		mailbox_repo::MailboxError::Storage(err) => err,
		mailbox_repo::MailboxError::Rewritten => sqlx::Error::Protocol(String::from("a mailbox file does not read back")),
	}
}

/// Keeps `problems` as `<week>.refused.log` in `status`, written whole.
async fn keep_refusal(status: &Path, week: &str, problems: &[Problem]) -> std::io::Result<()> {
	let mut text = String::new();
	for (field, why) in problems {
		text.push_str(field);
		text.push_str(": ");
		text.push_str(why);
		text.push('\n');
	}
	tokio::fs::create_dir_all(status).await?;
	// Not `.<week>.log`-shaped: the lane reads such a file as a render in
	// progress (`call_weeks::lane`).
	let partial = status.join(String::from(".") + week + ".refused.partial");
	tokio::fs::write(&partial, text).await?;
	tokio::fs::rename(&partial, status.join(String::from(week) + ".refused.log")).await
}

/// Writes the essay into `partial`, then renames it to `into`.
async fn write_whole(partial: &Path, into: &Path, essay: &[u8]) -> std::io::Result<()> {
	// What a stopped pass left is made again.
	match tokio::fs::remove_dir_all(partial).await {
		Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error),
		_ => {}
	}
	tokio::fs::create_dir_all(partial).await?;
	tokio::fs::write(partial.join(ESSAY_FILE), essay).await?;
	tokio::fs::rename(partial, into).await
}

/// What Rust Rounds essays' job takes of the drive
/// (`call_weeks::render::disk`), which the call's observation adds to its
/// own, so the drive's numbers and `CallRenderCacheFull` count every job.
pub async fn disk() -> crate::call_weeks::render::Disk {
	crate::call_weeks::render::disk(Path::new(RENDERS_FOLDER), Path::new(WORK_FOLDER), Path::new(STATUS_FOLDER)).await
}

/// What the worker has made of the essays given.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Seen {
	/// Essays home refused, each with its `refused.log`.
	pub refused: usize,
	pub renders: Renders,
}

/// Reads what the worker has made of each essay given to it.
///
/// An essay given is a folder in `essays`; it reads as rendered, waiting or
/// failed by the worker's files, as every job's (`call_weeks::render::tally`).
pub async fn observe(essays: &Path, renders: &Path, status: &Path) -> Seen {
	let mut held = Vec::new();
	if let Ok(mut entries) = tokio::fs::read_dir(essays).await {
		while let Ok(Some(entry)) = entries.next_entry().await {
			let Some(name) = entry.file_name().to_str().map(str::to_owned) else { continue };
			if !rust_round_repo::week::is_week_id(&name) {
				continue;
			}
			let taken_at = entry
				.metadata()
				.await
				.ok()
				.and_then(|meta| meta.modified().ok())
				.map(|at| chrono::DateTime::<chrono::Utc>::from(at).to_rfc3339())
				.unwrap_or_default();
			held.push(Held { key: name, taken_at });
		}
	}
	let mut refused = 0;
	if let Ok(mut entries) = tokio::fs::read_dir(status).await {
		while let Ok(Some(entry)) = entries.next_entry().await {
			let name = entry.file_name();
			let name = name.to_string_lossy();
			if name.ends_with(".refused.log") && !name.starts_with('.') {
				refused += 1;
			}
		}
	}
	Seen {
		refused,
		renders: tally(&held, &[], renders, status, VIDEO).await,
	}
}

async fn observe_renders() {
	let seen = observe(Path::new(ESSAYS_FOLDER), Path::new(RENDERS_FOLDER), Path::new(STATUS_FOLDER)).await;
	crate::metrics::rust_essays::record_renders(&seen);
}

/// The pass's one log line, when it gave an essay or something went wrong.
/// It counts; it never names a week's words.
fn log_pass(result: &Result<Exported, sqlx::Error>) {
	match result {
		Ok(done) if done.written == 0 && done.refused == 0 && done.failed == 0 => {}
		Ok(done) if done.refused == 0 && done.failed == 0 => info!(written = done.written, waiting = done.waiting, "rust essays pass"),
		Ok(done) => warn!(
			written = done.written,
			waiting = done.waiting,
			refused = done.refused,
			failed = done.failed,
			"rust essays pass"
		),
		Err(_) => error!(error = "storage", "rust essays pass failed"),
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use rust_round_repo::sent::{Answer, Sent};
	use serde_json::json;
	use sqlx::sqlite::SqlitePoolOptions;

	const ME: &str = "subject-me";
	const DUE: &str = "2026-11-15T23:59:00.000-08:00";
	const BEFORE: &str = "2026-11-15T20:00:00.000Z";
	const AFTER: &str = "2026-11-17T08:00:00.000Z";

	async fn pool() -> SqlitePool {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		crate::schema::MIGRATOR.run(&pool).await.unwrap();
		sqlx::query("INSERT INTO account (subject_id, user_handle) VALUES (?1, ?1)")
			.bind(ME)
			.execute(&pool)
			.await
			.unwrap();
		pool
	}

	async fn hold(pool: &SqlitePool, seq: i64, name: &str, body: &str) {
		let sha256 = rust_round_repo::content_hash(body.as_bytes());
		let item = mailbox_repo::ItemIn {
			folder: "brief",
			name,
			seq,
			sha256: &sha256,
			body: body.as_bytes(),
		};
		mailbox_repo::keep_item(&mut pool.acquire().await.unwrap(), ME, item, "2026-11-09T08:00:00.000Z")
			.await
			.unwrap();
	}

	async fn hold_week(pool: &SqlitePool, seq: i64, id: &str) {
		let week = json!({
			"schema": "some-ui.rust-week/1", "id": id, "opens": "2026-11-09T00:00:00.000-08:00", "due": DUE,
			"tickets": [{ "id": "QM-311", "program": "fn main() {}\n" }, { "id": "QM-312", "program": "fn main() {}\n" }],
		});
		hold(pool, seq, &rust_round_repo::week::mailbox_name(id), &week.to_string()).await;
	}

	fn essay(id: &str, kinds: [&str; 2]) -> String {
		json!({
			"schema": "some-ui.rust-essay/1",
			"week": { "id": id, "from": "2026-11-09", "to": "2026-11-15" },
			"tickets": [
				{ "id": "QM-311", "answer": { "kind": kinds[0] } },
				{ "id": "QM-312", "answer": { "kind": kinds[1] } },
			],
		})
		.to_string()
	}

	async fn send(pool: &SqlitePool, id: &str) {
		let sent = Sent {
			week: id.to_owned(),
			at: String::from("2026-11-15T19:30:00.000Z"),
			answers: vec![Answer {
				ticket: String::from("QM-311"),
				take: String::from("note-311"),
				recorded_at: String::from("2026-11-15T19:00:00.000Z"),
			}],
			passed: vec![String::from("QM-312")],
		};
		let mut conn = pool.acquire().await.unwrap();
		rust_round_repo::sent::put_draft(&mut conn, ME, &sent, BEFORE).await.unwrap();
	}

	/// An essay waits for its week's deadline, then is given once, from the
	/// week's final record; the week's draft is final after it.
	#[tokio::test]
	async fn an_essay_is_given_once_its_week_is_final_and_never_from_a_draft() {
		let (pool, dir) = (pool().await, tempfile::tempdir().unwrap());
		let (folder, status) = (dir.path().join("essays"), dir.path().join("status"));
		let body = essay("2026-w46", ["take", "silence"]);
		hold(&pool, 1, &essay::mailbox_name("2026-w46"), &body).await;

		let none = export(&pool, ME, &folder, &status, AFTER).await.unwrap();
		assert_eq!(
			none,
			Exported {
				waiting: 1,
				..Exported::default()
			},
			"no week held yet: it waits"
		);

		hold_week(&pool, 2, "2026-w46").await;
		send(&pool, "2026-w46").await;
		let early = export(&pool, ME, &folder, &status, BEFORE).await.unwrap();
		assert_eq!(
			early,
			Exported {
				waiting: 1,
				..Exported::default()
			},
			"before the deadline the send is a draft"
		);
		assert!(!folder.join("2026-w46").exists());

		let done = export(&pool, ME, &folder, &status, AFTER).await.unwrap();
		assert_eq!(
			done,
			Exported {
				written: 1,
				..Exported::default()
			}
		);
		assert_eq!(std::fs::read(folder.join("2026-w46").join(ESSAY_FILE)).unwrap(), body.as_bytes());
		let standing = rust_round_repo::sent::standing(&mut pool.acquire().await.unwrap(), ME, "2026-w46").await.unwrap().unwrap();
		assert!(!standing.draft, "the pass made the week final, as any read after its deadline does");

		std::fs::write(folder.join("2026-w46").join(ESSAY_FILE), b"touched").unwrap();
		assert_eq!(export(&pool, ME, &folder, &status, AFTER).await.unwrap(), Exported::default(), "given once");
		assert_eq!(std::fs::read(folder.join("2026-w46").join(ESSAY_FILE)).unwrap(), b"touched");
		let names: Vec<_> = std::fs::read_dir(&folder).unwrap().map(|entry| entry.unwrap().file_name()).collect();
		assert_eq!(names, ["2026-w46"], "no partial folder");
	}

	/// An essay that contradicts the week's final record, or is misnamed, is
	/// refused and kept as said; a week nothing was sent of is all silence.
	#[tokio::test]
	async fn an_essay_that_contradicts_the_weeks_final_is_refused_and_said() {
		let (pool, dir) = (pool().await, tempfile::tempdir().unwrap());
		let (folder, status) = (dir.path().join("essays"), dir.path().join("status"));
		hold_week(&pool, 1, "2026-w46").await;
		send(&pool, "2026-w46").await;
		hold(&pool, 2, &essay::mailbox_name("2026-w46"), &essay("2026-w46", ["silence", "silence"])).await;
		hold_week(&pool, 3, "2026-w47").await;
		hold(&pool, 4, &essay::mailbox_name("2026-w47"), &essay("2026-w47", ["silence", "silence"])).await;
		hold(&pool, 5, &essay::mailbox_name("2026-w45"), &essay("2026-w44", ["silence", "silence"])).await;

		let now = "2026-11-24T08:00:00.000Z";
		let done = export(&pool, ME, &folder, &status, now).await.unwrap();
		assert_eq!((done.written, done.refused, done.failed), (1, 2, 0), "w47 sent nothing: all silence, given");
		assert!(folder.join("2026-w47").join(ESSAY_FILE).exists());
		assert_eq!(
			std::fs::read_to_string(status.join("2026-w46.refused.log")).unwrap(),
			"tickets[0].answer.kind: is a silence, and the week's final record sent a take for this ticket\n"
		);
		assert_eq!(
			std::fs::read_to_string(status.join("2026-w45.refused.log")).unwrap(),
			"week.id: is not the week the essay's name gives\n"
		);
		let seen = observe(&folder, &dir.path().join("renders"), &status).await;
		assert_eq!((seen.refused, seen.renders.held, seen.renders.waiting), (2, 1, 1));
	}
}

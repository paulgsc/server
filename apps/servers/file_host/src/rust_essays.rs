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
use std::path::{Path, PathBuf};
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

/// Which revision of the week's essay its folder holds (C72), beside it.
pub const REVISION_FILE: &str = "revision";

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
	/// Essays withdrawn because a take they play was deleted from My data.
	pub withdrawn: usize,
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
					match tidy_tick(&db, configured.as_deref()).await {
						Ok(tidied) if tidied.failed > 0 => warn!(failed = tidied.failed, "rust essays: files that could not be removed are tried again next minute"),
						Ok(_) => {}
						Err(_) => error!(error = "storage", "rust essays tidy failed"),
					}
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

/// Where the worker reads and writes: the server's folders, a temporary one
/// in tests.
#[derive(Debug, Clone)]
pub struct Folders {
	/// [`ESSAYS_FOLDER`]: what the render worker is given.
	pub essays: PathBuf,
	/// [`RENDERS_FOLDER`]: what it made.
	pub renders: PathBuf,
	/// [`STATUS_FOLDER`]: how it went, and each essay refused.
	pub status: PathBuf,
}

impl Folders {
	#[must_use]
	pub fn server() -> Self {
		Self {
			essays: PathBuf::from(ESSAYS_FOLDER),
			renders: PathBuf::from(RENDERS_FOLDER),
			status: PathBuf::from(STATUS_FOLDER),
		}
	}
}

/// One pass for the mailbox's subject; nothing when there is none.
async fn pass(db: &SqlitePool, configured: Option<&str>) -> Result<Exported, sqlx::Error> {
	match crate::mailbox::pass_subject(db, configured).await? {
		Owner::Subject(subject) => {
			let now = crate::handlers::rust_rounds::now();
			export(db, subject.as_str(), &Folders::server(), &now).await
		}
		Owner::NoAccount | Owner::Several => Ok(Exported::default()),
	}
}

/// Each minute: [`tidy`] for the mailbox's subject, or for none once no
/// account stands (so an account deleted takes its essays' files with it).
async fn tidy_tick(db: &SqlitePool, configured: Option<&str>) -> Result<Tidied, sqlx::Error> {
	let now = crate::handlers::rust_rounds::now();
	match crate::mailbox::pass_subject(db, configured).await? {
		Owner::Subject(subject) => tidy(db, Some(subject.as_str()), &Folders::server(), &now).await,
		Owner::NoAccount => tidy(db, None, &Folders::server(), &now).await,
		// Whose is not known: nothing is removed on a guess.
		Owner::Several => Ok(Tidied::default()),
	}
}

/// Gives the render worker each essay `subject` holds that it has not been
/// given, once its week is final and the essay is that final's.
///
/// Beside the essay go the takes it plays
/// (`<week>/takes/<ticket>.<webm|ogg|m4a>` and, when home heard its words,
/// `<ticket>.words.json`).
///
/// An essay refused is kept as `<week>.refused.log` in the status folder,
/// its fields and why one a line, and that file is removed once the essay
/// is given.
///
/// **Revisions** (C72): a week's essay may be corrected under a new name,
/// `rust-essay-<week>-r<N>.json`, never a replacement; the highest revision
/// a week holds is its essay. It replaces a lower one given to the worker
/// only while that week has no video: a video made is not made again (a
/// re-render is the owner's to ask for, RR17). The revision given is kept
/// beside the essay as `<week>/revision`.
///
/// An essay of a week let go is never given again; one whose take
/// was deleted from My data is withdrawn for good
/// (`rust_round_repo::essay_video::withdraw`); one whose take has not
/// reached home yet waits.
///
/// # Errors
/// A storage failure.
pub async fn export(db: &SqlitePool, subject: &str, folders: &Folders, now: &str) -> Result<Exported, sqlx::Error> {
	// Held for the whole pass, so an essay let go while it runs is never
	// written back (`feeds::FOLDERS`).
	let _folders = crate::feeds::FOLDERS.lock().await;
	let mut done = Exported::default();
	let (held, let_go) = {
		let mut conn = db.acquire().await?;
		(
			mailbox_repo::rust_essays(&mut conn, subject).await.map_err(storage)?,
			rust_round_repo::essay_video::let_go(&mut conn, subject).await?,
		)
	};
	for (week, revision, held) in newest(held) {
		let input = folders.essays.join(&week);
		if let_go.contains(&week) || folders.renders.join(&week).join(VIDEO).exists() || given_revision(&input).await.is_some_and(|given| given >= revision) {
			continue;
		}
		match given(db, subject, &week, &held, now).await? {
			Given::Waiting => done.waiting += 1,
			Given::Refused(problems) => {
				done.refused += 1;
				if keep_refusal(&folders.status, &week, &problems).await.is_err() {
					done.failed += 1;
				}
			}
			Given::Final(played) => match takes(db, subject, &played).await? {
				Takes::Waiting => done.waiting += 1,
				Takes::Deleted(note) => {
					let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
					rust_round_repo::essay_video::withdraw(&mut tx, subject, &week, &note, 0, now).await?;
					tx.commit().await?;
					done.withdrawn += 1;
				}
				Takes::Ready(files) => {
					let partial = folders.essays.join(String::from(".") + &week + ".partial");
					if write_whole(&partial, &input, &held.body, revision, &files).await.is_ok() {
						done.written += 1;
						let _ = tokio::fs::remove_file(folders.status.join(week.clone() + ".refused.log")).await;
					} else {
						done.failed += 1;
					}
				}
			},
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

/// Each week's newest revision, oldest week first; a name that is not an
/// essay's is never read.
fn newest(held: Vec<mailbox_repo::HeldEssay>) -> Vec<(String, u32, mailbox_repo::HeldEssay)> {
	let mut weeks: std::collections::BTreeMap<String, (u32, mailbox_repo::HeldEssay)> = std::collections::BTreeMap::new();
	for file in held {
		let Some((week, revision)) = essay::parse_name(&file.name) else { continue };
		if weeks.get(&week).is_none_or(|(held, _)| revision > *held) {
			weeks.insert(week, (revision, file));
		}
	}
	weeks.into_iter().map(|(week, (revision, file))| (week, revision, file)).collect()
}

/// The revision of the essay given in `input`, if one was.
async fn given_revision(input: &Path) -> Option<u32> {
	let text = tokio::fs::read_to_string(input.join(REVISION_FILE)).await.ok()?;
	text.trim().parse().ok()
}

async fn given(db: &SqlitePool, subject: &str, week: &str, held: &mailbox_repo::HeldEssay, now: &str) -> Result<Given, sqlx::Error> {
	let essay = match essay::read(&held.body) {
		Ok(essay) if essay.week == week => essay,
		Ok(_) => return Ok(Given::Refused(vec![(String::from("week.id"), "is not the week the essay's name gives")])),
		Err(problems) => return Ok(Given::Refused(problems)),
	};
	let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
	let Some(due) = due_of(&mut tx, subject, &essay.week).await? else {
		return Ok(Given::Waiting);
	};
	if rust_round_repo::instant_order(now, &due) != Some(std::cmp::Ordering::Greater) {
		return Ok(Given::Waiting);
	}
	rust_round_repo::sent::finalize(&mut tx, subject, &essay.week, &due, now).await?;
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

/// The deadline of the week `subject` holds as `week`, when it reads as one
/// with a window.
async fn due_of(conn: &mut sqlx::SqliteConnection, subject: &str, week: &str) -> Result<Option<String>, sqlx::Error> {
	let file = mailbox_repo::rust_week(conn, subject, &rust_round_repo::week::mailbox_name(week)).await.map_err(storage)?;
	Ok(
		file
			.and_then(|file| crate::handlers::rust_rounds::reads_as_named(week, &file.body))
			.and_then(|week| week.window)
			.map(|window| window.due),
	)
}

/// The takes an essay plays, as home holds them.
enum Takes {
	/// Each take's file name in `takes/` and its bytes, and its words beside it.
	Ready(Vec<(String, Vec<u8>)>),
	/// A take has not reached home yet.
	Waiting,
	/// A take was deleted from My data: this note.
	Deleted(String),
}

async fn takes(db: &SqlitePool, subject: &str, played: &[Played]) -> Result<Takes, sqlx::Error> {
	let mut conn = db.acquire().await?;
	let mut files = Vec::with_capacity(played.len() * 2);
	for take in played {
		let head = voice_repo::audio_head(&mut conn, subject, &take.take, voice_repo::Of::RustRound).await.map_err(voice)?;
		let body = match head {
			Some(_) => voice_repo::audio_body(&mut conn, subject, &take.take, voice_repo::Of::RustRound).await.map_err(voice)?,
			None => None,
		};
		let (Some(head), Some(body)) = (head, body) else {
			if voice_repo::delete::receipt(&mut conn, subject, &take.take).await.map_err(voice)?.is_some() {
				return Ok(Takes::Deleted(take.take.clone()));
			}
			return Ok(Takes::Waiting);
		};
		let extension = match head.content_type {
			"audio/ogg" => "ogg",
			"audio/mp4" => "m4a",
			_ => "webm",
		};
		files.push((take.ticket.clone() + "." + extension, body));
		let words = voice_repo::transcript::words(&mut conn, subject, &take.take).await.map_err(voice)?;
		if !words.is_empty() {
			let timed: Vec<serde_json::Value> = words
				.iter()
				.map(|word| serde_json::json!({ "text": word.text, "startMs": word.start_ms, "endMs": word.end_ms }))
				.collect();
			files.push((take.ticket.clone() + ".words.json", serde_json::Value::Array(timed).to_string().into_bytes()));
		}
	}
	Ok(Takes::Ready(files))
}

fn voice(err: voice_repo::VoiceError) -> sqlx::Error {
	match err {
		voice_repo::VoiceError::Storage(err) => err,
		_ => sqlx::Error::Protocol(String::from("a held take does not read back")),
	}
}

/// What [`tidy`] did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Tidied {
	/// Takes folders removed once their essay's video was made.
	pub takes_removed: usize,
	/// Essay weeks whose files went: no longer the subject's, or let go.
	pub weeks_removed: usize,
	/// Weeks withdrawn because a take they play was deleted.
	pub withdrawn: usize,
	/// Files that could not be removed; tried again next minute.
	pub failed: usize,
}

/// Each minute, the drive brought in line with what home holds:
///
/// - an essay week `subject` no longer holds (its account was deleted, or
///   none stands: `subject` is `None`) leaves the drive, input and render;
/// - a week let go or withdrawn leaves the input folder (its render goes by
///   the let-go's own steps, `feeds::videos`);
/// - a week whose final record names a take deleted from My data is
///   withdrawn, its input and render removed;
/// - a week whose video is made keeps no takes in its input.
///
/// # Errors
/// A storage failure.
pub async fn tidy(db: &SqlitePool, subject: Option<&str>, folders: &Folders, now: &str) -> Result<Tidied, sqlx::Error> {
	let _folders = crate::feeds::FOLDERS.lock().await;
	let mut done = Tidied::default();
	let (held, let_go) = match subject {
		Some(subject) => {
			let mut conn = db.acquire().await?;
			(
				mailbox_repo::rust_essay_weeks(&mut conn, subject).await.map_err(storage)?,
				rust_round_repo::essay_video::let_go(&mut conn, subject).await?,
			)
		}
		None => (Vec::new(), Vec::new()),
	};
	let on_drive = {
		let mut weeks = weeks_in(&folders.essays).await;
		for week in weeks_in(&folders.renders).await {
			if !weeks.contains(&week) {
				weeks.push(week);
			}
		}
		weeks
	};
	for week in on_drive {
		let (input, render) = (folders.essays.join(&week), folders.renders.join(&week));
		if !held.contains(&week) {
			done.weeks_removed += 1;
			done.failed += usize::from(remove(&input).await.is_err()) + usize::from(remove(&render).await.is_err());
			continue;
		}
		if let_go.contains(&week) {
			if input.exists() {
				done.weeks_removed += 1;
				done.failed += usize::from(remove(&input).await.is_err());
			}
			continue;
		}
		let Some(subject) = subject else { continue };
		if let Some(note) = deleted_take(db, subject, &week).await? {
			let size = i64::try_from(crate::call_weeks::render::bytes_under(&render).await).unwrap_or(i64::MAX);
			let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
			rust_round_repo::essay_video::withdraw(&mut tx, subject, &week, &note, size, now).await?;
			tx.commit().await?;
			done.withdrawn += 1;
			done.failed += usize::from(remove(&input).await.is_err()) + usize::from(remove(&render).await.is_err());
			continue;
		}
		let takes = input.join("takes");
		if render.join(VIDEO).exists() && takes.exists() {
			done.takes_removed += 1;
			done.failed += usize::from(remove(&takes).await.is_err());
		}
	}
	Ok(done)
}

/// A take `week`'s final record names that was deleted from My data, if any.
async fn deleted_take(db: &SqlitePool, subject: &str, week: &str) -> Result<Option<String>, sqlx::Error> {
	let mut conn = db.acquire().await?;
	let Some(standing) = rust_round_repo::sent::standing(&mut conn, subject, week).await? else {
		return Ok(None);
	};
	for answer in standing.sent.answers {
		if voice_repo::delete::receipt(&mut conn, subject, &answer.take).await.map_err(voice)?.is_some() {
			return Ok(Some(answer.take));
		}
	}
	Ok(None)
}

/// The week folders directly under `folder`; none when it can't be read.
async fn weeks_in(folder: &Path) -> Vec<String> {
	let mut weeks = Vec::new();
	if let Ok(mut entries) = tokio::fs::read_dir(folder).await {
		while let Ok(Some(entry)) = entries.next_entry().await {
			if let Some(name) = entry.file_name().to_str() {
				if rust_round_repo::week::is_week_id(name) {
					weeks.push(name.to_owned());
				}
			}
		}
	}
	weeks.sort();
	weeks
}

async fn remove(dir: &Path) -> std::io::Result<()> {
	match tokio::fs::remove_dir_all(dir).await {
		Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error),
		_ => Ok(()),
	}
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

/// Writes the essay, its revision and its takes into `partial`, then puts it
/// in place of `into`: a lower revision given before is replaced whole.
async fn write_whole(partial: &Path, into: &Path, essay: &[u8], revision: u32, takes: &[(String, Vec<u8>)]) -> std::io::Result<()> {
	// What a stopped pass left is made again.
	match tokio::fs::remove_dir_all(partial).await {
		Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error),
		_ => {}
	}
	tokio::fs::create_dir_all(partial).await?;
	tokio::fs::write(partial.join(ESSAY_FILE), essay).await?;
	tokio::fs::write(partial.join(REVISION_FILE), revision.to_string() + "\n").await?;
	// The recorder takes exactly these files in takes/, and an essay that
	// plays none still has the folder it is pointed at.
	tokio::fs::create_dir_all(partial.join("takes")).await?;
	for (name, body) in takes {
		tokio::fs::write(partial.join("takes").join(name), body).await?;
	}
	remove(into).await?;
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

/// The essays `subject` holds that were given to the worker, each with when
/// it was given (its input folder's time), and the weeks let go or
/// withdrawn, for the phone's Renders page (`feeds::renders`).
///
/// # Errors
/// A storage failure.
pub(crate) async fn held(db: &SqlitePool, subject: &str, essays: &Path) -> Result<(Vec<Held>, Vec<String>), sqlx::Error> {
	let (weeks, let_go) = {
		let mut conn = db.acquire().await?;
		(
			mailbox_repo::rust_essay_weeks(&mut conn, subject).await.map_err(storage)?,
			rust_round_repo::essay_video::let_go(&mut conn, subject).await?,
		)
	};
	let mut held = Vec::new();
	for week in weeks {
		let Ok(meta) = tokio::fs::metadata(essays.join(&week)).await else {
			if let_go.contains(&week) {
				held.push(Held {
					key: week,
					taken_at: String::new(),
				});
			}
			continue;
		};
		let taken_at = meta.modified().ok().map(|at| chrono::DateTime::<chrono::Utc>::from(at).to_rfc3339()).unwrap_or_default();
		held.push(Held { key: week, taken_at });
	}
	Ok((held, let_go))
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
	const TAKE: &str = "note-311";

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

	fn folders(dir: &Path) -> Folders {
		Folders {
			essays: dir.join("essays"),
			renders: dir.join("renders"),
			status: dir.join("status"),
		}
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
				take: String::from(TAKE),
				recorded_at: String::from("2026-11-15T19:00:00.000Z"),
			}],
			passed: vec![String::from("QM-312")],
		};
		let mut conn = pool.acquire().await.unwrap();
		rust_round_repo::sent::put_draft(&mut conn, ME, &sent, BEFORE).await.unwrap();
	}

	/// The take, as the phone sends it home, with the words home heard in it.
	async fn hold_take(pool: &SqlitePool) {
		for statement in [
			"INSERT INTO voice_note (subject_id, note_id, of_kind, of_ref, recorded_at, content_type, size, sha256, stored_at)
			 VALUES (?1, 'note-311', 'rust-round', '2026-w46:QM-311', 'then', 'audio/ogg', 4, 's', 'now')",
			"INSERT INTO voice_note_audio (subject_id, note_id, body) VALUES (?1, 'note-311', x'4f676753')",
			"INSERT INTO voice_transcript (subject_id, note_id, model, engine, attempt, outcome, language, language_p, duration_ms, text, took_ms, heard_at)
			 VALUES (?1, 'note-311', 'm', 'e', 1, 'heard', 'en', 1.0, 900, 'every field', 1, 'now')",
			"INSERT INTO voice_transcript_word (subject_id, note_id, model, word_index, text, start_ms, end_ms) VALUES (?1, 'note-311', 'm', 0, 'every', 0, 400)",
			"INSERT INTO voice_transcript_word (subject_id, note_id, model, word_index, text, start_ms, end_ms) VALUES (?1, 'note-311', 'm', 1, 'field', 450, 900)",
		] {
			sqlx::query(statement).bind(ME).execute(pool).await.unwrap();
		}
	}

	/// A take deleted from My data: its note gone, its receipt kept.
	async fn delete_take(pool: &SqlitePool) {
		for statement in [
			"DELETE FROM voice_transcript_word WHERE subject_id = ?1 AND note_id = 'note-311'",
			"DELETE FROM voice_transcript WHERE subject_id = ?1 AND note_id = 'note-311'",
			"DELETE FROM voice_note_audio WHERE subject_id = ?1 AND note_id = 'note-311'",
			"DELETE FROM voice_note WHERE subject_id = ?1 AND note_id = 'note-311'",
			"INSERT INTO voice_note_deletion (subject_id, note_id, of_kind, recorded_at, size, stored_at, deleted_at, went, stayed)
			 VALUES (?1, 'note-311', 'rust-round', 'then', 4, 'then', 'now', '[]', '[]')",
		] {
			sqlx::query(statement).bind(ME).execute(pool).await.unwrap();
		}
	}

	/// A week held, sent and past its deadline, with its essay.
	async fn final_week(pool: &SqlitePool) -> String {
		let body = essay("2026-w46", ["take", "silence"]);
		hold(pool, 1, &essay::mailbox_name("2026-w46"), &body).await;
		hold_week(pool, 2, "2026-w46").await;
		send(pool, "2026-w46").await;
		body
	}

	/// An essay waits for its week's deadline and for its takes, then is
	/// given once, from the week's final record, with each take it plays and
	/// the words home heard in it; the week's draft is final after it.
	#[tokio::test]
	async fn an_essay_is_given_once_its_week_is_final_and_never_from_a_draft() {
		let (pool, dir) = (pool().await, tempfile::tempdir().unwrap());
		let folders = folders(dir.path());
		let body = essay("2026-w46", ["take", "silence"]);
		hold(&pool, 1, &essay::mailbox_name("2026-w46"), &body).await;

		let none = export(&pool, ME, &folders, AFTER).await.unwrap();
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
		let early = export(&pool, ME, &folders, BEFORE).await.unwrap();
		assert_eq!(
			early,
			Exported {
				waiting: 1,
				..Exported::default()
			},
			"before the deadline the send is a draft"
		);
		let no_take = export(&pool, ME, &folders, AFTER).await.unwrap();
		assert_eq!(
			no_take,
			Exported {
				waiting: 1,
				..Exported::default()
			},
			"its take has not reached home: it waits"
		);
		assert!(!folders.essays.join("2026-w46").exists());
		let standing = rust_round_repo::sent::standing(&mut pool.acquire().await.unwrap(), ME, "2026-w46").await.unwrap().unwrap();
		assert!(!standing.draft, "the pass made the week final, as any read after its deadline does");

		hold_take(&pool).await;
		let done = export(&pool, ME, &folders, AFTER).await.unwrap();
		assert_eq!(
			done,
			Exported {
				written: 1,
				..Exported::default()
			}
		);
		let week = folders.essays.join("2026-w46");
		assert_eq!(std::fs::read(week.join(ESSAY_FILE)).unwrap(), body.as_bytes());
		assert_eq!(std::fs::read(week.join("takes").join("QM-311.ogg")).unwrap(), b"OggS");
		let words: serde_json::Value = serde_json::from_slice(&std::fs::read(week.join("takes").join("QM-311.words.json")).unwrap()).unwrap();
		assert_eq!(
			words,
			json!([{ "text": "every", "startMs": 0, "endMs": 400 }, { "text": "field", "startMs": 450, "endMs": 900 }])
		);
		let mut names: Vec<_> = std::fs::read_dir(week.join("takes")).unwrap().map(|entry| entry.unwrap().file_name()).collect();
		names.sort();
		assert_eq!(names, ["QM-311.ogg", "QM-311.words.json"], "only what the recorder takes: no file for a silence");

		std::fs::write(week.join(ESSAY_FILE), b"touched").unwrap();
		assert_eq!(export(&pool, ME, &folders, AFTER).await.unwrap(), Exported::default(), "given once");
		assert_eq!(std::fs::read(week.join(ESSAY_FILE)).unwrap(), b"touched");
		let names: Vec<_> = std::fs::read_dir(&folders.essays).unwrap().map(|entry| entry.unwrap().file_name()).collect();
		assert_eq!(names, ["2026-w46"], "no partial folder");
	}

	/// An essay that contradicts the week's final record, or is misnamed, is
	/// refused and kept as said; a week nothing was sent of is all silence.
	#[tokio::test]
	async fn an_essay_that_contradicts_the_weeks_final_is_refused_and_said() {
		let (pool, dir) = (pool().await, tempfile::tempdir().unwrap());
		let folders = folders(dir.path());
		hold_week(&pool, 1, "2026-w46").await;
		send(&pool, "2026-w46").await;
		hold(&pool, 2, &essay::mailbox_name("2026-w46"), &essay("2026-w46", ["silence", "silence"])).await;
		hold_week(&pool, 3, "2026-w47").await;
		hold(&pool, 4, &essay::mailbox_name("2026-w47"), &essay("2026-w47", ["silence", "silence"])).await;
		hold(&pool, 5, &essay::mailbox_name("2026-w45"), &essay("2026-w44", ["silence", "silence"])).await;

		let now = "2026-11-24T08:00:00.000Z";
		let done = export(&pool, ME, &folders, now).await.unwrap();
		assert_eq!((done.written, done.refused, done.failed), (1, 2, 0), "w47 sent nothing: all silence, given");
		assert!(folders.essays.join("2026-w47").join(ESSAY_FILE).exists());
		assert!(folders.essays.join("2026-w47").join("takes").is_dir(), "an empty takes folder for an essay of silences");
		assert_eq!(
			std::fs::read_to_string(folders.status.join("2026-w46.refused.log")).unwrap(),
			"tickets[0].answer.kind: is a silence, and the week's final record sent a take for this ticket\n"
		);
		assert_eq!(
			std::fs::read_to_string(folders.status.join("2026-w45.refused.log")).unwrap(),
			"week.id: is not the week the essay's name gives\n"
		);
		let seen = observe(&folders.essays, &folders.renders, &folders.status).await;
		assert_eq!((seen.refused, seen.renders.held, seen.renders.waiting), (2, 1, 1));
	}

	/// Invariant 23's exception, (b): the takes are in the render's input
	/// only while the week waits for its video.
	#[tokio::test]
	async fn a_kept_essay_copies_its_takes_only_until_its_video_is_made() {
		let (pool, dir) = (pool().await, tempfile::tempdir().unwrap());
		let folders = folders(dir.path());
		final_week(&pool).await;
		hold_take(&pool).await;
		assert_eq!(export(&pool, ME, &folders, AFTER).await.unwrap().written, 1);
		let takes = folders.essays.join("2026-w46").join("takes");
		assert_eq!(tidy(&pool, Some(ME), &folders, AFTER).await.unwrap(), Tidied::default(), "waiting: the takes stay");
		assert!(takes.join("QM-311.ogg").exists());

		std::fs::create_dir_all(folders.renders.join("2026-w46")).unwrap();
		std::fs::write(folders.renders.join("2026-w46").join(VIDEO), b"video").unwrap();
		let tidied = tidy(&pool, Some(ME), &folders, AFTER).await.unwrap();
		assert_eq!(
			tidied,
			Tidied {
				takes_removed: 1,
				..Tidied::default()
			}
		);
		assert!(!takes.exists(), "the video is made: no copy of a take is left in its input");
		assert!(
			folders.essays.join("2026-w46").join(ESSAY_FILE).exists(),
			"the essay stays, so the worker still holds the week"
		);
		assert_eq!(export(&pool, ME, &folders, AFTER).await.unwrap(), Exported::default(), "and it is never given again");
	}

	/// Invariant 23's exception, (c): a take deleted from My data takes the
	/// week's essay video and folder with it, and the week is never rendered
	/// again; before it was given, it is never given.
	#[tokio::test]
	async fn an_essay_whose_take_is_deleted_is_withdrawn_and_its_files_go() {
		let (pool, dir) = (pool().await, tempfile::tempdir().unwrap());
		let folders = folders(dir.path());
		final_week(&pool).await;
		hold_take(&pool).await;
		export(&pool, ME, &folders, AFTER).await.unwrap();
		std::fs::create_dir_all(folders.renders.join("2026-w46")).unwrap();
		std::fs::write(folders.renders.join("2026-w46").join(VIDEO), b"video").unwrap();

		delete_take(&pool).await;
		let tidied = tidy(&pool, Some(ME), &folders, AFTER).await.unwrap();
		assert_eq!(
			tidied,
			Tidied {
				withdrawn: 1,
				..Tidied::default()
			}
		);
		assert!(
			!folders.essays.join("2026-w46").exists() && !folders.renders.join("2026-w46").exists(),
			"its input and its video went"
		);
		let mut conn = pool.acquire().await.unwrap();
		let withdrawn = rust_round_repo::essay_video::withdrawn(&mut conn, ME).await.unwrap();
		assert_eq!((withdrawn[0].week.as_str(), withdrawn[0].note.as_str(), withdrawn[0].size), ("2026-w46", TAKE, 5));
		drop(conn);
		assert_eq!(export(&pool, ME, &folders, AFTER).await.unwrap(), Exported::default(), "never given again");
		assert!(!folders.essays.join("2026-w46").exists());

		// Deleted before it was ever given: withdrawn at the pass, never given.
		let (pool, dir) = (super::tests::pool().await, tempfile::tempdir().unwrap());
		let folders = super::tests::folders(dir.path());
		final_week(&pool).await;
		hold_take(&pool).await;
		delete_take(&pool).await;
		assert_eq!(
			export(&pool, ME, &folders, AFTER).await.unwrap(),
			Exported {
				withdrawn: 1,
				..Exported::default()
			}
		);
		assert!(!folders.essays.join("2026-w46").exists());
	}

	/// Invariant 23's exception, (a): once the subject no longer holds an
	/// essay (its account was deleted, or none stands), its files leave the
	/// drive at the next minute; a week let go leaves the input.
	#[tokio::test]
	async fn an_essay_the_subject_no_longer_holds_leaves_the_drive() {
		let (pool, dir) = (pool().await, tempfile::tempdir().unwrap());
		let folders = folders(dir.path());
		final_week(&pool).await;
		hold_take(&pool).await;
		export(&pool, ME, &folders, AFTER).await.unwrap();
		std::fs::create_dir_all(folders.renders.join("2026-w46")).unwrap();
		std::fs::write(folders.renders.join("2026-w46").join(VIDEO), b"video").unwrap();
		std::fs::create_dir_all(folders.renders.join("2026-w40")).unwrap();

		let tidied = tidy(&pool, Some(ME), &folders, AFTER).await.unwrap();
		assert_eq!((tidied.weeks_removed, tidied.takes_removed), (1, 1), "a render of a week with no essay goes");
		assert!(!folders.renders.join("2026-w40").exists());

		sqlx::query("DELETE FROM account WHERE subject_id = ?1").bind(ME).execute(&pool).await.unwrap();
		let tidied = tidy(&pool, None, &folders, AFTER).await.unwrap();
		assert_eq!(tidied.weeks_removed, 1);
		assert!(
			!folders.essays.join("2026-w46").exists() && !folders.renders.join("2026-w46").exists(),
			"the account went: so did its essays"
		);
	}

	/// C72: a correction is a new name, `-r<N>`; the highest revision is
	/// the week's essay, and it replaces a lower one given only while the
	/// week has no video.
	#[tokio::test]
	async fn a_corrected_essay_replaces_the_one_given_only_until_its_video_is_made() {
		let (pool, dir) = (pool().await, tempfile::tempdir().unwrap());
		let folders = folders(dir.path());
		final_week(&pool).await;
		hold_take(&pool).await;
		hold(&pool, 3, "rust-essay-2026-w46-r02.json", "not an essay's name").await;
		assert_eq!(export(&pool, ME, &folders, AFTER).await.unwrap().written, 1);
		let week = folders.essays.join("2026-w46");
		assert_eq!(std::fs::read_to_string(week.join(REVISION_FILE)).unwrap(), "1\n", "a name not a revision's is never read");

		let r2 = essay("2026-w46", ["take", "silence"]).replace("2026-11-09", "2026-11-10");
		hold(&pool, 4, "rust-essay-2026-w46-r2.json", &r2).await;
		assert_eq!(export(&pool, ME, &folders, AFTER).await.unwrap().written, 1, "no video yet: the correction is given");
		assert_eq!(std::fs::read_to_string(week.join(REVISION_FILE)).unwrap(), "2\n");
		assert_eq!(std::fs::read(week.join(ESSAY_FILE)).unwrap(), r2.as_bytes());
		assert!(week.join("takes").join("QM-311.ogg").exists());

		std::fs::create_dir_all(folders.renders.join("2026-w46")).unwrap();
		std::fs::write(folders.renders.join("2026-w46").join(VIDEO), b"video").unwrap();
		hold(&pool, 5, "rust-essay-2026-w46-r3.json", &essay("2026-w46", ["take", "silence"])).await;
		assert_eq!(
			export(&pool, ME, &folders, AFTER).await.unwrap(),
			Exported::default(),
			"a video made is not made again (RR17)"
		);
		assert_eq!(std::fs::read_to_string(week.join(REVISION_FILE)).unwrap(), "2\n");
		let mut conn = pool.acquire().await.unwrap();
		assert_eq!(
			mailbox_repo::rust_essay_weeks(&mut conn, ME).await.unwrap(),
			["2026-w46"],
			"one week, however many revisions"
		);
	}
}

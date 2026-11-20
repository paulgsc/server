//! Home renders each week's essay: one export over the essay's kind.
//!
//! **Why this module is named `rust_essays`** though it is every kind's
//! ([`Kind`]): it keeps its first kind's name because paulgsc/some-ui's
//! assurance ledger names this file and its tests as proofs, so a rename
//! moves together with the ledger (server#899). Game Week's essay is a third
//! kind here, not a new module.
//!
//! The kinds: a Rust Rounds week's (paulgsc/some-ui#2691, RR16, epic
//! paulgsc/some-ui#2231) and a week of mock interview's (paulgsc/some-ui#2937,
//! IW6, epic paulgsc/some-ui#2931; server#899). What differs between kinds is
//! named once, in [`Spec`] and in the few reads that match on the kind (its
//! essays in the mailbox, its week's final, its takes); every rule below is
//! one rule for every kind. A third kind (Game Week Bites' essay,
//! paulgsc/some-ui#2074) is a [`Kind`] and a [`Spec`], not a copy.
//!
//! The rest of this says it for Rust Rounds; a week of mock interview's essay
//! is the same with `interview-essay-<week>.json`, the interview week's final
//! (`rust_round_repo::interview`, IW4, server#847), its questions in place of
//! tickets (`takes/<question>.<ext>`) and its cards read beside them
//! (`takes/read.<card>.<ext>`), the `interview-essay` render job and
//! its own folders, metrics (`interview_essay*`) and alerts.
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
//! the final names, and a ticket nothing answered is a silence. The owner's
//! opening (paulgsc/some-ui#3269, C736) is checked against the week's
//! record instead: each answer the essay plays names its take, which must be
//! the newest kept one of that answer, whatever week it was said in. An essay that
//! contradicts the final is never given to the worker; it is refused, and the
//! refusal is kept in [`STATUS_FOLDER`] as `<week>.refused.log` (the fields
//! and why, nothing the essay says), for the `RustEssayRefused` alert.
//!
//! **What the worker is given** ([`export`]): `<week>/essay.json` in
//! [`ESSAYS_FOLDER`], byte for byte as the mailbox held it, and beside a
//! Rust Rounds essay its week as `week.json` ([`WEEK_FILE`], RR31): what
//! `GET /rust-weeks/<week>` answers then, runs included, by that route's own
//! serializer. All of it is written in a
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
//! and one `BEGIN IMMEDIATE` transaction; per Rust Rounds essay given, once,
//! its week again and one indexed read of each of its programs' runs (the
//! route's [`WEEK_FILE`]); one `stat` per essay each minute.

use crate::auth::owner::Owner;
use crate::call_weeks::render::{tally, Held, Renders};
use crate::AppState;
pub use rust_round_repo::draft::Kind;
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

/// A Rust Rounds essay's week, beside it (RR31, server#1049).
///
/// Exactly what `GET /rust-weeks/<week>` answers the essay's subject when the
/// essay is given (`handlers::rust_rounds::week_with_runs`), so the recorder
/// can show the phone's own screens and rustc's runs. Not written for an
/// interview essay, nor when the subject holds no such week.
pub const WEEK_FILE: &str = "week.json";

/// Which revision of the week's essay its folder holds (C72), beside it.
pub const REVISION_FILE: &str = "revision";

/// Where each interview essay home gives the render worker is written:
/// `${STORAGE}/interview-essays` on the host, `/essays` in the worker's
/// `interview-essay` job.
pub const INTERVIEW_ESSAYS_FOLDER: &str = "/app/dbs/interview-essays";

/// Where the worker keeps each interview essay's video.
pub const INTERVIEW_RENDERS_FOLDER: &str = "/app/dbs/renders/interview-essay";

/// Where the worker says how its interview essay passes went, and each
/// interview essay refused is kept.
pub const INTERVIEW_STATUS_FOLDER: &str = "/app/dbs/renders/interview-essay-worker";

/// Where the worker keeps a waiting interview essay's work.
pub const INTERVIEW_WORK_FOLDER: &str = "/app/dbs/renders/interview-essay-work";

/// What a kind of essay is called and where its files go: everything about
/// a kind that is a name, not a rule.
#[derive(Debug)]
pub struct Spec {
	/// The worker's name in its log lines: `rust essays`.
	pub name: &'static str,
	/// What the render worker is given ([`ESSAYS_FOLDER`]).
	pub essays: &'static str,
	/// What it made ([`RENDERS_FOLDER`]).
	pub renders: &'static str,
	/// How it went, and each essay refused ([`STATUS_FOLDER`]).
	pub status: &'static str,
	/// A waiting essay's work ([`WORK_FOLDER`]).
	pub work: &'static str,
}

/// What `kind` is called and where its files go.
#[must_use]
pub const fn spec(kind: Kind) -> &'static Spec {
	match kind {
		Kind::RustRounds => &Spec {
			name: "rust essays",
			essays: ESSAYS_FOLDER,
			renders: RENDERS_FOLDER,
			status: STATUS_FOLDER,
			work: WORK_FOLDER,
		},
		Kind::Interview => &Spec {
			name: "interview essays",
			essays: INTERVIEW_ESSAYS_FOLDER,
			renders: INTERVIEW_RENDERS_FOLDER,
			status: INTERVIEW_STATUS_FOLDER,
			work: INTERVIEW_WORK_FOLDER,
		},
	}
}

/// Every kind of essay home renders, each with its own worker.
pub const KINDS: [Kind; 2] = [Kind::RustRounds, Kind::Interview];

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

/// Start each kind's worker.
pub fn start(state: &AppState) {
	for kind in KINDS {
		start_kind(state, kind);
	}
}

/// Start `kind`'s worker.
///
/// Its folders and the render worker's are made first, before the server
/// answers its health check, by this server's uid, which the worker runs as
/// too, so the worker never finds one missing or not its own.
fn start_kind(state: &AppState, kind: Kind) {
	let spec = spec(kind);
	for folder in [spec.essays, spec.renders, spec.work, spec.status] {
		if std::fs::create_dir_all(folder).is_err() {
			error!(error = "render folder", "{} worker cannot make the render worker's folders", spec.name);
		}
	}
	let Some(configured) = state.mailbox.as_ref().map(|mailbox| mailbox.subject.clone()) else {
		info!("no mailbox, so no essay reaches home: the {} worker is off", spec.name);
		return;
	};
	let cancel = state.core.cancel_token.clone();
	let db = state.core.shared_db.clone();
	tokio::spawn(async move {
		info!(interval_secs = INTERVAL.as_secs(), "{} worker started", spec.name);
		crate::metrics::rust_essays::record_started(kind);
		let mut ticker = tokio::time::interval(TICK);
		ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
		let mut last: Option<tokio::time::Instant> = None;
		loop {
			tokio::select! {
				() = cancel.cancelled() => {
					info!("{} worker cancelled", spec.name);
					return;
				}
				_ = ticker.tick() => {
					match tidy_tick(&db, kind, configured.as_deref()).await {
						Ok(tidied) if tidied.failed > 0 => warn!(failed = tidied.failed, "{}: files that could not be removed are tried again next minute", spec.name),
						Ok(_) => {}
						Err(_) => error!(error = "storage", "{} tidy failed", spec.name),
					}
					observe_renders(kind).await;
					if last.is_some_and(|last| last.elapsed() < INTERVAL) {
						continue;
					}
					last = Some(tokio::time::Instant::now());
					let result = pass(&db, kind, configured.as_deref()).await;
					crate::metrics::rust_essays::record_pass(kind, &result);
					log_pass(kind, &result);
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
	/// [`WORK_FOLDER`]: a waiting essay's work and step cache, the owner's
	/// takes and a half-made video among them.
	pub work: PathBuf,
}

impl Folders {
	/// `kind`'s folders, as this server sees them.
	#[must_use]
	pub fn server(kind: Kind) -> Self {
		let spec = spec(kind);
		Self {
			essays: PathBuf::from(spec.essays),
			renders: PathBuf::from(spec.renders),
			status: PathBuf::from(spec.status),
			work: PathBuf::from(spec.work),
		}
	}
}

/// One pass for the mailbox's subject; nothing when there is none.
async fn pass(db: &SqlitePool, kind: Kind, configured: Option<&str>) -> Result<Exported, sqlx::Error> {
	match crate::mailbox::pass_subject(db, configured).await? {
		Owner::Subject(subject) => {
			let now = crate::handlers::rust_rounds::now();
			export(db, kind, subject.as_str(), &Folders::server(kind), &now).await
		}
		Owner::NoAccount | Owner::Several => Ok(Exported::default()),
	}
}

/// Each minute: [`tidy`] for the mailbox's subject, or for none once no
/// account stands (so an account deleted takes its essays' files with it).
async fn tidy_tick(db: &SqlitePool, kind: Kind, configured: Option<&str>) -> Result<Tidied, sqlx::Error> {
	let now = crate::handlers::rust_rounds::now();
	match crate::mailbox::pass_subject(db, configured).await? {
		Owner::Subject(subject) => tidy(db, kind, Some(subject.as_str()), &Folders::server(kind), &now).await,
		Owner::NoAccount => tidy(db, kind, None, &Folders::server(kind), &now).await,
		// Whose is not known: nothing is removed on a guess.
		Owner::Several => Ok(Tidied::default()),
	}
}

/// Gives the render worker each essay `subject` holds that it has not been
/// given, once its week is final and the essay is that final's.
///
/// Beside the essay go the takes it plays
/// (`<week>/takes/<ticket>.<webm|ogg|m4a>`, or `<ticket>.<n>.<ext>` for a
/// take of a ticket's point n (RR24), or `opening.<n>.<ext>` for the answer
/// to the opening's question n the essay plays (paulgsc/some-ui#3269), and,
/// when home heard its words,
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
pub async fn export(db: &SqlitePool, kind: Kind, subject: &str, folders: &Folders, now: &str) -> Result<Exported, sqlx::Error> {
	// Held for the whole pass, so an essay let go while it runs is never
	// written back (`feeds::FOLDERS`).
	let _folders = crate::feeds::FOLDERS.lock().await;
	let mut done = Exported::default();
	let (held, let_go) = {
		let mut conn = db.acquire().await?;
		(
			held_essays(&mut conn, kind, subject).await?,
			rust_round_repo::essay_video::let_go(&mut conn, kind, subject).await?,
		)
	};
	for (week, revision, held) in newest(kind, held) {
		let input = folders.essays.join(&week);
		if shared(db, kind, &week).await? {
			// The drive keys an essay by its week alone, so a week two
			// accounts hold is never given: neither could reach the other's.
			done.refused += 1;
			let why = [(
				String::from("week"),
				"is held by another account's essay too, so neither is given (the drive keys an essay by its week)",
			)];
			if keep_refusal(&folders.status, &week, &why).await.is_err() {
				done.failed += 1;
			}
			continue;
		}
		if let_go.contains(&week) || folders.renders.join(&week).join(VIDEO).exists() || given_revision(&input).await.is_some_and(|given| given >= revision) {
			continue;
		}
		match given(db, kind, subject, &week, &held, now).await? {
			Given::Waiting => done.waiting += 1,
			Given::Refused(problems) => {
				done.refused += 1;
				if keep_refusal(&folders.status, &week, &problems).await.is_err() {
					done.failed += 1;
				}
			}
			Given::Final(played) => match takes(db, kind, subject, &played).await? {
				Takes::Waiting => done.waiting += 1,
				Takes::Deleted(note) => {
					let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
					rust_round_repo::essay_video::withdraw(&mut tx, kind, subject, &week, &note, 0, now).await?;
					tx.commit().await?;
					done.withdrawn += 1;
				}
				Takes::Ready(files) => {
					let week_file = week_file(db, kind, subject, &week).await?;
					let partial = folders.essays.join(String::from(".") + &week + ".partial");
					if write_whole(&partial, &input, &held.body, week_file.as_deref(), revision, &files).await.is_ok() {
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
fn newest(kind: Kind, held: Vec<mailbox_repo::HeldEssay>) -> Vec<(String, u32, mailbox_repo::HeldEssay)> {
	let mut weeks: std::collections::BTreeMap<String, (u32, mailbox_repo::HeldEssay)> = std::collections::BTreeMap::new();
	for file in held {
		let Some((week, revision)) = essay::parse_name_of(kind, &file.name) else { continue };
		if weeks.get(&week).is_none_or(|(held, _)| revision > *held) {
			weeks.insert(week, (revision, file));
		}
	}
	weeks.into_iter().map(|(week, (revision, file))| (week, revision, file)).collect()
}

/// Whether more than one account holds an essay of `week`.
async fn shared(db: &SqlitePool, kind: Kind, week: &str) -> Result<bool, sqlx::Error> {
	let mut conn = db.acquire().await?;
	Ok(holders(&mut conn, kind, week).await? > 1)
}

/// Every essay file of `kind` `subject` holds, each revision included.
async fn held_essays(conn: &mut sqlx::SqliteConnection, kind: Kind, subject: &str) -> Result<Vec<mailbox_repo::HeldEssay>, sqlx::Error> {
	match kind {
		Kind::RustRounds => mailbox_repo::rust_essays(conn, subject).await,
		Kind::Interview => mailbox_repo::interview_essays(conn, subject).await,
	}
	.map_err(storage)
}

/// The weeks `subject` holds an essay of `kind` of.
pub(crate) async fn essay_weeks(conn: &mut sqlx::SqliteConnection, kind: Kind, subject: &str) -> Result<Vec<String>, sqlx::Error> {
	match kind {
		Kind::RustRounds => mailbox_repo::rust_essay_weeks(conn, subject).await,
		Kind::Interview => mailbox_repo::interview_essay_weeks(conn, subject).await,
	}
	.map_err(storage)
}

/// How many accounts hold an essay of `kind` of `week`.
pub(crate) async fn holders(conn: &mut sqlx::SqliteConnection, kind: Kind, week: &str) -> Result<i64, sqlx::Error> {
	match kind {
		Kind::RustRounds => mailbox_repo::rust_essay_holders(conn, week).await,
		Kind::Interview => mailbox_repo::interview_essay_holders(conn, week).await,
	}
	.map_err(storage)
}

/// The revision of the essay given in `input`, if one was.
async fn given_revision(input: &Path) -> Option<u32> {
	let text = tokio::fs::read_to_string(input.join(REVISION_FILE)).await.ok()?;
	text.trim().parse().ok()
}

async fn given(db: &SqlitePool, kind: Kind, subject: &str, week: &str, held: &mailbox_repo::HeldEssay, now: &str) -> Result<Given, sqlx::Error> {
	let essay = match essay::read_of(kind, &held.body) {
		Ok(essay) if essay.week == week => essay,
		Ok(_) => return Ok(Given::Refused(vec![(String::from("week.id"), "is not the week the essay's name gives")])),
		Err(problems) => return Ok(Given::Refused(problems)),
	};
	let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
	let Some(due) = due_of(&mut tx, kind, subject, &essay.week).await? else {
		return Ok(Given::Waiting);
	};
	if rust_round_repo::instant_order(now, &due) != Some(std::cmp::Ordering::Greater) {
		return Ok(Given::Waiting);
	}
	match kind {
		Kind::RustRounds => {
			rust_round_repo::draft::finalize(&mut tx, kind, subject, &essay.week, &due, now).await?;
		}
		// An interview week becoming final lets go of its middle takes'
		// recordings (IW8), whichever reader makes it final first.
		Kind::Interview => crate::handlers::interview_weeks::finalize_at(&mut tx, subject, &essay.week, &due, now).await?,
	}
	let standing = standing(&mut tx, kind, subject, &essay.week).await?;
	let opening = match kind {
		Kind::RustRounds => match held_rust_week(&mut tx, subject, &essay.week).await? {
			Some(week) => crate::handlers::rust_week_record::opening(&mut tx, subject, &week).await?,
			None => Vec::new(),
		},
		Kind::Interview => Vec::new(),
	};
	let unheard = match &standing {
		Some(standing) => unheard(&mut tx, kind, subject, standing).await?,
		None => Vec::new(),
	};
	tx.commit().await?;
	if standing.as_ref().is_some_and(|standing| standing.draft) {
		// A draft finalize could not make final (sent after the deadline):
		// left for a person to read, and never rendered from.
		return Ok(Given::Refused(vec![(String::from("week"), "has only a draft past its deadline, never a final")]));
	}
	let sent = standing.as_ref().map_or_else(essay::Final::default, |standing| essay::Final {
		answers: pairs(&standing.answers),
		reads: pairs(&standing.reads),
		opening: Vec::new(),
		unheard: unheard.iter().map(String::as_str).collect(),
	});
	let sent = essay::Final {
		opening: opening.iter().map(|take| (take.n, take.take.as_str())).collect(),
		..sent
	};
	Ok(match essay::against(&essay, &sent) {
		Ok(played) => Given::Final(played),
		Err(problems) => Given::Refused(problems),
	})
}

/// The takes of `standing` an essay may call a silence (paulgsc/server#908,
/// HEARD1): home judged the transcript the routine reads of each garbled,
/// failed or nothing said (`voice_repo::verdict::as_the_routine_reads`), so
/// the routine treats it as unanswered. A few indexed reads per take the
/// week names.
async fn unheard(conn: &mut sqlx::SqliteConnection, kind: Kind, subject: &str, standing: &Standing) -> Result<Vec<String>, sqlx::Error> {
	let mut unheard = Vec::new();
	for (_, take) in standing.answers.iter().chain(&standing.reads) {
		let judged = voice_repo::verdict::as_the_routine_reads(conn, subject, take, kind == Kind::Interview)
			.await
			.map_err(|err| match err {
				voice_repo::VoiceError::Storage(err) => err,
				other => sqlx::Error::Protocol(other.to_string()),
			})?;
		if judged.is_some_and(voice_repo::verdict::Kind::may_be_silent) {
			unheard.push(take.clone());
		}
	}
	Ok(unheard)
}

/// Each `(item, take)` of `list`, borrowed, as [`essay::Final`] holds them.
fn pairs(list: &[(String, String)]) -> Vec<(&str, &str)> {
	list.iter().map(|(item, take)| (item.as_str(), take.as_str())).collect()
}

/// What stands of a week's send, as an essay is checked against it.
struct Standing {
	/// Whether it is still a draft.
	draft: bool,
	/// Each item (a ticket, a question) answered, with its take's voice-note
	/// id.
	answers: Vec<(String, String)>,
	/// Each card read, with its take's voice-note id: a week of mock
	/// interview's teleprompter reads, none for Rust Rounds.
	reads: Vec<(String, String)>,
}

/// What stands of `subject`'s send of `week`, of `kind`
/// (`rust_round_repo::draft::standing`).
async fn standing(conn: &mut sqlx::SqliteConnection, kind: Kind, subject: &str, week: &str) -> Result<Option<Standing>, sqlx::Error> {
	Ok(match kind {
		Kind::RustRounds => rust_round_repo::sent::standing(conn, subject, week).await?.map(|stored| Standing {
			draft: stored.draft,
			answers: rust_round_repo::essay::rust_answers(Some(&stored.sent)),
			reads: Vec::new(),
		}),
		Kind::Interview => rust_round_repo::draft::standing::<rust_round_repo::interview::Sent>(conn, subject, week)
			.await?
			.map(|stored| Standing {
				draft: stored.draft,
				answers: stored.sent.answers.into_iter().map(|answer| (answer.question, answer.take)).collect(),
				reads: stored.sent.reads.into_iter().map(|read| (read.card, read.take)).collect(),
			}),
	})
}

/// The deadline of the week `subject` holds as `week`, when it reads as one
/// with a window.
async fn due_of(conn: &mut sqlx::SqliteConnection, kind: Kind, subject: &str, week: &str) -> Result<Option<String>, sqlx::Error> {
	let window = match kind {
		Kind::RustRounds => held_rust_week(conn, subject, week).await?.and_then(|week| week.window),
		Kind::Interview => {
			let file = mailbox_repo::rust_week(conn, subject, &rust_round_repo::interview::mailbox_name(week))
				.await
				.map_err(storage)?;
			file
				.and_then(|file| crate::handlers::interview_weeks::reads_as_named(week, &file.body))
				.and_then(|week| week.window)
		}
	};
	Ok(window.map(|window| window.due))
}

/// The Rust Rounds week `subject` holds as `week`, when it reads as one.
async fn held_rust_week(conn: &mut sqlx::SqliteConnection, subject: &str, week: &str) -> Result<Option<rust_round_repo::week::Week>, sqlx::Error> {
	let file = mailbox_repo::rust_week(conn, subject, &rust_round_repo::week::mailbox_name(week)).await.map_err(storage)?;
	Ok(file.and_then(|file| crate::handlers::rust_rounds::reads_as_named(week, &file.body)))
}

/// What [`WEEK_FILE`] holds for `kind`'s essay of `week`: the Rust Rounds
/// week with its runs, as the route answers it, or `None` for an interview
/// essay or a week `subject` does not hold. One read of the week and one
/// of each of its programs' runs, once per essay given.
async fn week_file(db: &SqlitePool, kind: Kind, subject: &str, week: &str) -> Result<Option<Vec<u8>>, sqlx::Error> {
	if kind != Kind::RustRounds {
		return Ok(None);
	}
	let mut conn = db.acquire().await?;
	crate::handlers::rust_rounds::week_with_runs(&mut conn, subject, week).await.map_err(|err| match err {
		crate::FileHostError::Sqlite(err) => err,
		other => sqlx::Error::Protocol(other.to_string()),
	})
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

async fn takes(db: &SqlitePool, kind: Kind, subject: &str, played: &[Played]) -> Result<Takes, sqlx::Error> {
	let of = match kind {
		Kind::RustRounds => voice_repo::Of::RustRound,
		Kind::Interview => voice_repo::Of::Interview,
	};
	let mut conn = db.acquire().await?;
	let mut files = Vec::with_capacity(played.len() * 2);
	for take in played {
		let head = voice_repo::audio_head(&mut conn, subject, &take.take, of).await.map_err(voice)?;
		let body = match head {
			Some(_) => voice_repo::audio_body(&mut conn, subject, &take.take, of).await.map_err(voice)?,
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
pub async fn tidy(db: &SqlitePool, kind: Kind, subject: Option<&str>, folders: &Folders, now: &str) -> Result<Tidied, sqlx::Error> {
	let _folders = crate::feeds::FOLDERS.lock().await;
	let mut done = Tidied::default();
	let (held, let_go, withdrawn) = match subject {
		Some(subject) => {
			let mut conn = db.acquire().await?;
			(
				essay_weeks(&mut conn, kind, subject).await?,
				rust_round_repo::essay_video::let_go(&mut conn, kind, subject).await?,
				rust_round_repo::essay_video::withdrawn(&mut conn, kind, subject)
					.await?
					.into_iter()
					.map(|w| w.week)
					.collect::<Vec<_>>(),
			)
		}
		None => (Vec::new(), Vec::new(), Vec::new()),
	};
	// A `.<week>.partial` is an export that stopped half way: never one in
	// progress, since export holds the same lock. It may hold takes, so it
	// goes, whoever's it was.
	done.failed += remove_partials(&folders.essays).await;
	let on_drive = {
		let mut weeks = weeks_in(&folders.essays).await;
		for week in weeks_in(&folders.renders).await.into_iter().chain(weeks_in(&folders.work).await) {
			if !weeks.contains(&week) {
				weeks.push(week);
			}
		}
		weeks
	};
	for week in on_drive {
		let (input, render, work) = (folders.essays.join(&week), folders.renders.join(&week), folders.work.join(&week));
		// Not the subject's, or withdrawn for a deleted take: everything of
		// it goes, its waiting work included, and is tried again each minute
		// until it has.
		if !held.contains(&week) || withdrawn.contains(&week) {
			done.weeks_removed += 1;
			for dir in [&input, &render, &work] {
				done.failed += usize::from(remove(dir).await.is_err());
			}
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
		if let Some(note) = deleted_take(db, kind, subject, &week, given_revision(&input).await).await? {
			let size = i64::try_from(crate::call_weeks::render::bytes_under(&render).await).unwrap_or(i64::MAX);
			let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
			rust_round_repo::essay_video::withdraw(&mut tx, kind, subject, &week, &note, size, now).await?;
			tx.commit().await?;
			done.withdrawn += 1;
			for dir in [&input, &render, &work] {
				done.failed += usize::from(remove(dir).await.is_err());
			}
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

/// A take `week`'s final record names that was deleted and that its essay
/// may play, if any.
///
/// An essay may answer an item whose take home could not hear as a silence,
/// and then plays nothing for it (HEARD1); it may also play it. So a
/// deleted take withdraws the week unless every revision of the week's
/// essay `subject` holds that could hold it answers its item as a silence:
/// the revision `given` to the render, if one was, and the newest, which the
/// next export would give. Deleting a take the essay never played, as the
/// owner's reconciliation of an unheard one may (paulgsc/server#912,
/// HEARD3), withdraws nothing. A revision that does not read counts as
/// playing it.
///
/// An answer of the owner's opening (paulgsc/some-ui#3269) names its take in
/// the essay, which may be another week's: a deleted one withdraws the week
/// when the revision given to the render plays it. One only a revision not
/// yet given plays withdraws nothing: [`essay::against`] refuses that
/// revision when it is given, so one playing the next older take can follow.
async fn deleted_take(db: &SqlitePool, kind: Kind, subject: &str, week: &str, given: Option<u32>) -> Result<Option<String>, sqlx::Error> {
	let mut conn = db.acquire().await?;
	if let Some(given) = given.filter(|_| kind == Kind::RustRounds) {
		let given_opening = held_essays(&mut conn, kind, subject)
			.await?
			.into_iter()
			.filter(|file| essay::parse_name_of(kind, &file.name) == Some((week.to_owned(), given)))
			.filter_map(|file| essay::read_of(kind, &file.body).ok())
			.flat_map(|essay| essay.opening);
		for (_, take) in given_opening {
			if voice_repo::delete::receipt(&mut conn, subject, &take).await.map_err(voice)?.is_some() {
				return Ok(Some(take));
			}
		}
	}
	let Some(standing) = standing(&mut conn, kind, subject, week).await? else {
		return Ok(None);
	};
	let mut deleted = Vec::new();
	for ((item, take), read) in standing.answers.iter().map(|pair| (pair, false)).chain(standing.reads.iter().map(|pair| (pair, true))) {
		if voice_repo::delete::receipt(&mut conn, subject, take).await.map_err(voice)?.is_some() {
			deleted.push((item.clone(), take.clone(), read));
		}
	}
	if deleted.is_empty() {
		return Ok(None);
	}
	let held: Vec<_> = held_essays(&mut conn, kind, subject)
		.await?
		.into_iter()
		.filter_map(|file| {
			let (of, revision) = essay::parse_name_of(kind, &file.name)?;
			(of == week).then(|| (revision, essay::read_of(kind, &file.body).ok()))
		})
		.collect();
	let newest = held.iter().map(|(revision, _)| *revision).max();
	let revisions: Vec<_> = held
		.into_iter()
		.filter(|(revision, _)| Some(*revision) == newest || Some(*revision) == given)
		.map(|(_, essay)| essay)
		.collect();
	for (item, take, read) in deleted {
		let silent = |essay: &Option<essay::Essay>| {
			essay.as_ref().is_some_and(|essay| {
				let items = if read { &essay.reads } else { &essay.tickets };
				items.iter().any(|(id, answered)| *id == item && *answered == essay::Answered::Silence)
			})
		};
		if revisions.is_empty() || !revisions.iter().all(silent) {
			return Ok(Some(take));
		}
	}
	Ok(None)
}

/// Removes every `.<name>.partial` directly under `folder`; how many could
/// not be removed.
async fn remove_partials(folder: &Path) -> usize {
	let mut failed = 0;
	if let Ok(mut entries) = tokio::fs::read_dir(folder).await {
		while let Ok(Some(entry)) = entries.next_entry().await {
			let name = entry.file_name();
			let name = name.to_string_lossy();
			if name.starts_with('.') && name.ends_with(".partial") {
				failed += usize::from(remove(&entry.path()).await.is_err());
			}
		}
	}
	failed
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

/// A mailbox read's failure as the `sqlx` error it is.
pub(crate) fn storage(err: mailbox_repo::MailboxError) -> sqlx::Error {
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

/// Writes the essay, its week when it has one ([`WEEK_FILE`]), its revision
/// and its takes into `partial`, then puts it in place of `into`: a lower
/// revision given before is replaced whole.
async fn write_whole(partial: &Path, into: &Path, essay: &[u8], week: Option<&[u8]>, revision: u32, takes: &[(String, Vec<u8>)]) -> std::io::Result<()> {
	// What a stopped pass left is made again.
	match tokio::fs::remove_dir_all(partial).await {
		Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error),
		_ => {}
	}
	tokio::fs::create_dir_all(partial).await?;
	tokio::fs::write(partial.join(ESSAY_FILE), essay).await?;
	if let Some(week) = week {
		tokio::fs::write(partial.join(WEEK_FILE), week).await?;
	}
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

/// What `kind`'s essays' job takes of the drive
/// (`call_weeks::render::disk`), which the call's observation adds to its
/// own, so the drive's numbers and `CallRenderCacheFull` count every job.
pub async fn disk(kind: Kind) -> crate::call_weeks::render::Disk {
	let spec = spec(kind);
	crate::call_weeks::render::disk(Path::new(spec.renders), Path::new(spec.work), Path::new(spec.status)).await
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
pub(crate) async fn held(db: &SqlitePool, kind: Kind, subject: &str, essays: &Path) -> Result<(Vec<Held>, Vec<String>), sqlx::Error> {
	let (weeks, let_go) = {
		let mut conn = db.acquire().await?;
		(
			essay_weeks(&mut conn, kind, subject).await?,
			rust_round_repo::essay_video::let_go(&mut conn, kind, subject).await?,
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

async fn observe_renders(kind: Kind) {
	let spec = spec(kind);
	let seen = observe(Path::new(spec.essays), Path::new(spec.renders), Path::new(spec.status)).await;
	crate::metrics::rust_essays::record_renders(kind, &seen);
}

/// The pass's one log line, when it gave an essay or something went wrong.
/// It counts; it never names a week's words.
fn log_pass(kind: Kind, result: &Result<Exported, sqlx::Error>) {
	let name = spec(kind).name;
	match result {
		Ok(done) if done.written == 0 && done.refused == 0 && done.failed == 0 => {}
		Ok(done) if done.refused == 0 && done.failed == 0 => info!(written = done.written, waiting = done.waiting, "{name} pass"),
		Ok(done) => warn!(written = done.written, waiting = done.waiting, refused = done.refused, failed = done.failed, "{name} pass"),
		Err(_) => error!(error = "storage", "{name} pass failed"),
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use rust_round_repo::sent::{Answer, OpeningTake, Sent};
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
			work: dir.join("work"),
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
		send_opening(pool, id, Vec::new()).await;
	}

	/// The week's send, with the takes of its opening said that week
	/// (paulgsc/some-ui#3269).
	async fn send_opening(pool: &SqlitePool, id: &str, opening: Vec<OpeningTake>) {
		let sent = Sent {
			week: id.to_owned(),
			at: String::from("2026-11-15T19:30:00.000Z"),
			answers: vec![Answer {
				ticket: String::from("QM-311"),
				point: None,
				take: String::from(TAKE),
				recorded_at: String::from("2026-11-15T19:00:00.000Z"),
			}],
			passed: vec![String::from("QM-312")],
			verdicts: Vec::new(),
			opening,
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

		let none = export(&pool, Kind::RustRounds, ME, &folders, AFTER).await.unwrap();
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
		let early = export(&pool, Kind::RustRounds, ME, &folders, BEFORE).await.unwrap();
		assert_eq!(
			early,
			Exported {
				waiting: 1,
				..Exported::default()
			},
			"before the deadline the send is a draft"
		);
		let no_take = export(&pool, Kind::RustRounds, ME, &folders, AFTER).await.unwrap();
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
		let done = export(&pool, Kind::RustRounds, ME, &folders, AFTER).await.unwrap();
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
		let route = crate::handlers::rust_rounds::week_with_runs(&mut pool.acquire().await.unwrap(), ME, "2026-w46")
			.await
			.unwrap()
			.unwrap();
		assert_eq!(
			std::fs::read(week.join(WEEK_FILE)).unwrap(),
			route,
			"its week beside it, as `GET /rust-weeks/2026-w46` answers it"
		);
		let given: serde_json::Value = serde_json::from_slice(&route).unwrap();
		assert_eq!((&given["id"], &given["tickets"][0]["ticket"]), (&json!("2026-w46"), &json!("QM-311")));
		let words: serde_json::Value = serde_json::from_slice(&std::fs::read(week.join("takes").join("QM-311.words.json")).unwrap()).unwrap();
		assert_eq!(
			words,
			json!([{ "text": "every", "startMs": 0, "endMs": 400 }, { "text": "field", "startMs": 450, "endMs": 900 }])
		);
		let mut names: Vec<_> = std::fs::read_dir(week.join("takes")).unwrap().map(|entry| entry.unwrap().file_name()).collect();
		names.sort();
		assert_eq!(names, ["QM-311.ogg", "QM-311.words.json"], "only what the recorder takes: no file for a silence");

		std::fs::write(week.join(ESSAY_FILE), b"touched").unwrap();
		assert_eq!(export(&pool, Kind::RustRounds, ME, &folders, AFTER).await.unwrap(), Exported::default(), "given once");
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
		let done = export(&pool, Kind::RustRounds, ME, &folders, now).await.unwrap();
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

	/// HEARD1: a take home judged garbled, failed or nothing said may be a
	/// silence in the essay, as the routines treat it as unanswered; a take it
	/// heard may not. Judged by the transcript the routine reads.
	#[tokio::test]
	async fn a_take_home_could_not_hear_may_be_a_silence() {
		let now = "2026-11-24T08:00:00.000Z";
		for (verdict, written, refused) in [("heard", 0, 1), ("garbled", 1, 0), ("failed", 1, 0), ("nothing_said", 1, 0)] {
			let (pool, dir) = (pool().await, tempfile::tempdir().unwrap());
			let folders = folders(dir.path());
			hold_week(&pool, 1, "2026-w46").await;
			send(&pool, "2026-w46").await;
			for statement in [
				"INSERT INTO voice_note (subject_id, note_id, of_kind, of_ref, recorded_at, content_type, size, sha256, stored_at)
				 VALUES (?1, 'note-311', 'rust-round', '2026-w46:QM-311', 'then', 'audio/ogg', 4, 's', 'now')",
				"INSERT INTO voice_note_audio (subject_id, note_id, body) VALUES (?1, 'note-311', x'4f676753')",
			] {
				sqlx::query(statement).bind(ME).execute(&pool).await.unwrap();
			}
			let (outcome, language, text, speech) = match verdict {
				"heard" | "garbled" => ("heard", Some("en"), Some("every field"), Some(900)),
				"nothing_said" => ("silent", None, None, None),
				_ => ("deadline", None, None, None),
			};
			sqlx::query(
				"INSERT INTO voice_transcript (subject_id, note_id, model, engine, attempt, outcome, language, language_p, duration_ms, text, took_ms, heard_at)
				 VALUES (?1, 'note-311', 'm', 'e', 1, ?2, ?3, ?4, ?5, ?6, 1, 'now')",
			)
			.bind(ME)
			.bind(outcome)
			.bind(language)
			.bind(language.map(|_| 1.0))
			.bind(if outcome == "deadline" { None } else { Some(900) })
			.bind(text)
			.execute(&pool)
			.await
			.unwrap();
			sqlx::query(
				"INSERT INTO voice_transcript_verdict (subject_id, note_id, model, attempt, rule, verdict, reasons, speech_ms, judged_at)
				 VALUES (?1, 'note-311', 'm', 1, 'heard-1', ?2, '', ?3, 'now')",
			)
			.bind(ME)
			.bind(verdict)
			.bind(speech)
			.execute(&pool)
			.await
			.unwrap();
			// Filed for the routine and taken by the mailbox: the routine judges
			// by the file it reads.
			for statement in [
				"INSERT INTO mailbox_transcript (subject_id, note_id, model, n, filed_at) VALUES (?1, 'note-311', 'm', 1, 'now')",
				"INSERT INTO mailbox_handed (subject_id, folder, name, answer, handed_at) VALUES (?1, 'transcript', 'transcript-note-311-1.json', 'added', 'now')",
			] {
				sqlx::query(statement).bind(ME).execute(&pool).await.unwrap();
			}
			hold(&pool, 2, &essay::mailbox_name("2026-w46"), &essay("2026-w46", ["silence", "silence"])).await;
			let done = export(&pool, Kind::RustRounds, ME, &folders, now).await.unwrap();
			assert_eq!((done.written, done.refused, done.failed), (written, refused, 0), "{verdict}");
			if written == 1 {
				let takes: Vec<_> = std::fs::read_dir(folders.essays.join("2026-w46").join("takes")).unwrap().collect();
				assert!(takes.is_empty(), "{verdict}: a silence plays nothing");
			}
		}
	}

	/// Invariant 23's exception, (b): the takes are in the render's input
	/// only while the week waits for its video.
	#[tokio::test]
	async fn a_kept_essay_copies_its_takes_only_until_its_video_is_made() {
		let (pool, dir) = (pool().await, tempfile::tempdir().unwrap());
		let folders = folders(dir.path());
		final_week(&pool).await;
		hold_take(&pool).await;
		assert_eq!(export(&pool, Kind::RustRounds, ME, &folders, AFTER).await.unwrap().written, 1);
		let takes = folders.essays.join("2026-w46").join("takes");
		assert_eq!(
			tidy(&pool, Kind::RustRounds, Some(ME), &folders, AFTER).await.unwrap(),
			Tidied::default(),
			"waiting: the takes stay"
		);
		assert!(takes.join("QM-311.ogg").exists());

		std::fs::create_dir_all(folders.renders.join("2026-w46")).unwrap();
		std::fs::write(folders.renders.join("2026-w46").join(VIDEO), b"video").unwrap();
		let tidied = tidy(&pool, Kind::RustRounds, Some(ME), &folders, AFTER).await.unwrap();
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
		assert_eq!(
			export(&pool, Kind::RustRounds, ME, &folders, AFTER).await.unwrap(),
			Exported::default(),
			"and it is never given again"
		);
	}

	/// EV5: a week the owner let go from My data, video then rest, is never
	/// given to the worker again, even once its renders folder is gone and
	/// nothing on the drive says it was made; `tidy` leaves no input for it.
	#[tokio::test]
	async fn a_week_let_go_is_never_given_again() {
		use crate::feeds::videos::{let_go, record_link, Folders as FeedFolders, LetGo, LinkRecorded};
		use crate::feeds::Feed;
		use call_week_repo::video::Step;

		let (pool, dir) = (pool().await, tempfile::tempdir().unwrap());
		let folders = folders(dir.path());
		final_week(&pool).await;
		hold_take(&pool).await;
		assert_eq!(export(&pool, Kind::RustRounds, ME, &folders, AFTER).await.unwrap().written, 1, "given once");
		let rendered = folders.renders.join("2026-w46");
		std::fs::create_dir_all(&rendered).unwrap();
		std::fs::write(rendered.join(VIDEO), b"video").unwrap();

		assert_eq!(
			record_link(&pool, Feed::RustEssay, ME, &folders.renders, "2026-w46", "https://youtu.be/dQw4w9WgXcQ", AFTER)
				.await
				.unwrap(),
			LinkRecorded::Kept(String::from("dQw4w9WgXcQ"))
		);
		let feed = FeedFolders {
			renders: &folders.renders,
			input: &folders.essays,
		};
		for step in [Step::Video, Step::Rest] {
			assert!(matches!(let_go(&pool, Feed::RustEssay, ME, feed, "2026-w46", step, AFTER).await.unwrap(), LetGo::Gone(_)));
		}
		// Whatever the let-go left, or a drive emptied by hand: no render says
		// the video was made, so only the receipt keeps the week from coming back.
		let _ = std::fs::remove_dir_all(&folders.renders);
		assert!(!folders.essays.join("2026-w46").exists() && !rendered.exists());

		let again = export(&pool, Kind::RustRounds, ME, &folders, AFTER).await.unwrap();
		assert_eq!(again.written, 0, "a week let go is never given again");
		assert_eq!(again, Exported::default());
		assert!(!folders.essays.join("2026-w46").exists(), "its input is not recreated");
		tidy(&pool, Kind::RustRounds, Some(ME), &folders, AFTER).await.unwrap();
		assert!(!folders.essays.join("2026-w46").exists(), "and tidy leaves none");
	}

	/// paulgsc/some-ui#3269, C736: an essay plays each opening answer's
	/// newest kept take, whatever week said it, handed over as
	/// `opening.<n>.<ext>`; one naming a take a newer one replaced is refused;
	/// and deleting a take the given essay plays withdraws the week.
	#[tokio::test]
	async fn an_essay_plays_the_newest_kept_take_of_each_opening_answer() {
		let opening = |n: u8, take: &str, recorded: &str| OpeningTake {
			n,
			take: take.to_owned(),
			recorded_at: recorded.to_owned(),
			version: 1,
			ms: Some(20_000),
		};
		let (pool, dir) = (pool().await, tempfile::tempdir().unwrap());
		let folders = folders(dir.path());
		let week = json!({
			"schema": "some-ui.rust-week/1", "id": "2026-w46", "opens": "2026-11-09T00:00:00.000-08:00", "due": DUE,
			"opening": { "version": 1, "exchanges": [{ "ask": "Why?", "script": "paulgsc here." }, { "ask": "And?", "script": "Then." }] },
			"tickets": [{ "id": "QM-311", "program": "fn main() {}\n" }, { "id": "QM-312", "program": "fn main() {}\n" }],
		});
		hold(&pool, 1, &rust_round_repo::week::mailbox_name("2026-w46"), &week.to_string()).await;
		// Question 1 said again this week; question 2 last said in w45, reused.
		send_opening(&pool, "2026-w46", vec![opening(1, "note-o1", "2026-11-14T10:00:00.000Z")]).await;
		let w45 = json!({ "week": "2026-w45", "at": "2026-11-08T10:00:00.000Z", "answers": [], "passed": [], "opening": [
			opening_json(1, "note-o1-old", "2026-11-07T10:00:00.000Z"),
			opening_json(2, "note-o2", "2026-11-07T10:05:00.000Z"),
		] });
		sqlx::query("INSERT INTO rust_week_final (subject_id, week_id, sent_at, body, received_at) VALUES (?1, '2026-w45', 'then', ?2, 'then')")
			.bind(ME)
			.bind(w45.to_string())
			.execute(&pool)
			.await
			.unwrap();
		hold_take(&pool).await;
		for (note, of_ref) in [("note-o1", "2026-w46:opening:1"), ("note-o1-old", "2026-w45:opening:1"), ("note-o2", "2026-w45:opening:2")] {
			sqlx::query(
				"INSERT INTO voice_note (subject_id, note_id, of_kind, of_ref, recorded_at, content_type, size, sha256, stored_at)
				 VALUES (?1, ?2, 'rust-round', ?3, 'then', 'audio/ogg', 4, 's', 'now')",
			)
			.bind(ME)
			.bind(note)
			.bind(of_ref)
			.execute(&pool)
			.await
			.unwrap();
			sqlx::query("INSERT INTO voice_note_audio (subject_id, note_id, body) VALUES (?1, ?2, x'4f676753')")
				.bind(ME)
				.bind(note)
				.execute(&pool)
				.await
				.unwrap();
		}
		let with_opening = |takes: [&str; 2]| {
			let mut body: serde_json::Value = serde_json::from_str(&essay("2026-w46", ["take", "silence"])).unwrap();
			body["opening"] = json!({ "exchanges": [
				{ "n": 1, "ask": "Why?", "transcript": "paulgsc here", "take": takes[0] },
				{ "n": 2, "ask": "And?", "transcript": "then", "take": takes[1] },
			] });
			body.to_string()
		};

		// The older take of question 1 is not the one home holds as its newest.
		hold(&pool, 2, &essay::mailbox_name("2026-w46"), &with_opening(["note-o1-old", "note-o2"])).await;
		assert_eq!(export(&pool, Kind::RustRounds, ME, &folders, AFTER).await.unwrap().refused, 1);
		assert_eq!(
			std::fs::read_to_string(folders.status.join("2026-w46.refused.log")).unwrap(),
			"opening.exchanges[0].take: is not the take home holds as the newest of this answer\n"
		);

		hold(&pool, 3, "rust-essay-2026-w46-r2.json", &with_opening(["note-o1", "note-o2"])).await;
		assert_eq!(export(&pool, Kind::RustRounds, ME, &folders, AFTER).await.unwrap().written, 1);
		let mut names: Vec<_> = std::fs::read_dir(folders.essays.join("2026-w46").join("takes"))
			.unwrap()
			.map(|entry| entry.unwrap().file_name().into_string().unwrap())
			.collect();
		names.sort();
		assert_eq!(names, ["QM-311.ogg", "QM-311.words.json", "opening.1.ogg", "opening.2.ogg"]);

		// Deleting w45's take of question 2, which the given essay plays.
		for statement in [
			"DELETE FROM voice_note_audio WHERE subject_id = ?1 AND note_id = 'note-o2'",
			"DELETE FROM voice_note WHERE subject_id = ?1 AND note_id = 'note-o2'",
			"INSERT INTO voice_note_deletion (subject_id, note_id, of_kind, recorded_at, size, stored_at, deleted_at, went, stayed)
			 VALUES (?1, 'note-o2', 'rust-round', 'then', 4, 'then', 'now', '[]', '[]')",
		] {
			sqlx::query(statement).bind(ME).execute(&pool).await.unwrap();
		}
		assert_eq!(tidy(&pool, Kind::RustRounds, Some(ME), &folders, AFTER).await.unwrap().withdrawn, 1);
	}

	fn opening_json(n: u8, take: &str, recorded: &str) -> serde_json::Value {
		json!({ "n": n, "take": take, "recordedAt": recorded, "version": 1 })
	}

	/// HEARD3 (paulgsc/server#912): a take home could not hear that the
	/// essay answers as a silence is never played, so the owner deleting it
	/// (paulgsc/some-ui#3076) withdraws nothing: a made video stays, and a
	/// week not yet given still is, the silence standing. One the essay plays
	/// anyway withdraws its week, as any played take does; one only an older
	/// revision played, superseded before it was given, does not.
	#[tokio::test]
	async fn deleting_a_take_withdraws_its_week_only_if_the_essay_plays_it() {
		for (given_first, plays, older_plays) in [(true, false, false), (false, false, false), (true, true, false), (true, false, true)] {
			let (pool, dir) = (pool().await, tempfile::tempdir().unwrap());
			let folders = folders(dir.path());
			hold_week(&pool, 1, "2026-w46").await;
			send(&pool, "2026-w46").await;
			for statement in [
				"INSERT INTO voice_note (subject_id, note_id, of_kind, of_ref, recorded_at, content_type, size, sha256, stored_at)
				 VALUES (?1, 'note-311', 'rust-round', '2026-w46:QM-311', 'then', 'audio/ogg', 4, 's', 'now')",
				"INSERT INTO voice_note_audio (subject_id, note_id, body) VALUES (?1, 'note-311', x'4f676753')",
				"INSERT INTO voice_transcript (subject_id, note_id, model, engine, attempt, outcome, language, language_p, duration_ms, text, took_ms, heard_at)
				 VALUES (?1, 'note-311', 'm', 'e', 1, 'heard', 'en', 1.0, 900, 'mumble', 1, 'now')",
				"INSERT INTO voice_transcript_verdict (subject_id, note_id, model, attempt, rule, verdict, reasons, speech_ms, judged_at)
				 VALUES (?1, 'note-311', 'm', 1, 'heard-1', 'garbled', 'low_confidence', 900, 'now')",
				"INSERT INTO mailbox_transcript (subject_id, note_id, model, n, filed_at) VALUES (?1, 'note-311', 'm', 1, 'now')",
				"INSERT INTO mailbox_handed (subject_id, folder, name, answer, handed_at) VALUES (?1, 'transcript', 'transcript-note-311-1.json', 'added', 'now')",
			] {
				sqlx::query(statement).bind(ME).execute(&pool).await.unwrap();
			}
			let answers = if plays { ["take", "silence"] } else { ["silence", "silence"] };
			if older_plays {
				hold(&pool, 2, &essay::mailbox_name("2026-w46"), &essay("2026-w46", ["take", "silence"])).await;
				let second = essay::mailbox_name("2026-w46").replace(".json", "-r2.json");
				hold(&pool, 3, &second, &essay("2026-w46", answers)).await;
			} else {
				hold(&pool, 2, &essay::mailbox_name("2026-w46"), &essay("2026-w46", answers)).await;
			}
			let now = "2026-11-24T08:00:00.000Z";
			if given_first {
				assert_eq!(export(&pool, Kind::RustRounds, ME, &folders, now).await.unwrap().written, 1);
				std::fs::create_dir_all(folders.renders.join("2026-w46")).unwrap();
				std::fs::write(folders.renders.join("2026-w46").join(VIDEO), b"video").unwrap();
			}
			let mut conn = pool.acquire().await.unwrap();
			let deleted = voice_repo::delete::delete(&mut conn, ME, "note-311", voice_repo::delete::SWITCHED_ON, now).await.unwrap();
			assert!(matches!(deleted, voice_repo::delete::Deleted::Gone(_)), "an unheard take may go");
			drop(conn);
			let tidied = tidy(&pool, Kind::RustRounds, Some(ME), &folders, now).await.unwrap();
			assert_eq!(tidied.withdrawn, usize::from(plays), "given first: {given_first}, plays: {plays}");
			let mut conn = pool.acquire().await.unwrap();
			let withdrawn = rust_round_repo::essay_video::withdrawn(&mut conn, Kind::RustRounds, ME).await.unwrap();
			assert_eq!(withdrawn.len(), usize::from(plays));
			drop(conn);
			if plays {
				assert!(!folders.renders.join("2026-w46").exists(), "a played take's week is withdrawn");
			} else if given_first {
				assert!(folders.renders.join("2026-w46").join(VIDEO).exists(), "its video stays");
			} else {
				let done = export(&pool, Kind::RustRounds, ME, &folders, now).await.unwrap();
				assert_eq!((done.written, done.withdrawn, done.refused), (1, 0, 0), "the silence stands");
			}
		}
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
		export(&pool, Kind::RustRounds, ME, &folders, AFTER).await.unwrap();
		std::fs::create_dir_all(folders.renders.join("2026-w46")).unwrap();
		std::fs::write(folders.renders.join("2026-w46").join(VIDEO), b"video").unwrap();

		delete_take(&pool).await;
		let tidied = tidy(&pool, Kind::RustRounds, Some(ME), &folders, AFTER).await.unwrap();
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
		let withdrawn = rust_round_repo::essay_video::withdrawn(&mut conn, Kind::RustRounds, ME).await.unwrap();
		assert_eq!((withdrawn[0].week.as_str(), withdrawn[0].note.as_str(), withdrawn[0].size), ("2026-w46", TAKE, 5));
		drop(conn);
		// A removal that failed, or a process that stopped between the
		// receipt and the files: the next minute removes them.
		std::fs::create_dir_all(folders.renders.join("2026-w46")).unwrap();
		std::fs::write(folders.renders.join("2026-w46").join(VIDEO), b"video").unwrap();
		std::fs::create_dir_all(folders.work.join("2026-w46")).unwrap();
		assert_eq!(tidy(&pool, Kind::RustRounds, Some(ME), &folders, AFTER).await.unwrap().weeks_removed, 1);
		assert!(
			!folders.renders.join("2026-w46").exists() && !folders.work.join("2026-w46").exists(),
			"a withdrawn week's files are removed until gone"
		);
		assert_eq!(
			export(&pool, Kind::RustRounds, ME, &folders, AFTER).await.unwrap(),
			Exported::default(),
			"never given again"
		);
		assert!(!folders.essays.join("2026-w46").exists());

		// Deleted before it was ever given: withdrawn at the pass, never given.
		let (pool, dir) = (super::tests::pool().await, tempfile::tempdir().unwrap());
		let folders = super::tests::folders(dir.path());
		final_week(&pool).await;
		hold_take(&pool).await;
		delete_take(&pool).await;
		assert_eq!(
			export(&pool, Kind::RustRounds, ME, &folders, AFTER).await.unwrap(),
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
		export(&pool, Kind::RustRounds, ME, &folders, AFTER).await.unwrap();
		std::fs::create_dir_all(folders.renders.join("2026-w46")).unwrap();
		std::fs::write(folders.renders.join("2026-w46").join(VIDEO), b"video").unwrap();
		std::fs::create_dir_all(folders.renders.join("2026-w40")).unwrap();
		std::fs::create_dir_all(folders.essays.join(".2026-w47.partial").join("takes")).unwrap();
		std::fs::write(folders.essays.join(".2026-w47.partial").join("takes").join("QM-311.ogg"), b"OggS").unwrap();
		std::fs::create_dir_all(folders.work.join("2026-w46").join("cache")).unwrap();
		std::fs::write(folders.work.join("2026-w46").join("cache").join("take.wav"), b"the owner's voice").unwrap();

		let tidied = tidy(&pool, Kind::RustRounds, Some(ME), &folders, AFTER).await.unwrap();
		assert_eq!((tidied.weeks_removed, tidied.takes_removed), (1, 1), "a render of a week with no essay goes");
		assert!(!folders.renders.join("2026-w40").exists());

		sqlx::query("DELETE FROM account WHERE subject_id = ?1").bind(ME).execute(&pool).await.unwrap();
		let tidied = tidy(&pool, Kind::RustRounds, None, &folders, AFTER).await.unwrap();
		assert_eq!(tidied.weeks_removed, 1);
		assert!(
			!folders.essays.join("2026-w46").exists() && !folders.renders.join("2026-w46").exists(),
			"the account went: so did its essays"
		);
		assert!(!folders.work.join("2026-w46").exists(), "and their unfinished work, the owner's takes in it");
		assert!(!folders.essays.join(".2026-w47.partial").exists(), "and an export that stopped half way");
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
		assert_eq!(export(&pool, Kind::RustRounds, ME, &folders, AFTER).await.unwrap().written, 1);
		let week = folders.essays.join("2026-w46");
		assert_eq!(std::fs::read_to_string(week.join(REVISION_FILE)).unwrap(), "1\n", "a name not a revision's is never read");

		let r2 = essay("2026-w46", ["take", "silence"]).replace("2026-11-09", "2026-11-10");
		hold(&pool, 4, "rust-essay-2026-w46-r2.json", &r2).await;
		assert_eq!(
			export(&pool, Kind::RustRounds, ME, &folders, AFTER).await.unwrap().written,
			1,
			"no video yet: the correction is given"
		);
		assert_eq!(std::fs::read_to_string(week.join(REVISION_FILE)).unwrap(), "2\n");
		assert_eq!(std::fs::read(week.join(ESSAY_FILE)).unwrap(), r2.as_bytes());
		assert!(week.join("takes").join("QM-311.ogg").exists());

		std::fs::create_dir_all(folders.renders.join("2026-w46")).unwrap();
		std::fs::write(folders.renders.join("2026-w46").join(VIDEO), b"video").unwrap();
		hold(&pool, 5, "rust-essay-2026-w46-r3.json", &essay("2026-w46", ["take", "silence"])).await;
		assert_eq!(
			export(&pool, Kind::RustRounds, ME, &folders, AFTER).await.unwrap(),
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

	/// The drive keys an essay by its week alone, so a week two accounts hold
	/// an essay of is given to neither, and neither's feed lists it.
	#[tokio::test]
	async fn a_week_two_accounts_hold_is_given_to_neither() {
		let (pool, dir) = (pool().await, tempfile::tempdir().unwrap());
		let folders = folders(dir.path());
		final_week(&pool).await;
		hold_take(&pool).await;
		sqlx::query("INSERT INTO account (subject_id, user_handle) VALUES ('them', 'them')")
			.execute(&pool)
			.await
			.unwrap();
		sqlx::query("INSERT INTO mailbox_item (subject_id, folder, name, seq, sha256, size, body, pulled_at) VALUES ('them', 'brief', 'rust-essay-2026-w46.json', 1, 's', 2, x'7b7d', 'now')")
			.execute(&pool)
			.await
			.unwrap();
		let done = export(&pool, Kind::RustRounds, ME, &folders, AFTER).await.unwrap();
		assert_eq!((done.written, done.refused), (0, 1));
		assert!(!folders.essays.join("2026-w46").exists());
		assert!(std::fs::read_to_string(folders.status.join("2026-w46.refused.log")).unwrap().contains("another account"));
		let mut conn = pool.acquire().await.unwrap();
		for subject in [ME, "them"] {
			assert!(
				!crate::feeds::Feed::RustEssay.holds(&mut conn, subject, "2026-w46").await.unwrap(),
				"{subject} can't reach it"
			);
		}
	}

	/// A week of mock interview held, sent (one question answered, one card
	/// read) and past its deadline, with its essay and its two takes.
	async fn interview_week(pool: &SqlitePool, kinds: [&str; 2]) -> String {
		let week = json!({
			"schema": "some-ui.interview-week/1", "id": "2026-w46", "opens": "2026-11-09T00:00:00.000-08:00", "due": DUE,
			"questions": [{ "id": "about-you" }, { "id": "a-conflict" }],
			"cards": [{ "id": "intro" }],
		});
		hold(pool, 1, &rust_round_repo::interview::mailbox_name("2026-w46"), &week.to_string()).await;
		let body = json!({
			"schema": "some-ui.interview-essay/1",
			"week": { "id": "2026-w46" },
			"questions": [
				{ "id": "about-you", "answer": { "kind": kinds[0] } },
				{ "id": "a-conflict", "answer": { "kind": kinds[1] } },
			],
			"reads": [{ "id": "intro", "answer": { "kind": "take" } }],
		})
		.to_string();
		hold(pool, 2, "interview-essay-2026-w46.json", &body).await;
		let sent = rust_round_repo::interview::Sent {
			week: String::from("2026-w46"),
			at: String::from("2026-11-15T19:30:00.000Z"),
			answers: vec![rust_round_repo::interview::Answer {
				question: String::from("about-you"),
				take: String::from("note-iw"),
				recorded_at: String::from("2026-11-15T19:00:00.000Z"),
				rung: None,
			}],
			reads: vec![rust_round_repo::interview::Read {
				card: String::from("intro"),
				take: String::from("note-read"),
				recorded_at: String::from("2026-11-15T19:00:00.000Z"),
			}],
			passed: vec![String::from("a-conflict")],
			passed_reads: Vec::new(),
		};
		rust_round_repo::draft::put_draft(&mut pool.acquire().await.unwrap(), ME, &sent, BEFORE).await.unwrap();
		for (note, of_ref) in [("note-iw", "2026-w46:about-you"), ("note-read", "2026-w46:read:intro")] {
			sqlx::query(
				"INSERT INTO voice_note (subject_id, note_id, of_kind, of_ref, recorded_at, content_type, size, sha256, stored_at)
				 VALUES (?1, ?2, 'interview', ?3, 'then', 'audio/webm', 4, 's', 'now')",
			)
			.bind(ME)
			.bind(note)
			.bind(of_ref)
			.execute(pool)
			.await
			.unwrap();
			sqlx::query("INSERT INTO voice_note_audio (subject_id, note_id, body) VALUES (?1, ?2, x'1a45dfa3')")
				.bind(ME)
				.bind(note)
				.execute(pool)
				.await
				.unwrap();
		}
		body
	}

	/// The interview kind is the same export: it waits for the week's
	/// deadline, is checked against the interview week's final, and is given
	/// with the take of each question and each card read it plays; the
	/// Rust Rounds kind never reads it.
	#[tokio::test]
	async fn an_interview_essay_is_given_from_its_weeks_final_with_its_takes() {
		let (pool, dir) = (pool().await, tempfile::tempdir().unwrap());
		let folders = folders(dir.path());
		let body = interview_week(&pool, ["take", "silence"]).await;
		assert_eq!(export(&pool, Kind::Interview, ME, &folders, BEFORE).await.unwrap().waiting, 1, "not before its deadline");
		assert_eq!(
			export(&pool, Kind::RustRounds, ME, &folders, AFTER).await.unwrap(),
			Exported::default(),
			"not a Rust Rounds essay"
		);
		let done = export(&pool, Kind::Interview, ME, &folders, AFTER).await.unwrap();
		assert_eq!(
			done,
			Exported {
				written: 1,
				..Exported::default()
			}
		);
		let given = folders.essays.join("2026-w46");
		assert_eq!(std::fs::read_to_string(given.join(ESSAY_FILE)).unwrap(), body);
		assert!(!given.join(WEEK_FILE).exists(), "an interview essay is given no Rust Rounds week");
		let mut takes: Vec<_> = std::fs::read_dir(given.join("takes"))
			.unwrap()
			.map(|e| e.unwrap().file_name().into_string().unwrap())
			.collect();
		takes.sort();
		assert_eq!(takes, ["about-you.webm", "read.intro.webm"], "the question's take and the card's read, named apart");
		assert!(rust_round_repo::draft::is_final(&mut pool.acquire().await.unwrap(), Kind::Interview, ME, "2026-w46")
			.await
			.unwrap());
		assert_eq!(export(&pool, Kind::Interview, ME, &folders, AFTER).await.unwrap(), Exported::default(), "given once");
		let (held, _) = held(&pool, Kind::Interview, ME, &folders.essays).await.unwrap();
		assert_eq!(held.iter().map(|held| held.key.as_str()).collect::<Vec<_>>(), ["2026-w46"]);
		assert!(held_essays(&mut pool.acquire().await.unwrap(), Kind::RustRounds, ME).await.unwrap().is_empty());
	}

	/// IW8: when the essay pass is what makes an interview week final, the
	/// week's middle takes' recordings are let go with it, as any reader
	/// making it final does: the question keeps its earliest and its newest,
	/// which here is the take the final names.
	#[tokio::test]
	async fn the_essay_pass_making_a_week_final_lets_go_of_its_middle_takes() {
		let (pool, dir) = (pool().await, tempfile::tempdir().unwrap());
		let folders = folders(dir.path());
		interview_week(&pool, ["take", "silence"]).await;
		for (note, at) in [
			("early", "2026-11-10T09:00:00.000Z"),
			("mid", "2026-11-11T09:00:00.000Z"),
			("late", "2026-11-12T09:00:00.000Z"),
		] {
			for statement in [
				"INSERT INTO voice_note (subject_id, note_id, of_kind, of_ref, recorded_at, content_type, size, sha256, stored_at)
				 VALUES (?1, ?2, 'interview', '2026-w46:about-you', ?3, 'audio/webm', 4, 's', 'now')",
				"INSERT INTO voice_note_audio (subject_id, note_id, body) SELECT ?1, ?2, x'1a45dfa3' WHERE ?3 IS NOT NULL",
				"INSERT INTO voice_transcript (subject_id, note_id, model, engine, attempt, outcome, language, language_p, duration_ms, text, took_ms, heard_at)
				 SELECT ?1, ?2, 'm', 'e', 1, 'heard', 'en', 0.9, 1, 'w', 1, ?3",
				"INSERT INTO voice_transcript_verdict (subject_id, note_id, model, attempt, rule, verdict, reasons, speech_ms, judged_at)
				 SELECT ?1, ?2, 'm', 1, 'r', 'heard', '', 1, ?3",
			] {
				sqlx::query(statement).bind(ME).bind(note).bind(at).execute(&pool).await.unwrap();
			}
		}
		assert_eq!(export(&pool, Kind::Interview, ME, &folders, AFTER).await.unwrap().written, 1);
		let mut held: Vec<String> = sqlx::query_scalar("SELECT note_id FROM voice_note_audio WHERE subject_id = ?1")
			.bind(ME)
			.fetch_all(&pool)
			.await
			.unwrap();
		held.sort();
		// `note-iw`, the take the final names, was recorded at `then`, which
		// sorts after every instant: it is the newest too.
		assert_eq!(held, ["early", "note-iw", "note-read"], "mid and late let go");
		let receipts: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM voice_note_deletion WHERE note_id IN ('mid', 'late')")
			.fetch_one(&pool)
			.await
			.unwrap();
		assert_eq!(receipts, 2);
	}

	#[tokio::test]
	async fn an_interview_essay_that_contradicts_its_weeks_final_is_refused() {
		let (pool, dir) = (pool().await, tempfile::tempdir().unwrap());
		let folders = folders(dir.path());
		interview_week(&pool, ["take", "take"]).await;
		let done = export(&pool, Kind::Interview, ME, &folders, AFTER).await.unwrap();
		assert_eq!(done.refused, 1);
		let refusal = std::fs::read_to_string(folders.status.join("2026-w46.refused.log")).unwrap();
		assert_eq!(refusal, "questions[1].answer.kind: is a take, and the week's final record sent none for this question\n");
		assert!(!folders.essays.join("2026-w46").exists());
	}

	/// A question's take or a card's read deleted from My data withdraws the
	/// week's essay, as a Rust Rounds take does.
	#[tokio::test]
	async fn an_interview_essay_whose_take_is_deleted_is_withdrawn() {
		for note in ["note-iw", "note-read"] {
			withdrawn_for(note).await;
		}
	}

	async fn withdrawn_for(note: &str) {
		let (pool, dir) = (pool().await, tempfile::tempdir().unwrap());
		let folders = folders(dir.path());
		interview_week(&pool, ["take", "silence"]).await;
		assert_eq!(export(&pool, Kind::Interview, ME, &folders, AFTER).await.unwrap().written, 1);
		for statement in [
			"DELETE FROM voice_note_audio WHERE subject_id = ?1 AND note_id = ?2",
			"DELETE FROM voice_note WHERE subject_id = ?1 AND note_id = ?2",
			"INSERT INTO voice_note_deletion (subject_id, note_id, of_kind, recorded_at, size, stored_at, deleted_at, went, stayed)
			 VALUES (?1, ?2, 'interview', 'then', 4, 'then', 'now', '[]', '[]')",
		] {
			sqlx::query(statement).bind(ME).bind(note).execute(&pool).await.unwrap();
		}
		let tidied = tidy(&pool, Kind::Interview, Some(ME), &folders, AFTER).await.unwrap();
		assert_eq!(tidied.withdrawn, 1, "{note}");
		assert!(!folders.essays.join("2026-w46").exists());
		let mut conn = pool.acquire().await.unwrap();
		let withdrawn = rust_round_repo::essay_video::withdrawn(&mut conn, Kind::Interview, ME).await.unwrap();
		assert_eq!(withdrawn.iter().map(|w| (w.week.as_str(), w.note.as_str())).collect::<Vec<_>>(), [("2026-w46", note)]);
		assert!(rust_round_repo::essay_video::let_go(&mut conn, Kind::RustRounds, ME).await.unwrap().is_empty());
	}

	#[test]
	fn each_kind_has_its_own_folders() {
		let folders: Vec<&str> = KINDS
			.iter()
			.flat_map(|kind| {
				let spec = spec(*kind);
				[spec.essays, spec.renders, spec.status, spec.work]
			})
			.collect();
		let mut unique = folders.clone();
		unique.sort_unstable();
		unique.dedup();
		assert_eq!(unique.len(), folders.len());
		assert_eq!(spec(Kind::RustRounds).essays, "/app/dbs/rust-essays", "the Rust Rounds kind's are unchanged");
		assert_eq!(spec(Kind::RustRounds).renders, "/app/dbs/renders/rust-essay");
	}
}

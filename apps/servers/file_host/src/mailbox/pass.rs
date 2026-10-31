//! One pass of the mailbox poller.
//!
//! It reads what each folder home reads ([`READS`]: the Jobs digests, the
//! reflections routine's word sets, dramas and run records, and the `LeetType`
//! session files) holds after home's cursor for it and keeps each file that
//! checks out, then files each transcript home heard and has not filed
//! ([`TRANSCRIPT_SCHEMA`]) and each finished `LeetType` session whose log is
//! due ([`LEETYPE_LOG_SCHEMA`]), and hands the mailbox what is filed: the
//! decisions and scouts the phone filed (`PUT /jobs/files/:name`, text that
//! names a voice note by id at most), which lessons it opened, those
//! transcripts and those logs.
//!
//! A failure to reach or read the mailbox, or to write, ends the pass with
//! what it kept so far, and the next pass tries again from the cursor. A file
//! that does not check out (its bytes are not what the listing said, it is
//! not JSON, or its name already holds other bytes) is refused and counted,
//! and the pass goes on: one bad file never holds back the days after it.
//! A refused file that is the newest in its folder is asked for again each
//! pass until a newer one is kept, since the cursor is the highest `seq` held.
//!
//! A transcript is text only: the note's id, what it was said for and when,
//! what was heard, its language and model, and when home heard it. Never
//! audio and never a Drive id; nothing it names points at audio. Each is
//! filed once, under the next number its note's names have
//! (`mailbox_repo::file_transcript`), so a better model's transcript goes
//! under a new name.
//!
//! A `LeetType` session's log is what home holds of the session
//! (`leetype_session_event`), rebuilt from that table and never from a
//! request: every event, in the order `GET /leetype/sessions/:session/events`
//! serves them, and the session file it started. When a session is due, and
//! why every session home holds is finished, is docs/identity.md invariant
//! 18's; [`file_leetype_logs`] files them.
//!
//! Handing over is once per file: the mailbox's answer, whatever it is, is
//! recorded and final (a `conflict` or a refusal is not retried), and only a
//! failure to reach it leaves the file to be handed again.
//!
//! With the archive token (`MAILBOX_ARCHIVE_TOKEN`), a pass also keeps the
//! mailbox small. It first puts back each archived file the routine asked
//! for, from `mailbox_archive`. Then it archives the files live there for
//! `MAILBOX_ARCHIVE_DAYS`, oldest first, at most [`ITEMS_PER_PASS`]: each is
//! read, checked against its listing and stored in `mailbox_archive`, and
//! only then does the mailbox drop its bytes, and only if they are still the
//! ones home stored. A file home could not store stays live.

use crate::auth::DeletionLock;
use chrono::{DateTime, SecondsFormat, Utc};
use leetype_log_repo::{Event, LogError, SessionEvents};
use mailbox_kit::{Archive, Folder, Mailbox, MailboxError, MailboxTransport, Put, Restore, PAGE_LIMIT};
use mailbox_repo::{Answer, ArchiveIn, Change, Heard, ItemIn};
use serde::de::IgnoredAny;
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;
use std::fmt;

/// The most files one pass takes from a folder. More waiting are taken by
/// the next pass.
pub const ITEMS_PER_PASS: u32 = 20;

/// The folders home reads and keeps in `mailbox_item`.
///
/// The routine's digests (`GET /jobs/digests`), its word sets, dramas and run
/// records (`GET /routine/files`), and its `LeetType` session files
/// (`GET /leetype/session-files`).
pub const READS: [Folder; 5] = [Folder::Brief, Folder::Scene, Folder::Drama, Folder::Run, Folder::Leetype];

/// What a transcript put to the mailbox's `transcript` folder is
/// (paulgsc/some-ui `apps/mailbox/README.md`, "What a put must satisfy").
pub const TRANSCRIPT_SCHEMA: &str = "some-ui.voice-transcript/1";

/// What a `LeetType` session's log put to the mailbox's `leetype-log` folder
/// is (paulgsc/some-ui `apps/mailbox`, the `leetype-log` folder's rules).
pub const LEETYPE_LOG_SCHEMA: &str = "some-ui.leetype-log/1";

/// The largest body the mailbox takes (paulgsc/some-ui `apps/mailbox`,
/// `MAX_BODY_BYTES`): a log over it is refused at filing, never handed.
pub const MAILBOX_BODY_CEILING: usize = 1_048_576;

/// The most pages one pass lists per folder. Inbound passes over a file home
/// already holds (one put back after archiving, under a new `seq`) without
/// fetching or counting it, so a page of them never stops the pass short of
/// a new digest behind them. The bound: home's cursor moves only when it
/// keeps a new file, so more than 200 files put back at once ahead of a new
/// digest hold it back until they are archived again.
const PAGES_PER_PASS: usize = 10;

/// What a pass did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Outcome {
	/// Files kept this pass.
	pub kept: usize,
	/// Files refused this pass, each by why: `mismatch`, `bad_name`,
	/// `not_json` or `rewritten` for a file read, and `unreadable`, `no_file`
	/// or `too_big` for a `LeetType` session's log not filed
	/// (`metrics::mailbox::REFUSALS`).
	pub refused: Vec<&'static str>,
	/// Files the poller filed for the mailbox itself this pass: transcripts
	/// and `LeetType` session logs.
	pub filed: usize,
	/// Filed files handed to the mailbox this pass, each by its answer:
	/// `added`, `unchanged`, `conflict` or `refused` (`mailbox_repo::Answer`).
	pub handed: Vec<&'static str>,
	/// Files archived home this pass, each by what came of it
	/// (`metrics::mailbox::ARCHIVE_ANSWERS`).
	pub archived: Vec<&'static str>,
	/// Asked-for files put back this pass, each by what came of it
	/// (`metrics::mailbox::RESTORE_ANSWERS`).
	pub restored: Vec<&'static str>,
	/// The subject has no account: the pass stopped and wrote nothing more.
	pub no_account: bool,
}

/// Why a pass ended early, and what it had done by then.
///
/// Every phase that failed is in `failures`, in the order the pass ran them,
/// so a second failure in an independent phase is never hidden behind the
/// first. Never empty.
#[derive(Debug)]
pub struct PassError {
	pub outcome: Outcome,
	pub failures: Vec<(Phase, Failure)>,
}

/// The independent phases of a pass, in the order it runs them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
	/// Reading [`READS`] after the cursor.
	Inbound,
	/// Filing transcripts home heard and `LeetType` sessions' logs.
	Filing,
	/// Handing the mailbox what is filed.
	Outbound,
	/// Putting back what the routine asked for.
	Asks,
	/// Archiving what is old.
	Archive,
}

impl Phase {
	#[must_use]
	pub const fn name(self) -> &'static str {
		match self {
			Self::Inbound => "inbound",
			Self::Filing => "filing",
			Self::Outbound => "outbound",
			Self::Asks => "asks",
			Self::Archive => "archive",
		}
	}
}

#[derive(Debug)]
pub enum Failure {
	Mailbox(MailboxError),
	Storage(sqlx::Error),
}

impl Failure {
	/// Which `metrics::mailbox::PASS_ERROR_CLASSES` bucket this lands in.
	#[must_use]
	pub const fn class(&self) -> &'static str {
		match self {
			Self::Mailbox(err) => err.kind(),
			Self::Storage(_) => "storage",
		}
	}
}

impl fmt::Display for Failure {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::Mailbox(err) => err.fmt(f),
			Self::Storage(err) => write!(f, "database error: {err}"),
		}
	}
}

impl PassError {
	/// Each `metrics::mailbox::PASS_ERROR_CLASSES` bucket a failed phase lands
	/// in, once each, in the order the phases ran.
	#[must_use]
	pub fn classes(&self) -> Vec<&'static str> {
		let mut classes: Vec<&'static str> = Vec::new();
		for (_, failure) in &self.failures {
			if !classes.contains(&failure.class()) {
				classes.push(failure.class());
			}
		}
		classes
	}
}

impl fmt::Display for PassError {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		for (n, (phase, failure)) in self.failures.iter().enumerate() {
			if n > 0 {
				f.write_str("; ")?;
			}
			write!(f, "{}: {failure}", phase.name())?;
		}
		Ok(())
	}
}

impl std::error::Error for PassError {}

/// Now, as ISO-8601 UTC to the millisecond.
#[must_use]
pub fn now() -> String {
	Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// The mailbox as the archive token reaches it, and how long a file stays
/// live there before home archives it.
pub struct Archiving<'a, T> {
	pub mailbox: &'a Mailbox<T>,
	pub after_days: u64,
}

/// `days` before `now`, in the mailbox's own format, so the two compare as
/// text; `None` when `now` does not read.
fn days_before(now: &str, days: u64) -> Option<String> {
	let now = DateTime::parse_from_rfc3339(now).ok()?.with_timezone(&Utc);
	let back = chrono::Duration::try_days(i64::try_from(days).ok()?)?;
	Some((now - back).to_rfc3339_opts(SecondsFormat::Millis, true))
}

/// `back` before `now`, in the mailbox's own format, so the two compare as
/// text; `None` when `now` does not read.
fn before(now: &str, back: std::time::Duration) -> Option<String> {
	let now = DateTime::parse_from_rfc3339(now).ok()?.with_timezone(&Utc);
	let back = chrono::Duration::from_std(back).ok()?;
	Some((now - back).to_rfc3339_opts(SecondsFormat::Millis, true))
}

/// One pass of the poller.
///
/// It reads each of [`READS`] after `subject`'s cursor and keeps what checks out, then
/// hands the mailbox what the phone filed and it has not answered, then, with
/// `archiving`, answers the routine's asks and archives what is old.
///
/// The phases are independent: a failure ends its own phase, and the pass
/// reports every phase that failed. `settle` is how long a `LeetType`
/// session's newest event must have been home before its log is filed: the
/// poll interval, so a send still in flight is never filed in part.
///
/// # Errors
/// A [`PassError`] when the mailbox cannot be reached or read (anything but
/// a file that does not check out, or an answer that is final), or the
/// database fails.
pub async fn run_once<T: MailboxTransport>(
	db: &SqlitePool,
	mailbox: &Mailbox<T>,
	archiving: Option<&Archiving<'_, T>>,
	subject: &str,
	deletion: &DeletionLock,
	settle: std::time::Duration,
	now: &(impl Fn() -> String + Sync),
) -> Result<Outcome, PassError> {
	let mut outcome = Outcome::default();
	let mut inbound = Ok(());
	for folder in READS {
		if inbound.is_err() || outcome.no_account {
			break;
		}
		inbound = read_folder(db, mailbox, folder, subject, deletion, now, &mut outcome).await;
	}
	let filing = if outcome.no_account {
		Ok(())
	} else {
		let transcripts = file_transcripts(db, subject, deletion, now, &mut outcome).await;
		let logs = if outcome.no_account {
			Ok(())
		} else {
			file_leetype_logs(db, subject, deletion, settle, now, &mut outcome).await
		};
		transcripts.and(logs)
	};
	let outbound = if outcome.no_account {
		Ok(())
	} else {
		hand_over(db, mailbox, subject, deletion, now, &mut outcome).await
	};
	let asks = match archiving {
		Some(archiving) if !outcome.no_account => answer_asks(db, archiving, subject, &mut outcome).await,
		_ => Ok(()),
	};
	let old = match archiving {
		Some(archiving) if !outcome.no_account => archive_old(db, archiving, subject, deletion, now, &mut outcome).await,
		_ => Ok(()),
	};
	let failures: Vec<(Phase, Failure)> = [
		(Phase::Inbound, inbound),
		(Phase::Filing, filing),
		(Phase::Outbound, outbound),
		(Phase::Asks, asks),
		(Phase::Archive, old),
	]
	.into_iter()
	.filter_map(|(phase, result)| result.err().map(|failure| (phase, failure)))
	.collect();
	if failures.is_empty() {
		Ok(outcome)
	} else {
		Err(PassError { outcome, failures })
	}
}

async fn read_folder<T: MailboxTransport>(
	db: &SqlitePool,
	mailbox: &Mailbox<T>,
	folder: Folder,
	subject: &str,
	deletion: &DeletionLock,
	now: &(impl Fn() -> String + Sync),
	outcome: &mut Outcome,
) -> Result<(), Failure> {
	let mut after = cursor(db, subject, folder).await.map_err(Failure::Storage)?;
	let mut fetched = 0;
	for _ in 0..PAGES_PER_PASS {
		let items = mailbox.list(folder, after, ITEMS_PER_PASS).await.map_err(Failure::Mailbox)?;
		let full = items.len() >= ITEMS_PER_PASS as usize;
		for item in &items {
			after = after.max(item.seq);
			if held_sha256(db, subject, folder.as_str(), &item.name).await?.as_deref() == Some(item.sha256.to_ascii_lowercase().as_str()) {
				continue;
			}
			if fetched == ITEMS_PER_PASS {
				return Ok(());
			}
			fetched += 1;
			if !read_one(db, (mailbox, folder), subject, deletion, now, outcome, item).await? {
				return Ok(());
			}
		}
		if !full {
			break;
		}
	}
	Ok(())
}

/// Fetches, checks and keeps one file; `false` when the subject has no
/// account, which ends the pass.
async fn read_one<T: MailboxTransport>(
	db: &SqlitePool,
	(mailbox, folder): (&Mailbox<T>, Folder),
	subject: &str,
	deletion: &DeletionLock,
	now: &(impl Fn() -> String + Sync),
	outcome: &mut Outcome,
	item: &mailbox_kit::ItemMeta,
) -> Result<bool, Failure> {
	let body = match mailbox.get(folder, item).await {
		Ok(body) => body,
		Err(err @ (MailboxError::Mismatch | MailboxError::BadName)) => {
			outcome.refused.push(err.kind());
			return Ok(true);
		}
		Err(err) => return Err(Failure::Mailbox(err)),
	};
	if serde_json::from_slice::<IgnoredAny>(&body).is_err() {
		outcome.refused.push("not_json");
		return Ok(true);
	}
	let kept = ItemIn {
		folder: folder.as_str(),
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
			return Ok(false);
		}
		Err(mailbox_repo::MailboxError::Rewritten) => outcome.refused.push("rewritten"),
		Err(mailbox_repo::MailboxError::Storage(err)) => return Err(Failure::Storage(err)),
	}
	Ok(true)
}

/// The SHA-256 `mailbox_item` holds for `subject`'s (`folder`, `name`), if any.
async fn held_sha256(db: &SqlitePool, subject: &str, folder: &str, name: &str) -> Result<Option<String>, Failure> {
	let mut conn = db.acquire().await.map_err(Failure::Storage)?;
	mailbox_repo::held_sha256(&mut conn, subject, folder, name).await.map_err(storage)
}

/// Hands each filed file the mailbox has not answered, oldest first, at most
/// [`ITEMS_PER_PASS`]. Every answer is final and recorded: taken, already
/// held, held with other bytes (`conflict`), or refused. A failure to reach
/// the mailbox records nothing, so the next pass hands that file again.
async fn hand_over<T: MailboxTransport>(
	db: &SqlitePool,
	mailbox: &Mailbox<T>,
	subject: &str,
	deletion: &DeletionLock,
	now: &(impl Fn() -> String + Sync),
	outcome: &mut Outcome,
) -> Result<(), Failure> {
	let pending = {
		let mut conn = db.acquire().await.map_err(Failure::Storage)?;
		mailbox_repo::pending_outgoing(&mut conn, subject, i64::from(ITEMS_PER_PASS)).await.map_err(storage)?
	};
	for file in pending {
		let folder = match file.folder.as_str() {
			"decisions" => Folder::Decisions,
			"scout" => Folder::Scout,
			"opened" => Folder::Opened,
			"transcript" => Folder::Transcript,
			"leetype-log" => Folder::LeetypeLog,
			// `PUT /jobs/files/:name`, `PUT /routine/opened`,
			// `file_transcripts` and `file_leetype_logs` file only these.
			_ => continue,
		};
		let answer = match mailbox.put(folder, &file.name, file.body).await {
			Ok(Put::Added { .. }) => Answer::Added,
			Ok(Put::Unchanged { .. }) => Answer::Unchanged,
			Ok(Put::Conflict) => Answer::Conflict,
			Err(MailboxError::Refused(_) | MailboxError::TooBig | MailboxError::BadName) => Answer::Refused,
			Err(err) => return Err(Failure::Mailbox(err)),
		};
		match record(db, deletion, subject, (&file.folder, &file.name), answer, &now()).await.map_err(storage)? {
			Change::NoAccount => {
				outcome.no_account = true;
				return Ok(());
			}
			Change::Added | Change::Unchanged => outcome.handed.push(answer.as_str()),
		}
	}
	Ok(())
}

/// Files each transcript home heard and has not filed, oldest heard first,
/// at most [`ITEMS_PER_PASS`]; more are filed by the next pass.
async fn file_transcripts(db: &SqlitePool, subject: &str, deletion: &DeletionLock, now: &(impl Fn() -> String + Sync), outcome: &mut Outcome) -> Result<(), Failure> {
	let heard = {
		let mut conn = db.acquire().await.map_err(Failure::Storage)?;
		mailbox_repo::unfiled_transcripts(&mut conn, subject, i64::from(ITEMS_PER_PASS)).await.map_err(storage)?
	};
	for transcript in &heard {
		match file_transcript(db, deletion, subject, transcript, &now()).await.map_err(storage)? {
			Change::NoAccount => {
				outcome.no_account = true;
				return Ok(());
			}
			Change::Added | Change::Unchanged => outcome.filed += 1,
		}
	}
	Ok(())
}

/// One heard word as the mailbox takes it (paulgsc/server#577, ST22): its
/// text, when it was said in milliseconds from the note's start, and how
/// sure the model was of it, left out where the model did not say.
#[derive(serde::Serialize)]
struct FiledWord<'a> {
	text: &'a str,
	#[serde(rename = "startMs")]
	start_ms: i64,
	#[serde(rename = "endMs")]
	end_ms: i64,
	#[serde(skip_serializing_if = "Option::is_none")]
	p: Option<f64>,
	#[serde(skip_serializing_if = "Option::is_none")]
	language: Option<&'a str>,
}

/// A transcript as the mailbox takes it: text, its words with their times
/// and how sure the model was of each, and nothing that points at audio.
fn transcript_body(heard: &Heard, words: &[voice_repo::transcript::Word]) -> Vec<u8> {
	let words: Vec<FiledWord<'_>> = words
		.iter()
		.map(|word| FiledWord {
			text: &word.text,
			start_ms: word.start_ms,
			end_ms: word.end_ms,
			p: word.p,
			language: word.language.as_deref(),
		})
		.collect();
	serde_json::json!({
		"schema": TRANSCRIPT_SCHEMA,
		"note": heard.note_id,
		"of": heard.of_kind,
		"ref": heard.of_ref,
		"at": heard.recorded_at,
		"text": heard.text,
		"language": heard.language,
		"model": heard.model,
		"heardAt": heard.heard_at,
		"words": words,
	})
	.to_string()
	.into_bytes()
}

/// Files the log of each `LeetType` session with events that arrived after
/// the last log filed, in the order they arrived, at most [`ITEMS_PER_PASS`];
/// more are filed by the next pass.
///
/// It stops at the first session whose newest event reached home less than
/// `settle` ago: the phone may still be sending it, and the mark is the
/// subject's, so filing a later session would pass over the rest of it. A
/// session that cannot be filed (an event that does not read back, no
/// `started` event, a body over the mailbox's ceiling) is recorded and
/// counted as refused, and the mark moves past it: it is looked at again
/// only once an event of it arrives.
async fn file_leetype_logs(
	db: &SqlitePool,
	subject: &str,
	deletion: &DeletionLock,
	settle: std::time::Duration,
	now: &(impl Fn() -> String + Sync),
	outcome: &mut Outcome,
) -> Result<(), Failure> {
	let Some(settled) = before(&now(), settle) else {
		return Ok(());
	};
	let due = {
		let mut conn = db.acquire().await.map_err(Failure::Storage)?;
		mailbox_repo::unfiled_leetype_logs(&mut conn, subject, i64::from(ITEMS_PER_PASS)).await.map_err(storage)?
	};
	for due in &due {
		if due.last_written >= settled {
			break;
		}
		match file_leetype_log(db, deletion, subject, due, &now()).await? {
			Logged::NoAccount => {
				outcome.no_account = true;
				return Ok(());
			}
			Logged::Filed => outcome.filed += 1,
			Logged::Refused(why) => outcome.refused.push(why),
		}
	}
	Ok(())
}

/// What filing one session's log came to.
enum Logged {
	Filed,
	/// Not filed, and why (`metrics::mailbox::REFUSALS`): recorded as such.
	Refused(&'static str),
	/// The subject has no account: nothing written.
	NoAccount,
}

/// A `LeetType` session's log as the mailbox takes it: the session, the
/// session file it started, and every event home holds of it, as
/// `GET /leetype/sessions/:session/events` serves them. Nothing else: an
/// event home holds carries no soundbite (`leetype_log_repo::SOUNDBITES`).
/// `None` for a session home holds no `started` event of.
fn leetype_log_body(held: &SessionEvents) -> Option<Vec<u8>> {
	let file = held.events.iter().find_map(|event| match event {
		Event::Started { file, .. } => Some(file.as_str()),
		_ => None,
	})?;
	let body = serde_json::json!({
		"schema": LEETYPE_LOG_SCHEMA,
		"session": held.session,
		"file": file,
		"events": held.events,
	});
	Some(body.to_string().into_bytes())
}

/// Puts back each archived file the routine asked for, from home's archive.
/// The mailbox takes only the bytes it archived; an ask home holds nothing
/// for (`missing`) stays open.
async fn answer_asks<T: MailboxTransport>(db: &SqlitePool, archiving: &Archiving<'_, T>, subject: &str, outcome: &mut Outcome) -> Result<(), Failure> {
	for folder in Folder::ALL {
		let asks = archiving.mailbox.wanted(folder, PAGE_LIMIT).await.map_err(Failure::Mailbox)?;
		for ask in asks {
			let body = {
				let mut conn = db.acquire().await.map_err(Failure::Storage)?;
				mailbox_repo::archived_body(&mut conn, subject, folder.as_str(), &ask.name).await.map_err(storage)?
			};
			let Some(body) = body else {
				outcome.restored.push("missing");
				continue;
			};
			let answer = match archiving.mailbox.restore(folder, &ask.name, body).await {
				Ok(Restore::Restored { .. }) => "restored",
				Ok(Restore::Unchanged { .. }) => "unchanged",
				Ok(Restore::Conflict) => "conflict",
				Err(MailboxError::Refused(_) | MailboxError::TooBig | MailboxError::BadName | MailboxError::NotFound) => "refused",
				Err(err) => return Err(Failure::Mailbox(err)),
			};
			outcome.restored.push(answer);
		}
	}
	Ok(())
}

/// Archives the files live in the mailbox for `archiving.after_days`, oldest
/// first, at most [`ITEMS_PER_PASS`]. Each is stored in `mailbox_archive`
/// before the mailbox is asked to drop it, and the ask names the bytes home
/// stored, so the mailbox drops nothing home does not hold.
async fn archive_old<T: MailboxTransport>(
	db: &SqlitePool,
	archiving: &Archiving<'_, T>,
	subject: &str,
	deletion: &DeletionLock,
	now: &(impl Fn() -> String + Sync),
	outcome: &mut Outcome,
) -> Result<(), Failure> {
	let Some(cutoff) = days_before(&now(), archiving.after_days) else {
		return Ok(());
	};
	let mut budget = ITEMS_PER_PASS as usize;
	for folder in Folder::ALL {
		// Paged, so files that stay live at the head (each counted below)
		// never hold back the old ones behind them.
		let mut after = 0;
		for _ in 0..PAGES_PER_PASS {
			let items = archiving.mailbox.list(folder, after, PAGE_LIMIT).await.map_err(Failure::Mailbox)?;
			let full = items.len() >= PAGE_LIMIT as usize;
			after = items.last().map_or(after, |item| item.seq);
			// Later `seq`s went live later: the first young file ends the folder.
			let old: Vec<_> = items.iter().take_while(|item| item.live_since() <= cutoff.as_str()).collect();
			let young = old.len() < items.len();
			for item in old {
				if budget == 0 {
					return Ok(());
				}
				let Some(answer) = archive_one(db, archiving, subject, deletion, now, folder, item).await? else {
					outcome.no_account = true;
					return Ok(());
				};
				outcome.archived.push(answer);
				// Only a file fetched spends the budget.
				if !matches!(answer, "unread" | "rewritten") {
					budget -= 1;
				}
			}
			if young || !full {
				break;
			}
		}
	}
	Ok(())
}

/// Archives one old file: what came of it, or `None` when the subject has no
/// account. A digest the phone's feed does not hold, or a name home archived
/// with other bytes, is answered without fetching it.
async fn archive_one<T: MailboxTransport>(
	db: &SqlitePool,
	archiving: &Archiving<'_, T>,
	subject: &str,
	deletion: &DeletionLock,
	now: &(impl Fn() -> String + Sync),
	folder: Folder,
	item: &mailbox_kit::ItemMeta,
) -> Result<Option<&'static str>, Failure> {
	let sha256 = item.sha256.to_ascii_lowercase();
	// A file home reads goes home only once home has kept it for the phone.
	if READS.contains(&folder) {
		match held_sha256(db, subject, folder.as_str(), &item.name).await? {
			None => return Ok(Some("unread")),
			Some(held) if held != sha256 => return Ok(Some("rewritten")),
			Some(_) => {}
		}
	}
	let archived = {
		let mut conn = db.acquire().await.map_err(Failure::Storage)?;
		mailbox_repo::archived_sha256(&mut conn, subject, folder.as_str(), &item.name).await.map_err(storage)?
	};
	if archived.as_ref().is_some_and(|archived| *archived != sha256) {
		return Ok(Some("rewritten"));
	}
	let body = match archiving.mailbox.get(folder, item).await {
		Ok(body) => body,
		Err(err @ (MailboxError::Mismatch | MailboxError::BadName)) => return Ok(Some(err.kind())),
		Err(err) => return Err(Failure::Mailbox(err)),
	};
	let file = ArchiveIn {
		folder: folder.as_str(),
		name: &item.name,
		seq: item.seq,
		sha256: &sha256,
		put_at: &item.put_at,
		body: &body,
	};
	match keep_archived(db, deletion, subject, file, &now()).await {
		Ok(Change::Added | Change::Unchanged) => {}
		Ok(Change::NoAccount) => return Ok(None),
		Err(mailbox_repo::MailboxError::Rewritten) => return Ok(Some("rewritten")),
		Err(mailbox_repo::MailboxError::Storage(err)) => return Err(Failure::Storage(err)),
	}
	let answer = match archiving.mailbox.archive(folder, item).await {
		Ok(Archive::Archived) => "archived",
		Ok(Archive::Unchanged) => "unchanged",
		Err(err @ MailboxError::Mismatch) => err.kind(),
		Err(MailboxError::Refused(_)) => "refused",
		Err(MailboxError::NotFound) => "gone",
		Err(err) => return Err(Failure::Mailbox(err)),
	};
	Ok(Some(answer))
}

fn storage(err: mailbox_repo::MailboxError) -> Failure {
	match err {
		mailbox_repo::MailboxError::Storage(err) => Failure::Storage(err),
		mailbox_repo::MailboxError::Rewritten => Failure::Storage(sqlx::Error::Protocol(String::from("a read or an answer cannot rewrite"))),
	}
}

async fn cursor(db: &SqlitePool, subject: &str, folder: Folder) -> Result<i64, sqlx::Error> {
	let mut conn = db.acquire().await?;
	mailbox_repo::cursor(&mut conn, subject, folder.as_str()).await.map_err(|err| match err {
		mailbox_repo::MailboxError::Storage(err) => err,
		mailbox_repo::MailboxError::Rewritten => sqlx::Error::Protocol(String::from("a cursor read cannot rewrite")),
	})
}

/// One write, under the deletion hold (docs/identity.md invariant 9):
/// `keep_item` and `record_handed` check the subject's account in the
/// statement that writes, and the hold keeps a deletion from landing between
/// them. Held for this write only, never across a request to the mailbox.
async fn keep(db: &SqlitePool, deletion: &DeletionLock, subject: &str, item: ItemIn<'_>, now: &str) -> Result<Change, mailbox_repo::MailboxError> {
	let _hold = deletion.hold().await;
	let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
	let change = mailbox_repo::keep_item(&mut tx, subject, item, now).await?;
	tx.commit().await?;
	Ok(change)
}

/// As [`keep`], for a file about to be archived.
async fn keep_archived(db: &SqlitePool, deletion: &DeletionLock, subject: &str, file: ArchiveIn<'_>, now: &str) -> Result<Change, mailbox_repo::MailboxError> {
	let _hold = deletion.hold().await;
	let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
	let change = mailbox_repo::keep_archived(&mut tx, subject, file, now).await?;
	tx.commit().await?;
	Ok(change)
}

/// As [`keep`], for a transcript filed under its note's next number, which
/// is read in the same transaction.
async fn file_transcript(db: &SqlitePool, deletion: &DeletionLock, subject: &str, heard: &Heard, now: &str) -> Result<Change, mailbox_repo::MailboxError> {
	let _hold = deletion.hold().await;
	let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
	let words = voice_repo::transcript::words_heard_by(&mut tx, subject, &heard.note_id, &heard.model)
		.await
		.map_err(mailbox_repo::MailboxError::Storage)?;
	let body = transcript_body(heard, &words);
	let sha256 = hex::encode(Sha256::digest(&body));
	let n = mailbox_repo::next_transcript_number(&mut tx, subject, &heard.note_id).await?;
	let change = mailbox_repo::file_transcript(&mut tx, subject, (&heard.note_id, &heard.model, n), (&sha256, &body), now).await?;
	tx.commit().await?;
	Ok(change)
}

/// As [`keep`], for a `LeetType` session's log, built from the session's
/// events as they stand and filed under its next number with `due.top` as
/// its mark, all in one transaction. A session it cannot file is recorded as
/// refused, with the same mark, and nothing is filed for the mailbox.
async fn file_leetype_log(db: &SqlitePool, deletion: &DeletionLock, subject: &str, due: &mailbox_repo::Due, now: &str) -> Result<Logged, Failure> {
	let _hold = deletion.hold().await;
	let mut tx = db.begin_with("BEGIN IMMEDIATE").await.map_err(Failure::Storage)?;
	let body = match leetype_log_repo::session_events(&mut tx, subject, &due.session).await {
		Ok(held) => leetype_log_body(&held).ok_or("no_file"),
		Err(LogError::Storage(err)) => return Err(Failure::Storage(err)),
		Err(LogError::Unreadable | LogError::Invalid(_) | LogError::Rewritten) => Err("unreadable"),
	};
	let body = match body {
		Ok(body) if body.len() > MAILBOX_BODY_CEILING => Err("too_big"),
		other => other,
	};
	let body = match body {
		Ok(body) => body,
		Err(why) => {
			// Recorded, so the mark moves past it: no run of refusals ever
			// holds back the sessions after them.
			let change = mailbox_repo::refuse_leetype_log(&mut tx, subject, (&due.session, due.top), why, now)
				.await
				.map_err(storage)?;
			tx.commit().await.map_err(Failure::Storage)?;
			return Ok(if change == Change::NoAccount { Logged::NoAccount } else { Logged::Refused(why) });
		}
	};
	let sha256 = hex::encode(Sha256::digest(&body));
	let n = mailbox_repo::next_leetype_log_number(&mut tx, subject, &due.session).await.map_err(storage)?;
	let change = mailbox_repo::file_leetype_log(&mut tx, subject, (&due.session, n, due.top), (&sha256, &body), now)
		.await
		.map_err(storage)?;
	tx.commit().await.map_err(Failure::Storage)?;
	Ok(if change == Change::NoAccount { Logged::NoAccount } else { Logged::Filed })
}

/// As [`keep`], for the mailbox's answer to a handed file.
async fn record(
	db: &SqlitePool,
	deletion: &DeletionLock,
	subject: &str,
	(folder, name): (&str, &str),
	answer: Answer,
	now: &str,
) -> Result<Change, mailbox_repo::MailboxError> {
	let _hold = deletion.hold().await;
	let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
	let change = mailbox_repo::record_handed(&mut tx, subject, folder, name, answer, now).await?;
	tx.commit().await?;
	Ok(change)
}

#[cfg(test)]
mod tests {
	use super::{run_once, Failure, Outcome, Phase, ITEMS_PER_PASS};
	use crate::auth::DeletionLock;
	use mailbox_kit::{Mailbox, MailboxError, MailboxTransport, Method, Request, Response, Token};
	use sha2::{Digest, Sha256};
	use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};
	use std::collections::HashMap;
	use std::sync::Mutex;

	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");
	const OWNER: &str = "subject-owner";
	const BASE: &str = "https://mailbox.example.test";
	/// The poll interval a `LeetType` session's newest event must be older than.
	const SETTLE: std::time::Duration = std::time::Duration::from_secs(900);

	/// A mailbox in memory: `brief`'s items in `seq` order, the listed SHA-256
	/// of each (which a test may make lie), and a switch that fails listings.
	#[derive(Default)]
	struct Fake {
		items: Mutex<Vec<(i64, String, String, String)>>,
		bodies: Mutex<HashMap<String, String>>,
		listing_fails: Mutex<bool>,
		gets: Mutex<Vec<String>>,
		/// What each put said: the next canned answer, else `201 added`.
		put_answers: Mutex<Vec<(u16, &'static str)>>,
		puts_fail: Mutex<bool>,
		puts: Mutex<Vec<String>>,
		/// The body of each put, in `puts` order.
		put_bodies: Mutex<Vec<String>>,
		/// The body of each `LeetType` session log put.
		logs: Mutex<Vec<String>>,
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
			let path = request.url.strip_prefix(BASE).unwrap().to_owned();
			if request.method == Method::Put {
				assert!(
					[
						"/v1/decisions/items/",
						"/v1/scout/items/",
						"/v1/opened/items/",
						"/v1/transcript/items/",
						"/v1/leetype-log/items/"
					]
					.iter()
					.any(|put| path.starts_with(put)),
					"puts only decisions, scouts, which lessons were opened, transcripts and LeetType session logs: {path}"
				);
				if *self.puts_fail.lock().unwrap() {
					return Err("connection refused");
				}
				let body = String::from_utf8_lossy(request.body.as_deref().unwrap_or_default()).into_owned();
				if path.starts_with("/v1/leetype-log/items/") {
					self.logs.lock().unwrap().push(body.clone());
				}
				self.put_bodies.lock().unwrap().push(body);
				self.puts.lock().unwrap().push(path);
				let mut answers = self.put_answers.lock().unwrap();
				let (status, body) = if answers.is_empty() {
					(201, r#"{"change":"added","seq":1}"#)
				} else {
					answers.remove(0)
				};
				return Ok(answer(status, body));
			}
			assert_eq!(request.method, Method::Get, "and reads nothing but the folders home reads");
			if ["/v1/scene/items?", "/v1/drama/items?", "/v1/run/items?", "/v1/leetype/items?"]
				.iter()
				.any(|read| path.starts_with(read))
			{
				return Ok(answer(200, r#"{"items":[]}"#));
			}
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
		pass_at(pool, fake, "2026-10-23T15:00:00.000Z").await
	}

	async fn pass_at(pool: &SqlitePool, fake: &Fake, now: &str) -> Result<Outcome, super::PassError> {
		let mailbox = Mailbox::new(BASE, Token::new(String::from("home-token")), fake).unwrap();
		run_once(pool, &mailbox, None, OWNER, &DeletionLock::default(), SETTLE, &|| String::from(now)).await
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
		assert!(matches!(err.failures[..], [(Phase::Inbound, Failure::Mailbox(MailboxError::Transport(_)))]));
		assert_eq!((err.classes(), err.outcome), (vec!["transport"], Outcome::default()));
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
		let running = run_once(&pool, &mailbox, None, OWNER, &deletion, SETTLE, &|| String::from("2026-10-23T15:00:00.000Z"));
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
		assert!(context.archive.is_none(), "no archive token, nothing archived");

		let archive = ["--mailbox-archive-token", "archive-token"];
		let context = on(&[&all[..], &archive[..]].concat()).unwrap().unwrap();
		assert_eq!((context.archive.is_some(), context.archive_days), (true, 90));
		let err = on(&[&all[..], &["--mailbox-archive-token", "home-token"][..]].concat()).err().unwrap().to_string();
		assert!(err.contains("MAILBOX_ARCHIVE_TOKEN") && !err.contains("home-token"), "{err}");
		let err = on(&[&all[..], &archive[..], &["--mailbox-archive-days", "29"][..]].concat()).err().unwrap().to_string();
		assert!(err.contains("MAILBOX_ARCHIVE_DAYS"), "{err}");
		let err = on(&[&all[..], &archive[..], &["--mailbox-archive-days", "36501"][..]].concat()).err().unwrap().to_string();
		assert!(err.contains("MAILBOX_ARCHIVE_DAYS"), "{err}");
	}

	/// A file as the phone files it (`PUT /jobs/files/:name`, `PUT /routine/opened`).
	async fn filed(pool: &SqlitePool, name: &str) {
		filed_as(pool, name, b"{}").await;
	}

	async fn filed_as(pool: &SqlitePool, name: &str, body: &[u8]) {
		let folder = if name.starts_with("scouts-") {
			"scout"
		} else if name.starts_with("opened-") {
			"opened"
		} else {
			"decisions"
		};
		mailbox_repo::file_outgoing(&mut pool.acquire().await.unwrap(), OWNER, folder, name, "sha", body, "2026-10-23T15:00:00.000Z")
			.await
			.unwrap();
	}

	async fn answers(pool: &SqlitePool) -> Vec<(String, String)> {
		sqlx::query_as("SELECT name, answer FROM mailbox_handed ORDER BY rowid").fetch_all(pool).await.unwrap()
	}

	#[tokio::test]
	async fn a_filed_file_is_handed_over_once_and_its_answer_recorded() {
		let (pool, fake) = (pool().await, Fake::default());
		filed(&pool, "decisions-2026-10-23.json").await;
		filed(&pool, "scouts-20261023T101500.json").await;
		filed(&pool, "opened-20261023T101500123Z.json").await;
		assert_eq!(pass(&pool, &fake).await.unwrap().handed, ["added", "added", "added"]);
		assert_eq!(
			*fake.puts.lock().unwrap(),
			[
				"/v1/decisions/items/decisions-2026-10-23.json",
				"/v1/scout/items/scouts-20261023T101500.json",
				"/v1/opened/items/opened-20261023T101500123Z.json"
			]
		);
		assert!(pass(&pool, &fake).await.unwrap().handed.is_empty(), "an answered file is not handed again");
		assert_eq!(fake.puts.lock().unwrap().len(), 3);
	}

	/// A decisions file that names a voice note by id is text, and goes to
	/// the mailbox's `decisions` folder as the phone filed it (the pass
	/// reads nothing in it, `handlers::jobs` the schema only); the note's
	/// transcript goes to `transcript` by the same id.
	#[tokio::test]
	async fn a_filed_file_is_handed_over_as_filed() {
		let (pool, fake) = (pool().await, Fake::default());
		let body = r#"{"schema":"some-ui.job-decisions/1","picks":[{"id":"e1","voice":["0b6f0f8e-1f2a-4c55-9d7e-3a1b2c3d4e5f"]}]}"#;
		filed_as(&pool, "decisions-2026-10-23.json", body.as_bytes()).await;
		assert_eq!(pass(&pool, &fake).await.unwrap().handed, ["added"]);
		assert_eq!(*fake.puts.lock().unwrap(), ["/v1/decisions/items/decisions-2026-10-23.json"]);
		assert_eq!(*fake.put_bodies.lock().unwrap(), [body]);
	}

	#[tokio::test]
	async fn a_transport_error_records_nothing_and_the_next_pass_hands_it_again() {
		let (pool, fake) = (pool().await, Fake::default());
		filed(&pool, "decisions-2026-10-23.json").await;
		*fake.puts_fail.lock().unwrap() = true;
		let err = pass(&pool, &fake).await.unwrap_err();
		assert_eq!(err.classes(), ["transport"]);
		assert!(answers(&pool).await.is_empty());
		*fake.puts_fail.lock().unwrap() = false;
		assert_eq!(pass(&pool, &fake).await.unwrap().handed, ["added"]);
	}

	#[tokio::test]
	async fn a_conflict_or_a_refusal_is_final_and_not_handed_again() {
		let (pool, fake) = (pool().await, Fake::default());
		filed(&pool, "decisions-2026-10-22.json").await;
		filed(&pool, "decisions-2026-10-23.json").await;
		*fake.put_answers.lock().unwrap() = vec![(409, r#"{"error":"conflict"}"#), (422, r#"{"error":"the schema does not match the folder"}"#)];
		assert_eq!(pass(&pool, &fake).await.unwrap().handed, ["conflict", "refused"]);
		assert_eq!(
			answers(&pool).await,
			[
				(String::from("decisions-2026-10-22.json"), String::from("conflict")),
				(String::from("decisions-2026-10-23.json"), String::from("refused")),
			]
		);
		pass(&pool, &fake).await.unwrap();
		assert_eq!(fake.puts.lock().unwrap().len(), 2, "neither is handed again");
	}

	/// A voice note of the owner's, and what `model` heard in it.
	async fn heard(pool: &SqlitePool, note: &str, model: &str, text: &str) {
		sqlx::query(
			"INSERT OR IGNORE INTO voice_note (subject_id, note_id, of_kind, of_ref, recorded_at, content_type, size, sha256, stored_at)
			 VALUES (?1, ?2, 'scene', 'drama-20261025T031000000Z', '2026-10-25T03:10:00.000Z', 'audio/ogg', 1, 's', 'now')",
		)
		.bind(OWNER)
		.bind(note)
		.execute(pool)
		.await
		.unwrap();
		sqlx::query(
			"INSERT INTO voice_transcript (subject_id, note_id, model, engine, attempt, outcome, language, language_p, duration_ms, text, took_ms, heard_at)
			 VALUES (?1, ?2, ?3, 'whisper.cpp', 1, 'heard', 'ko', 0.9, 1000, ?4, 10, '2026-10-25T03:12:00.000Z')",
		)
		.bind(OWNER)
		.bind(note)
		.bind(model)
		.bind(text)
		.execute(pool)
		.await
		.unwrap();
		// Each word half a second long, as sure as its place in the text,
		// except the last, heard before confidence was kept.
		let words: Vec<&str> = text.split_whitespace().collect();
		for (at, word) in words.iter().enumerate() {
			let index = i64::try_from(at).unwrap();
			let p: Option<f64> = (at + 1 < words.len()).then_some(0.1_f64.mul_add(f64::from(u8::try_from(at).unwrap()), 0.5));
			let language = (at == 0).then_some("ko");
			sqlx::query(
				"INSERT INTO voice_transcript_word (subject_id, note_id, model, word_index, text, start_ms, end_ms, p, language) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
			)
			.bind(OWNER)
			.bind(note)
			.bind(model)
			.bind(index)
			.bind(word)
			.bind(500 * index)
			.bind(500 * index + 400)
			.bind(p)
			.bind(language)
			.execute(pool)
			.await
			.unwrap();
		}
	}

	async fn outgoing_transcripts(pool: &SqlitePool) -> Vec<String> {
		sqlx::query_scalar("SELECT name FROM mailbox_outgoing WHERE folder = 'transcript' ORDER BY rowid")
			.fetch_all(pool)
			.await
			.unwrap()
	}

	#[tokio::test]
	async fn a_transcript_is_filed_and_handed_once_as_text_and_a_better_models_under_a_new_name() {
		let (pool, fake) = (pool().await, Fake::default());
		heard(&pool, "n1", "ggml-small-q5_1.bin", "다음 장면은 시장에서").await;
		let outcome = pass(&pool, &fake).await.unwrap();
		assert_eq!((outcome.filed, outcome.handed), (1, vec!["added"]));
		assert_eq!(*fake.puts.lock().unwrap(), ["/v1/transcript/items/transcript-n1-1.json"]);
		let body: serde_json::Value = serde_json::from_str(&fake.put_bodies.lock().unwrap()[0]).unwrap();
		assert_eq!(
			body,
			serde_json::json!({
				"schema": "some-ui.voice-transcript/1",
				"note": "n1",
				"of": "scene",
				"ref": "drama-20261025T031000000Z",
				"at": "2026-10-25T03:10:00.000Z",
				"text": "다음 장면은 시장에서",
				"language": "ko",
				"model": "ggml-small-q5_1.bin",
				"heardAt": "2026-10-25T03:12:00.000Z",
				"words": [
					{ "text": "다음", "startMs": 0, "endMs": 400, "p": 0.5, "language": "ko" },
					{ "text": "장면은", "startMs": 500, "endMs": 900, "p": 0.6 },
					{ "text": "시장에서", "startMs": 1000, "endMs": 1400 },
				],
			}),
			"text only, in the mailbox's shape, each word with its time and how sure the model was: nothing that points at audio"
		);
		assert_eq!(pass(&pool, &fake).await.unwrap(), Outcome::default(), "filed and handed once");

		heard(&pool, "n1", "ggml-medium-q5_0.bin", "다음 장면은 시장에서요").await;
		*fake.put_answers.lock().unwrap() = vec![(409, r#"{"error":"conflict"}"#)];
		assert_eq!(pass(&pool, &fake).await.unwrap().handed, ["conflict"]);
		assert_eq!(outgoing_transcripts(&pool).await, ["transcript-n1-1.json", "transcript-n1-2.json"]);
		pass(&pool, &fake).await.unwrap();
		assert_eq!(fake.puts.lock().unwrap().len(), 2, "a conflict is final: not filed or handed again");
	}

	/// A note id may hold a `:` (`voice_repo::is_name`), and the mailbox's
	/// transcript names allow one: its transcript is handed, not refused.
	#[tokio::test]
	async fn a_transcript_of_a_note_whose_id_holds_a_colon_is_handed_over() {
		let (pool, fake) = (pool().await, Fake::default());
		heard(&pool, "note:1", "ggml-small-q5_1.bin", "하나").await;
		assert_eq!(pass(&pool, &fake).await.unwrap().handed, ["added"]);
		assert_eq!(*fake.puts.lock().unwrap(), ["/v1/transcript/items/transcript-note:1-1.json"]);
	}

	#[tokio::test]
	async fn no_transcript_is_filed_for_a_deleted_subject() {
		let (pool, fake) = (pool().await, Fake::default());
		heard(&pool, "n1", "ggml-small-q5_1.bin", "하나").await;
		sqlx::query("DELETE FROM account WHERE subject_id = ?1").bind(OWNER).execute(&pool).await.unwrap();
		// The note and transcript a deletion took, written again by hand, as a
		// race with the transcriber could leave them for a moment.
		heard(&pool, "n1", "ggml-small-q5_1.bin", "하나").await;
		let outcome = pass(&pool, &fake).await.unwrap();
		assert!(outcome.no_account);
		assert_eq!(outcome.filed, 0);
		assert!(outgoing_transcripts(&pool).await.is_empty());
		assert!(fake.puts.lock().unwrap().is_empty());
	}

	/// docs/identity.md invariant 19: a transcript's text reaches no log line,
	/// and neither does the mailbox's URL.
	#[tokio::test]
	async fn no_transcript_text_or_mailbox_url_reaches_a_log_line() {
		let (captured, _guard) = crate::privacy::capture();
		let (pool, fake) = (pool().await, Fake::default());
		heard(&pool, "n1", "ggml-small-q5_1.bin", "비밀스러운 문장").await;
		crate::mailbox::log_pass(&pass(&pool, &fake).await);
		*fake.puts_fail.lock().unwrap() = true;
		heard(&pool, "n2", "ggml-small-q5_1.bin", "비밀스러운 문장").await;
		crate::mailbox::log_pass(&pass(&pool, &fake).await);
		let lines = captured.lines();
		assert!(lines.iter().any(|line| line.contains("filed")), "sanity check: the pass was logged: {lines:?}");
		assert!(lines.iter().any(|line| line.contains("failed")), "sanity check: the failure was logged: {lines:?}");
		for line in &lines {
			assert!(!line.contains("비밀"), "a transcript's text reached a log line");
			assert!(!line.contains("mailbox.example.test"), "the mailbox's URL reached a log line");
		}
	}

	/// The mailbox as the archive token sees it: every folder's live files,
	/// what it archived, the routine's open asks, and every archive asked of
	/// it. A request with home's token gets an empty `brief` and nothing else.
	/// One live file: (folder, seq, name, body, live since).
	type Live = (String, i64, String, String, String);

	#[derive(Default)]
	struct Vault {
		live: Mutex<Vec<Live>>,
		/// (folder, name, sha256)
		archived: Mutex<Vec<(String, String, String)>>,
		/// (folder, name)
		wanted: Mutex<Vec<(String, String)>>,
		/// (path, If-Match) of each archive asked for
		deletes: Mutex<Vec<(String, Option<String>)>>,
	}

	fn sha(body: &str) -> String {
		hex::encode(Sha256::digest(body.as_bytes()))
	}

	impl Vault {
		fn add(&self, folder: &str, seq: i64, name: &str, body: &str, live_since: &str) {
			self
				.live
				.lock()
				.unwrap()
				.push((folder.to_owned(), seq, name.to_owned(), body.to_owned(), live_since.to_owned()));
		}

		fn names(&self) -> Vec<String> {
			self.live.lock().unwrap().iter().map(|(_, _, name, ..)| name.clone()).collect()
		}
	}

	impl MailboxTransport for &Vault {
		type Error = &'static str;

		async fn send(&self, request: Request) -> Result<Response, Self::Error> {
			let path = request.url.strip_prefix(BASE).unwrap().to_owned();
			if request.token.expose() == "home-token" {
				assert!(
					request.method == Method::Get
						&& ["/v1/brief/items", "/v1/scene/items", "/v1/drama/items", "/v1/run/items", "/v1/leetype/items"]
							.iter()
							.any(|read| path.starts_with(read)),
					"home's token: {path}"
				);
			} else {
				assert_eq!(request.token.expose(), "archive-token");
			}
			let query = |key: &str| -> Option<i64> { path.split(['?', '&']).find_map(|pair| pair.strip_prefix(key)?.strip_prefix('=')?.parse().ok()) };
			let (after, limit) = (query("after").unwrap_or(0), query("limit").unwrap_or(50));
			let mut parts = path.trim_start_matches("/v1/").splitn(3, '/');
			let (folder, rest) = (parts.next().unwrap().to_owned(), parts.next().unwrap().to_owned());
			let name = parts.next().map(str::to_owned);
			match (request.method, rest.split('?').next().unwrap(), name) {
				(Method::Get, "items", None) => {
					let listed: Vec<serde_json::Value> = self
						.live
						.lock()
						.unwrap()
						.iter()
						.filter(|(f, seq, ..)| *f == folder && *seq > after)
						.take(usize::try_from(limit).unwrap())
						.map(|(_, seq, name, body, since)| {
							serde_json::json!({
								"seq": seq, "name": name, "size": body.len(), "sha256": sha(body),
								"contentType": "application/json", "putAt": "2026-01-01T00:00:00.000Z", "liveSince": since
							})
						})
						.collect();
					Ok(answer(200, &serde_json::json!({ "items": listed }).to_string()))
				}
				(Method::Get, "items", Some(name)) => {
					let found = self
						.live
						.lock()
						.unwrap()
						.iter()
						.find(|(f, _, n, ..)| *f == folder && *n == name)
						.map(|(_, _, _, body, _)| body.clone());
					Ok(found.map_or_else(|| answer(404, "{}"), |body| answer(200, &body)))
				}
				(Method::Get, "wanted", None) => {
					let asks: Vec<serde_json::Value> = self
						.wanted
						.lock()
						.unwrap()
						.iter()
						.filter(|(f, _)| *f == folder)
						.map(|(_, name)| serde_json::json!({ "name": name, "askedAt": "2026-10-23T14:00:00.000Z" }))
						.collect();
					Ok(answer(200, &serde_json::json!({ "items": asks }).to_string()))
				}
				(Method::Delete, "items", Some(name)) => {
					self.deletes.lock().unwrap().push((path.clone(), request.if_match.clone()));
					let held = self
						.live
						.lock()
						.unwrap()
						.iter()
						.find(|(f, _, n, ..)| *f == folder && *n == name)
						.map(|(.., body, _)| sha(body));
					let Some(held) = held else {
						return Ok(answer(404, r#"{"error":"not_found"}"#));
					};
					if request.if_match != Some(String::from("\"") + &held + "\"") {
						return Ok(answer(412, r#"{"error":"mismatch"}"#));
					}
					self.live.lock().unwrap().retain(|(f, _, n, ..)| !(*f == folder && *n == name));
					self.archived.lock().unwrap().push((folder, name, held));
					Ok(answer(200, r#"{"change":"archived"}"#))
				}
				(Method::Put, "items", Some(name)) => {
					let body = String::from_utf8(request.body.unwrap()).unwrap();
					let archived = self.archived.lock().unwrap().iter().any(|(f, n, h)| *f == folder && *n == name && *h == sha(&body));
					if !archived {
						return Ok(answer(409, r#"{"error":"conflict"}"#));
					}
					self.wanted.lock().unwrap().retain(|(f, n)| !(*f == folder && *n == name));
					self.add(&folder, 99, &name, &body, "2026-10-23T15:00:00.000Z");
					Ok(answer(201, r#"{"change":"restored","seq":99}"#))
				}
				other => panic!("the archive token asked for {other:?}"),
			}
		}
	}

	const OLD: &str = "2026-07-01T00:00:00.000Z";
	const YOUNG: &str = "2026-10-01T00:00:00.000Z";

	async fn archive_pass(pool: &SqlitePool, vault: &Vault) -> Outcome {
		let home = Mailbox::new(BASE, Token::new(String::from("home-token")), vault).unwrap();
		let archive = Mailbox::new(BASE, Token::new(String::from("archive-token")), vault).unwrap();
		let archiving = super::Archiving {
			mailbox: &archive,
			after_days: 90,
		};
		run_once(pool, &home, Some(&archiving), OWNER, &DeletionLock::default(), SETTLE, &|| {
			String::from("2026-10-23T15:00:00.000Z")
		})
		.await
		.unwrap()
	}

	async fn archived_home(pool: &SqlitePool) -> Vec<(String, String, Vec<u8>)> {
		sqlx::query_as("SELECT folder, name, body FROM mailbox_archive ORDER BY folder, name")
			.fetch_all(pool)
			.await
			.unwrap()
	}

	#[tokio::test]
	async fn an_old_file_is_stored_home_before_the_mailbox_drops_it_and_a_young_one_stays() {
		let (pool, vault) = (pool().await, Vault::default());
		let (old, old_body) = digest("2026-07-01");
		let (young, young_body) = digest("2026-10-01");
		vault.add("brief", 1, &old, &old_body, OLD);
		vault.add("brief", 2, &young, &young_body, YOUNG);
		vault.add("decisions", 3, "decisions-2026-07-01.json", "{}", OLD);
		let outcome = archive_pass(&pool, &vault).await;
		assert_eq!(outcome.archived, ["archived", "archived"]);
		assert_eq!(vault.names(), [young], "ninety days are counted from when it went live");
		assert_eq!(
			archived_home(&pool).await,
			[
				(String::from("brief"), old.clone(), old_body.clone().into_bytes()),
				(String::from("decisions"), String::from("decisions-2026-07-01.json"), b"{}".to_vec()),
			]
		);
		assert_eq!(
			vault.deletes.lock().unwrap()[0],
			(String::from("/v1/brief/items/") + &old, Some(String::from("\"") + &sha(&old_body) + "\"")),
			"the archive names the bytes home stored"
		);
		assert!(archive_pass(&pool, &vault).await.archived.is_empty(), "nothing old is left");
	}

	#[tokio::test]
	async fn a_file_home_cannot_store_stays_live() {
		let (held, vault) = (pool().await, Vault::default());
		let (old, body) = digest("2026-07-01");
		vault.add("brief", 1, &old, &body, OLD);
		sqlx::query(
			"INSERT INTO mailbox_archive (subject_id, folder, name, seq, sha256, size, put_at, body, kept_at) VALUES (?1, 'brief', ?2, 1, 'other', 2, 'then', x'7b7d', 'now')",
		)
		.bind(OWNER)
		.bind(&old)
		.execute(&held)
		.await
		.unwrap();
		assert_eq!(archive_pass(&held, &vault).await.archived, ["rewritten"]);
		assert!(vault.deletes.lock().unwrap().is_empty(), "the mailbox was not asked to drop it");
		assert_eq!(vault.names(), std::slice::from_ref(&old));

		let (gone, vault) = (pool().await, Vault::default());
		sqlx::query("DELETE FROM account WHERE subject_id = ?1").bind(OWNER).execute(&gone).await.unwrap();
		vault.add("brief", 1, &old, &body, OLD);
		assert!(archive_pass(&gone, &vault).await.no_account);
		assert!(vault.deletes.lock().unwrap().is_empty(), "nor for a subject with no account");
	}

	#[tokio::test]
	async fn an_ask_is_answered_from_home_and_one_home_lacks_stays_open() {
		let (pool, vault) = (pool().await, Vault::default());
		let (kept, body) = digest("2026-07-01");
		vault.add("brief", 1, &kept, &body, OLD);
		archive_pass(&pool, &vault).await;
		vault
			.archived
			.lock()
			.unwrap()
			.push((String::from("brief"), String::from("digest-2026-06-01.json"), String::from("sha")));
		*vault.wanted.lock().unwrap() = vec![(String::from("brief"), kept.clone()), (String::from("brief"), String::from("digest-2026-06-01.json"))];
		let outcome = archive_pass(&pool, &vault).await;
		assert_eq!(outcome.restored, ["restored", "missing"]);
		assert_eq!(vault.names(), std::slice::from_ref(&kept), "the same bytes are live again");
		assert_eq!(vault.live.lock().unwrap()[0].3, body);
		assert_eq!(vault.wanted.lock().unwrap().len(), 1, "the one home lacks stays asked");
		assert!(outcome.archived.is_empty(), "a file just put back is not old");
	}

	#[tokio::test]
	async fn files_put_back_never_hold_back_a_new_digest() {
		let (pool, vault) = (pool().await, Vault::default());
		let days: Vec<(String, String)> = (1..=25).map(|n| digest(&(String::from("2026-09-") + &format_day(n)))).collect();
		for (n, (name, body)) in (1..).zip(&days) {
			vault.add("brief", n, name, body, YOUNG);
		}
		while archive_pass(&pool, &vault).await.kept > 0 {}
		assert_eq!(held(&pool).await.len(), 25);
		// All 25 come back under new seqs, ahead of the next digest.
		vault.live.lock().unwrap().clear();
		for (n, (name, body)) in (100..).zip(&days) {
			vault.add("brief", n, name, body, YOUNG);
		}
		let (next, body) = digest("2026-10-23");
		vault.add("brief", 200, &next, &body, YOUNG);
		assert_eq!(archive_pass(&pool, &vault).await.kept, 1, "the new digest, in one pass");
		assert_eq!(held(&pool).await.last(), Some(&next));
	}

	#[tokio::test]
	async fn the_routines_word_sets_and_dramas_are_kept_like_digests() {
		let (pool, vault) = (pool().await, Vault::default());
		vault.add("scene", 1, "scene-2026-10-13_morning_session-.json", r#"{"schema":"some-ui.routine-scene/1"}"#, YOUNG);
		vault.add("drama", 2, "drama-20261012T145200123Z.json", "not json", OLD);
		vault.add("drama", 3, "drama-20261013T145200123Z.json", r#"{"schema":"some-ui.routine-drama/1"}"#, YOUNG);
		let outcome = archive_pass(&pool, &vault).await;
		assert_eq!((outcome.kept, outcome.refused), (2, vec!["not_json"]));
		let kept: Vec<(String, String)> = sqlx::query_as("SELECT folder, name FROM mailbox_item ORDER BY seq").fetch_all(&pool).await.unwrap();
		assert_eq!(
			kept,
			[
				(String::from("scene"), String::from("scene-2026-10-13_morning_session-.json")),
				(String::from("drama"), String::from("drama-20261013T145200123Z.json")),
			]
		);
		assert_eq!(outcome.archived, ["unread"], "a drama the phone never got stays in the mailbox");
	}

	#[tokio::test]
	async fn a_digest_the_feed_does_not_hold_is_not_archived() {
		let (pool, vault) = (pool().await, Vault::default());
		vault.add("brief", 1, "digest-2026-07-01.json", "not json", OLD);
		let outcome = archive_pass(&pool, &vault).await;
		assert_eq!((outcome.refused, outcome.archived), (vec!["not_json"], vec!["unread"]));
		assert!(vault.deletes.lock().unwrap().is_empty());
	}

	#[tokio::test]
	async fn files_that_stay_live_at_the_head_never_hold_back_older_ones() {
		let (pool, vault) = (pool().await, Vault::default());
		for n in 1..=55 {
			let name = String::from("decisions-2026-06-01-") + &n.to_string() + ".json";
			vault.add("decisions", n, &name, "{}", OLD);
			sqlx::query("INSERT INTO mailbox_archive (subject_id, folder, name, seq, sha256, size, put_at, body, kept_at) VALUES (?1, 'decisions', ?2, ?3, 'other', 2, 'then', x'7b7d', 'now')")
				.bind(OWNER)
				.bind(&name)
				.bind(n)
				.execute(&pool)
				.await
				.unwrap();
		}
		vault.add("decisions", 56, "decisions-2026-06-02.json", "{}", OLD);
		let outcome = archive_pass(&pool, &vault).await;
		assert_eq!(outcome.archived.iter().filter(|answer| **answer == "rewritten").count(), 55);
		assert_eq!(outcome.archived.last(), Some(&"archived"), "past two pages of stuck files");
		assert_eq!(vault.deletes.lock().unwrap().len(), 1, "and none of the stuck ones was asked to go");
	}

	#[tokio::test]
	async fn a_pass_archives_at_most_twenty() {
		let (pool, vault) = (pool().await, Vault::default());
		for n in 1..=25 {
			let (name, body) = digest(&(String::from("2026-06-") + &format_day(n)));
			vault.add("brief", i64::from(n), &name, &body, OLD);
		}
		assert_eq!(archive_pass(&pool, &vault).await.archived.len(), ITEMS_PER_PASS as usize);
		assert_eq!(archive_pass(&pool, &vault).await.archived.len(), 5, "the rest on the next pass");
		assert_eq!(archived_home(&pool).await.len(), 25);
	}

	#[tokio::test]
	async fn a_listing_error_still_hands_over_what_was_filed() {
		let (pool, fake) = (pool().await, Fake::default());
		filed(&pool, "decisions-2026-10-23.json").await;
		*fake.listing_fails.lock().unwrap() = true;
		let err = pass(&pool, &fake).await.unwrap_err();
		assert_eq!((err.classes(), err.outcome.handed), (vec!["transport"], vec!["added"]), "the phases are independent");
	}

	#[tokio::test]
	async fn every_failed_phase_is_reported_not_only_the_first() {
		let (pool, fake) = (pool().await, Fake::default());
		filed(&pool, "decisions-2026-10-23.json").await;
		*fake.listing_fails.lock().unwrap() = true;
		*fake.puts_fail.lock().unwrap() = true;
		let err = pass(&pool, &fake).await.unwrap_err();
		let phases: Vec<Phase> = err.failures.iter().map(|(phase, _)| *phase).collect();
		assert_eq!(phases, [Phase::Inbound, Phase::Outbound], "the second failure is not hidden behind the first");
		assert_eq!(err.classes(), ["transport"], "one class, counted once");
		assert_eq!(
			err.to_string(),
			"inbound: the mailbox could not be reached: connection refused; outbound: the mailbox could not be reached: connection refused"
		);
	}

	/// A `LeetType` session file as the routine puts it.
	fn session_file(title: &str) -> String {
		serde_json::json!({
			"schema": "some-ui.leetype-session/1",
			"file": { "title": title, "oneLine": "One line.", "session": { "kind": "incident", "budgetMinutes": 25 }, "start": "a", "screens": {} },
		})
		.to_string()
	}

	/// paulgsc/some-ui#2026: a session file put into `leetype` is in
	/// `mailbox_item` after one pass, under the same checks as a digest.
	#[tokio::test]
	async fn the_routines_session_files_are_kept_and_a_bad_one_refused_and_counted() {
		let (pool, vault) = (pool().await, Vault::default());
		sqlx::query(
			"INSERT INTO mailbox_item (subject_id, folder, name, seq, sha256, size, body, pulled_at) VALUES (?1, 'leetype', 't1-04-held.json', 1, 'other', 2, x'7b7d', 'now')",
		)
		.bind(OWNER)
		.execute(&pool)
		.await
		.unwrap();
		vault.add("leetype", 2, "t1-01-raft.json", &session_file("Raft"), YOUNG);
		vault.add("leetype", 3, "t1-02-paxos.json", "not json", YOUNG);
		vault.add("leetype", 4, "t1-04-held.json", &session_file("Other bytes"), YOUNG);
		vault.add("leetype", 5, "t1-03-zab.json", &session_file("Zab"), YOUNG);
		let outcome = archive_pass(&pool, &vault).await;
		assert_eq!((outcome.kept, outcome.refused), (2, vec!["not_json", "rewritten"]));
		let kept: Vec<(String, Vec<u8>)> = sqlx::query_as("SELECT name, body FROM mailbox_item WHERE folder = 'leetype' AND seq > 1 ORDER BY seq")
			.fetch_all(&pool)
			.await
			.unwrap();
		assert_eq!(
			kept,
			[
				(String::from("t1-01-raft.json"), session_file("Raft").into_bytes()),
				(String::from("t1-03-zab.json"), session_file("Zab").into_bytes()),
			],
			"each verbatim, and a bad one never holds back the file after it"
		);
		assert_eq!(archive_pass(&pool, &vault).await.kept, 0, "kept once");
	}

	/// One event of the owner's `LeetType` session, through the log's own
	/// checks, as `PUT /leetype/sessions/:session/events/:id` stores it,
	/// reaching home at `written`.
	async fn played_at(pool: &SqlitePool, event: &serde_json::Value, written: &str) {
		let (session, id) = (event["session"].as_str().unwrap(), event["id"].as_str().unwrap());
		leetype_log_repo::put_event(&mut pool.acquire().await.unwrap(), OWNER, session, id, event.to_string().as_bytes(), written)
			.await
			.unwrap();
	}

	/// As [`played_at`], well before the test's passes.
	async fn played(pool: &SqlitePool, event: &serde_json::Value) {
		played_at(pool, event, "2026-10-23T14:00:00.000Z").await;
	}

	fn started(session: &str, id: &str) -> serde_json::Value {
		serde_json::json!({ "id": id, "at": "2026-10-23T13:00:00.000Z", "session": session, "type": "started", "file": "t1-01-raft.json", "form": "full", "declaredMinutes": 25 })
	}

	fn ended(session: &str, id: &str) -> serde_json::Value {
		serde_json::json!({ "id": id, "at": "2026-10-23T13:20:00.000Z", "session": session, "type": "ended", "file": "t1-01-raft.json", "screen": "end", "how": "declared" })
	}

	/// Every key in `value`, at any depth.
	fn keys(value: &serde_json::Value, into: &mut Vec<String>) {
		match value {
			serde_json::Value::Object(map) => {
				for (key, inner) in map {
					into.push(key.clone());
					keys(inner, into);
				}
			}
			serde_json::Value::Array(items) => items.iter().for_each(|inner| keys(inner, into)),
			_ => {}
		}
	}

	/// paulgsc/some-ui#2027: a session's log goes up once, holding exactly
	/// what `GET /leetype/sessions/:session/events` serves and no soundbite;
	/// an event of it that arrives later sends it up again as `.r2`.
	#[tokio::test]
	async fn a_sessions_log_is_filed_and_handed_with_what_home_holds_and_no_soundbite() {
		use serde_json::json;
		let (pool, fake) = (pool().await, Fake::default());
		let file = "t1-01-raft.json";
		played(&pool, &started("s1", "e1")).await;
		played(
			&pool,
			&json!({ "id": "e2", "at": "2026-10-23T13:05:00.000Z", "session": "s1", "type": "spoken", "file": file, "screen": "say-why",
			"answer": { "as": "recorded", "seconds": 41, "strong": true } }),
		)
		.await;
		played(&pool, &ended("s1", "e3")).await;
		let outcome = pass(&pool, &fake).await.unwrap();
		assert_eq!((outcome.filed, outcome.handed), (1, vec!["added"]));
		assert_eq!(*fake.puts.lock().unwrap(), ["/v1/leetype-log/items/log-s1.json"]);
		let held = leetype_log_repo::session_events(&mut pool.acquire().await.unwrap(), OWNER, "s1").await.unwrap();
		let body: serde_json::Value = serde_json::from_str(&fake.logs.lock().unwrap()[0]).unwrap();
		assert_eq!(
			body,
			json!({ "schema": "some-ui.leetype-log/1", "session": "s1", "file": file, "events": serde_json::to_value(&held.events).unwrap() }),
			"the events home serves, in its order, and nothing else"
		);
		assert_eq!(body["events"].as_array().unwrap().len(), 3);
		let mut found = Vec::new();
		keys(&body, &mut found);
		for soundbite in leetype_log_repo::SOUNDBITES {
			assert!(!found.iter().any(|key| key == soundbite), "a filed log holds no {soundbite}");
		}
		assert_eq!(pass(&pool, &fake).await.unwrap(), Outcome::default(), "filed and handed once");

		played(
			&pool,
			&json!({ "id": "e4", "at": "2026-10-23T13:21:00.000Z", "session": "s1", "type": "reading", "file": file, "reading": "r1", "action": "read" }),
		)
		.await;
		assert_eq!(pass(&pool, &fake).await.unwrap().handed, ["added"]);
		assert_eq!(fake.puts.lock().unwrap()[1], "/v1/leetype-log/items/log-s1.r2.json");
		let again: serde_json::Value = serde_json::from_str(&fake.logs.lock().unwrap()[1]).unwrap();
		assert_eq!(again["events"].as_array().unwrap().len(), 4, "with every event, the earlier ones included");
		assert_eq!(pass(&pool, &fake).await.unwrap(), Outcome::default());
	}

	/// Every session home holds is finished, whether or not it ended (the
	/// phone sends only finished ones), but one still arriving waits a poll
	/// interval, and so does every session after it, so a send in flight is
	/// never filed in part.
	#[tokio::test]
	async fn a_session_still_arriving_waits_a_poll_interval_and_holds_back_the_ones_after_it() {
		let (pool, fake) = (pool().await, Fake::default());
		played(&pool, &started("s1", "e1")).await;
		played_at(&pool, &started("s2", "e1"), "2026-10-23T14:50:00.000Z").await;
		played_at(&pool, &started("s3", "e1"), "2026-10-23T14:51:00.000Z").await;
		let outcome = pass(&pool, &fake).await.unwrap();
		assert_eq!(outcome.filed, 1, "s1, which never ended, is filed; s2 arrived ten minutes ago");
		assert_eq!(*fake.puts.lock().unwrap(), ["/v1/leetype-log/items/log-s1.json"]);
		played_at(&pool, &ended("s2", "e2"), "2026-10-23T14:55:00.000Z").await;
		assert_eq!(pass_at(&pool, &fake, "2026-10-23T15:20:00.000Z").await.unwrap().filed, 2, "both, once quiet");
		let names = fake.puts.lock().unwrap()[1..].to_vec();
		assert_eq!(
			names,
			["/v1/leetype-log/items/log-s3.json", "/v1/leetype-log/items/log-s2.json"],
			"in the order they arrived"
		);
		let s2: serde_json::Value = serde_json::from_str(&fake.logs.lock().unwrap()[2]).unwrap();
		assert_eq!(s2["events"].as_array().unwrap().len(), 2, "whole");
	}

	/// A session that cannot be filed is counted and passed over, and never
	/// holds back the sessions after it.
	#[tokio::test]
	async fn a_session_that_cannot_be_filed_is_refused_and_the_next_still_goes() {
		let (pool, fake) = (pool().await, Fake::default());
		let raw = |session: &'static str, id: &'static str, body: String| {
			let pool = pool.clone();
			async move {
				sqlx::query("INSERT INTO leetype_session_event (subject_id, session_id, event_id, at, type, body, written_at) VALUES (?1, ?2, ?3, '2026-10-23T13:00:00.000Z', 'parked', ?4, '2026-10-23T14:00:00.000Z')")
					.bind(OWNER)
					.bind(session)
					.bind(id)
					.bind(body)
					.execute(&pool)
					.await
					.unwrap();
			}
		};
		raw("bad", "e1", String::from("not an event")).await;
		played(&pool, &ended("nofile", "e1")).await;
		played(&pool, &started("big", "e1")).await;
		let screen = "x".repeat(super::MAILBOX_BODY_CEILING);
		raw(
			"big",
			"e2",
			serde_json::json!({ "id": "e2", "at": "2026-10-23T13:00:00.000Z", "session": "big", "type": "parked", "file": "t1-01-raft.json", "screen": screen }).to_string(),
		)
		.await;
		played(&pool, &started("good", "e1")).await;
		let outcome = pass(&pool, &fake).await.unwrap();
		assert_eq!(outcome.refused, ["unreadable", "no_file", "too_big"]);
		assert_eq!((outcome.filed, outcome.handed), (1, vec!["added"]));
		assert_eq!(*fake.puts.lock().unwrap(), ["/v1/leetype-log/items/log-good.json"]);
		assert_eq!(pass(&pool, &fake).await.unwrap(), Outcome::default(), "each refusal is recorded, and not looked at again");
	}

	/// Codex on paulgsc/server#583: more refused sessions than a pass takes
	/// never hold back a valid one after them, since each refusal is recorded
	/// and moves the mark; a refused session that gains an event is tried
	/// again, and filed once it can be.
	#[tokio::test]
	async fn refusals_never_hold_back_a_later_session_and_a_refused_one_is_retried_with_new_events() {
		let (pool, fake) = (pool().await, Fake::default());
		for n in 0..25 {
			played(&pool, &ended(&(String::from("r") + &n.to_string()), "e1")).await;
		}
		played(&pool, &started("good", "e1")).await;
		let first = pass(&pool, &fake).await.unwrap();
		assert_eq!((first.refused.len(), first.filed), (ITEMS_PER_PASS as usize, 0));
		let second = pass(&pool, &fake).await.unwrap();
		assert_eq!((second.refused, second.filed), (vec!["no_file"; 5], 1), "the valid one, on the second pass");
		assert_eq!(*fake.puts.lock().unwrap(), ["/v1/leetype-log/items/log-good.json"]);
		assert_eq!(pass(&pool, &fake).await.unwrap(), Outcome::default(), "a refusal is not looked at again on its own");

		played(&pool, &started("r3", "e0")).await;
		let retried = pass(&pool, &fake).await.unwrap();
		assert_eq!((retried.refused.len(), retried.filed), (0, 1), "its new event makes it due, and now it can be filed");
		assert_eq!(fake.puts.lock().unwrap()[1], "/v1/leetype-log/items/log-r3.json");
		let refusals: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM mailbox_leetype_log WHERE refused = 'no_file'")
			.fetch_one(&pool)
			.await
			.unwrap();
		assert_eq!(refusals, 25, "each refusal recorded once");
	}

	#[tokio::test]
	async fn no_leetype_log_is_filed_for_a_deleted_subject() {
		let (pool, fake) = (pool().await, Fake::default());
		played(&pool, &ended("s1", "e1")).await;
		sqlx::query("DELETE FROM account WHERE subject_id = ?1").bind(OWNER).execute(&pool).await.unwrap();
		// The events a deletion took, written again by hand, as a race with
		// the phone's send could leave them for a moment.
		played(&pool, &started("s1", "e0")).await;
		played(&pool, &ended("s1", "e1")).await;
		let outcome = pass(&pool, &fake).await.unwrap();
		assert!(outcome.no_account);
		assert_eq!(outcome.filed, 0);
		let filed: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM mailbox_outgoing").fetch_one(&pool).await.unwrap();
		assert_eq!(filed, 0);
		assert!(fake.puts.lock().unwrap().is_empty());
	}
}

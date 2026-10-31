//! Home's copy of the Jobs mailbox (paulgsc/some-ui `apps/mailbox`): the
//! files home has read from it, per subject.
//!
//! `20261023000100_create_mailbox_copy.up.sql` records the design; in short:
//!
//! - **Append-only.** [`keep_item`] stores a file under its (folder, name),
//!   or nothing when that name already holds the same bytes; other bytes
//!   under a held name are refused ([`MailboxError::Rewritten`]), never
//!   stored. No row is ever updated (a trigger aborts an `UPDATE`), nothing
//!   here deletes, and nothing here writes `INSERT OR REPLACE`.
//! - **No cursor to keep.** The next read asks the mailbox for items after
//!   [`cursor`], the highest `seq` held per folder.
//! - **Only for a subject with an account.** [`keep_item`] writes in the same
//!   statement that checks the subject's `account` row, and its caller holds
//!   `auth::DeletionLock` around it (docs/identity.md invariants 9 and 19).
//! - **Per subject, and only per subject.** Every query takes the subject and
//!   filters on it.
//! - **One writer per table.** `file_host`'s `mailbox` poller is the only
//!   caller of [`keep_item`], [`record_handed`], [`keep_archived`],
//!   [`file_transcript`] and [`file_leetype_log`], and
//!   `PUT /api/v1/jobs/files/:name` and `PUT /api/v1/routine/opened` the only
//!   callers of [`file_outgoing`] besides it;
//!   `git grep -n 'mailbox_item\|mailbox_outgoing\|mailbox_handed\|mailbox_archive\|mailbox_transcript\|mailbox_leetype_log' -- '*.rs'`
//!   shows no other.
//! - **A transcript is filed once, under a number decided once.**
//!   [`file_transcript`] records which of home's transcripts it filed, by note
//!   and model, and the number its name carries, so a better model's goes
//!   under a new name and none is filed twice
//!   (`20261026000100_create_mailbox_transcript.up.sql`).
//! - **A `LeetType` session's log is filed once per count of its events.**
//!   [`file_leetype_log`] records each log of a finished session it filed,
//!   its number and how many events it held, so a session is filed again only
//!   once home holds more of it, under the next number
//!   (`20261030000400_create_mailbox_leetype_log.up.sql`).
//! - **Archived here first** (docs/identity.md invariant 21): see
//!   [`keep_archived`].
//! - **Handed once.** A file filed for the mailbox is handed until the
//!   mailbox answers it, and the answer is final: [`pending_outgoing`] is the
//!   filed files with no answer recorded.

use sqlx::SqliteConnection;

/// The folder the routine's digests arrive in.
pub const BRIEF: &str = "brief";

/// What a keep did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
	/// The copy did not hold it: it is stored now.
	Added,
	/// The copy already holds these very bytes under the name: nothing written.
	Unchanged,
	/// The subject has no account (it was deleted): nothing written.
	NoAccount,
}

/// Why a keep wrote nothing.
#[derive(Debug)]
pub enum MailboxError {
	/// The name already holds other bytes, which are never replaced.
	Rewritten,
	Storage(sqlx::Error),
}

impl std::fmt::Display for MailboxError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::Rewritten => f.write_str("this name already holds other bytes: home's mailbox copy is append-only"),
			Self::Storage(err) => write!(f, "database error: {err}"),
		}
	}
}

impl std::error::Error for MailboxError {}

impl From<sqlx::Error> for MailboxError {
	fn from(err: sqlx::Error) -> Self {
		Self::Storage(err)
	}
}

/// One file read from the mailbox, checked by the caller against its listing.
#[derive(Debug, Clone, Copy)]
pub struct ItemIn<'a> {
	pub folder: &'a str,
	pub name: &'a str,
	pub seq: i64,
	/// Lowercase hex SHA-256 of `body`.
	pub sha256: &'a str,
	pub body: &'a [u8],
}

/// One file to archive, read from the mailbox and checked by the caller
/// against its listing.
#[derive(Debug, Clone, Copy)]
pub struct ArchiveIn<'a> {
	pub folder: &'a str,
	pub name: &'a str,
	pub seq: i64,
	/// Lowercase hex SHA-256 of `body`.
	pub sha256: &'a str,
	/// When it was first put in the mailbox.
	pub put_at: &'a str,
	pub body: &'a [u8],
}

/// One held digest: its day, and the file's bytes as the mailbox held them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Digest {
	/// `YYYY-MM-DD`, from the file's name, and always a real date.
	pub day: String,
	pub body: Vec<u8>,
}

/// The highest `seq` `subject`'s copy holds from `folder`, or 0 when it holds
/// nothing: the mailbox is asked for items after this.
///
/// # Errors
/// Any `sqlx` failure.
pub async fn cursor(conn: &mut SqliteConnection, subject: &str, folder: &str) -> Result<i64, MailboxError> {
	let highest = sqlx::query_scalar!(r#"SELECT MAX(seq) AS "seq?: i64" FROM mailbox_item WHERE subject_id = ?1 AND folder = ?2"#, subject, folder)
		.fetch_one(&mut *conn)
		.await?;
	Ok(highest.unwrap_or(0))
}

/// The SHA-256 `subject`'s copy holds under (`folder`, `name`), if any.
///
/// # Errors
/// Any `sqlx` failure.
pub async fn held_sha256(conn: &mut SqliteConnection, subject: &str, folder: &str, name: &str) -> Result<Option<String>, MailboxError> {
	let held = sqlx::query_scalar!("SELECT sha256 FROM mailbox_item WHERE subject_id = ?1 AND folder = ?2 AND name = ?3", subject, folder, name)
		.fetch_optional(&mut *conn)
		.await?;
	Ok(held)
}

/// Keeps `item` for `subject`, as of `now`.
///
/// Run it in a `BEGIN IMMEDIATE` transaction, under the deletion hold, so the
/// read that decides and the write see one snapshot.
///
/// # Errors
/// [`MailboxError::Rewritten`] when the name already holds other bytes, and
/// [`MailboxError::Storage`] for any `sqlx` failure.
pub async fn keep_item(conn: &mut SqliteConnection, subject: &str, item: ItemIn<'_>, now: &str) -> Result<Change, MailboxError> {
	let standing = sqlx::query_scalar!(
		"SELECT sha256 FROM mailbox_item WHERE subject_id = ?1 AND folder = ?2 AND name = ?3",
		subject,
		item.folder,
		item.name
	)
	.fetch_optional(&mut *conn)
	.await?;
	if let Some(standing) = standing {
		return if standing == item.sha256 { Ok(Change::Unchanged) } else { Err(MailboxError::Rewritten) };
	}
	let size = i64::try_from(item.body.len()).unwrap_or(i64::MAX);
	let written = sqlx::query!(
		"INSERT INTO mailbox_item (subject_id, folder, name, seq, sha256, size, body, pulled_at)
		 SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8 WHERE EXISTS (SELECT 1 FROM account WHERE subject_id = ?1)",
		subject,
		item.folder,
		item.name,
		item.seq,
		item.sha256,
		size,
		item.body,
		now
	)
	.execute(&mut *conn)
	.await?;
	Ok(if written.rows_affected() == 0 { Change::NoAccount } else { Change::Added })
}

/// `subject`'s held digests, oldest day first, at most `limit`: the days
/// after `after`, or with no `after`, the newest `limit` days.
///
/// A digest is a `brief` file named `digest-YYYY-MM-DD.json` whose day is a
/// real date; any other name is not one, and is left out.
///
/// # Errors
/// Any `sqlx` failure.
pub async fn digests_after(conn: &mut SqliteConnection, subject: &str, after: Option<&str>, limit: i64) -> Result<Vec<Digest>, MailboxError> {
	// `date(day) = day` holds only for a real date: SQLite answers NULL for
	// month 13 and normalises 02-30 to 03-02.
	let digests = if let Some(after) = after {
		sqlx::query_as!(
			Digest,
			r#"SELECT substr(name, 8, 10) AS "day!: String", body
			   FROM mailbox_item
			   WHERE subject_id = ?1 AND folder = 'brief'
			     AND name GLOB 'digest-[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9].json'
			     AND date(substr(name, 8, 10)) = substr(name, 8, 10)
			     AND substr(name, 8, 10) > ?2
			   ORDER BY name
			   LIMIT ?3"#,
			subject,
			after,
			limit
		)
		.fetch_all(&mut *conn)
		.await?
	} else {
		let mut newest = sqlx::query_as!(
			Digest,
			r#"SELECT substr(name, 8, 10) AS "day!: String", body
			   FROM mailbox_item
			   WHERE subject_id = ?1 AND folder = 'brief'
			     AND name GLOB 'digest-[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9].json'
			     AND date(substr(name, 8, 10)) = substr(name, 8, 10)
			   ORDER BY name DESC
			   LIMIT ?2"#,
			subject,
			limit
		)
		.fetch_all(&mut *conn)
		.await?;
		newest.reverse();
		newest
	};
	Ok(digests)
}

/// Stores `file`'s bytes for `subject`, as of `now`.
///
/// It runs before the mailbox is asked to archive the file. A name holds one
/// body: the same bytes again are [`Change::Unchanged`] (a file put back and
/// archived again).
///
/// Run it in a `BEGIN IMMEDIATE` transaction, under the deletion hold, as for
/// [`keep_item`].
///
/// # Errors
/// [`MailboxError::Rewritten`] when the name already holds other bytes, and
/// [`MailboxError::Storage`] for any `sqlx` failure.
pub async fn keep_archived(conn: &mut SqliteConnection, subject: &str, file: ArchiveIn<'_>, now: &str) -> Result<Change, MailboxError> {
	let standing = sqlx::query_scalar!(
		"SELECT sha256 FROM mailbox_archive WHERE subject_id = ?1 AND folder = ?2 AND name = ?3",
		subject,
		file.folder,
		file.name
	)
	.fetch_optional(&mut *conn)
	.await?;
	if let Some(standing) = standing {
		return if standing == file.sha256 { Ok(Change::Unchanged) } else { Err(MailboxError::Rewritten) };
	}
	let size = i64::try_from(file.body.len()).unwrap_or(i64::MAX);
	let written = sqlx::query!(
		"INSERT INTO mailbox_archive (subject_id, folder, name, seq, sha256, size, put_at, body, kept_at)
		 SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9 WHERE EXISTS (SELECT 1 FROM account WHERE subject_id = ?1)",
		subject,
		file.folder,
		file.name,
		file.seq,
		file.sha256,
		size,
		file.put_at,
		file.body,
		now
	)
	.execute(&mut *conn)
	.await?;
	Ok(if written.rows_affected() == 0 { Change::NoAccount } else { Change::Added })
}

/// The bytes `subject`'s archive holds under (`folder`, `name`), if any: what
/// home puts back when the routine asks for an archived file.
///
/// # Errors
/// Any `sqlx` failure.
pub async fn archived_body(conn: &mut SqliteConnection, subject: &str, folder: &str, name: &str) -> Result<Option<Vec<u8>>, MailboxError> {
	let body = sqlx::query_scalar!(
		"SELECT body FROM mailbox_archive WHERE subject_id = ?1 AND folder = ?2 AND name = ?3",
		subject,
		folder,
		name
	)
	.fetch_optional(&mut *conn)
	.await?;
	Ok(body)
}

/// The SHA-256 `subject`'s archive holds under (`folder`, `name`), if any.
///
/// # Errors
/// Any `sqlx` failure.
pub async fn archived_sha256(conn: &mut SqliteConnection, subject: &str, folder: &str, name: &str) -> Result<Option<String>, MailboxError> {
	let sha = sqlx::query_scalar!(
		"SELECT sha256 FROM mailbox_archive WHERE subject_id = ?1 AND folder = ?2 AND name = ?3",
		subject,
		folder,
		name
	)
	.fetch_optional(&mut *conn)
	.await?;
	Ok(sha)
}

/// One file a routine left for the phone: a word set, a drama or a run record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutineFile {
	/// `scene`, `drama` or `run`.
	pub folder: String,
	pub name: String,
	pub body: Vec<u8>,
}

/// `subject`'s newest word sets and dramas from the reflections routine
/// (the `scene` and `drama` folders), newest first, at most `limit`.
///
/// # Errors
/// Any `sqlx` failure.
pub async fn routine_files(conn: &mut SqliteConnection, subject: &str, limit: i64) -> Result<Vec<RoutineFile>, MailboxError> {
	let files = sqlx::query_as!(
		RoutineFile,
		"SELECT folder, name, body FROM mailbox_item
		 WHERE subject_id = ?1 AND folder IN ('scene', 'drama')
		 ORDER BY seq DESC
		 LIMIT ?2",
		subject,
		limit
	)
	.fetch_all(&mut *conn)
	.await?;
	Ok(files)
}

/// `subject`'s newest run records from the reflections routine (the `run`
/// folder: what each run found and added), newest first, at most `limit`.
///
/// The jobs routine's (`run-jobs-*`) are read by [`jobs_runs`], so neither
/// routine's runs crowd out the other's.
///
/// # Errors
/// Any `sqlx` failure.
pub async fn routine_runs(conn: &mut SqliteConnection, subject: &str, limit: i64) -> Result<Vec<RoutineFile>, MailboxError> {
	let runs = sqlx::query_as!(
		RoutineFile,
		"SELECT folder, name, body FROM mailbox_item
		 WHERE subject_id = ?1 AND folder = 'run' AND name NOT LIKE 'run-jobs-%'
		 ORDER BY seq DESC
		 LIMIT ?2",
		subject,
		limit
	)
	.fetch_all(&mut *conn)
	.await?;
	Ok(runs)
}

/// `subject`'s newest run records from the jobs routine (`run-jobs-*` in
/// the `run` folder: what each morning run found and added, a stopped one
/// included), newest first, at most `limit`.
///
/// # Errors
/// Any `sqlx` failure.
pub async fn jobs_runs(conn: &mut SqliteConnection, subject: &str, limit: i64) -> Result<Vec<RoutineFile>, MailboxError> {
	let runs = sqlx::query_as!(
		RoutineFile,
		"SELECT folder, name, body FROM mailbox_item
		 WHERE subject_id = ?1 AND folder = 'run' AND name LIKE 'run-jobs-%'
		 ORDER BY seq DESC
		 LIMIT ?2",
		subject,
		limit
	)
	.fetch_all(&mut *conn)
	.await?;
	Ok(runs)
}

/// One file filed for the mailbox and not yet answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outgoing {
	pub folder: String,
	pub name: String,
	pub body: Vec<u8>,
}

/// What the mailbox said to a file home handed it. Every answer is final.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
	/// It took the file.
	Added,
	/// It already held these very bytes under the name.
	Unchanged,
	/// It holds other bytes under the name, which it never replaces.
	Conflict,
	/// It refused the file (a `422` or a `413`).
	Refused,
}

impl Answer {
	/// The value `mailbox_handed.answer` stores.
	#[must_use]
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::Added => "added",
			Self::Unchanged => "unchanged",
			Self::Conflict => "conflict",
			Self::Refused => "refused",
		}
	}
}

/// Files `body` under (`folder`, `name`) for `subject`, to be handed to the
/// mailbox, as of `now`.
///
/// Run it in the request's `BEGIN IMMEDIATE` transaction.
///
/// # Errors
/// [`MailboxError::Rewritten`] when the name already holds other bytes, and
/// [`MailboxError::Storage`] for any `sqlx` failure.
pub async fn file_outgoing(conn: &mut SqliteConnection, subject: &str, folder: &str, name: &str, sha256: &str, body: &[u8], now: &str) -> Result<Change, MailboxError> {
	let standing = sqlx::query_scalar!(
		"SELECT sha256 FROM mailbox_outgoing WHERE subject_id = ?1 AND folder = ?2 AND name = ?3",
		subject,
		folder,
		name
	)
	.fetch_optional(&mut *conn)
	.await?;
	if let Some(standing) = standing {
		return if standing == sha256 { Ok(Change::Unchanged) } else { Err(MailboxError::Rewritten) };
	}
	let written = sqlx::query!(
		"INSERT INTO mailbox_outgoing (subject_id, folder, name, sha256, body, filed_at)
		 SELECT ?1, ?2, ?3, ?4, ?5, ?6 WHERE EXISTS (SELECT 1 FROM account WHERE subject_id = ?1)",
		subject,
		folder,
		name,
		sha256,
		body,
		now
	)
	.execute(&mut *conn)
	.await?;
	Ok(if written.rows_affected() == 0 { Change::NoAccount } else { Change::Added })
}

/// `subject`'s filed files the mailbox has not answered, oldest filed first,
/// at most `limit`.
///
/// # Errors
/// Any `sqlx` failure.
pub async fn pending_outgoing(conn: &mut SqliteConnection, subject: &str, limit: i64) -> Result<Vec<Outgoing>, MailboxError> {
	let pending = sqlx::query_as!(
		Outgoing,
		"SELECT folder, name, body FROM mailbox_outgoing AS filed
		 WHERE subject_id = ?1
		   AND NOT EXISTS (
		     SELECT 1 FROM mailbox_handed AS handed
		     WHERE handed.subject_id = filed.subject_id AND handed.folder = filed.folder AND handed.name = filed.name
		   )
		 ORDER BY filed_at, rowid
		 LIMIT ?2",
		subject,
		limit
	)
	.fetch_all(&mut *conn)
	.await?;
	Ok(pending)
}

/// Records the mailbox's `answer` to `subject`'s file (`folder`, `name`), as
/// of `now`. Its caller holds the deletion hold, as for [`keep_item`].
///
/// # Errors
/// Any `sqlx` failure.
pub async fn record_handed(conn: &mut SqliteConnection, subject: &str, folder: &str, name: &str, answer: Answer, now: &str) -> Result<Change, MailboxError> {
	let answer = answer.as_str();
	let written = sqlx::query!(
		"INSERT INTO mailbox_handed (subject_id, folder, name, answer, handed_at)
		 SELECT ?1, ?2, ?3, ?4, ?5 WHERE EXISTS (SELECT 1 FROM account WHERE subject_id = ?1)",
		subject,
		folder,
		name,
		answer,
		now
	)
	.execute(&mut *conn)
	.await?;
	Ok(if written.rows_affected() == 0 { Change::NoAccount } else { Change::Added })
}

/// The folder home's transcripts go to.
pub const TRANSCRIPT: &str = "transcript";

/// One transcript home heard and has not filed for the mailbox: what the
/// mailbox's `some-ui.voice-transcript/1` body is built from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Heard {
	pub note_id: String,
	pub model: String,
	/// What the note was said for, and which one (`voice_note`).
	pub of_kind: String,
	pub of_ref: String,
	/// When the phone recorded it.
	pub recorded_at: String,
	pub text: String,
	pub language: String,
	pub heard_at: String,
}

/// `subject`'s transcripts that heard something and are not filed yet.
///
/// Oldest heard first, at most `limit`. A note heard as nothing at all, or
/// not heard, is never one: the routine reads that from the absence. A game
/// bite is never one either: bites leave home only inside the weekly bundle
/// (paulgsc/some-ui#2074, GWB8), and the mailbox's `transcript` folder
/// refuses `of: game`.
///
/// # Errors
/// Any `sqlx` failure.
pub async fn unfiled_transcripts(conn: &mut SqliteConnection, subject: &str, limit: i64) -> Result<Vec<Heard>, MailboxError> {
	let heard = sqlx::query_as!(
		Heard,
		r#"SELECT heard.note_id, heard.model, note.of_kind, note.of_ref, note.recorded_at,
		          heard.text AS "text!", heard.language AS "language!", heard.heard_at
		   FROM voice_transcript AS heard
		   JOIN voice_note AS note ON note.subject_id = heard.subject_id AND note.note_id = heard.note_id
		   WHERE heard.subject_id = ?1
		     AND heard.outcome = 'heard'
		     AND trim(heard.text) <> ''
		     AND note.of_kind <> 'game'
		     AND NOT EXISTS (
		       SELECT 1 FROM mailbox_transcript AS filed
		       WHERE filed.subject_id = heard.subject_id AND filed.note_id = heard.note_id AND filed.model = heard.model
		     )
		   ORDER BY heard.heard_at, heard.note_id, heard.model
		   LIMIT ?2"#,
		subject,
		limit
	)
	.fetch_all(&mut *conn)
	.await?;
	Ok(heard)
}

/// The number `subject`'s next filed transcript of `note_id` takes, from 1.
///
/// # Errors
/// Any `sqlx` failure.
pub async fn next_transcript_number(conn: &mut SqliteConnection, subject: &str, note_id: &str) -> Result<i64, MailboxError> {
	let last = sqlx::query_scalar!(
		r#"SELECT MAX(n) AS "n: i64" FROM mailbox_transcript WHERE subject_id = ?1 AND note_id = ?2"#,
		subject,
		note_id
	)
	.fetch_one(&mut *conn)
	.await?;
	Ok(last.unwrap_or(0) + 1)
}

/// The name a note's `n`th filed transcript goes under in the mailbox.
#[must_use]
pub fn transcript_name(note_id: &str, n: i64) -> String {
	String::from("transcript-") + note_id + "-" + &n.to_string() + ".json"
}

/// Files `subject`'s transcript of `note_id` by `model` as number `n`.
///
/// The number goes in `mailbox_transcript`, and `body`, as of `now`, in
/// `mailbox_outgoing` under [`transcript_name`], for the poller to hand on.
///
/// Run it in one `BEGIN IMMEDIATE` transaction with
/// [`next_transcript_number`], under the deletion hold.
///
/// # Errors
/// [`MailboxError::Rewritten`] when that name already holds other bytes, and
/// [`MailboxError::Storage`] for any `sqlx` failure, a transcript filed
/// twice or a number taken included.
pub async fn file_transcript(
	conn: &mut SqliteConnection,
	subject: &str,
	(note_id, model, n): (&str, &str, i64),
	(sha256, body): (&str, &[u8]),
	now: &str,
) -> Result<Change, MailboxError> {
	let written = sqlx::query!(
		"INSERT INTO mailbox_transcript (subject_id, note_id, model, n, filed_at)
		 SELECT ?1, ?2, ?3, ?4, ?5 WHERE EXISTS (SELECT 1 FROM account WHERE subject_id = ?1)",
		subject,
		note_id,
		model,
		n,
		now
	)
	.execute(&mut *conn)
	.await?;
	if written.rows_affected() == 0 {
		return Ok(Change::NoAccount);
	}
	file_outgoing(conn, subject, TRANSCRIPT, &transcript_name(note_id, n), sha256, body, now).await
}

/// The folder the routine's `LeetType` session files arrive in.
pub const LEETYPE: &str = "leetype";

/// One `LeetType` session file home holds from the routine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionFile {
	/// Its name in the mailbox, `t<thread>-<NN>-<slug>.json`.
	pub name: String,
	/// Lowercase hex SHA-256 of `body`, as the mailbox listed it.
	pub sha256: String,
	/// The file as the mailbox held it: `some-ui.leetype-session/1`, the
	/// session file inside.
	pub body: Vec<u8>,
	/// When home read it.
	pub pulled_at: String,
}

/// `subject`'s newest `LeetType` session files (the `leetype` folder), newest
/// first, at most `limit`.
///
/// # Errors
/// Any `sqlx` failure.
pub async fn leetype_files(conn: &mut SqliteConnection, subject: &str, limit: i64) -> Result<Vec<SessionFile>, MailboxError> {
	let files = sqlx::query_as!(
		SessionFile,
		"SELECT name, sha256, body, pulled_at FROM mailbox_item
		 WHERE subject_id = ?1 AND folder = 'leetype'
		 ORDER BY seq DESC
		 LIMIT ?2",
		subject,
		limit
	)
	.fetch_all(&mut *conn)
	.await?;
	Ok(files)
}

/// `subject`'s `LeetType` session file named `name`, if home holds it.
///
/// # Errors
/// Any `sqlx` failure.
pub async fn leetype_file(conn: &mut SqliteConnection, subject: &str, name: &str) -> Result<Option<SessionFile>, MailboxError> {
	let file = sqlx::query_as!(
		SessionFile,
		"SELECT name, sha256, body, pulled_at FROM mailbox_item WHERE subject_id = ?1 AND folder = 'leetype' AND name = ?2",
		subject,
		name
	)
	.fetch_optional(&mut *conn)
	.await?;
	Ok(file)
}

/// The folder home's `LeetType` session logs go to.
pub const LEETYPE_LOG: &str = "leetype-log";

/// One finished `LeetType` session whose log is due: home has filed none of
/// it, or holds more of its events than the last log it filed did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finished {
	/// The phone's id for the session.
	pub session: String,
	/// How many of its events home holds.
	pub events: i64,
}

/// `subject`'s finished `LeetType` sessions whose log is due, the session that
/// started first first, at most `limit`.
///
/// A session is finished once it holds an `ended` event, or once another
/// session started after its last event, as the phone decides which sessions
/// go home (paulgsc/some-ui `finishedSessions`,
/// `packages/ui/leetype/src/lib/leetype/session-log/home.ts`). A `started`
/// event later than a session's last event is always another session's,
/// since a session's own `started` is one of its events. The session still
/// being played is never one, so a log is not filed mid-session.
///
/// # Errors
/// Any `sqlx` failure.
pub async fn unfiled_leetype_logs(conn: &mut SqliteConnection, subject: &str, limit: i64) -> Result<Vec<Finished>, MailboxError> {
	let due = sqlx::query_as!(
		Finished,
		r#"WITH held AS (
		     SELECT session_id, COUNT(*) AS events, MIN(at) AS first_at, MAX(at) AS last_at, MAX(type = 'ended') AS ended
		     FROM leetype_session_event
		     WHERE subject_id = ?1
		     GROUP BY session_id
		   ),
		   latest AS (
		     SELECT MAX(at) AS started_at FROM leetype_session_event WHERE subject_id = ?1 AND type = 'started'
		   )
		   SELECT held.session_id AS "session!: String", held.events AS "events!: i64"
		   FROM held, latest
		   WHERE (held.ended = 1 OR latest.started_at > held.last_at)
		     AND held.events > COALESCE(
		       (SELECT MAX(filed.events) FROM mailbox_leetype_log AS filed WHERE filed.subject_id = ?1 AND filed.session_id = held.session_id),
		       0
		     )
		   ORDER BY held.first_at, held.session_id
		   LIMIT ?2"#,
		subject,
		limit
	)
	.fetch_all(&mut *conn)
	.await?;
	Ok(due)
}

/// The number `subject`'s next filed log of `session` takes, from 1.
///
/// # Errors
/// Any `sqlx` failure.
pub async fn next_leetype_log_number(conn: &mut SqliteConnection, subject: &str, session: &str) -> Result<i64, MailboxError> {
	let last = sqlx::query_scalar!(
		r#"SELECT MAX(n) AS "n: i64" FROM mailbox_leetype_log WHERE subject_id = ?1 AND session_id = ?2"#,
		subject,
		session
	)
	.fetch_one(&mut *conn)
	.await?;
	Ok(last.unwrap_or(0) + 1)
}

/// The name a session's `n`th filed log goes under in the mailbox:
/// `log-<session>.json` for the first, `log-<session>.r<n>.json` after.
#[must_use]
pub fn leetype_log_name(session: &str, n: i64) -> String {
	if n <= 1 {
		String::from("log-") + session + ".json"
	} else {
		String::from("log-") + session + ".r" + &n.to_string() + ".json"
	}
}

/// Files `subject`'s log of `session` as number `n`, holding `events` events.
///
/// The number and the count go in `mailbox_leetype_log`, and `body`, as of
/// `now`, in `mailbox_outgoing` under [`leetype_log_name`], for the poller
/// to hand on.
///
/// Run it in one `BEGIN IMMEDIATE` transaction with
/// [`next_leetype_log_number`] and the read of the session's events, under
/// the deletion hold.
///
/// # Errors
/// [`MailboxError::Rewritten`] when that name already holds other bytes, and
/// [`MailboxError::Storage`] for any `sqlx` failure, a number taken
/// included.
pub async fn file_leetype_log(
	conn: &mut SqliteConnection,
	subject: &str,
	(session, n, events): (&str, i64, i64),
	(sha256, body): (&str, &[u8]),
	now: &str,
) -> Result<Change, MailboxError> {
	let written = sqlx::query!(
		"INSERT INTO mailbox_leetype_log (subject_id, session_id, n, events, filed_at)
		 SELECT ?1, ?2, ?3, ?4, ?5 WHERE EXISTS (SELECT 1 FROM account WHERE subject_id = ?1)",
		subject,
		session,
		n,
		events,
		now
	)
	.execute(&mut *conn)
	.await?;
	if written.rows_affected() == 0 {
		return Ok(Change::NoAccount);
	}
	file_outgoing(conn, subject, LEETYPE_LOG, &leetype_log_name(session, n), sha256, body, now).await
}

#[cfg(test)]
mod tests {
	use super::{
		archived_body, cursor, digests_after, file_leetype_log, file_outgoing, file_transcript, keep_archived, keep_item, leetype_file, leetype_files, leetype_log_name,
		next_leetype_log_number, next_transcript_number, pending_outgoing, record_handed, unfiled_leetype_logs, unfiled_transcripts, Answer, ArchiveIn, Change, ItemIn,
		MailboxError, BRIEF, LEETYPE,
	};
	use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};

	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");
	const PREVIOUS_MIGRATION: i64 = 20_261_020_000_100;
	const NOW: &str = "2026-10-23T15:00:00.000Z";
	const A: &str = "subject-a";
	const B: &str = "subject-b";

	async fn pool() -> SqlitePool {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();
		for subject in [A, B] {
			sqlx::query("INSERT INTO account (subject_id, user_handle) VALUES (?1, ?2)")
				.bind(subject)
				.bind(subject.as_bytes())
				.execute(&pool)
				.await
				.unwrap();
		}
		pool
	}

	async fn keep(pool: &SqlitePool, subject: &str, name: &str, seq: i64, body: &str) -> Result<Change, MailboxError> {
		let sha = String::from("sha-of-") + body;
		let item = ItemIn {
			folder: BRIEF,
			name,
			seq,
			sha256: &sha,
			body: body.as_bytes(),
		};
		keep_item(&mut pool.acquire().await.unwrap(), subject, item, NOW).await
	}

	async fn days(pool: &SqlitePool, subject: &str, after: Option<&str>, limit: i64) -> Vec<String> {
		digests_after(&mut pool.acquire().await.unwrap(), subject, after, limit)
			.await
			.unwrap()
			.into_iter()
			.map(|digest| digest.day)
			.collect()
	}

	fn digest(day: &str) -> String {
		String::from("digest-") + day + ".json"
	}

	#[tokio::test]
	async fn the_migration_round_trips() {
		let pool = pool().await;
		let tables = || sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name LIKE 'mailbox_%'");
		assert_eq!(tables().fetch_one(&pool).await.unwrap(), 6);
		MIGRATOR.undo(&pool, PREVIOUS_MIGRATION).await.unwrap();
		assert_eq!(tables().fetch_one(&pool).await.unwrap(), 0, "down drops them");
		sqlx::query("DELETE FROM account WHERE subject_id = ?1").bind(A).execute(&pool).await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();
		assert_eq!(tables().fetch_one(&pool).await.unwrap(), 6, "up recreates them");
	}

	/// The columns the migration describes and no others: a new one is a
	/// decision about what home keeps from the mailbox.
	#[tokio::test]
	async fn the_tables_hold_exactly_their_described_columns() {
		let pool = pool().await;
		for (table, expected) in [
			("mailbox_item", &["subject_id", "folder", "name", "seq", "sha256", "size", "body", "pulled_at"][..]),
			("mailbox_outgoing", &["subject_id", "folder", "name", "sha256", "body", "filed_at"][..]),
			("mailbox_handed", &["subject_id", "folder", "name", "answer", "handed_at"][..]),
			(
				"mailbox_archive",
				&["subject_id", "folder", "name", "seq", "sha256", "size", "put_at", "body", "kept_at"][..],
			),
			("mailbox_transcript", &["subject_id", "note_id", "model", "n", "filed_at"][..]),
			("mailbox_leetype_log", &["subject_id", "session_id", "n", "events", "filed_at"][..]),
		] {
			let columns: Vec<String> = sqlx::query_scalar("SELECT name FROM pragma_table_info(?1) ORDER BY cid")
				.bind(table)
				.fetch_all(&pool)
				.await
				.unwrap();
			assert_eq!(columns, expected, "{table}");
		}
	}

	#[tokio::test]
	async fn a_held_name_is_not_kept_twice() {
		let pool = pool().await;
		assert_eq!(keep(&pool, A, &digest("2026-10-22"), 9, "{}").await.unwrap(), Change::Added);
		assert_eq!(
			keep(&pool, A, &digest("2026-10-22"), 9, "{}").await.unwrap(),
			Change::Unchanged,
			"the same bytes write nothing"
		);
		assert!(
			matches!(keep(&pool, A, &digest("2026-10-22"), 9, "[]").await, Err(MailboxError::Rewritten)),
			"other bytes under a held name"
		);
		assert_eq!(keep(&pool, B, &digest("2026-10-22"), 9, "[]").await.unwrap(), Change::Added, "B's copy is B's");
		let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM mailbox_item").fetch_one(&pool).await.unwrap();
		assert_eq!(rows, 2);
	}

	#[tokio::test]
	async fn no_row_is_ever_updated() {
		let pool = pool().await;
		keep(&pool, A, &digest("2026-10-22"), 9, "{}").await.unwrap();
		for statement in [
			"INSERT INTO mailbox_outgoing (subject_id, folder, name, sha256, body, filed_at) VALUES (?1, 'decisions', 'd', 's', x'00', 'now')",
			"INSERT INTO mailbox_handed (subject_id, folder, name, answer, handed_at) VALUES (?1, 'decisions', 'd', 'added', 'now')",
			"INSERT INTO mailbox_archive (subject_id, folder, name, seq, sha256, size, put_at, body, kept_at) VALUES (?1, 'brief', 'b', 1, 's', 1, 'then', x'00', 'now')",
			"INSERT INTO mailbox_transcript (subject_id, note_id, model, n, filed_at) VALUES (?1, 'n', 'm', 1, 'now')",
			"INSERT INTO mailbox_leetype_log (subject_id, session_id, n, events, filed_at) VALUES (?1, 's', 1, 1, 'now')",
		] {
			sqlx::query(statement).bind(A).execute(&pool).await.unwrap();
		}
		for (table, column) in [
			("mailbox_item", "name"),
			("mailbox_outgoing", "name"),
			("mailbox_handed", "name"),
			("mailbox_archive", "name"),
			("mailbox_transcript", "n"),
			("mailbox_leetype_log", "n"),
		] {
			let update = sqlx::query(&(String::from("UPDATE ") + table + " SET " + column + " = 2")).execute(&pool).await;
			assert!(update.unwrap_err().to_string().contains("append-only"), "{table}");
		}
	}

	#[tokio::test]
	async fn the_cursor_is_the_highest_seq_held() {
		let pool = pool().await;
		let at = |subject, folder| {
			let pool = pool.clone();
			async move { cursor(&mut pool.acquire().await.unwrap(), subject, folder).await.unwrap() }
		};
		assert_eq!(at(A, BRIEF).await, 0, "nothing held");
		keep(&pool, A, &digest("2026-10-23"), 10, "{}").await.unwrap();
		keep(&pool, A, &digest("2026-10-22"), 9, "{}").await.unwrap();
		assert_eq!(at(A, BRIEF).await, 10);
		assert_eq!(at(B, BRIEF).await, 0, "per subject");
		assert_eq!(at(A, "decisions").await, 0, "per folder");
	}

	#[tokio::test]
	async fn digests_after_answers_later_days_oldest_first() {
		let pool = pool().await;
		for (seq, day) in [(1, "2026-10-21"), (3, "2026-10-23"), (2, "2026-10-22"), (4, "2026-10-24")] {
			keep(&pool, A, &digest(day), seq, day).await.unwrap();
		}
		assert_eq!(days(&pool, A, Some("2026-10-21"), 7).await, ["2026-10-22", "2026-10-23", "2026-10-24"]);
		assert_eq!(
			days(&pool, A, Some("2026-10-21"), 2).await,
			["2026-10-22", "2026-10-23"],
			"the oldest first, up to the limit"
		);
		assert!(days(&pool, A, Some("2026-10-24"), 7).await.is_empty());
		assert!(days(&pool, B, Some("2026-01-01"), 7).await.is_empty(), "only the subject's own");
		let held = digests_after(&mut pool.acquire().await.unwrap(), A, Some("2026-10-23"), 7).await.unwrap();
		assert_eq!(held[0].body, b"2026-10-24", "the body as the mailbox held it");
	}

	#[tokio::test]
	async fn digests_after_with_no_cursor_answers_the_newest() {
		let pool = pool().await;
		for (seq, day) in [(1, "2026-10-21"), (2, "2026-10-22"), (3, "2026-10-23")] {
			keep(&pool, A, &digest(day), seq, day).await.unwrap();
		}
		assert_eq!(days(&pool, A, None, 2).await, ["2026-10-22", "2026-10-23"], "the newest two, oldest first");
		assert_eq!(days(&pool, A, None, 7).await, ["2026-10-21", "2026-10-22", "2026-10-23"]);
	}

	#[tokio::test]
	async fn names_that_are_not_digests_are_ignored() {
		let pool = pool().await;
		for (seq, name) in [
			(1, "digest-2026-10-22.json"),
			(2, "digest-2026-13-01.json"),
			(3, "digest-2026-02-30.json"),
			(4, "digest-2026-10-23.json.bak"),
			(5, "notes-2026-10-24.json"),
			(6, "digest-26-10-25.json"),
		] {
			keep(&pool, A, name, seq, name).await.unwrap();
		}
		assert_eq!(days(&pool, A, None, 7).await, ["2026-10-22"]);
		assert_eq!(days(&pool, A, Some("2026-01-01"), 7).await, ["2026-10-22"]);
	}

	#[tokio::test]
	async fn nothing_is_kept_for_a_subject_with_no_account() {
		let pool = pool().await;
		assert_eq!(keep(&pool, "subject-gone", &digest("2026-10-22"), 9, "{}").await.unwrap(), Change::NoAccount);
		let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM mailbox_item").fetch_one(&pool).await.unwrap();
		assert_eq!(rows, 0);
	}

	#[tokio::test]
	async fn the_copy_leaves_with_the_account() {
		let pool = pool().await;
		for subject in [A, B] {
			keep(&pool, subject, &digest("2026-10-22"), 9, "{}").await.unwrap();
			sqlx::query("INSERT INTO mailbox_outgoing (subject_id, folder, name, sha256, body, filed_at) VALUES (?1, 'decisions', 'd', 's', x'00', 'now')")
				.bind(subject)
				.execute(&pool)
				.await
				.unwrap();
			sqlx::query("INSERT INTO mailbox_handed (subject_id, folder, name, answer, handed_at) VALUES (?1, 'decisions', 'd', 'added', 'now')")
				.bind(subject)
				.execute(&pool)
				.await
				.unwrap();
			archive(&pool, subject, "digest-2026-07-01.json", "{}").await.unwrap();
			sqlx::query("INSERT INTO mailbox_transcript (subject_id, note_id, model, n, filed_at) VALUES (?1, 'n', 'm', 1, 'now')")
				.bind(subject)
				.execute(&pool)
				.await
				.unwrap();
			sqlx::query("INSERT INTO mailbox_leetype_log (subject_id, session_id, n, events, filed_at) VALUES (?1, 's', 1, 1, 'now')")
				.bind(subject)
				.execute(&pool)
				.await
				.unwrap();
		}
		sqlx::query("DELETE FROM account WHERE subject_id = ?1").bind(A).execute(&pool).await.unwrap();
		for table in [
			"mailbox_item",
			"mailbox_outgoing",
			"mailbox_handed",
			"mailbox_archive",
			"mailbox_transcript",
			"mailbox_leetype_log",
		] {
			let left: Vec<String> = sqlx::query_scalar(&(String::from("SELECT subject_id FROM ") + table)).fetch_all(&pool).await.unwrap();
			assert_eq!(left, [B], "{table}: A's rows went with A's account, B's stayed");
		}
	}

	async fn archive(pool: &SqlitePool, subject: &str, name: &str, body: &str) -> Result<Change, MailboxError> {
		let sha = String::from("sha-of-") + body;
		let file = ArchiveIn {
			folder: BRIEF,
			name,
			seq: 3,
			sha256: &sha,
			put_at: "2026-07-01T14:52:00.000Z",
			body: body.as_bytes(),
		};
		keep_archived(&mut pool.acquire().await.unwrap(), subject, file, NOW).await
	}

	async fn held_body(pool: &SqlitePool, subject: &str) -> Option<Vec<u8>> {
		archived_body(&mut pool.acquire().await.unwrap(), subject, BRIEF, "digest-2026-07-01.json").await.unwrap()
	}

	#[tokio::test]
	async fn an_archived_name_holds_one_body_and_hands_it_back() {
		let pool = pool().await;
		assert_eq!(held_body(&pool, A).await, None);
		assert_eq!(archive(&pool, A, "digest-2026-07-01.json", "{}").await.unwrap(), Change::Added);
		assert_eq!(
			archive(&pool, A, "digest-2026-07-01.json", "{}").await.unwrap(),
			Change::Unchanged,
			"archived again after being put back"
		);
		assert!(matches!(archive(&pool, A, "digest-2026-07-01.json", "[]").await, Err(MailboxError::Rewritten)));
		assert_eq!(held_body(&pool, A).await, Some(b"{}".to_vec()));
		assert_eq!(held_body(&pool, B).await, None, "B's archive is B's");
		assert_eq!(archive(&pool, "subject-gone", "digest-2026-07-01.json", "{}").await.unwrap(), Change::NoAccount);
	}

	async fn file(pool: &SqlitePool, subject: &str, name: &str, body: &str) -> Result<Change, MailboxError> {
		let sha = String::from("sha-of-") + body;
		file_outgoing(&mut pool.acquire().await.unwrap(), subject, "decisions", name, &sha, body.as_bytes(), NOW).await
	}

	async fn pending(pool: &SqlitePool, subject: &str) -> Vec<String> {
		pending_outgoing(&mut pool.acquire().await.unwrap(), subject, 20)
			.await
			.unwrap()
			.into_iter()
			.map(|outgoing| outgoing.name)
			.collect()
	}

	#[tokio::test]
	async fn a_filed_name_is_not_filed_twice() {
		let pool = pool().await;
		assert_eq!(file(&pool, A, "decisions-2026-10-23.json", "{}").await.unwrap(), Change::Added);
		assert_eq!(file(&pool, A, "decisions-2026-10-23.json", "{}").await.unwrap(), Change::Unchanged);
		assert!(matches!(file(&pool, A, "decisions-2026-10-23.json", "[]").await, Err(MailboxError::Rewritten)));
		assert_eq!(file(&pool, "subject-gone", "decisions-2026-10-23.json", "{}").await.unwrap(), Change::NoAccount);
	}

	#[tokio::test]
	async fn a_file_is_pending_until_the_mailbox_answers_it_and_any_answer_is_final() {
		let pool = pool().await;
		for name in ["decisions-2026-10-21.json", "decisions-2026-10-22.json", "decisions-2026-10-23.json"] {
			file(&pool, A, name, name).await.unwrap();
		}
		file(&pool, B, "decisions-2026-10-21.json", "b").await.unwrap();
		assert_eq!(
			pending(&pool, A).await,
			["decisions-2026-10-21.json", "decisions-2026-10-22.json", "decisions-2026-10-23.json"],
			"oldest filed first"
		);
		let mut conn = pool.acquire().await.unwrap();
		record_handed(&mut conn, A, "decisions", "decisions-2026-10-21.json", Answer::Added, NOW).await.unwrap();
		record_handed(&mut conn, A, "decisions", "decisions-2026-10-22.json", Answer::Conflict, NOW).await.unwrap();
		drop(conn);
		assert_eq!(pending(&pool, A).await, ["decisions-2026-10-23.json"], "a conflict is not handed again");
		assert_eq!(pending(&pool, B).await, ["decisions-2026-10-21.json"], "per subject");
		let mut conn = pool.acquire().await.unwrap();
		assert_eq!(
			record_handed(&mut conn, "subject-gone", "decisions", "x", Answer::Refused, NOW).await.unwrap(),
			Change::NoAccount
		);
	}

	/// A voice note `note` of `subject`'s, and one attempt by `model` to hear it.
	async fn heard(pool: &SqlitePool, subject: &str, (note, model, attempt): (&str, &str, i64), outcome: &str, text: Option<&str>, at: &str) {
		sqlx::query(
			"INSERT OR IGNORE INTO voice_note (subject_id, note_id, of_kind, of_ref, recorded_at, content_type, size, sha256, stored_at)
			 VALUES (?1, ?2, 'jobs', '2026-10-25:acme-sre', '2026-10-25T08:00:00.000Z', 'audio/webm', 1, 's', 'now')",
		)
		.bind(subject)
		.bind(note)
		.execute(pool)
		.await
		.unwrap();
		let language = text.map(|_| "ko");
		sqlx::query(
			"INSERT INTO voice_transcript (subject_id, note_id, model, engine, attempt, outcome, language, language_p, duration_ms, text, took_ms, heard_at)
			 VALUES (?1, ?2, ?3, 'whisper.cpp', ?4, ?5, ?6, CASE WHEN ?6 IS NULL THEN NULL ELSE 0.9 END, CASE WHEN ?5 IN ('heard', 'silent') THEN 1000 END, ?7, 10, ?8)",
		)
		.bind(subject)
		.bind(note)
		.bind(model)
		.bind(attempt)
		.bind(outcome)
		.bind(language)
		.bind(text)
		.bind(at)
		.execute(pool)
		.await
		.unwrap();
	}

	async fn unfiled(pool: &SqlitePool, subject: &str) -> Vec<(String, String)> {
		unfiled_transcripts(&mut pool.acquire().await.unwrap(), subject, 10)
			.await
			.unwrap()
			.into_iter()
			.map(|heard| (heard.note_id, heard.model))
			.collect()
	}

	async fn file_next(pool: &SqlitePool, subject: &str, note: &str, model: &str) -> Result<(i64, Change), MailboxError> {
		let mut tx = pool.begin().await.unwrap();
		let n = next_transcript_number(&mut tx, subject, note).await?;
		let change = file_transcript(&mut tx, subject, (note, model, n), ("sha", b"{}"), NOW).await?;
		tx.commit().await.unwrap();
		Ok((n, change))
	}

	#[tokio::test]
	async fn only_heard_transcripts_wait_to_be_filed_oldest_first() {
		let pool = pool().await;
		heard(&pool, A, ("n2", "small", 1), "heard", Some("둘"), "2026-10-25T09:02:00.000Z").await;
		heard(&pool, A, ("n1", "small", 1), "failed", None, "2026-10-25T09:00:00.000Z").await;
		heard(&pool, A, ("n1", "small", 2), "heard", Some("하나"), "2026-10-25T09:01:00.000Z").await;
		heard(&pool, A, ("n3", "small", 1), "silent", None, "2026-10-25T09:03:00.000Z").await;
		heard(&pool, A, ("n4", "small", 1), "heard", Some("   "), "2026-10-25T09:04:00.000Z").await;
		heard(&pool, B, ("n9", "small", 1), "heard", Some("b"), "2026-10-25T09:00:00.000Z").await;
		assert_eq!(
			unfiled(&pool, A).await,
			[(String::from("n1"), String::from("small")), (String::from("n2"), String::from("small"))],
			"oldest heard first; never a failure, a silence or nothing but blanks, nor another subject's"
		);
		let all = unfiled_transcripts(&mut pool.acquire().await.unwrap(), A, 10).await.unwrap();
		assert_eq!((all[0].text.as_str(), all[0].language.as_str(), all[0].of_kind.as_str()), ("하나", "ko", "jobs"));
		file_next(&pool, A, "n1", "small").await.unwrap();
		assert_eq!(unfiled(&pool, A).await, [(String::from("n2"), String::from("small"))], "a filed one waits no more");
	}

	#[tokio::test]
	async fn a_game_bite_is_never_filed_on_its_own() {
		let pool = pool().await;
		sqlx::query(
			"INSERT INTO voice_note (subject_id, note_id, of_kind, of_ref, recorded_at, content_type, size, sha256, stored_at)
			 VALUES (?1, 'bite', 'game', '401772001', '2026-10-25T20:00:00.000Z', 'audio/webm', 1, 's', 'now')",
		)
		.bind(A)
		.execute(&pool)
		.await
		.unwrap();
		heard(&pool, A, ("bite", "small", 1), "heard", Some("third and long"), "2026-10-25T20:01:00.000Z").await;
		heard(&pool, A, ("n1", "small", 1), "heard", Some("하나"), "2026-10-25T20:02:00.000Z").await;
		assert_eq!(
			unfiled(&pool, A).await,
			[(String::from("n1"), String::from("small"))],
			"bites leave home only in the weekly bundle"
		);
	}

	/// A Studio take's transcript is filed as any note's is, said for
	/// `studio` (paulgsc/server#552).
	#[tokio::test]
	async fn a_studio_takes_transcript_is_filed_like_any_note() {
		let pool = pool().await;
		sqlx::query(
			"INSERT INTO voice_note (subject_id, note_id, of_kind, of_ref, recorded_at, content_type, size, sha256, stored_at)
			 VALUES (?1, 't1', 'studio', 'build-journal-ep04:s3:2', '2026-10-30T18:20:00.000Z', 'audio/mp4', 1, 's', 'now')",
		)
		.bind(A)
		.execute(&pool)
		.await
		.unwrap();
		heard(&pool, A, ("t1", "small", 1), "heard", Some("shot three, take two"), "2026-10-30T18:22:00.000Z").await;
		let all = unfiled_transcripts(&mut pool.acquire().await.unwrap(), A, 10).await.unwrap();
		assert_eq!(
			all
				.iter()
				.map(|heard| (heard.note_id.as_str(), heard.of_kind.as_str(), heard.of_ref.as_str()))
				.collect::<Vec<_>>(),
			[("t1", "studio", "build-journal-ep04:s3:2")]
		);
	}

	#[tokio::test]
	async fn each_transcript_of_a_note_takes_the_next_number_once() {
		let pool = pool().await;
		heard(&pool, A, ("n1", "small", 1), "heard", Some("하나"), "2026-10-25T09:01:00.000Z").await;
		heard(&pool, A, ("n1", "medium", 1), "heard", Some("하나요"), "2026-10-26T09:01:00.000Z").await;
		assert_eq!(file_next(&pool, A, "n1", "small").await.unwrap(), (1, Change::Added));
		assert_eq!(
			file_next(&pool, A, "n1", "medium").await.unwrap(),
			(2, Change::Added),
			"a better model's goes under a new name"
		);
		assert_eq!(file_next(&pool, B, "n1", "small").await.unwrap(), (1, Change::Added), "numbers are per subject");
		assert!(
			matches!(file_next(&pool, A, "n1", "small").await, Err(MailboxError::Storage(_))),
			"a transcript is filed once"
		);
		let names: Vec<String> = sqlx::query_scalar("SELECT name FROM mailbox_outgoing WHERE subject_id = ?1 AND folder = 'transcript' ORDER BY rowid")
			.bind(A)
			.fetch_all(&pool)
			.await
			.unwrap();
		assert_eq!(names, ["transcript-n1-1.json", "transcript-n1-2.json"]);
		assert_eq!(file_next(&pool, "subject-gone", "n1", "small").await.unwrap(), (1, Change::NoAccount));
	}

	/// A `LeetType` session file the poller kept for `subject`.
	async fn keep_session_file(pool: &SqlitePool, subject: &str, name: &str, seq: i64) {
		let body = String::from(r#"{"schema":"some-ui.leetype-session/1","file":{"title":""#) + name + r#""}}"#;
		let sha = String::from("sha-of-") + name;
		let item = ItemIn {
			folder: LEETYPE,
			name,
			seq,
			sha256: &sha,
			body: body.as_bytes(),
		};
		keep_item(&mut pool.acquire().await.unwrap(), subject, item, NOW).await.unwrap();
	}

	#[tokio::test]
	async fn session_files_answer_the_subjects_own_newest_first_and_one_by_name() {
		let pool = pool().await;
		keep_session_file(&pool, A, "t1-01-raft.json", 3).await;
		keep_session_file(&pool, A, "t1-02-paxos.json", 5).await;
		keep_session_file(&pool, B, "t1-03-zab.json", 7).await;
		keep(&pool, A, &digest("2026-10-22"), 9, "{}").await.unwrap();
		let mut conn = pool.acquire().await.unwrap();
		let names: Vec<String> = leetype_files(&mut conn, A, 10).await.unwrap().into_iter().map(|file| file.name).collect();
		assert_eq!(names, ["t1-02-paxos.json", "t1-01-raft.json"], "only the leetype folder, only A's, newest first");
		assert_eq!(leetype_files(&mut conn, A, 1).await.unwrap().len(), 1, "at most the limit");
		let one = leetype_file(&mut conn, A, "t1-01-raft.json").await.unwrap().unwrap();
		assert_eq!((one.sha256.as_str(), one.pulled_at.as_str()), ("sha-of-t1-01-raft.json", NOW));
		assert!(leetype_file(&mut conn, A, "t1-03-zab.json").await.unwrap().is_none(), "B's is not A's");
		assert!(leetype_file(&mut conn, A, &digest("2026-10-22")).await.unwrap().is_none(), "nor is a digest a session file");
	}

	/// One event of `subject`'s session, as the phone's log holds it.
	async fn event(pool: &SqlitePool, subject: &str, session: &str, id: &str, kind: &str, at: &str) {
		sqlx::query("INSERT INTO leetype_session_event (subject_id, session_id, event_id, at, type, body, written_at) VALUES (?1, ?2, ?3, ?4, ?5, '{}', 'now')")
			.bind(subject)
			.bind(session)
			.bind(id)
			.bind(at)
			.bind(kind)
			.execute(pool)
			.await
			.unwrap();
	}

	async fn due(pool: &SqlitePool, subject: &str) -> Vec<(String, i64)> {
		unfiled_leetype_logs(&mut pool.acquire().await.unwrap(), subject, 10)
			.await
			.unwrap()
			.into_iter()
			.map(|due| (due.session, due.events))
			.collect()
	}

	async fn file_log(pool: &SqlitePool, subject: &str, session: &str, events: i64) -> Result<(i64, Change), MailboxError> {
		let mut tx = pool.begin().await.unwrap();
		let n = next_leetype_log_number(&mut tx, subject, session).await?;
		let body = String::from("{\"n\":") + &n.to_string() + "}";
		let change = file_leetype_log(&mut tx, subject, (session, n, events), ("sha", body.as_bytes()), NOW).await?;
		tx.commit().await.unwrap();
		Ok((n, change))
	}

	/// The phone's rule for a finished session: one that ended, or one another
	/// session started after. The session still being played waits.
	#[tokio::test]
	async fn a_session_is_due_once_it_ended_or_another_started_after_it() {
		let pool = pool().await;
		event(&pool, A, "s1", "e1", "started", "2026-10-28T09:00:00.000Z").await;
		event(&pool, A, "s1", "e2", "picked", "2026-10-28T09:01:00.000Z").await;
		assert!(due(&pool, A).await.is_empty(), "a session still being played is not due");
		event(&pool, A, "s2", "e1", "started", "2026-10-28T10:00:00.000Z").await;
		assert_eq!(due(&pool, A).await, [(String::from("s1"), 2)], "s2 started after s1's last event");
		event(&pool, A, "s2", "e2", "ended", "2026-10-28T10:30:00.000Z").await;
		event(&pool, A, "s3", "e1", "started", "2026-10-28T09:30:00.000Z").await;
		event(&pool, A, "s3", "e2", "picked", "2026-10-28T10:45:00.000Z").await;
		assert_eq!(
			due(&pool, A).await,
			[(String::from("s1"), 2), (String::from("s2"), 2)],
			"s2 ended; s3 started before s2 but its last event is the newest, so it waits"
		);
		event(&pool, B, "s9", "e1", "ended", "2026-10-28T09:00:00.000Z").await;
		assert_eq!(due(&pool, B).await, [(String::from("s9"), 1)], "per subject");
	}

	#[tokio::test]
	async fn a_filed_log_waits_again_only_once_its_session_holds_more() {
		let pool = pool().await;
		event(&pool, A, "s1", "e1", "started", "2026-10-28T09:00:00.000Z").await;
		event(&pool, A, "s1", "e2", "ended", "2026-10-28T09:30:00.000Z").await;
		assert_eq!(file_log(&pool, A, "s1", 2).await.unwrap(), (1, Change::Added));
		assert!(due(&pool, A).await.is_empty(), "filed with every event it holds");
		event(&pool, A, "s1", "e3", "parked", "2026-10-28T09:31:00.000Z").await;
		assert_eq!(due(&pool, A).await, [(String::from("s1"), 3)], "a later event makes it due again");
		assert_eq!(file_log(&pool, A, "s1", 3).await.unwrap(), (2, Change::Added), "under the next number");
		assert!(due(&pool, A).await.is_empty());
		assert_eq!(file_log(&pool, B, "s1", 1).await.unwrap(), (1, Change::Added), "numbers are per subject");
		let names: Vec<String> = sqlx::query_scalar("SELECT name FROM mailbox_outgoing WHERE subject_id = ?1 AND folder = 'leetype-log' ORDER BY rowid")
			.bind(A)
			.fetch_all(&pool)
			.await
			.unwrap();
		assert_eq!(names, ["log-s1.json", "log-s1.r2.json"]);
		assert_eq!(file_log(&pool, "subject-gone", "s1", 1).await.unwrap(), (1, Change::NoAccount));
		let mut tx = pool.begin().await.unwrap();
		assert!(
			matches!(file_leetype_log(&mut tx, A, ("s1", 2, 4), ("sha", b"{}"), NOW).await, Err(MailboxError::Storage(_))),
			"a number is taken once"
		);
	}

	#[test]
	fn a_logs_name_carries_its_number_after_the_first() {
		assert_eq!(leetype_log_name("4f0c-a_b", 1), "log-4f0c-a_b.json");
		assert_eq!(leetype_log_name("4f0c-a_b", 2), "log-4f0c-a_b.r2.json");
		assert_eq!(leetype_log_name("4f0c-a_b", 12), "log-4f0c-a_b.r12.json");
	}
}

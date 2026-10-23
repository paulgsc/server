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
//!   caller of [`keep_item`], [`record_handed`] and [`keep_archived`], and
//!   `PUT /api/v1/jobs/files/:name` the only caller of [`file_outgoing`];
//!   `git grep -n 'mailbox_item\|mailbox_outgoing\|mailbox_handed\|mailbox_archive' -- '*.rs'`
//!   shows no other.
//! - **Archived here before anywhere else lets go.** [`keep_archived`] stores
//!   a file's bytes before the poller asks the mailbox to archive it, and
//!   [`archived_body`] hands them back when the routine asks for it again.
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

#[cfg(test)]
mod tests {
	use super::{
		archived_body, cursor, digests_after, file_outgoing, keep_archived, keep_item, pending_outgoing, record_handed, Answer, ArchiveIn, Change, ItemIn, MailboxError, BRIEF,
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
		assert_eq!(tables().fetch_one(&pool).await.unwrap(), 4);
		MIGRATOR.undo(&pool, PREVIOUS_MIGRATION).await.unwrap();
		assert_eq!(tables().fetch_one(&pool).await.unwrap(), 0, "down drops them");
		sqlx::query("DELETE FROM account WHERE subject_id = ?1").bind(A).execute(&pool).await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();
		assert_eq!(tables().fetch_one(&pool).await.unwrap(), 4, "up recreates them");
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
		] {
			sqlx::query(statement).bind(A).execute(&pool).await.unwrap();
		}
		for table in ["mailbox_item", "mailbox_outgoing", "mailbox_handed", "mailbox_archive"] {
			let update = sqlx::query(&(String::from("UPDATE ") + table + " SET name = 'other'")).execute(&pool).await;
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
		}
		sqlx::query("DELETE FROM account WHERE subject_id = ?1").bind(A).execute(&pool).await.unwrap();
		for table in ["mailbox_item", "mailbox_outgoing", "mailbox_handed", "mailbox_archive"] {
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
}

//! Home's word that Drive holds a Studio take too (paulgsc/some-ui#2401,
//! HF11): `studio_take_on_drive`.
//!
//! `20261101000800_create_studio_take_on_drive.up.sql` records the design;
//! in short:
//!
//! - **From the build-journal session's list, never from Drive.** Home has no
//!   Drive credential (the owner's route a). The session lists Drive's takes
//!   folder and puts the SHA-256 of each recording there in the Jobs mailbox;
//!   `file_host`'s poller keeps the list in `mailbox_item` and, in the
//!   transaction that keeps it, calls [`record`].
//! - **Whichever arrives second matches** (paulgsc/some-ui#2416, option 1,
//!   approved by the owner 2026-11-01). A list matches the takes home holds
//!   when it arrives ([`record`]); a `studio` take that reaches home after a
//!   list naming it is matched when it arrives ([`match_take`], from
//!   `voice_repo::put_note`, in the transaction that stores it), against the
//!   subject's lists made no earlier than [`SKEW`] before the take was
//!   recorded and containing its SHA-256: one range read over
//!   `mailbox_item`'s key, since a list's name is its stamp. These two are
//!   the table's only writers, both here.
//! - **Only takes home holds.** A list is matched against the subject's
//!   `studio` voice notes by their SHA-256 (`voice_note.sha256`), and only
//!   those are recorded; a hash naming nothing home holds is dropped, never
//!   kept here (the list itself stays in `mailbox_item`, as every `brief`
//!   file does).
//! - **Append-only.** A take is recorded once, by the first list that named
//!   it; nothing is updated (the trigger aborts an `UPDATE`) or deleted but
//!   with the account, and a later list that no longer names it takes
//!   nothing away.
//! - **Per subject.** Every query takes the subject and filters on it.
//! - **Read only by `GET /voice-notes/on-drive`** ([`on_drive`]), for the
//!   phone's own takes, by the phone's ids.

use chrono::{DateTime, TimeDelta};
use serde::Deserialize;
use sqlx::SqliteConnection;
use std::collections::HashSet;

/// The schema of a list of the takes Drive holds, as the mailbox takes it
/// (paulgsc/some-ui `apps/mailbox`, `brief`'s `drive-takes` kind).
pub const DRIVE_TAKES_SCHEMA: &str = "some-ui.drive-takes/1";

/// The most takes one read of [`on_drive`] answers about.
pub const MAX_IDS: usize = 100;

/// The name every list of the takes Drive holds goes under in `brief`.
///
/// The mailbox holds each as `drive-takes-<stamp>.json`, its stamp the list's
/// `at` as `YYYYMMDDTHHMMSSZ` (paulgsc/some-ui `apps/mailbox`), so the names
/// sort as the lists were made.
pub const DRIVE_TAKES_PREFIX: &str = "drive-takes-";

/// The first name past every `drive-takes-*` one (`.` follows `-`): the
/// exclusive end of [`match_take`]'s range read.
const PAST_DRIVE_TAKES: &str = "drive-takes.";

/// How long before a take was recorded a list naming it may say it was
/// made.
///
/// The most a phone's clock may be off home's (`MAX_OFFSET_MS`, as for a
/// bite's offset). A list made earlier than that cannot name the take, since
/// the phone had not recorded it yet, so [`match_take`] does not read it.
pub const SKEW: TimeDelta = TimeDelta::milliseconds(super::MAX_OFFSET_MS);

/// A list of the takes Drive holds, from the bytes of a `brief` file named
/// `drive-takes-*` ([`DRIVE_TAKES_PREFIX`]).
///
/// When the session looked, and each recording's SHA-256 that reads as
/// lower-case hex. `None` for bytes that do not read as a list of the schema
/// [`DRIVE_TAKES_SCHEMA`]. Which file is one is the caller's to check.
#[must_use]
pub fn listed_in(body: &[u8]) -> Option<(String, Vec<String>)> {
	#[derive(Deserialize)]
	struct Take {
		sha256: String,
	}
	#[derive(Deserialize)]
	struct List {
		schema: String,
		at: String,
		takes: Vec<Take>,
	}
	let list: List = serde_json::from_slice(body).ok()?;
	if list.schema != DRIVE_TAKES_SCHEMA {
		return None;
	}
	let sha256 = list
		.takes
		.into_iter()
		.map(|take| take.sha256)
		.filter(|sha256| sha256.len() == 64 && sha256.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
		.collect();
	Some((list.at, sha256))
}

/// One list of the takes Drive holds: when the session looked, the mailbox
/// name it came under, and each recording's SHA-256.
#[derive(Debug, Clone, Copy)]
pub struct Listed<'a> {
	/// When the session listed Drive's takes folder (the list's `at`).
	pub at: &'a str,
	/// The list's name in the mailbox (`drive-takes-<stamp>.json`).
	pub name: &'a str,
	/// The SHA-256 of each recording, lower-case hex.
	pub sha256: &'a [String],
}

/// A take home has recorded as on Drive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OnDrive {
	/// The phone's id for the take, which names its voice note.
	pub note: String,
	/// When the session that first listed it there looked.
	pub listed_at: String,
}

/// Records each of `subject`'s `studio` voice notes whose SHA-256 `listed`
/// names, as of `now`; how many it recorded for the first time.
///
/// Run it in the transaction that keeps the list, so a list is kept and its
/// takes recorded together or not at all.
///
/// # Errors
/// Any `sqlx` failure.
pub async fn record(conn: &mut SqliteConnection, subject: &str, listed: Listed<'_>, now: &str) -> Result<u64, sqlx::Error> {
	let named: HashSet<&str> = listed.sha256.iter().map(String::as_str).collect();
	if named.is_empty() {
		return Ok(0);
	}
	let takes = sqlx::query!(
		r#"SELECT note_id AS "note_id!: String", sha256 AS "sha256!: String"
		   FROM voice_note WHERE subject_id = ?1 AND of_kind = 'studio'"#,
		subject
	)
	.fetch_all(&mut *conn)
	.await?;
	let mut recorded = 0;
	for take in takes.iter().filter(|take| named.contains(take.sha256.as_str())) {
		recorded += sqlx::query!(
			"INSERT INTO studio_take_on_drive (subject_id, note_id, listed_at, list_name, kept_at)
			 VALUES (?1, ?2, ?3, ?4, ?5)
			 ON CONFLICT (subject_id, note_id) DO NOTHING",
			subject,
			take.note_id,
			listed.at,
			listed.name,
			now
		)
		.execute(&mut *conn)
		.await?
		.rows_affected();
	}
	Ok(recorded)
}

/// Records a `studio` take home has just stored as on Drive, if a list home
/// already holds names it; how many it recorded (0 or 1).
///
/// The take is `subject`'s `note_id`, recorded at `recorded_at`, its bytes'
/// SHA-256 `sha256`. It is matched, as of `now`, against the subject's lists
/// of the takes Drive holds that `mailbox_item` holds and that were made no
/// earlier than [`SKEW`] before it was recorded.
///
/// The earliest such list naming it records it, as [`record`] would have had
/// the list come second. The read is one range over `mailbox_item`'s key
/// `(subject_id, folder, name)`, from the lists made a day before the take
/// was recorded onward, with the body filter in SQL: the query returns only
/// the first list whose bytes contain the take's SHA-256, and only that list
/// is parsed. A list that contains it but does not name it (another schema, or
/// the hash in another field) moves the read past it, to the next that
/// contains it.
///
/// Run it in the transaction that stores the take, so the take and its
/// record land together or not at all.
///
/// # Errors
/// Any `sqlx` failure.
pub async fn match_take(conn: &mut SqliteConnection, subject: &str, note_id: &str, sha256: &str, recorded_at: &str, now: &str) -> Result<u64, sqlx::Error> {
	// `voice_repo::describe` has made `recorded_at` an RFC 3339 instant.
	let Some(from) = DateTime::parse_from_rfc3339(recorded_at)
		.ok()
		.and_then(|at| at.checked_sub_signed(SKEW))
		.map(|at| String::from(DRIVE_TAKES_PREFIX) + &at.to_utc().format("%Y%m%dT%H%M%SZ").to_string())
	else {
		return Ok(0);
	};
	// Every list's name goes on past its stamp (`.json`), so `name > after`
	// takes in a list stamped `from` itself.
	let mut after = from;
	let needle = sha256.as_bytes();
	let (at, name) = loop {
		let Some(list) = sqlx::query!(
			r#"SELECT name AS "name!: String", body AS "body!: Vec<u8>"
			   FROM mailbox_item
			   WHERE subject_id = ?1 AND folder = 'brief' AND name > ?2 AND name < ?3 AND instr(body, ?4) > 0
			   ORDER BY name
			   LIMIT 1"#,
			subject,
			after,
			PAST_DRIVE_TAKES,
			needle
		)
		.fetch_optional(&mut *conn)
		.await?
		else {
			return Ok(0);
		};
		if let Some((at, named)) = listed_in(&list.body) {
			if named.iter().any(|named| named == sha256) {
				break (at, list.name);
			}
		}
		after = list.name;
	};
	Ok(
		sqlx::query!(
			"INSERT INTO studio_take_on_drive (subject_id, note_id, listed_at, list_name, kept_at)
		 VALUES (?1, ?2, ?3, ?4, ?5)
		 ON CONFLICT (subject_id, note_id) DO NOTHING",
			subject,
			note_id,
			at,
			name,
			now
		)
		.execute(&mut *conn)
		.await?
		.rows_affected(),
	)
}

/// Which of `ids`, `subject`'s takes by the phone's ids, home has recorded
/// as on Drive, in the order asked; at most [`MAX_IDS`] are read.
///
/// # Errors
/// Any `sqlx` failure.
pub async fn on_drive(conn: &mut SqliteConnection, subject: &str, ids: &[&str]) -> Result<Vec<OnDrive>, sqlx::Error> {
	let mut found = Vec::new();
	for id in ids.iter().take(MAX_IDS) {
		let row = sqlx::query!(
			r#"SELECT listed_at AS "listed_at!: String" FROM studio_take_on_drive WHERE subject_id = ?1 AND note_id = ?2"#,
			subject,
			id
		)
		.fetch_optional(&mut *conn)
		.await?;
		if let Some(row) = row {
			found.push(OnDrive {
				note: (*id).to_owned(),
				listed_at: row.listed_at,
			});
		}
	}
	Ok(found)
}

#[cfg(test)]
mod tests {
	use super::{on_drive, record, Listed};
	use crate::{describe, put_note, Change, Note, Said, SaidBite};
	use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};

	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");
	const A: &str = "subject-a";
	const B: &str = "subject-b";
	const NOW: &str = "2026-11-01T10:00:00.000Z";
	const AT: &str = "2026-11-01T09:15:00.000Z";
	const LIST: &str = "drive-takes-20261101T091500Z.json";

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

	async fn note(pool: &SqlitePool, subject: &str, id: &str, of: &str, sha256: &str) {
		sqlx::query(
			"INSERT INTO voice_note (subject_id, note_id, of_kind, of_ref, recorded_at, content_type, size, sha256, stored_at)
			 VALUES (?1, ?2, ?3, 'p:s:t', 'then', 'audio/webm', 1, ?4, 'now')",
		)
		.bind(subject)
		.bind(id)
		.bind(of)
		.bind(sha256)
		.execute(pool)
		.await
		.unwrap();
	}

	fn hashes(names: &[&str]) -> Vec<String> {
		names.iter().map(|name| (*name).repeat(64)).collect()
	}

	async fn read(pool: &SqlitePool, subject: &str, ids: &[&str]) -> Vec<String> {
		let mut conn = pool.acquire().await.unwrap();
		on_drive(&mut conn, subject, ids).await.unwrap().into_iter().map(|take| take.note).collect()
	}

	#[tokio::test]
	async fn only_the_subjects_own_studio_takes_a_list_names_are_recorded_once() {
		let pool = pool().await;
		note(&pool, A, "t1", "studio", &"a".repeat(64)).await;
		note(&pool, A, "t2", "studio", &"b".repeat(64)).await;
		// The same bytes said for another kind, and another subject's take.
		note(&pool, A, "r1", "reflection", &"c".repeat(64)).await;
		note(&pool, B, "t9", "studio", &"a".repeat(64)).await;
		let sha256 = hashes(&["a", "c", "d"]);
		let mut conn = pool.acquire().await.unwrap();
		let listed = Listed {
			at: AT,
			name: LIST,
			sha256: &sha256,
		};
		assert_eq!(record(&mut conn, A, listed, NOW).await.unwrap(), 1);
		// A later list naming it again changes nothing.
		let later = Listed {
			at: "2026-11-02T09:00:00.000Z",
			name: "drive-takes-20261102T090000Z.json",
			sha256: &sha256,
		};
		assert_eq!(record(&mut conn, A, later, NOW).await.unwrap(), 0);
		drop(conn);
		assert_eq!(read(&pool, A, &["t2", "t1", "r1", "nope"]).await, vec!["t1"]);
		assert_eq!(read(&pool, B, &["t9", "t1"]).await, Vec::<String>::new());
		let mut conn = pool.acquire().await.unwrap();
		let first = on_drive(&mut conn, A, &["t1"]).await.unwrap();
		assert_eq!(first[0].listed_at, AT);
		// Nothing a list names that home does not hold is kept.
		let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM studio_take_on_drive").fetch_one(&mut *conn).await.unwrap();
		assert_eq!(rows, 1);
	}

	#[tokio::test]
	async fn no_row_is_ever_updated_and_rows_leave_with_the_account() {
		let pool = pool().await;
		note(&pool, A, "t1", "studio", &"a".repeat(64)).await;
		let sha256 = hashes(&["a"]);
		let mut conn = pool.acquire().await.unwrap();
		record(
			&mut conn,
			A,
			Listed {
				at: AT,
				name: LIST,
				sha256: &sha256,
			},
			NOW,
		)
		.await
		.unwrap();
		let updated = sqlx::query("UPDATE studio_take_on_drive SET listed_at = 'later'").execute(&mut *conn).await;
		assert!(updated.is_err());
		sqlx::query("DELETE FROM account WHERE subject_id = ?1").bind(A).execute(&mut *conn).await.unwrap();
		let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM studio_take_on_drive").fetch_one(&mut *conn).await.unwrap();
		assert_eq!(rows, 0);
	}

	#[tokio::test]
	async fn an_empty_list_records_nothing_and_a_read_stops_at_its_cap() {
		let pool = pool().await;
		note(&pool, A, "t1", "studio", &"a".repeat(64)).await;
		let mut conn = pool.acquire().await.unwrap();
		assert_eq!(record(&mut conn, A, Listed { at: AT, name: LIST, sha256: &[] }, NOW).await.unwrap(), 0);
		let sha256 = hashes(&["a"]);
		record(
			&mut conn,
			A,
			Listed {
				at: AT,
				name: LIST,
				sha256: &sha256,
			},
			NOW,
		)
		.await
		.unwrap();
		drop(conn);
		let mut ids: Vec<&str> = vec!["x"; super::MAX_IDS];
		ids.push("t1");
		assert_eq!(read(&pool, A, &ids).await, Vec::<String>::new());
	}

	/// When the phone recorded the takes the arrival tests send home.
	const RECORDED: &str = "2026-11-01T08:00:00.000Z";

	/// `subject`'s take `id` as the phone describes it, said for `of`, its
	/// audio `body`.
	fn take(id: &str, of: &str, body: &[u8]) -> Note {
		let said = Said {
			of: Some(of),
			of_ref: Some(if of == "studio" { "build-journal-ep04:s3:2" } else { "session-7f2a" }),
			at: Some(RECORDED),
			content_type: Some("audio/mp4"),
			bite: SaidBite::default(),
		};
		describe(id, said, body).unwrap()
	}

	/// Sends `note` home for `subject`, as `PUT /voice-notes/:id` does.
	async fn send(pool: &SqlitePool, subject: &str, note: &Note, body: &[u8]) -> Change {
		let mut tx = pool.begin().await.unwrap();
		let change = put_note(&mut tx, subject, note, body, NOW).await.unwrap();
		tx.commit().await.unwrap();
		change
	}

	/// The mailbox name of a list made at `stamp` (`YYYYMMDDTHHMMSSZ`).
	fn list_name(stamp: &str) -> String {
		String::from(super::DRIVE_TAKES_PREFIX) + stamp + ".json"
	}

	/// The bytes of a list made at `at` naming `sha256`.
	fn list(at: &str, sha256: &[&str]) -> Vec<u8> {
		let takes: Vec<serde_json::Value> = sha256.iter().map(|sha256| serde_json::json!({ "sha256": sha256 })).collect();
		serde_json::json!({ "schema": super::DRIVE_TAKES_SCHEMA, "at": at, "takes": takes })
			.to_string()
			.into_bytes()
	}

	/// Keeps `body` in `subject`'s `brief` as the list stamped `stamp`, as the
	/// poller keeps one.
	async fn hold_list(pool: &SqlitePool, subject: &str, stamp: &str, body: &[u8]) {
		sqlx::query(
			"INSERT INTO mailbox_item (subject_id, folder, name, seq, sha256, size, body, pulled_at)
			 VALUES (?1, 'brief', ?2, 1, ?3, ?4, ?5, ?6)",
		)
		.bind(subject)
		.bind(list_name(stamp))
		.bind("f".repeat(64))
		.bind(i64::try_from(body.len()).unwrap())
		.bind(body)
		.bind(NOW)
		.execute(pool)
		.await
		.unwrap();
	}

	async fn recorded(pool: &SqlitePool, subject: &str, id: &str) -> Option<(String, String)> {
		sqlx::query_as("SELECT listed_at, list_name FROM studio_take_on_drive WHERE subject_id = ?1 AND note_id = ?2")
			.bind(subject)
			.bind(id)
			.fetch_optional(pool)
			.await
			.unwrap()
	}

	async fn rows(pool: &SqlitePool) -> i64 {
		sqlx::query_scalar("SELECT COUNT(*) FROM studio_take_on_drive").fetch_one(pool).await.unwrap()
	}

	#[tokio::test]
	async fn a_take_that_reaches_home_after_its_list_is_recorded_when_it_arrives() {
		let pool = pool().await;
		let t1 = take("t1", "studio", b"take one");
		let t2 = take("t2", "studio", b"take two");
		// A list naming t2 in another subject's mailbox only: never read for A,
		// so t2 reaches A with no list naming it, and records nothing.
		hold_list(&pool, B, "20261101T093000Z", &list("2026-11-01T09:30:00.000Z", &[&t2.sha256])).await;
		assert_eq!(send(&pool, A, &t2, b"take two").await, Change::Added);
		assert_eq!(recorded(&pool, A, "t2").await, None);
		assert_eq!(rows(&pool).await, 0);
		// A's own list names t1; the same bytes said for a reflection are no match.
		let r1 = take("r1", "reflection", b"take one");
		hold_list(&pool, A, "20261101T091500Z", &list(AT, &[&t1.sha256, &"d".repeat(64)])).await;
		assert_eq!(send(&pool, A, &t1, b"take one").await, Change::Added);
		assert_eq!(send(&pool, A, &r1, b"take one").await, Change::Added);
		assert_eq!(recorded(&pool, A, "t1").await, Some((AT.to_owned(), LIST.to_owned())));
		assert_eq!(read(&pool, A, &["t1", "r1", "t2"]).await, vec!["t1"]);
		assert_eq!(rows(&pool).await, 1);
	}

	#[tokio::test]
	async fn a_take_named_by_many_lists_is_recorded_once_by_the_first() {
		let pool = pool().await;
		let t1 = take("t1", "studio", b"take one");
		let later = "2026-11-02T09:00:00.000Z";
		hold_list(&pool, A, "20261102T090000Z", &list(later, &[&t1.sha256])).await;
		hold_list(&pool, A, "20261101T091500Z", &list(AT, &[&t1.sha256])).await;
		assert_eq!(send(&pool, A, &t1, b"take one").await, Change::Added);
		assert_eq!(recorded(&pool, A, "t1").await, Some((AT.to_owned(), LIST.to_owned())));
		// Sent again, it is the same note, and nothing more is recorded.
		assert_eq!(send(&pool, A, &t1, b"take one").await, Change::Unchanged);
		// A list arriving later that names it again changes nothing.
		let sha256 = vec![t1.sha256.clone()];
		let mut conn = pool.acquire().await.unwrap();
		let again = Listed {
			at: "2026-11-03T09:00:00.000Z",
			name: "drive-takes-20261103T090000Z.json",
			sha256: &sha256,
		};
		assert_eq!(record(&mut conn, A, again, NOW).await.unwrap(), 0);
		drop(conn);
		assert_eq!(recorded(&pool, A, "t1").await, Some((AT.to_owned(), LIST.to_owned())));
		assert_eq!(rows(&pool).await, 1);
	}

	#[tokio::test]
	async fn a_list_made_more_than_a_day_before_the_take_was_recorded_is_not_read() {
		let pool = pool().await;
		let t1 = take("t1", "studio", b"take one");
		let t2 = take("t2", "studio", b"take two");
		let t3 = take("t3", "studio", b"take three");
		// A day and a second before the takes were recorded: past the bound.
		hold_list(&pool, A, "20261031T075959Z", &list("2026-10-31T07:59:59.000Z", &[&t1.sha256])).await;
		// Twelve hours before: a phone's clock may be that far off, so it is read.
		hold_list(&pool, A, "20261031T200000Z", &list("2026-10-31T20:00:00.000Z", &[&t2.sha256])).await;
		// Not a list of Drive's takes, though it contains t1's and t3's hashes
		// and sorts in range: read past, not matched, to the list after it.
		let other = serde_json::json!({ "schema": "other", "at": "x", "takes": [{ "sha256": t1.sha256 }, { "sha256": t3.sha256 }] });
		hold_list(&pool, A, "20261101T100000Z", other.to_string().as_bytes()).await;
		hold_list(&pool, A, "20261101T110000Z", &list("2026-11-01T11:00:00.000Z", &[&t3.sha256])).await;
		send(&pool, A, &t1, b"take one").await;
		send(&pool, A, &t2, b"take two").await;
		send(&pool, A, &t3, b"take three").await;
		assert_eq!(recorded(&pool, A, "t1").await, None);
		assert_eq!(recorded(&pool, A, "t2").await, Some(("2026-10-31T20:00:00.000Z".to_owned(), list_name("20261031T200000Z"))));
		assert_eq!(recorded(&pool, A, "t3").await, Some(("2026-11-01T11:00:00.000Z".to_owned(), list_name("20261101T110000Z"))));
	}
}

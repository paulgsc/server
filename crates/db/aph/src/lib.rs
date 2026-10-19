//! The owner's aph record (paulgsc/some-ui `packages/ui/aph`): entries of a
//! figure of mine against a figure the other side reports, at fixed
//! checkpoints of the day, and the settings they are read against.
//!
//! `20261017000100_create_aph_record.up.sql` records the design; in short:
//!
//! - **Append-only.** [`put_entry`] and [`put_settings`] add a revision, or
//!   nothing when it would equal the one standing; no row is ever updated (a
//!   trigger aborts an `UPDATE`), and nothing here deletes. Account deletion
//!   is the one delete: its generic sweep of `SUBJECT_SCOPED_TABLES`, and the
//!   migration's triggers on `account`.
//! - **The phone's rules, held here too.** A write passes the same checks the
//!   phone's store makes (`violations` and `breaches` in its model.ts), so
//!   what is stored is a record the phone can open: one entry's own rules
//!   ([`parse_entry`]), what a revision may not change ([`breaches`]), at most
//!   one plain entry per checkpoint a day, and settings that keep every
//!   checkpoint an entry stands at.
//! - **Per subject, and only per subject.** Every query takes the subject and
//!   filters on it; nothing reads across subjects.
//! - **No background writer.** The callers are `file_host`'s `handlers::aph`
//!   (`PUT /api/v1/aph/entries/:id`, `PUT /api/v1/aph/settings`), which the
//!   phone sends once its person has opted in. A
//!   `git grep -n -e aph_entry -e aph_settings -- '*.rs'` shows no other writer.

mod model;

pub use model::{breaches, parse_entry, parse_settings, Checkpoint, Entry, Mine, Problem, Review, Settings, Theirs, ENTRY_BODY_CEILING, SETTINGS_BODY_CEILING};
use serde::Serialize;
use sqlx::SqliteConnection;

/// What a put did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
	/// The first revision: the record did not hold it.
	Kept,
	/// A new revision over a different standing one.
	Revised,
	/// The standing revision already equals it: nothing written.
	Unchanged,
}

impl Change {
	/// The wire name.
	#[must_use]
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::Kept => "kept",
			Self::Revised => "revised",
			Self::Unchanged => "unchanged",
		}
	}
}

/// Why a put wrote nothing.
#[derive(Debug)]
pub enum AphError {
	/// The write breaks a rule; every problem, by field.
	Invalid(Vec<Problem>),
	/// A stored body this crate wrote does not read back.
	Unreadable,
	Storage(sqlx::Error),
}

impl std::fmt::Display for AphError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::Invalid(problems) => {
				f.write_str("not a write the aph record takes:")?;
				for (field, why) in problems {
					write!(f, " {field} {why};")?;
				}
				Ok(())
			}
			Self::Unreadable => f.write_str("a stored aph revision does not read back"),
			Self::Storage(err) => write!(f, "database error: {err}"),
		}
	}
}

impl std::error::Error for AphError {}

impl From<sqlx::Error> for AphError {
	fn from(err: sqlx::Error) -> Self {
		Self::Storage(err)
	}
}

/// The record as it stands: the settings, and every entry's standing revision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Record {
	/// `None` until the phone first sends them.
	pub settings: Option<Settings>,
	/// By day, then id.
	pub entries: Vec<Entry>,
}

fn body_of<T: Serialize>(value: &T) -> Result<String, AphError> {
	serde_json::to_value(value).map(|json| json.to_string()).map_err(|_| AphError::Unreadable)
}

fn read_body<T: for<'de> serde::Deserialize<'de>>(body: &str) -> Result<T, AphError> {
	serde_json::from_str(body).map_err(|_| AphError::Unreadable)
}

/// The settings' standing revision and its number.
async fn standing_settings(conn: &mut SqliteConnection, subject: &str) -> Result<Option<(i64, Settings)>, AphError> {
	let row = sqlx::query!("SELECT revision, body FROM aph_settings WHERE subject_id = ?1 ORDER BY revision DESC LIMIT 1", subject)
		.fetch_optional(&mut *conn)
		.await?;
	row.map(|row| Ok((row.revision, read_body(&row.body)?))).transpose()
}

/// `id`'s standing revision and its number.
async fn standing_entry(conn: &mut SqliteConnection, subject: &str, id: &str) -> Result<Option<(i64, Entry)>, AphError> {
	let row = sqlx::query!(
		"SELECT revision, body FROM aph_entry WHERE subject_id = ?1 AND entry_id = ?2 ORDER BY revision DESC LIMIT 1",
		subject,
		id
	)
	.fetch_optional(&mut *conn)
	.await?;
	row.map(|row| Ok((row.revision, read_body(&row.body)?))).transpose()
}

/// Whether an entry other than `id` stands plain at `day`'s `checkpoint`.
async fn slot_taken(conn: &mut SqliteConnection, subject: &str, day: &str, checkpoint: &str, id: &str) -> Result<bool, AphError> {
	let taken = sqlx::query_scalar!(
		r#"
		SELECT COUNT(*) AS "count!: i64" FROM aph_entry e
		WHERE e.subject_id = ?1 AND e.day = ?2 AND e.checkpoint = ?3 AND e.entry_id <> ?4 AND e.plain = 1
		  AND e.revision = (SELECT MAX(r.revision) FROM aph_entry r WHERE r.subject_id = e.subject_id AND r.entry_id = e.entry_id)
		"#,
		subject,
		day,
		checkpoint,
		id
	)
	.fetch_one(&mut *conn)
	.await?;
	Ok(taken > 0)
}

/// Writes `body` as entry `id`'s next revision for `subject`, as of `now`,
/// and answers what it did with the entry as it now stands.
///
/// Run it in a `BEGIN IMMEDIATE` transaction, so the reads that decide (the
/// standing revision, the settings, the plain slot) and the write see one
/// snapshot.
///
/// # Errors
/// [`AphError::Invalid`] naming every problem: the entry's own
/// ([`parse_entry`]), a change [`breaches`] refuses, or a second plain entry
/// at its checkpoint that day. [`AphError::Storage`] for any `sqlx` failure.
pub async fn put_entry(conn: &mut SqliteConnection, subject: &str, id: &str, body: &[u8], now: &str) -> Result<(Change, Entry), AphError> {
	let settings = standing_settings(conn, subject).await?.map(|(_, settings)| settings);
	let entry = parse_entry(id, body, settings.as_ref()).map_err(AphError::Invalid)?;
	let standing = standing_entry(conn, subject, id).await?;
	let mut problems = Vec::new();
	if let Some((_, was)) = &standing {
		if *was == entry {
			return Ok((Change::Unchanged, entry));
		}
		problems.extend(breaches(was, &entry));
	}
	if let (Some(checkpoint), true) = (&entry.checkpoint, entry.is_plain()) {
		if slot_taken(conn, subject, &entry.day, checkpoint, id).await? {
			problems.push(("labels", "another entry already stands plain at this checkpoint that day: label one of them"));
		}
	}
	if !problems.is_empty() {
		return Err(AphError::Invalid(problems));
	}
	let revision = standing.as_ref().map_or(1, |(revision, _)| revision + 1);
	let text = body_of(&entry)?;
	let plain = i64::from(entry.is_plain());
	sqlx::query!(
		"INSERT INTO aph_entry (subject_id, entry_id, revision, day, checkpoint, plain, body, written_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
		subject,
		id,
		revision,
		entry.day,
		entry.checkpoint,
		plain,
		text,
		now
	)
	.execute(&mut *conn)
	.await?;
	Ok((if standing.is_some() { Change::Revised } else { Change::Kept }, entry))
}

/// Writes `body` as `subject`'s next settings revision, as of `now`.
///
/// Run it in a `BEGIN IMMEDIATE` transaction, as [`put_entry`].
///
/// # Errors
/// [`AphError::Invalid`] for settings [`parse_settings`] refuses, or that
/// drop a checkpoint a standing entry is logged at; [`AphError::Storage`] for
/// any `sqlx` failure.
pub async fn put_settings(conn: &mut SqliteConnection, subject: &str, body: &[u8], now: &str) -> Result<(Change, Settings), AphError> {
	let settings = parse_settings(body).map_err(AphError::Invalid)?;
	let standing = standing_settings(conn, subject).await?;
	if let Some((_, was)) = &standing {
		if *was == settings {
			return Ok((Change::Unchanged, settings));
		}
	}
	let used = sqlx::query_scalar!(
		r#"SELECT DISTINCT checkpoint AS "checkpoint!" FROM aph_entry WHERE subject_id = ?1 AND checkpoint IS NOT NULL"#,
		subject
	)
	.fetch_all(&mut *conn)
	.await?;
	if used.iter().any(|checkpoint| !settings.has_checkpoint(checkpoint)) {
		return Err(AphError::Invalid(vec![("checkpoints", "drops a checkpoint entries are logged at")]));
	}
	let revision = standing.as_ref().map_or(1, |(revision, _)| revision + 1);
	let text = body_of(&settings)?;
	sqlx::query!(
		"INSERT INTO aph_settings (subject_id, revision, body, written_at) VALUES (?1, ?2, ?3, ?4)",
		subject,
		revision,
		text,
		now
	)
	.execute(&mut *conn)
	.await?;
	Ok((if standing.is_some() { Change::Revised } else { Change::Kept }, settings))
}

/// `subject`'s record as it stands.
///
/// Run it in a transaction, so the settings and the entries are read from one
/// snapshot.
///
/// Unbounded on purpose: it is one person's log, a few entries a day, and
/// the phone reads it whole to show History and the trend.
///
/// # Errors
/// [`AphError::Unreadable`] for a stored body that does not read back, and
/// [`AphError::Storage`] for any `sqlx` failure.
pub async fn record(conn: &mut SqliteConnection, subject: &str) -> Result<Record, AphError> {
	let settings = standing_settings(conn, subject).await?.map(|(_, settings)| settings);
	let bodies = sqlx::query_scalar!(
		r#"
		SELECT e.body FROM aph_entry e
		WHERE e.subject_id = ?1
		  AND e.revision = (SELECT MAX(r.revision) FROM aph_entry r WHERE r.subject_id = e.subject_id AND r.entry_id = e.entry_id)
		ORDER BY e.day, e.entry_id
		"#,
		subject
	)
	.fetch_all(&mut *conn)
	.await?;
	let entries = bodies.iter().map(|body| read_body(body)).collect::<Result<Vec<Entry>, AphError>>()?;
	Ok(Record { settings, entries })
}

/// Where a read of the entries left off: the last revision's `written_at`,
/// entry id and revision. The default comes before every revision.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Mark {
	pub written_at: String,
	pub entry_id: String,
	pub revision: i64,
}

/// Up to `limit` of `subject`'s entry revisions stored after `after`.
///
/// Each is as it was written, in the order they reached home, with the mark
/// to read on from. A later revision of an entry stands over an earlier one.
///
/// # Errors
/// [`AphError::Unreadable`] for a stored body that does not read back, and
/// [`AphError::Storage`] for any `sqlx` failure.
pub async fn since(conn: &mut SqliteConnection, subject: &str, after: &Mark, limit: i64) -> Result<(Vec<Entry>, Mark), AphError> {
	let rows = sqlx::query!(
		r#"
		SELECT entry_id, revision, written_at, body FROM aph_entry
		WHERE subject_id = ?1
		  AND (written_at > ?2 OR (written_at = ?2 AND (entry_id > ?3 OR (entry_id = ?3 AND revision > ?4))))
		ORDER BY written_at, entry_id, revision
		LIMIT ?5
		"#,
		subject,
		after.written_at,
		after.entry_id,
		after.revision,
		limit
	)
	.fetch_all(&mut *conn)
	.await?;
	let next = rows.last().map_or_else(
		|| after.clone(),
		|row| Mark {
			written_at: row.written_at.clone(),
			entry_id: row.entry_id.clone(),
			revision: row.revision,
		},
	);
	let entries = rows.iter().map(|row| read_body(&row.body)).collect::<Result<Vec<Entry>, AphError>>()?;
	Ok((entries, next))
}

#[cfg(test)]
mod tests {
	use super::{put_entry, put_settings, record, since, AphError, Change, Mark};
	use serde_json::{json, Value};
	use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};

	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");
	const PREVIOUS_MIGRATION: i64 = 20_260_929_000_300;
	const T0: &str = "2026-10-17T07:00:00.000Z";
	const T1: &str = "2026-10-17T12:00:00.000Z";
	const A: &str = "subject-a";
	const B: &str = "subject-b";

	async fn pool() -> SqlitePool {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();
		pool
	}

	fn settings() -> Value {
		json!({
			"checkpoints": [
				{ "id": "7", "label": "7:00", "minutes": 420, "goal": 4100 },
				{ "id": "12", "label": "12:00", "minutes": 720, "goal": 5100 }
			],
			"labels": ["no-bs"], "step": 100, "usualLow": 3000, "usualHigh": 6500, "tolerance": 100,
			"since": "2026-09-14"
		})
	}

	fn entry(id: &str) -> Value {
		json!({
			"id": id, "day": "2026-10-17", "checkpoint": "7", "time": null,
			"mine": { "value": 4300, "approx": true }, "theirs": null, "goal": 4100,
			"labels": [], "note": null, "review": null
		})
	}

	async fn put(pool: &SqlitePool, subject: &str, value: &Value, now: &str) -> Result<Change, AphError> {
		let id = value["id"].as_str().unwrap();
		put_entry(&mut pool.acquire().await.unwrap(), subject, id, value.to_string().as_bytes(), now)
			.await
			.map(|(change, _)| change)
	}

	async fn set(pool: &SqlitePool, subject: &str, value: &Value) -> Result<Change, AphError> {
		put_settings(&mut pool.acquire().await.unwrap(), subject, value.to_string().as_bytes(), T0)
			.await
			.map(|(change, _)| change)
	}

	fn refused(result: Result<Change, AphError>) -> Vec<&'static str> {
		match result {
			Err(AphError::Invalid(problems)) => {
				let mut fields: Vec<&str> = problems.iter().map(|(field, _)| *field).collect();
				fields.sort_unstable();
				fields
			}
			other => panic!("not refused: {other:?}"),
		}
	}

	async fn rows(pool: &SqlitePool, table: &str) -> i64 {
		sqlx::query_scalar(&(String::from("SELECT COUNT(*) FROM ") + table)).fetch_one(pool).await.unwrap()
	}

	#[tokio::test]
	async fn the_migration_round_trips() {
		let pool = pool().await;
		let tables = || sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name IN ('aph_entry', 'aph_settings')");
		assert_eq!(tables().fetch_one(&pool).await.unwrap(), 2);
		MIGRATOR.undo(&pool, PREVIOUS_MIGRATION).await.unwrap();
		assert_eq!(tables().fetch_one(&pool).await.unwrap(), 0, "down drops them");
		MIGRATOR.run(&pool).await.unwrap();
		assert_eq!(tables().fetch_one(&pool).await.unwrap(), 2, "up recreates them");
	}

	/// The columns the migration describes and no others: a new one is a
	/// decision about what the server learns from the record.
	#[tokio::test]
	async fn the_tables_hold_exactly_their_described_columns() {
		let pool = pool().await;
		for (table, wanted) in [
			("aph_entry", &["subject_id", "entry_id", "revision", "day", "checkpoint", "plain", "body", "written_at"][..]),
			("aph_settings", &["subject_id", "revision", "body", "written_at"]),
		] {
			let columns: Vec<String> = sqlx::query_scalar("SELECT name FROM pragma_table_info(?1) ORDER BY cid")
				.bind(table)
				.fetch_all(&pool)
				.await
				.unwrap();
			assert_eq!(columns, wanted, "{table}");
		}
	}

	#[tokio::test]
	async fn every_change_is_a_new_revision_and_every_old_one_stays() {
		let pool = pool().await;
		assert_eq!(set(&pool, A, &settings()).await.unwrap(), Change::Kept);
		assert_eq!(set(&pool, A, &settings()).await.unwrap(), Change::Unchanged);
		let mut first = entry("e1");
		assert_eq!(put(&pool, A, &first, T0).await.unwrap(), Change::Kept);
		assert_eq!(put(&pool, A, &first, T1).await.unwrap(), Change::Unchanged, "the same entry writes nothing");
		first["theirs"] = json!({ "value": 4250 });
		assert_eq!(put(&pool, A, &first, T1).await.unwrap(), Change::Revised);
		first["note"] = json!("their app lagged");
		assert_eq!(put(&pool, A, &first, T1).await.unwrap(), Change::Revised);
		assert_eq!(rows(&pool, "aph_entry").await, 3, "every revision kept");

		let held = record(&mut pool.acquire().await.unwrap(), A).await.unwrap();
		assert_eq!(serde_json::to_value(&held.entries).unwrap(), json!([first]), "the standing revision");
		assert_eq!(serde_json::to_value(&held.settings).unwrap(), settings());
		let none = record(&mut pool.acquire().await.unwrap(), B).await.unwrap();
		assert_eq!((none.settings, none.entries.len()), (None, 0), "another subject's record is empty");

		let update = sqlx::query("UPDATE aph_entry SET body = '{}'").execute(&pool).await;
		assert!(update.unwrap_err().to_string().contains("append-only"), "no row is ever updated");
		let update = sqlx::query("UPDATE aph_settings SET body = '{}'").execute(&pool).await;
		assert!(update.unwrap_err().to_string().contains("append-only"));
	}

	#[tokio::test]
	async fn a_read_from_a_mark_takes_each_revision_once_in_the_order_it_reached_home() {
		let pool = pool().await;
		set(&pool, A, &settings()).await.unwrap();
		let mut first = entry("e1");
		put(&pool, A, &first, T0).await.unwrap();
		first["note"] = json!("again");
		put(&pool, A, &first, T1).await.unwrap();
		let mut other_day = entry("e0");
		other_day["day"] = json!("2026-10-16");
		put(&pool, A, &other_day, T1).await.unwrap();

		let conn = &mut pool.acquire().await.unwrap();
		assert!(since(conn, B, &Mark::default(), 9).await.unwrap().0.is_empty(), "another subject's are its own");
		let (head, mark) = since(conn, A, &Mark::default(), 2).await.unwrap();
		let ids = |entries: &[super::Entry]| {
			serde_json::to_value(entries)
				.unwrap()
				.as_array()
				.unwrap()
				.iter()
				.map(|e| e["id"].clone())
				.collect::<Vec<_>>()
		};
		assert_eq!(ids(&head), [json!("e1"), json!("e0")], "by when it reached home, then id");
		let (tail, end) = since(conn, A, &mark, 2).await.unwrap();
		assert_eq!(serde_json::to_value(&tail).unwrap(), json!([first]), "the later revision, once");
		let (none, same) = since(conn, A, &end, 2).await.unwrap();
		assert!(none.is_empty());
		assert_eq!(same, end);
	}

	#[tokio::test]
	async fn a_revision_that_rewrites_the_record_is_refused_and_writes_nothing() {
		let pool = pool().await;
		set(&pool, A, &settings()).await.unwrap();
		let mut logged = entry("e1");
		logged["theirs"] = json!({ "value": 4250 });
		put(&pool, A, &logged, T0).await.unwrap();

		let mut moved = logged.clone();
		moved["day"] = json!("2026-10-16");
		moved["goal"] = json!(4000);
		moved["theirs"] = json!(null);
		assert_eq!(refused(put(&pool, A, &moved, T1).await), ["day", "goal", "theirs"]);

		let mut second = entry("e2");
		assert_eq!(refused(put(&pool, A, &second, T1).await), ["labels"], "a second plain figure at 7:00 that day");
		second["labels"] = json!(["no-bs"]);
		assert_eq!(
			put(&pool, A, &second, T1).await.unwrap(),
			Change::Kept,
			"a labelled one compares, so it may stand beside it"
		);
		second["labels"] = json!([]);
		assert_eq!(
			refused(put(&pool, A, &second, T1).await),
			["labels"],
			"and may not drop its label while the other stands plain"
		);
		assert_eq!(
			put(&pool, B, &entry("e2"), T1).await.map_err(|err| err.to_string()).unwrap_err(),
			"not a write the aph record takes: checkpoint is not one of the settings' checkpoints;",
			"B has no settings: A's are not B's"
		);

		let mut fewer = settings();
		fewer["checkpoints"] = json!([{ "id": "12", "label": "12:00", "minutes": 720, "goal": 5100 }]);
		assert_eq!(refused(set(&pool, A, &fewer).await), ["checkpoints"], "7:00 holds entries");
		assert_eq!((rows(&pool, "aph_entry").await, rows(&pool, "aph_settings").await), (2, 1), "nothing refused was written");
	}

	#[tokio::test]
	async fn the_record_leaves_with_the_account() {
		let pool = pool().await;
		for subject in [A, B] {
			sqlx::query("INSERT INTO account (subject_id, user_handle) VALUES (?1, ?2)")
				.bind(subject)
				.bind(subject.as_bytes())
				.execute(&pool)
				.await
				.unwrap();
			set(&pool, subject, &settings()).await.unwrap();
			put(&pool, subject, &entry("e1"), T0).await.unwrap();
		}
		sqlx::query("DELETE FROM account WHERE subject_id = ?1").bind(A).execute(&pool).await.unwrap();
		let left: Vec<String> = sqlx::query_scalar("SELECT subject_id FROM aph_entry UNION ALL SELECT subject_id FROM aph_settings")
			.fetch_all(&pool)
			.await
			.unwrap();
		assert_eq!(left, [B, B], "A's record went with A's account, B's stayed");
	}
}

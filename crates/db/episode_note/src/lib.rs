//! The owner's notes on the feeds' episodes (paulgsc/some-ui#2745, SFEED6):
//! each note the phone's Feedback page left at a moment of a weekly call or
//! Playoff Delta episode, sent home by its drain.
//!
//! `20261108075000_create_episode_note.up.sql` records the design; in short:
//!
//! - **Append-only.** [`put_note`] stores a note under its id, or nothing
//!   when that id already holds the same one; a different note under a held
//!   id is refused ([`NoteError::Rewritten`]), never stored. No row is ever
//!   updated (a trigger aborts an `UPDATE`), and nothing here deletes: there
//!   is no delete and no switch (the owner's answer on
//!   paulgsc/some-ui#2749). Account deletion is the one delete: its generic
//!   sweep of `SUBJECT_SCOPED_TABLES`, and the migration's trigger on
//!   `account`.
//! - **The phone's rules, held here too.** A write passes [`parse`]: the
//!   categories are [`CATEGORIES`]', each once, in its order, at least one,
//!   as the phone's `NoteSchema` takes them; the migration's trigger holds
//!   the table to the same.
//! - **A spoken note names its take by id only.** `take` is a voice note's
//!   id (a library take); what home heard in it goes to the routine as that
//!   take's transcript, never through here.
//! - **Per subject, and only per subject.** Every query takes the subject and
//!   filters on it; nothing reads across subjects.
//! - **No background writer.** The one caller is `file_host`'s
//!   `handlers::episode_notes` (`PUT /api/v1/feeds/:feed/notes/:id`). A
//!   `git grep -n episode_note -- '*.rs'` shows no other writer.

use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use sqlx::SqliteConnection;

/// A refused field and why, as `file_host` answers it in a `422`.
pub type Problem = (&'static str, &'static str);

/// The largest body a write takes: far above any note.
pub const BODY_CEILING: usize = 16 * 1024;

/// The fixed set of categories, in its order (paulgsc/some-ui
/// `episode-feedback/model.ts`, `CATEGORIES`: what it said, then how it
/// looked or sounded). The migration's trigger names the same list.
pub const CATEGORIES: [&str; 14] = [
	"shallow",
	"technical",
	"missedIntuition",
	"unsupported",
	"tooLong",
	"tooShort",
	"textClipped",
	"slideShakes",
	"overlap",
	"hardToRead",
	"pacing",
	"voice",
	"musicLoud",
	"subtitlesOff",
];

/// The feeds a note can be about (`file_host::feeds::Feed`).
pub const FEEDS: [&str; 2] = ["call", "playoff-delta"];

/// The most characters of the phone's id for a note.
const MAX_ID_CHARS: usize = 64;

/// The most characters of an episode's key.
const MAX_EPISODE_CHARS: usize = 64;

/// The most characters of a take's id: a voice note's.
const MAX_TAKE_CHARS: usize = 64;

/// What a put did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
	/// Home did not hold it: it is stored now.
	Added,
	/// Home already holds this very note: nothing written.
	Unchanged,
}

impl Change {
	/// The wire name.
	#[must_use]
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::Added => "added",
			Self::Unchanged => "unchanged",
		}
	}
}

/// Why a put wrote nothing.
#[derive(Debug)]
pub enum NoteError {
	/// The write breaks a rule; every problem, by field.
	Invalid(Vec<Problem>),
	/// The id already holds a different note, which is never replaced.
	Rewritten,
	Storage(sqlx::Error),
}

impl std::fmt::Display for NoteError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::Invalid(problems) => {
				f.write_str("not an episode note home takes:")?;
				for (field, why) in problems {
					write!(f, " {field} {why};")?;
				}
				Ok(())
			}
			Self::Rewritten => f.write_str(REWRITTEN),
			Self::Storage(err) => write!(f, "database error: {err}"),
		}
	}
}

impl std::error::Error for NoteError {}

impl From<sqlx::Error> for NoteError {
	fn from(err: sqlx::Error) -> Self {
		Self::Storage(err)
	}
}

/// What [`NoteError::Rewritten`] says.
pub const REWRITTEN: &str = "this note id already holds a different note: the record is append-only";

/// A note, in the phone's own shape (paulgsc/some-ui
/// `episode-feedback/model.ts`, `Note`), with the take said for it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Note {
	/// The phone's id for it: what makes a retried send store nothing.
	pub id: String,
	/// `call` or `playoff-delta`.
	pub feed: String,
	/// The episode's key, as home lists it.
	pub episode: String,
	/// The moment in the episode, in milliseconds from its start.
	pub at_ms: i64,
	/// The categories picked, in [`CATEGORIES`]' order.
	pub categories: Vec<String>,
	/// The voice note id of the library take said for it, if any. Left out
	/// when there is none, as the phone sends it.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub take: Option<String>,
	/// When it was written, as ISO-8601 UTC to the millisecond.
	pub written_at: String,
}

/// A note as home holds it: `GET /feeds/:feed/notes`'s shape.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Held {
	pub id: String,
	pub episode: String,
	pub at_ms: i64,
	pub categories: Vec<String>,
	/// The take's voice note id, or `null`.
	pub take: Option<String>,
	pub written_at: String,
	/// When it reached home.
	pub stored_at: String,
}

/// What the phone sends, before it is checked.
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Wire {
	id: String,
	feed: String,
	episode: String,
	at_ms: serde_json::Value,
	categories: Vec<String>,
	#[serde(default)]
	take: Option<String>,
	written_at: String,
}

/// The phone's own ids: URL-safe, short.
fn is_id(text: &str) -> bool {
	!text.is_empty() && text.len() <= MAX_ID_CHARS && text.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

/// An episode's key: one plain path segment (`2026-10-30`, `2026-SF-08`).
fn is_episode(text: &str) -> bool {
	!text.is_empty() && text.len() <= MAX_EPISODE_CHARS && text.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

/// A voice note's id, as `voice_repo::is_note_id` reads one and the
/// mailbox's transcript names allow it.
fn is_take(text: &str) -> bool {
	!text.is_empty() && text.len() <= MAX_TAKE_CHARS && text.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
}

/// Whether `picked` is the fixed set's names, each once, in its order, at
/// least one.
fn in_set_order(picked: &[String]) -> bool {
	let mut from = 0;
	for name in picked {
		match CATEGORIES[from..].iter().position(|category| category == name) {
			Some(at) => from += at + 1,
			None => return false,
		}
	}
	!picked.is_empty()
}

/// The latest moment a note can be at: a day into an episode, far past any
/// the feeds render, and well inside the integers the mailbox's JSON reads
/// exactly (`Number.MAX_SAFE_INTEGER`).
pub const MAX_AT_MS: i64 = 24 * 60 * 60 * 1000;

/// `at` as ISO-8601 UTC to the millisecond: what is stored, served and
/// filed. A finer time is cut to the millisecond, as the phone's
/// `toISOString()` never writes one.
///
/// `None` when it is not an RFC 3339 instant in UTC (`Z`, as the phone
/// writes), or names a leap second, which chrono would write back as `:60`
/// and the mailbox refuse, a refusal that is final for a filed note.
#[must_use]
pub fn instant(at: &str) -> Option<String> {
	if !at.ends_with('Z') {
		return None;
	}
	DateTime::parse_from_rfc3339(at)
		.ok()
		.filter(|at| at.timestamp_subsec_nanos() < 1_000_000_000)
		.map(|at| at.with_timezone(&Utc).to_rfc3339_opts(SecondsFormat::Millis, true))
}

/// The note in `body`, checked: the one `id` names, about an episode of
/// `feed`.
///
/// # Errors
/// Every problem found, as `(field, why)`.
pub fn parse(feed: &str, id: &str, body: &[u8]) -> Result<Note, Vec<Problem>> {
	if body.len() > BODY_CEILING {
		return Err(vec![("body", "is over the size ceiling")]);
	}
	// Straight from the bytes, so a key written twice is refused rather than
	// read as its last copy.
	let wire: Wire = serde_json::from_slice(body).map_err(|_| vec![("body", "is not the shape a note has")])?;
	let mut problems = Vec::new();
	if !is_id(id) {
		problems.push(("id", "must be the phone's id: 1 to 64 letters, digits, `-` or `_`"));
	} else if wire.id != id {
		problems.push(("id", "must be the id the path names"));
	}
	if !FEEDS.contains(&wire.feed.as_str()) || wire.feed != feed {
		problems.push(("feed", "must be the feed the path names"));
	}
	if !is_episode(&wire.episode) {
		problems.push(("episode", "must be an episode's key: 1 to 64 letters, digits or `-`"));
	}
	let at_ms = wire.at_ms.as_i64().filter(|at_ms| (0..=MAX_AT_MS).contains(at_ms));
	if at_ms.is_none() {
		problems.push(("atMs", "must be the moment in the episode: whole milliseconds from its start, within a day"));
	}
	if !in_set_order(&wire.categories) {
		problems.push(("categories", "must be names from the fixed set, each once, in its order, at least one"));
	}
	if wire.take.as_deref().is_some_and(|take| !is_take(take)) {
		problems.push(("take", "must be a take's voice note id: 1 to 64 letters, digits, '-', '_', '.' or ':', or null"));
	}
	let written_at = instant(&wire.written_at);
	if written_at.is_none() {
		problems.push(("writtenAt", "must be an RFC 3339 instant in UTC, ending `Z`"));
	}
	match (at_ms, written_at) {
		(Some(at_ms), Some(written_at)) if problems.is_empty() => Ok(Note {
			id: wire.id,
			feed: wire.feed,
			episode: wire.episode,
			at_ms,
			categories: wire.categories,
			take: wire.take,
			written_at,
		}),
		_ => Err(problems),
	}
}

/// The categories as stored: a compact JSON array, as the migration's
/// trigger reads one. Each name is one of [`CATEGORIES`], plain letters, so
/// none needs escaping.
fn stored(categories: &[String]) -> String {
	let quoted: Vec<String> = categories.iter().map(|name| String::from("\"") + name + "\"").collect();
	String::from("[") + &quoted.join(",") + "]"
}

/// Stores `body` as note `id` of `feed` for `subject`, as of `now`, and
/// answers what it did with the note as it now stands.
///
/// Run it in a `BEGIN IMMEDIATE` transaction, so the read that decides and
/// the write see one snapshot.
///
/// # Errors
/// [`NoteError::Invalid`] naming every problem [`parse`] finds,
/// [`NoteError::Rewritten`] when `id` already holds a different note, and
/// [`NoteError::Storage`] for any `sqlx` failure. A refusal is decided before
/// anything is written.
pub async fn put_note(conn: &mut SqliteConnection, subject: &str, feed: &str, id: &str, body: &[u8], now: &str) -> Result<(Change, Note), NoteError> {
	let note = parse(feed, id, body).map_err(NoteError::Invalid)?;
	let categories = stored(&note.categories);
	let held = sqlx::query!(
		"SELECT feed, episode_key, at_ms, categories, take_id, written_at FROM episode_note WHERE subject_id = ?1 AND note_id = ?2",
		subject,
		id
	)
	.fetch_optional(&mut *conn)
	.await?;
	if let Some(held) = held {
		let held = (
			held.feed.as_str(),
			held.episode_key.as_str(),
			held.at_ms,
			held.categories.as_str(),
			held.take_id.as_deref(),
			held.written_at.as_str(),
		);
		let sent = (
			note.feed.as_str(),
			note.episode.as_str(),
			note.at_ms,
			categories.as_str(),
			note.take.as_deref(),
			note.written_at.as_str(),
		);
		return if held == sent { Ok((Change::Unchanged, note)) } else { Err(NoteError::Rewritten) };
	}
	sqlx::query!(
		"INSERT INTO episode_note (subject_id, note_id, feed, episode_key, at_ms, categories, take_id, written_at, stored_at)
		 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
		subject,
		id,
		note.feed,
		note.episode,
		note.at_ms,
		categories,
		note.take,
		note.written_at,
		now
	)
	.execute(&mut *conn)
	.await?;
	Ok((Change::Added, note))
}

/// Every note `subject` left on `feed`'s episodes, oldest written first.
///
/// One range read of `episode_note_by_feed`.
///
/// # Errors
/// Any `sqlx` failure.
pub async fn notes(conn: &mut SqliteConnection, subject: &str, feed: &str) -> Result<Vec<Held>, sqlx::Error> {
	let rows = sqlx::query!(
		"SELECT note_id, episode_key, at_ms, categories, take_id, written_at, stored_at FROM episode_note
		 WHERE subject_id = ?1 AND feed = ?2 ORDER BY written_at, note_id",
		subject,
		feed
	)
	.fetch_all(&mut *conn)
	.await?;
	Ok(
		rows
			.into_iter()
			.map(|row| Held {
				id: row.note_id,
				episode: row.episode_key,
				at_ms: row.at_ms,
				// The trigger holds every stored array to the set, so this reads.
				categories: serde_json::from_str(&row.categories).unwrap_or_default(),
				take: row.take_id,
				written_at: row.written_at,
				stored_at: row.stored_at,
			})
			.collect(),
	)
}

#[cfg(test)]
mod tests {
	use super::{notes, parse, put_note, Change, NoteError, CATEGORIES};
	use serde_json::{json, Value};
	use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};

	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");
	const PREVIOUS_MIGRATION: i64 = 20_261_108_000_300;
	const NOW: &str = "2026-11-08T09:00:00.000Z";
	const A: &str = "subject-a";
	const B: &str = "subject-b";

	async fn pool() -> SqlitePool {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();
		pool
	}

	fn note(id: &str, categories: &[&str]) -> Value {
		json!({ "id": id, "feed": "call", "episode": "2026-10-30", "atMs": 61_000, "categories": categories, "writtenAt": "2026-11-08T08:15:00.000Z" })
	}

	async fn put(pool: &SqlitePool, subject: &str, value: &Value) -> Result<Change, NoteError> {
		let id = value["id"].as_str().unwrap();
		let feed = value["feed"].as_str().unwrap_or("call");
		put_note(&mut pool.acquire().await.unwrap(), subject, feed, id, value.to_string().as_bytes(), NOW)
			.await
			.map(|(change, _)| change)
	}

	async fn rows(pool: &SqlitePool) -> i64 {
		sqlx::query_scalar("SELECT COUNT(*) FROM episode_note").fetch_one(pool).await.unwrap()
	}

	fn fields(feed: &str, id: &str, value: &Value) -> Vec<&'static str> {
		let mut fields: Vec<&'static str> = parse(feed, id, value.to_string().as_bytes()).unwrap_err().iter().map(|(field, _)| *field).collect();
		fields.sort_unstable();
		fields
	}

	/// Down refuses while the table holds a row, and drops it once empty.
	#[tokio::test]
	async fn the_migration_round_trips_and_refuses_to_drop_a_record() {
		let pool = pool().await;
		let tables = || sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'episode_note'");
		assert_eq!(tables().fetch_one(&pool).await.unwrap(), 1);
		MIGRATOR.undo(&pool, PREVIOUS_MIGRATION).await.unwrap();
		assert_eq!(tables().fetch_one(&pool).await.unwrap(), 0, "down drops an empty table");
		MIGRATOR.run(&pool).await.unwrap();
		assert_eq!(tables().fetch_one(&pool).await.unwrap(), 1, "up recreates it");
		put(&pool, A, &note("n1", &["shallow"])).await.unwrap();
		assert!(MIGRATOR.undo(&pool, PREVIOUS_MIGRATION).await.is_err(), "down refuses a table holding a note");
		assert_eq!(rows(&pool).await, 1, "and leaves it as it was");
	}

	/// The columns the migration describes and no others: a new one is a
	/// decision about what home learns from a note.
	#[tokio::test]
	async fn the_table_holds_exactly_its_described_columns() {
		let pool = pool().await;
		let columns: Vec<String> = sqlx::query_scalar("SELECT name FROM pragma_table_info('episode_note') ORDER BY cid")
			.fetch_all(&pool)
			.await
			.unwrap();
		assert_eq!(
			columns,
			["subject_id", "note_id", "feed", "episode_key", "at_ms", "categories", "take_id", "written_at", "stored_at"]
		);
	}

	#[tokio::test]
	async fn a_note_is_stored_once_and_never_replaced() {
		let pool = pool().await;
		let first = note("n1", &["shallow", "voice"]);
		assert_eq!(put(&pool, A, &first).await.unwrap(), Change::Added);
		assert_eq!(put(&pool, A, &first).await.unwrap(), Change::Unchanged, "the same note writes nothing");
		let mut later = first.clone();
		later["atMs"] = json!(62_000);
		let mut spoken = first.clone();
		spoken["take"] = json!("t1");
		let mut other = first.clone();
		other["episode"] = json!("2026-11-06");
		let mut moved = first.clone();
		moved["feed"] = json!("playoff-delta");
		for changed in [note("n1", &["shallow"]), later, spoken, other, moved] {
			assert!(
				matches!(put(&pool, A, &changed).await, Err(NoteError::Rewritten)),
				"a different note under a held id: {changed}"
			);
		}
		assert_eq!(rows(&pool).await, 1);
		assert_eq!(put(&pool, B, &note("n1", &["tooLong"])).await.unwrap(), Change::Added, "B's ids are B's");

		let update = sqlx::query("UPDATE episode_note SET at_ms = 0").execute(&pool).await;
		assert!(update.unwrap_err().to_string().contains("append-only"), "no row is ever updated");
	}

	#[tokio::test]
	async fn the_answer_is_the_note_as_the_phone_keeps_it() {
		let pool = pool().await;
		let sent = note("n1", &["technical", "pacing"]);
		let (_, held) = put_note(&mut pool.acquire().await.unwrap(), A, "call", "n1", sent.to_string().as_bytes(), NOW)
			.await
			.unwrap();
		assert_eq!(serde_json::to_value(&held).unwrap(), sent, "what the phone keeps, with no take");
		let mut spoken = note("n2", &["voice"]);
		spoken["take"] = json!("take-7");
		let (_, held) = put_note(&mut pool.acquire().await.unwrap(), A, "call", "n2", spoken.to_string().as_bytes(), NOW)
			.await
			.unwrap();
		assert_eq!(serde_json::to_value(&held).unwrap(), spoken, "and with its take");
		let mut unspoken = note("n3", &["voice"]);
		unspoken["take"] = Value::Null;
		assert!(parse("call", "n3", unspoken.to_string().as_bytes()).unwrap().take.is_none(), "a null take is none");
	}

	#[tokio::test]
	async fn notes_answer_one_feed_of_the_callers_own_oldest_written_first() {
		let pool = pool().await;
		let mut late = note("late", &["shallow"]);
		late["writtenAt"] = json!("2026-11-08T08:30:00Z");
		put(&pool, A, &late).await.unwrap();
		let mut spoken = note("early", &["voice", "subtitlesOff"]);
		spoken["take"] = json!("t1");
		spoken["writtenAt"] = json!("2026-11-08T08:00:00.000Z");
		put(&pool, A, &spoken).await.unwrap();
		let mut pd = note("pd", &["overlap"]);
		pd["feed"] = json!("playoff-delta");
		pd["episode"] = json!("2026-SF-08");
		put(&pool, A, &pd).await.unwrap();
		put(&pool, B, &note("theirs", &["shallow"])).await.unwrap();

		let held = notes(&mut pool.acquire().await.unwrap(), A, "call").await.unwrap();
		assert_eq!(
			serde_json::to_value(&held).unwrap(),
			json!([
				{ "id": "early", "episode": "2026-10-30", "atMs": 61_000, "categories": ["voice", "subtitlesOff"], "take": "t1", "writtenAt": "2026-11-08T08:00:00.000Z", "storedAt": NOW },
				{ "id": "late", "episode": "2026-10-30", "atMs": 61_000, "categories": ["shallow"], "take": null, "writtenAt": "2026-11-08T08:30:00.000Z", "storedAt": NOW },
			])
		);
		let pd = notes(&mut pool.acquire().await.unwrap(), A, "playoff-delta").await.unwrap();
		assert_eq!(pd.iter().map(|note| note.id.as_str()).collect::<Vec<_>>(), ["pd"]);
	}

	#[test]
	fn categories_are_the_sets_each_once_in_its_order() {
		assert!(parse("call", "n1", note("n1", &CATEGORIES).to_string().as_bytes()).is_ok(), "every one, in order");
		for picked in [&[][..], &["voice", "shallow"], &["shallow", "shallow"], &["boring"], &["Shallow"]] {
			assert_eq!(fields("call", "n1", &note("n1", picked)), ["categories"], "{picked:?}");
		}
	}

	#[test]
	fn refusals_name_each_field() {
		let mut bad = note("n1", &["shallow"]);
		bad["episode"] = json!("a/b");
		bad["atMs"] = json!(-1);
		bad["writtenAt"] = json!("2026-11-08T08:15:00+09:00");
		bad["take"] = json!("a/b");
		assert_eq!(fields("call", "n1", &bad), ["atMs", "episode", "take", "writtenAt"]);
		assert_eq!(fields("call", "n2", &note("n1", &["shallow"])), ["id"], "the id the path names");
		assert_eq!(fields("call", "n/1", &note("n/1", &["shallow"])), ["id"]);
		assert_eq!(fields("playoff-delta", "n1", &note("n1", &["shallow"])), ["feed"], "the feed the path names");
		let mut leap = note("n1", &["shallow"]);
		leap["writtenAt"] = json!("2016-12-31T23:59:60.000Z");
		assert_eq!(fields("call", "n1", &leap), ["writtenAt"], "a leap second, which the mailbox would refuse");
		let mut far = note("n1", &["shallow"]);
		far["atMs"] = json!(super::MAX_AT_MS + 1);
		assert_eq!(fields("call", "n1", &far), ["atMs"], "past a day into the episode");
		far["atMs"] = json!(super::MAX_AT_MS);
		assert!(parse("call", "n1", far.to_string().as_bytes()).is_ok(), "a day in is the last moment");
		let mut fraction = note("n1", &["shallow"]);
		fraction["atMs"] = json!(1.5);
		assert_eq!(fields("call", "n1", &fraction), ["atMs"], "whole milliseconds");
		let mut unknown = note("n1", &["shallow"]);
		unknown["text"] = json!("a typed line");
		assert_eq!(fields("call", "n1", &unknown), ["body"], "a key the shape lacks");
		let twice = br#"{"id":"n1","id":"n1","feed":"call","episode":"e","atMs":0,"categories":["shallow"],"writtenAt":"2026-11-08T08:15:00.000Z"}"#;
		assert!(parse("call", "n1", twice).is_err(), "a key written twice");
	}

	#[tokio::test]
	async fn a_refused_note_writes_nothing() {
		let pool = pool().await;
		assert!(matches!(put(&pool, A, &note("n1", &[])).await, Err(NoteError::Invalid(_))));
		assert_eq!(rows(&pool).await, 0);
	}

	/// The table holds its own rules, should a writer other than
	/// [`put_note`] ever come.
	#[tokio::test]
	async fn the_table_refuses_categories_out_of_the_sets_order() {
		let pool = pool().await;
		for categories in [r#"["voice","shallow"]"#, r#"["shallow","shallow"]"#, r#"["boring"]"#, "[]", r#"["shallow", "voice"]"#] {
			let written = sqlx::query(
				"INSERT INTO episode_note (subject_id, note_id, feed, episode_key, at_ms, categories, take_id, written_at, stored_at)
				 VALUES ('a', 'n1', 'call', 'e', 0, ?1, NULL, 'w', 's')",
			)
			.bind(categories)
			.execute(&pool)
			.await;
			assert!(written.is_err(), "{categories}");
		}
		assert_eq!(rows(&pool).await, 0);
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
			put(&pool, subject, &note("n1", &["shallow"])).await.unwrap();
		}
		sqlx::query("DELETE FROM account WHERE subject_id = ?1").bind(A).execute(&pool).await.unwrap();
		let left: Vec<String> = sqlx::query_scalar("SELECT subject_id FROM episode_note").fetch_all(&pool).await.unwrap();
		assert_eq!(left, [B], "A's notes went with A's account, B's stayed");
	}
}

//! The owner's drama prompts (paulgsc/some-ui#2179, HF4): each next scene's
//! prompt the phone asks for ("Ask for the next scene"), sent home by its
//! drain.
//!
//! `20261101000700_create_drama_prompt.up.sql` records the design; in short:
//!
//! - **Append-only.** [`put_prompt`] stores a prompt under its stamp, or
//!   nothing when that stamp already holds the same one; a different prompt
//!   under a held stamp is refused ([`PromptError::Rewritten`]), never
//!   stored. No row is ever updated (a trigger aborts an `UPDATE`), and
//!   nothing here deletes. Account deletion is the one delete: its generic
//!   sweep of `SUBJECT_SCOPED_TABLES`, and the migration's trigger on
//!   `account`.
//! - **Text only.** A prompt is its markdown. Its scene notes went home as
//!   voice notes (`voice_note`, docs/identity.md invariant 23), and it names
//!   them by id: a prompt that carries audio or names a Drive file, by the
//!   Jobs mailbox's own tests ([`carries`]), is refused here, so home never
//!   keeps one it could not hand to the routine.
//! - **The phone's rules, held here too.** A write passes [`parse`].
//! - **Per subject, and only per subject.** Every query takes the subject and
//!   filters on it; nothing reads across subjects.
//! - **No background writer.** The one caller is `file_host`'s
//!   `handlers::routine` (`PUT /api/v1/routine/prompts/:stamp`). A
//!   `git grep -n drama_prompt -- '*.rs'` shows no other writer.

use chrono::DateTime;
use serde::{Deserialize, Serialize};
use sqlx::SqliteConnection;

/// A refused field and why, as `file_host` answers it in a `422`.
pub type Problem = (&'static str, &'static str);

/// The most bytes of one prompt's text.
///
/// Today's prompt is about 40 KB, most of it the tree prompt; this leaves
/// room for it to grow, and keeps the file home hands the mailbox, escapes
/// and all, well under the mailbox's 1 MiB (paulgsc/some-ui `apps/mailbox`,
/// `MAX_BODY_BYTES`).
pub const MAX_TEXT_BYTES: usize = 256 * 1024;

/// The largest body a write takes: the text, every character escaped, and
/// its stamp.
pub const BODY_CEILING: usize = 2 * MAX_TEXT_BYTES + 1024;

/// What a put did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
	/// Home did not hold it: it is stored now.
	Added,
	/// Home already holds this very prompt: nothing written.
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
pub enum PromptError {
	/// The write breaks a rule; every problem, by field.
	Invalid(Vec<Problem>),
	/// The stamp already holds a different prompt, which is never replaced.
	Rewritten,
	Storage(sqlx::Error),
}

impl std::fmt::Display for PromptError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::Invalid(problems) => {
				f.write_str("not a drama prompt home takes:")?;
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

impl std::error::Error for PromptError {}

impl From<sqlx::Error> for PromptError {
	fn from(err: sqlx::Error) -> Self {
		Self::Storage(err)
	}
}

/// What [`PromptError::Rewritten`] says.
pub const REWRITTEN: &str = "this stamp already holds a different prompt: the record is append-only";

/// A drama prompt, in the phone's own shape: its stamp and its markdown.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Prompt {
	/// `drama-` and the moment it was asked for, in UTC to the millisecond
	/// (`drama-20261031T071503120Z`, topik's `sceneStamp`).
	pub stamp: String,
	/// The prompt's markdown, as the phone wrote it.
	pub text: String,
}

/// Whether `stamp` is a prompt's stamp: `drama-YYYYMMDDTHHMMSSmmmZ`, at a
/// moment the calendar has.
#[must_use]
pub fn is_stamp(stamp: &str) -> bool {
	let Some(digits) = stamp.strip_prefix("drama-").and_then(|rest| rest.strip_suffix('Z')) else {
		return false;
	};
	let bytes = digits.as_bytes();
	if bytes.len() != 18 || bytes[8] != b'T' || !bytes.iter().enumerate().all(|(at, byte)| at == 8 || byte.is_ascii_digit()) {
		return false;
	}
	let part = |from: usize, to: usize| digits.get(from..to).unwrap_or_default();
	let instant = [
		part(0, 4),
		"-",
		part(4, 6),
		"-",
		part(6, 8),
		"T",
		part(9, 11),
		":",
		part(11, 13),
		":",
		part(13, 15),
		".",
		part(15, 18),
		"Z",
	]
	.concat();
	DateTime::parse_from_rfc3339(&instant).is_ok()
}

/// Why the Jobs mailbox would refuse a prompt whose text this is.
///
/// As paulgsc/some-ui `apps/mailbox`'s `carriesRefusal` reads it: a `data:`
/// URI or a run of 256 or more base64 characters once line breaks are gone,
/// which is audio, or a Drive link. `None` when it carries neither.
#[must_use]
pub fn carries(text: &str) -> Option<&'static str> {
	// The text as the mailbox reads it: inside the JSON home puts.
	let sent = serde_json::Value::String(text.to_owned()).to_string();
	let flat = sent.replace("\\n", "").replace("\\r", "").replace(['\n', '\r'], "").replace("\\/", "/");
	let lower = sent.to_ascii_lowercase();
	if has_data_uri(&lower) || longest_encoded_run(&flat) >= 256 {
		return Some("must carry no audio: a scene note goes home as a voice note, named here by its id");
	}
	if lower.contains("drive.google.com")
		|| ["docs.google.com/uc", "docs.google.com/open", "docs.google.com/file"]
			.iter()
			.any(|link| lower.contains(link))
	{
		return Some("must name no Drive file");
	}
	None
}

/// `data:<type>/<subtype>;` or `,`, as the mailbox's `DATA_URI` reads it.
fn has_data_uri(lower: &str) -> bool {
	let is_part = |byte: u8| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'+' | b'-');
	lower.match_indices("data:").any(|(at, _)| {
		let starts_word = lower
			.as_bytes()
			.get(at.wrapping_sub(1))
			.is_none_or(|before| !(before.is_ascii_alphanumeric() || *before == b'_'));
		let rest = &lower.as_bytes()[at + 5..];
		let kind = rest.iter().take_while(|byte| is_part(**byte)).count();
		let Some(b'/') = rest.get(kind) else {
			return false;
		};
		let subtype = rest[kind + 1..].iter().take_while(|byte| is_part(**byte)).count();
		starts_word && kind > 0 && subtype > 0 && matches!(rest.get(kind + 1 + subtype), Some(b';' | b','))
	})
}

/// The longest run in base64's alphabet (`ENCODED_RUN`'s `[A-Za-z0-9+/=_-]`).
fn longest_encoded_run(text: &str) -> usize {
	text
		.split(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '=' | '_' | '-')))
		.map(str::len)
		.max()
		.unwrap_or(0)
}

/// The drama prompt in `body`, checked: the one `stamp` names.
///
/// # Errors
/// Every problem found, as `(field, why)`.
pub fn parse(stamp: &str, body: &[u8]) -> Result<Prompt, Vec<Problem>> {
	if body.len() > BODY_CEILING {
		return Err(vec![("body", "is over the size ceiling")]);
	}
	// Straight from the bytes, so a key written twice is refused rather than
	// read as its last copy.
	let prompt: Prompt = serde_json::from_slice(body).map_err(|_| vec![("body", "is not the shape a prompt has: { stamp, text }")])?;
	let mut problems = Vec::new();
	if !is_stamp(stamp) {
		problems.push(("stamp", "must be the prompt's stamp: drama- and a UTC moment, YYYYMMDDTHHMMSSmmmZ"));
	} else if prompt.stamp != stamp {
		problems.push(("stamp", "must be the stamp the path names"));
	}
	if prompt.text.trim().is_empty() || prompt.text.len() > MAX_TEXT_BYTES {
		problems.push(("text", "must be the prompt: not blank, at most 256 KiB"));
	} else if let Some(why) = carries(&prompt.text) {
		problems.push(("text", why));
	}
	if problems.is_empty() {
		Ok(prompt)
	} else {
		Err(problems)
	}
}

/// Stores `body` as prompt `stamp` for `subject`, as of `now`, and answers
/// what it did with the prompt as it now stands.
///
/// Run it in a `BEGIN IMMEDIATE` transaction, so the read that decides and
/// the write see one snapshot.
///
/// # Errors
/// [`PromptError::Invalid`] naming every problem [`parse`] finds,
/// [`PromptError::Rewritten`] when `stamp` already holds a different prompt,
/// and [`PromptError::Storage`] for any `sqlx` failure. A refusal is decided
/// before anything is written.
pub async fn put_prompt(conn: &mut SqliteConnection, subject: &str, stamp: &str, body: &[u8], now: &str) -> Result<(Change, Prompt), PromptError> {
	let prompt = parse(stamp, body).map_err(PromptError::Invalid)?;
	let held = sqlx::query_scalar!("SELECT text FROM drama_prompt WHERE subject_id = ?1 AND stamp = ?2", subject, stamp)
		.fetch_optional(&mut *conn)
		.await?;
	if let Some(held) = held {
		return if held == prompt.text {
			Ok((Change::Unchanged, prompt))
		} else {
			Err(PromptError::Rewritten)
		};
	}
	sqlx::query!(
		"INSERT INTO drama_prompt (subject_id, stamp, text, written_at) VALUES (?1, ?2, ?3, ?4)",
		subject,
		stamp,
		prompt.text,
		now
	)
	.execute(&mut *conn)
	.await?;
	Ok((Change::Added, prompt))
}

#[cfg(test)]
mod tests {
	use super::{carries, is_stamp, parse, put_prompt, Change, PromptError, MAX_TEXT_BYTES};
	use serde_json::{json, Value};
	use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};

	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");
	/// The migration before this crate's. Undoing to it undoes this one.
	const PREVIOUS_MIGRATION: i64 = 20_261_101_000_100;
	const NOW: &str = "2026-11-01T09:00:00.000Z";
	const A: &str = "subject-a";
	const B: &str = "subject-b";
	const STAMP: &str = "drama-20261031T071503120Z";

	async fn pool() -> SqlitePool {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();
		pool
	}

	fn asked(stamp: &str, text: &str) -> Value {
		json!({ "stamp": stamp, "text": text })
	}

	async fn put(pool: &SqlitePool, subject: &str, value: &Value) -> Result<Change, PromptError> {
		let stamp = value["stamp"].as_str().unwrap();
		put_prompt(&mut pool.acquire().await.unwrap(), subject, stamp, value.to_string().as_bytes(), NOW)
			.await
			.map(|(change, _)| change)
	}

	async fn rows(pool: &SqlitePool) -> i64 {
		sqlx::query_scalar("SELECT COUNT(*) FROM drama_prompt").fetch_one(pool).await.unwrap()
	}

	fn fields(stamp: &str, value: &Value) -> Vec<&'static str> {
		let mut fields: Vec<&'static str> = parse(stamp, value.to_string().as_bytes()).unwrap_err().iter().map(|(field, _)| *field).collect();
		fields.sort_unstable();
		fields
	}

	/// Down refuses while the table holds a row, and drops it once empty.
	#[tokio::test]
	async fn the_migration_round_trips_and_refuses_to_drop_a_record() {
		let pool = pool().await;
		let tables = || sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'drama_prompt'");
		assert_eq!(tables().fetch_one(&pool).await.unwrap(), 1);
		MIGRATOR.undo(&pool, PREVIOUS_MIGRATION).await.unwrap();
		assert_eq!(tables().fetch_one(&pool).await.unwrap(), 0, "down drops an empty table");
		MIGRATOR.run(&pool).await.unwrap();
		assert_eq!(tables().fetch_one(&pool).await.unwrap(), 1, "up recreates it");
		put(&pool, A, &asked(STAMP, "Write the next scene.")).await.unwrap();
		assert!(MIGRATOR.undo(&pool, PREVIOUS_MIGRATION).await.is_err(), "down refuses a table holding a prompt");
		assert_eq!(rows(&pool).await, 1, "and leaves it as it was");
	}

	/// The columns the migration describes and no others: a new one is a
	/// decision about what home learns from a prompt.
	#[tokio::test]
	async fn the_table_holds_exactly_its_described_columns() {
		let pool = pool().await;
		let columns: Vec<String> = sqlx::query_scalar("SELECT name FROM pragma_table_info('drama_prompt') ORDER BY cid")
			.fetch_all(&pool)
			.await
			.unwrap();
		assert_eq!(columns, ["subject_id", "stamp", "text", "written_at"]);
	}

	#[tokio::test]
	async fn a_prompt_is_stored_once_and_never_replaced() {
		let pool = pool().await;
		let first = asked(STAMP, "Write the next scene: the fiancée meets his mother.");
		assert_eq!(put(&pool, A, &first).await.unwrap(), Change::Added);
		assert_eq!(put(&pool, A, &first).await.unwrap(), Change::Unchanged, "the same prompt writes nothing");
		assert!(
			matches!(put(&pool, A, &asked(STAMP, "Something else.")).await, Err(PromptError::Rewritten)),
			"a different prompt under a held stamp"
		);
		assert_eq!(rows(&pool).await, 1);
		assert_eq!(put(&pool, B, &asked(STAMP, "Something else.")).await.unwrap(), Change::Added, "B's stamps are B's");

		let update = sqlx::query("UPDATE drama_prompt SET text = 'x'").execute(&pool).await;
		assert!(update.unwrap_err().to_string().contains("append-only"), "no row is ever updated");
	}

	#[tokio::test]
	async fn the_answer_is_the_prompt_as_the_phone_wrote_it() {
		let pool = pool().await;
		let sent = asked(
			STAMP,
			"> **For an agent:** follow the prompt below.\n\n<!-- notes: {\"drama-20261031T071503120Z.voice-1.webm\":\"n1\"} -->\n  두 번째 장면  ",
		);
		let (_, held) = put_prompt(&mut pool.acquire().await.unwrap(), A, STAMP, sent.to_string().as_bytes(), NOW).await.unwrap();
		assert_eq!(serde_json::to_value(&held).unwrap(), sent, "byte for byte, spaces and all");
	}

	#[test]
	fn a_stamp_is_a_real_moment() {
		assert!(is_stamp(STAMP));
		for bad in [
			"drama-20261331T071503120Z",
			"drama-20261031T251503120Z",
			"drama-20261031T07150312Z",
			"drama-20261031071503120Z",
			"20261031T071503120Z",
			"drama-20261031T071503120",
			"drama-2026103xT071503120Z",
			"drama-20260230T000000000Z",
		] {
			assert!(!is_stamp(bad), "{bad}");
		}
	}

	#[test]
	fn refusals_name_each_field() {
		assert_eq!(fields(STAMP, &asked(STAMP, " ")), ["text"]);
		assert_eq!(fields("drama-20261031T071503121Z", &asked(STAMP, "x")), ["stamp"], "the stamp the path names");
		assert_eq!(fields("drama-x", &asked("drama-x", "x")), ["stamp"]);
		assert_eq!(fields(STAMP, &json!({ "stamp": STAMP, "text": "x", "notes": ["n1"] })), ["body"], "a key the shape lacks");
		assert_eq!(fields(STAMP, &json!({ "stamp": STAMP })), ["body"]);
		assert_eq!(fields(STAMP, &asked(STAMP, &"가".repeat(MAX_TEXT_BYTES / 3 + 1))), ["text"], "the ceiling counts bytes");
		assert!(parse(STAMP, asked(STAMP, &"가".repeat(MAX_TEXT_BYTES / 3)).to_string().as_bytes()).is_ok());
		let twice = br#"{"stamp":"drama-20261031T071503120Z","stamp":"drama-20261031T071503120Z","text":"x"}"#;
		assert!(parse(STAMP, twice).is_err(), "a key written twice");
	}

	/// The mailbox's own tests, so home keeps no prompt the routine could
	/// never be handed.
	#[test]
	fn a_prompt_carrying_audio_or_a_drive_link_is_refused() {
		for text in [
			"here data:audio/webm;base64,GkXfo59ChoEB".to_owned(),
			"GkXfo59C".repeat(40),
			["GkXfo59C".repeat(20), "GkXfo59C".repeat(20)].join("\n"),
			"see https://drive.google.com/file/d/1AbC/view".to_owned(),
			"or https://docs.google.com/open?id=1AbC".to_owned(),
		] {
			assert!(carries(&text).is_some(), "{text}");
			assert_eq!(fields(STAMP, &asked(STAMP, &text)), ["text"]);
		}
		for text in [
			"Scene, spoken: drama-20261031T071503120Z.voice-1.webm, drama-20261031T071503120Z.voice-2.m4a",
			"metadata: none, data:text here",
			"| --- | --- |\n| 기쁨 | joy |",
		] {
			assert_eq!(carries(text), None, "{text}");
		}
		assert_eq!(carries(&["a".repeat(255), "b".repeat(255)].join(" ")), None, "a space ends a run");
	}

	#[tokio::test]
	async fn a_refused_prompt_writes_nothing() {
		let pool = pool().await;
		assert!(matches!(put(&pool, A, &asked(STAMP, "")).await, Err(PromptError::Invalid(_))));
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
			put(&pool, subject, &asked(STAMP, "Write the next scene.")).await.unwrap();
		}
		sqlx::query("DELETE FROM account WHERE subject_id = ?1").bind(A).execute(&pool).await.unwrap();
		let left: Vec<String> = sqlx::query_scalar("SELECT subject_id FROM drama_prompt").fetch_all(&pool).await.unwrap();
		assert_eq!(left, [B], "A's prompts went with A's account, B's stayed");
	}
}

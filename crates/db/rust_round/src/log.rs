//! The card-era log on home (RR14).
//!
//! paulgsc/some-ui#2689, first asked for by RR7 #2238: each session of `some-ui.rust-round-log/1` the phone kept
//! while the cards were played, taken as it is, so no card-era record is
//! lost when RR20 retires the screens that wrote it.
//!
//! - **Taken as it is** ([`read_session`]): the session's bytes are kept as
//!   the phone sent them and read back the same. Home checks the shape (the
//!   format's keys at every level, and nothing else) so that what it keeps
//!   is a session, and reads none of it but its id and when it started.
//! - **A session grows; nothing in it is rewritten** ([`put_session`]). A
//!   session the phone keeps playing gains cards, pins and pastes at the
//!   end of its lists, so a session that arrives with every list it held
//!   still at the start of the new one is kept as a new revision beside the
//!   last ([`LogChange::Extended`]); one that drops or changes anything held
//!   is refused ([`RoundError::Rewritten`]). No row is updated or, while the
//!   account stands, deleted (the migration's triggers).
//! - **Notes are named by id; the log never holds audio** (the format's own
//!   rule). A card's `rustc` is what rustc printed for the tutor's program.
//! - **Per subject.** Every query takes the subject and filters on it.

use crate::round::Problem;
use crate::{content_hash, RoundError};
use serde_json::{Map, Value};
use sqlx::SqliteConnection;

/// The format's tag.
pub const LOG_SCHEMA: &str = "some-ui.rust-round-log/1";

/// The most bytes one session may hold. A card's event is a few hundred
/// bytes and what rustc printed a few KiB; a session is an opening of the
/// app.
pub const SESSION_CEILING: usize = 512 * 1024;

/// [`RoundError::Rewritten`]'s words for a session, for the `409`.
pub const SESSION_REWRITTEN: &str = "home holds this session with something this one drops or changes; a session only grows, so send it with everything it held";

/// The session's lists, in the format's order.
const LISTS: [&str; 4] = ["cards", "pinned", "fromMyDesk", "tutorGuessedWrong"];

const SESSION_KEYS: [&str; 7] = ["schema", "id", "startedAt", "cards", "pinned", "fromMyDesk", "tutorGuessedWrong"];
const CARD_KEYS: [&str; 8] = ["round", "card", "at", "picked", "note", "refines", "skipped", "rustc"];
const REF_KEYS: [&str; 2] = ["round", "card"];
const PASTE_KEYS: [&str; 3] = ["at", "pasted", "note"];

/// One session, read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
	pub id: String,
	/// When it started: UTC to the millisecond when the phone's `startedAt`
	/// is an RFC 3339 instant, else as written.
	pub started_at: String,
	/// The body, byte for byte as sent (JSON whitespace around it trimmed).
	pub text: String,
}

/// What a put did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogChange {
	/// A session home did not hold; its first revision is stored.
	Added,
	/// The session as home's last revision holds it; nothing was written.
	Unchanged,
	/// The session grew: a new revision is stored beside the last.
	Extended,
}

impl LogChange {
	/// The word the answer gives it.
	#[must_use]
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::Added => "added",
			Self::Unchanged => "unchanged",
			Self::Extended => "extended",
		}
	}
}

/// One session's last revision, as [`sessions`] lists it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionEntry {
	pub id: String,
	pub started_at: String,
	pub revision: i64,
	pub sha256: String,
	pub written_at: String,
}

/// A session's id: 1 to 64 letters, digits, `.`, `_`, `:` or `-`, so it
/// can name its route.
#[must_use]
pub fn is_session_id(id: &str) -> bool {
	crate::sent::is_note_id(id)
}

fn has_exactly(object: &Map<String, Value>, keys: &[&str]) -> bool {
	object.len() == keys.len() && keys.iter().all(|key| object.contains_key(*key))
}

fn is_text(value: Option<&Value>) -> bool {
	value.and_then(Value::as_str).is_some_and(|text| !text.is_empty())
}

fn is_text_or_null(value: Option<&Value>) -> bool {
	value.is_some_and(Value::is_null) || is_text(value)
}

/// Whether `item` is what the list `list` holds.
fn item_is(list: &str, item: &Value) -> bool {
	let Some(object) = item.as_object() else {
		return false;
	};
	match list {
		"cards" => {
			has_exactly(object, &CARD_KEYS)
				&& is_text(object.get("round"))
				&& is_text(object.get("card"))
				&& is_text(object.get("at"))
				&& is_text_or_null(object.get("picked"))
				&& is_text_or_null(object.get("note"))
				&& is_text_or_null(object.get("refines"))
				&& object.get("skipped").is_some_and(Value::is_boolean)
				&& object.get("rustc").is_some_and(|rustc| rustc.is_null() || rustc.is_object())
		}
		"fromMyDesk" => has_exactly(object, &PASTE_KEYS) && is_text(object.get("at")) && is_text(object.get("pasted")) && is_text_or_null(object.get("note")),
		_ => has_exactly(object, &REF_KEYS) && is_text(object.get("round")) && is_text(object.get("card")),
	}
}

/// Read `body` as the session the path names, `id`, or say every problem
/// found.
///
/// # Errors
/// Each [`Problem`]: a body past [`SESSION_CEILING`], not UTF-8 or not one
/// JSON object; a key the format does not name, or one it names missing; a
/// `schema` other than [`LOG_SCHEMA`]; an `id` that is not the path's or
/// not a session id; no `startedAt`; a list that is not one, or an item in
/// it that is not what the list holds.
pub fn read_session(body: &[u8], id: &str) -> Result<Session, Vec<Problem>> {
	let problem = |field: &str, why| vec![(field.to_owned(), why)];
	if body.len() > SESSION_CEILING {
		return Err(problem("body", "is larger than 512 KiB"));
	}
	let Ok(text) = std::str::from_utf8(body) else {
		return Err(problem("body", "is not UTF-8"));
	};
	let text = text.trim();
	let Ok(Value::Object(session)) = serde_json::from_str::<Value>(text) else {
		return Err(problem("body", "is not one JSON object"));
	};
	let mut problems: Vec<Problem> = Vec::new();
	if !has_exactly(&session, &SESSION_KEYS) {
		problems.push((String::from("body"), "must hold a session's keys, and nothing else"));
	}
	if session.get("schema").and_then(Value::as_str) != Some(LOG_SCHEMA) {
		problems.push((String::from("schema"), "must be \"some-ui.rust-round-log/1\""));
	}
	match session.get("id").and_then(Value::as_str) {
		Some(held) if held == id && is_session_id(held) => {}
		_ => problems.push((String::from("id"), "must be the session id in the path")),
	}
	if !is_text(session.get("startedAt")) {
		problems.push((String::from("startedAt"), "must say when the session started"));
	}
	for list in LISTS {
		match session.get(list).and_then(Value::as_array) {
			None => problems.push((list.to_owned(), "must be a list")),
			Some(items) => {
				if let Some(index) = items.iter().position(|item| !item_is(list, item)) {
					problems.push((String::from(list) + "[" + &index.to_string() + "]", "is not what this list holds"));
				}
			}
		}
	}
	if !problems.is_empty() {
		return Err(problems);
	}
	let field = |key: &str| session.get(key).and_then(Value::as_str).unwrap_or_default().to_owned();
	Ok(Session {
		id: field("id"),
		started_at: stored_instant(&field("startedAt")),
		text: text.to_owned(),
	})
}

/// `at` as UTC to the millisecond when it is an RFC 3339 instant, so that
/// text order is time order whatever offset the phone wrote; otherwise as
/// written. The body keeps the phone's own text either way.
fn stored_instant(at: &str) -> String {
	chrono::DateTime::parse_from_rfc3339(at).map_or_else(|_| at.to_owned(), |at| at.with_timezone(&chrono::Utc).to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
}

/// Whether `grown` holds everything `held` does, each list starting with
/// what it held, and the same start.
fn extends(held: &str, grown: &str) -> bool {
	let (Ok(held), Ok(grown)) = (serde_json::from_str::<Value>(held), serde_json::from_str::<Value>(grown)) else {
		return false;
	};
	held.get("startedAt") == grown.get("startedAt")
		&& LISTS
			.iter()
			.all(|list| match (held.get(*list).and_then(Value::as_array), grown.get(*list).and_then(Value::as_array)) {
				(Some(held), Some(grown)) => grown.starts_with(held),
				_ => false,
			})
}

/// Store `session` for `subject`: a first revision, nothing when its last
/// revision holds the same, or a new revision when it grew.
///
/// # Errors
/// [`RoundError::Rewritten`] for a session that drops or changes what the
/// last revision holds, which writes nothing; [`RoundError::Storage`] for
/// any `sqlx` failure.
pub async fn put_session(conn: &mut SqliteConnection, subject: &str, session: &Session, now: &str) -> Result<(LogChange, i64), RoundError> {
	let held = sqlx::query!(
		r#"SELECT revision AS "revision!: i64", body FROM rust_round_log
		   WHERE subject_id = ?1 AND session_id = ?2 ORDER BY revision DESC LIMIT 1"#,
		subject,
		session.id
	)
	.fetch_optional(&mut *conn)
	.await?;
	let (change, revision) = match held {
		None => (LogChange::Added, 1),
		Some(held) if held.body == session.text => return Ok((LogChange::Unchanged, held.revision)),
		Some(held) if extends(&held.body, &session.text) => (LogChange::Extended, held.revision + 1),
		Some(_) => return Err(RoundError::Rewritten),
	};
	let sha256 = content_hash(session.text.as_bytes());
	sqlx::query!(
		"INSERT INTO rust_round_log (subject_id, session_id, revision, started_at, body, sha256, written_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
		subject,
		session.id,
		revision,
		session.started_at,
		session.text,
		sha256,
		now
	)
	.execute(&mut *conn)
	.await?;
	Ok((change, revision))
}

/// Every session `subject` holds, each by its last revision, in the order
/// they started.
///
/// That is by the instant `startedAt` names, whatever its offset (one that
/// is not an instant after the rest), so sorted here, not by the text.
///
/// Unbounded on purpose: the card era is over once RR20 retires the screens
/// that write the log, so its sessions are a finite set, and the phone reads
/// them all back to know each is kept.
///
/// # Errors
/// Any `sqlx` failure.
pub async fn sessions(conn: &mut SqliteConnection, subject: &str) -> Result<Vec<SessionEntry>, sqlx::Error> {
	sqlx::query_as!(
		SessionEntry,
		r#"SELECT session_id AS "id!: String", started_at AS "started_at!: String", revision AS "revision!: i64",
		          sha256 AS "sha256!: String", written_at AS "written_at!: String"
		   FROM rust_round_log AS log
		   WHERE subject_id = ?1
		     AND revision = (SELECT MAX(revision) FROM rust_round_log WHERE subject_id = ?1 AND session_id = log.session_id)
		   ORDER BY session_id"#,
		subject
	)
	.fetch_all(&mut *conn)
	.await
	.map(|mut sessions| {
		sessions.sort_by(|a, b| (crate::time_order_key(&a.started_at), &a.id).cmp(&(crate::time_order_key(&b.started_at), &b.id)));
		sessions
	})
}

/// `subject`'s session `id`, its last revision's bytes, if home holds it.
///
/// # Errors
/// Any `sqlx` failure.
pub async fn session(conn: &mut SqliteConnection, subject: &str, id: &str) -> Result<Option<String>, sqlx::Error> {
	sqlx::query_scalar!(
		"SELECT body FROM rust_round_log WHERE subject_id = ?1 AND session_id = ?2 ORDER BY revision DESC LIMIT 1",
		subject,
		id
	)
	.fetch_optional(&mut *conn)
	.await
}

#[cfg(test)]
mod tests {
	use super::{put_session, read_session, session, sessions, LogChange};
	use crate::RoundError;
	use serde_json::{json, Value};
	use sqlx::sqlite::SqlitePoolOptions;
	use sqlx::SqlitePool;

	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");
	const A: &str = "subject-a";
	const B: &str = "subject-b";
	const NOW: &str = "2026-11-09T20:00:00.000Z";
	const ID: &str = "0f6c2a8e-3a7b-4bd8-9a55-0c1e2d3f4a5b";

	async fn pool() -> SqlitePool {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();
		for subject in [A, B] {
			sqlx::query("INSERT INTO account (subject_id, user_handle) VALUES (?1, ?1)")
				.bind(subject)
				.execute(&pool)
				.await
				.unwrap();
		}
		pool
	}

	fn card(card: &str) -> Value {
		json!({
			"round": "indexing-01", "card": card, "at": "2026-10-30T08:00:00.000Z",
			"picked": null, "note": "note-1", "refines": null, "skipped": false,
			"rustc": { "outcome": "prints", "stdout": "3\n" },
		})
	}

	fn body(cards: &[&str]) -> Value {
		json!({
			"schema": "some-ui.rust-round-log/1", "id": ID, "startedAt": "2026-10-30T07:59:00.000Z",
			"cards": cards.iter().map(|each| card(each)).collect::<Vec<_>>(),
			"pinned": [{ "round": "indexing-01", "card": "c1" }],
			"fromMyDesk": [{ "at": "2026-10-30T08:10:00.000Z", "pasted": "error[E0502]", "note": null }],
			"tutorGuessedWrong": [],
		})
	}

	fn fields(problems: &[(String, &'static str)]) -> Vec<String> {
		problems.iter().map(|(field, _)| field.clone()).collect()
	}

	/// A session is kept as sent, grows by revisions, and anything that
	/// drops or changes what it held is refused.
	#[tokio::test]
	async fn a_session_grows_by_revisions_and_is_never_rewritten() {
		let pool = pool().await;
		let mut conn = pool.acquire().await.unwrap();
		let put = |value: Value| read_session(value.to_string().as_bytes(), ID).unwrap();
		let first = put(body(&["c1"]));
		assert_eq!(put_session(&mut conn, A, &first, NOW).await.unwrap(), (LogChange::Added, 1));
		assert_eq!(put_session(&mut conn, A, &first, NOW).await.unwrap(), (LogChange::Unchanged, 1));
		let grown = put(body(&["c1", "c2"]));
		assert_eq!(put_session(&mut conn, A, &grown, NOW).await.unwrap(), (LogChange::Extended, 2));
		for changed in [body(&["c2"]), body(&["c1"]), {
			let mut moved = body(&["c1", "c2", "c3"]);
			moved["startedAt"] = json!("2026-10-30T07:58:00.000Z");
			moved
		}] {
			assert!(matches!(put_session(&mut conn, A, &put(changed), NOW).await, Err(RoundError::Rewritten)));
		}
		assert_eq!(session(&mut conn, A, ID).await.unwrap().unwrap(), grown.text, "read back as sent");
		let held = sessions(&mut conn, A).await.unwrap();
		assert_eq!(held.len(), 1);
		assert_eq!(held[0].revision, 2);
		assert!(sessions(&mut conn, B).await.unwrap().is_empty(), "another subject's log is not there");
		for (id, started) in [("z-utc", "2026-10-31T09:00:00Z"), ("a-east", "2026-10-31T10:00:00+02:00")] {
			let mut later = body(&[]);
			later["id"] = json!(id);
			later["startedAt"] = json!(started);
			put_session(&mut conn, B, &read_session(later.to_string().as_bytes(), id).unwrap(), NOW).await.unwrap();
		}
		let order: Vec<String> = sessions(&mut conn, B).await.unwrap().into_iter().map(|each| each.id).collect();
		assert_eq!(order, ["a-east", "z-utc"], "in time, whatever the offset");
		let earliest: String = sqlx::query_scalar("SELECT MIN(started_at) FROM rust_round_log WHERE subject_id = ?1")
			.bind(B)
			.fetch_one(&mut *conn)
			.await
			.unwrap();
		assert_eq!(earliest, "2026-10-31T08:00:00.000Z", "stored in UTC, so MIN is the earliest in time");
		let held: Value = serde_json::from_str(&session(&mut conn, B, "a-east").await.unwrap().unwrap()).unwrap();
		assert_eq!(held["startedAt"], json!("2026-10-31T10:00:00+02:00"), "the body keeps the phone's text");
		assert!(session(&mut conn, B, ID).await.unwrap().is_none());
	}

	/// The bytes are kept as sent, not re-serialised.
	#[test]
	fn a_session_is_taken_byte_for_byte() {
		let text = String::from("  ") + &body(&["c1"]).to_string().replace("\"cards\":", "\"cards\" : ") + "\n";
		assert_eq!(read_session(text.as_bytes(), ID).unwrap().text, text.trim());
	}

	#[test]
	fn a_session_holds_the_formats_keys_at_every_level() {
		let refused = |value: Value, id: &str| fields(&read_session(value.to_string().as_bytes(), id).unwrap_err());
		assert_eq!(refused(body(&["c1"]), "another"), ["id"]);
		let mut schema = body(&[]);
		schema["schema"] = json!("some-ui.rust-round-log/2");
		assert_eq!(refused(schema, ID), ["schema"]);
		let mut extra = body(&[]);
		extra["audio"] = json!("...");
		assert_eq!(refused(extra, ID), ["body"]);
		let mut card_extra = body(&["c1"]);
		card_extra["cards"][0]["grade"] = json!("A");
		assert_eq!(refused(card_extra, ID), ["cards[0]"]);
		let mut pin = body(&[]);
		pin["pinned"][0] = json!({ "round": "r" });
		assert_eq!(refused(pin, ID), ["pinned[0]"]);
		let mut paste = body(&[]);
		paste["fromMyDesk"][0]["pasted"] = json!("");
		assert_eq!(refused(paste, ID), ["fromMyDesk[0]"]);
		assert_eq!(
			fields(&read_session(&vec![b' '; super::SESSION_CEILING + 1], ID).unwrap_err()),
			["body"],
			"past the ceiling"
		);
	}

	#[tokio::test]
	async fn the_log_refuses_an_update_and_a_delete_and_leaves_with_the_account() {
		let pool = pool().await;
		let mut conn = pool.acquire().await.unwrap();
		put_session(&mut conn, A, &read_session(body(&["c1"]).to_string().as_bytes(), ID).unwrap(), NOW)
			.await
			.unwrap();
		assert!(sqlx::query("UPDATE rust_round_log SET body = '{}'").execute(&mut *conn).await.is_err());
		assert!(sqlx::query("DELETE FROM rust_round_log").execute(&mut *conn).await.is_err());
		sqlx::query("DELETE FROM account WHERE subject_id = ?1").bind(A).execute(&mut *conn).await.unwrap();
		let left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM rust_round_log").fetch_one(&mut *conn).await.unwrap();
		assert_eq!(left, 0);
	}
}

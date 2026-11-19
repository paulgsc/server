//! The owner's work on a week of Rust Rounds, as spans of time (RR26).
//!
//! paulgsc/some-ui#3250: the owner works on a ticket's steps (read, academy,
//! voice) mostly away from the phone. On the phone they start a work tracker,
//! which stops itself after two hours; when they come back, the phone asks
//! whether they were working the whole time (all of it, part of it, none),
//! and that answer closes the span.
//! `migrations/20261119000200_create_rust_work_span_event.up.sql` records the
//! design; in short:
//!
//! - **Two events a span, each written once** ([`start`], [`close`]): the
//!   same event again changes nothing ([`Change::Unchanged`]), another under
//!   the same span and kind is refused ([`SpanError::Rewritten`]). No row is
//!   updated or, while the account stands, deleted (the migration's
//!   triggers). The phone is the only writer.
//! - **One span open at a time** ([`open`]): a start while another span is
//!   open is refused ([`SpanError::Open`], naming it), and a close needs its
//!   start ([`SpanError::Unknown`]). The migration's triggers hold both too.
//!   The open span is the subject's newest start when it has no close, read
//!   in one indexed step.
//! - **What the owner says they worked is checked against the span**
//!   ([`close`]): at most the span's whole seconds, all of them for `all`, 0
//!   for `none`; the close no earlier than the start.
//! - **Per subject.** Every query takes the subject and filters on it.

use crate::round::Problem;
use crate::sent::{is_ahead, is_instant, json_object};
use crate::week::{is_ticket_key, is_week_id};
use crate::Change;
use serde::{Deserialize, Serialize};
use sqlx::SqliteConnection;

/// The most bytes a start or a close may hold: a few fields and a short list.
pub const SPAN_CEILING: usize = 4 * 1024;

/// The most speak-to points one span may name.
pub const MAX_POINTS: usize = 64;

/// The most characters a device's name may hold.
pub const DEVICE_MAX_CHARS: usize = 64;

/// [`SpanError::Rewritten`]'s words, for the `409`.
pub const SPAN_REWRITTEN: &str = "home holds another event under this span; a span starts once and closes once, so send it as it was";

/// [`SpanError::Open`]'s words, for the `409`.
pub const SPAN_OPEN: &str = "another span is open; close it before starting one";

/// A span's start, as the phone sends it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Start {
	/// The phone's UUID for the span.
	pub id: String,
	/// When the tracker started (RFC 3339).
	pub at: String,
	/// The week, like `2026-w47`.
	pub week: String,
	/// The ticket's key, like `TB-101`.
	pub ticket: String,
	/// `read`, `academy` or `voice`.
	pub step: String,
	/// The step's exercise, from 1, or null.
	pub exercise: Option<i64>,
	/// The speak-to points worked on, each from 1, distinct, never empty.
	pub points: Vec<i64>,
	/// The phone's own name for the device that started it.
	pub device: String,
}

/// A span's close, as the phone sends it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Close {
	/// The span's id, as its start named it.
	pub id: String,
	/// When the tracker stopped (RFC 3339).
	pub at: String,
	/// `you` or `itself`.
	pub stopped_by: String,
	/// `all`, `part` or `none`.
	pub answer: String,
	/// The seconds the owner says they worked.
	pub counted_seconds: i64,
}

/// A span as home keeps it: its start, and its close once it has one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Span {
	pub id: String,
	pub week: String,
	pub ticket: String,
	pub step: String,
	pub exercise: Option<i64>,
	pub points: Vec<i64>,
	pub device: String,
	/// When the tracker started, as the phone wrote it.
	pub started_at: String,
	/// When the start reached home.
	pub kept_at: String,
	/// The close, or null while the span is open.
	pub closed: Option<Closed>,
}

/// A span's close, as home keeps it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Closed {
	/// When the tracker stopped, as the phone wrote it.
	pub at: String,
	pub stopped_by: String,
	pub answer: String,
	pub counted_seconds: i64,
	/// When the close reached home.
	pub kept_at: String,
}

/// Why a start or a close was not stored. Nothing was written.
#[derive(Debug)]
pub enum SpanError {
	/// Another event under the same span and kind.
	Rewritten,
	/// A start while this span is open.
	Open(Box<Span>),
	/// A close of a span home holds no start of.
	Unknown,
	/// A close that does not fit its span, each problem named.
	Unfit(Vec<Problem>),
	Storage(sqlx::Error),
}

impl From<sqlx::Error> for SpanError {
	fn from(err: sqlx::Error) -> Self {
		Self::Storage(err)
	}
}

impl std::fmt::Display for SpanError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::Rewritten => f.write_str(SPAN_REWRITTEN),
			Self::Open(_) => f.write_str(SPAN_OPEN),
			Self::Unknown => f.write_str("no span home holds"),
			Self::Unfit(problems) => write!(f, "{} problem(s) with the close", problems.len()),
			Self::Storage(err) => write!(f, "storage: {err}"),
		}
	}
}

impl std::error::Error for SpanError {}

/// Whether `id` is a UUID as the phone writes one: lower-case hex, hyphenated
/// 8-4-4-4-12.
#[must_use]
pub fn is_span_id(id: &str) -> bool {
	id.len() == 36
		&& id.bytes().enumerate().all(|(at, byte)| {
			if matches!(at, 8 | 13 | 18 | 23) {
				byte == b'-'
			} else {
				byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
			}
		})
}

/// The problems with a span's id and moment, common to a start and a close.
fn id_and_at(problems: &mut Vec<Problem>, id: &str, at: &str, now: &str) {
	if !is_span_id(id) {
		problems.push((String::from("id"), "must be a UUID in lower case, like 0b0e9c1e-3f43-4c43-9a3e-5d8d6c0b6a51"));
	}
	if !is_instant(at) {
		problems.push((String::from("at"), "must be an RFC 3339 instant"));
	} else if is_ahead(at, now) {
		problems.push((String::from("at"), "is ahead of home's clock"));
	}
}

/// Read `body` as a span's start, with home's clock at `now`, or say every
/// problem found.
///
/// # Errors
/// Each [`Problem`]: a body past [`SPAN_CEILING`], not one JSON object or not
/// a start's shape; an id that is not a UUID; an `at` that is not an instant
/// or is ahead of `now`; a week that is not an ISO week, a ticket that is not
/// a key, a step that is none of the three; an exercise below 1; points
/// empty, past [`MAX_POINTS`], below 1 or repeated; a device's name empty,
/// past [`DEVICE_MAX_CHARS`] or holding a control character.
pub fn read_start(body: &[u8], now: &str) -> Result<Start, Vec<Problem>> {
	let value = json_object(body, SPAN_CEILING, "is larger than a span's start may be")?;
	let start: Start = serde_json::from_value(value).map_err(|_| {
		vec![(
			String::from("body"),
			"must be a start: id, at, week, ticket, step, exercise, points and device, and nothing else",
		)]
	})?;
	let mut problems: Vec<Problem> = Vec::new();
	id_and_at(&mut problems, &start.id, &start.at, now);
	if !is_week_id(&start.week) {
		problems.push((String::from("week"), "must be an ISO year and week, like 2026-w47"));
	}
	if !is_ticket_key(&start.ticket) {
		problems.push((String::from("ticket"), "must be a key like TB-101"));
	}
	if !matches!(start.step.as_str(), "read" | "academy" | "voice") {
		problems.push((String::from("step"), "must be read, academy or voice"));
	}
	if start.exercise.is_some_and(|exercise| exercise < 1) {
		problems.push((String::from("exercise"), "must be a number from 1, or null"));
	}
	if start.points.is_empty() || start.points.len() > MAX_POINTS {
		problems.push((String::from("points"), "must name between 1 and 64 points"));
	} else if start.points.iter().any(|point| *point < 1) {
		problems.push((String::from("points"), "must each be a number from 1"));
	} else if start.points.iter().enumerate().any(|(at, point)| start.points[..at].contains(point)) {
		problems.push((String::from("points"), "must each be named once"));
	}
	if start.device.trim().is_empty() || start.device.chars().count() > DEVICE_MAX_CHARS || start.device.chars().any(char::is_control) {
		problems.push((String::from("device"), "must be a name of 1 to 64 characters, with no control characters"));
	}
	if problems.is_empty() {
		Ok(start)
	} else {
		Err(problems)
	}
}

/// Read `body` as a span's close, with home's clock at `now`, or say every
/// problem it shows by itself. What depends on the span's start is
/// [`close`]'s.
///
/// # Errors
/// Each [`Problem`]: a body past [`SPAN_CEILING`], not one JSON object or not
/// a close's shape; an id that is not a UUID; an `at` that is not an instant
/// or is ahead of `now`; a `stoppedBy` or an `answer` that is none of its
/// words; `countedSeconds` below 0, or above 0 for `none`.
pub fn read_close(body: &[u8], now: &str) -> Result<Close, Vec<Problem>> {
	let value = json_object(body, SPAN_CEILING, "is larger than a span's close may be")?;
	let close: Close =
		serde_json::from_value(value).map_err(|_| vec![(String::from("body"), "must be a close: id, at, stoppedBy, answer and countedSeconds, and nothing else")])?;
	let mut problems: Vec<Problem> = Vec::new();
	id_and_at(&mut problems, &close.id, &close.at, now);
	if !matches!(close.stopped_by.as_str(), "you" | "itself") {
		problems.push((String::from("stoppedBy"), "must be you or itself"));
	}
	if !matches!(close.answer.as_str(), "all" | "part" | "none") {
		problems.push((String::from("answer"), "must be all, part or none"));
	}
	if close.counted_seconds < 0 {
		problems.push((String::from("countedSeconds"), "must be 0 or more"));
	} else if close.answer == "none" && close.counted_seconds != 0 {
		problems.push((String::from("countedSeconds"), "must be 0 when the answer is none"));
	}
	if problems.is_empty() {
		Ok(close)
	} else {
		Err(problems)
	}
}

/// The whole seconds from `from` to `to`, rounded down; `None` when either is
/// not an instant or `to` is before `from`.
fn whole_seconds(from: &str, to: &str) -> Option<i64> {
	let from = chrono::DateTime::parse_from_rfc3339(from).ok()?;
	let to = chrono::DateTime::parse_from_rfc3339(to).ok()?;
	let seconds = to.signed_duration_since(from).num_milliseconds().div_euclid(1000);
	(seconds >= 0).then_some(seconds)
}

/// The problems with `close` against its span's start at `started_at`.
fn unfit(close: &Close, started_at: &str) -> Vec<Problem> {
	let Some(span) = whole_seconds(started_at, &close.at) else {
		return vec![(String::from("at"), "is before the span started")];
	};
	if close.counted_seconds > span {
		vec![(String::from("countedSeconds"), "is more than the span's whole seconds")]
	} else if close.answer == "all" && close.counted_seconds != span {
		vec![(String::from("countedSeconds"), "must be the span's whole seconds when the answer is all")]
	} else {
		Vec::new()
	}
}

/// Store `start` for `subject`, or nothing when the same start is held, in
/// the caller's transaction. Answers the span as home keeps it after.
///
/// # Errors
/// [`SpanError::Rewritten`] for another start under the same span,
/// [`SpanError::Open`] while another span is open, each writing nothing;
/// [`SpanError::Storage`] for any `sqlx` failure, or a held span that no
/// longer reads.
pub async fn start(conn: &mut SqliteConnection, subject: &str, start: &Start, now: &str) -> Result<(Change, Span), SpanError> {
	if let Some(held) = span(&mut *conn, subject, &start.id).await? {
		return if started_as(&held, start) {
			Ok((Change::Unchanged, held))
		} else {
			Err(SpanError::Rewritten)
		};
	}
	if let Some(open) = open(&mut *conn, subject).await? {
		return Err(SpanError::Open(Box::new(open)));
	}
	let points = serde_json::Value::from(start.points.clone()).to_string();
	sqlx::query!(
		"INSERT INTO rust_work_span_event (subject_id, span_id, kind, at, week_id, ticket, step, exercise, points, device, kept_at)
		 VALUES (?1, ?2, 'started', ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
		subject,
		start.id,
		start.at,
		start.week,
		start.ticket,
		start.step,
		start.exercise,
		points,
		start.device,
		now
	)
	.execute(&mut *conn)
	.await?;
	let held = span(&mut *conn, subject, &start.id).await?.ok_or_else(|| sqlx::Error::RowNotFound)?;
	Ok((Change::Added, held))
}

fn started_as(held: &Span, start: &Start) -> bool {
	(&held.started_at, &held.week, &held.ticket, &held.step, held.exercise, &held.points, &held.device)
		== (&start.at, &start.week, &start.ticket, &start.step, start.exercise, &start.points, &start.device)
}

/// Store `close` for `subject`, or nothing when the same close is held, in
/// the caller's transaction. Answers the span as home keeps it after.
///
/// # Errors
/// [`SpanError::Unknown`] for a span home holds no start of,
/// [`SpanError::Rewritten`] for another close of the span,
/// [`SpanError::Unfit`] for a close that does not fit its start, each
/// writing nothing; [`SpanError::Storage`] for any `sqlx` failure, or a held
/// span that no longer reads.
pub async fn close(conn: &mut SqliteConnection, subject: &str, close: &Close, now: &str) -> Result<(Change, Span), SpanError> {
	let held = span(&mut *conn, subject, &close.id).await?.ok_or(SpanError::Unknown)?;
	if let Some(closed) = &held.closed {
		let same = (&closed.at, &closed.stopped_by, &closed.answer, closed.counted_seconds) == (&close.at, &close.stopped_by, &close.answer, close.counted_seconds);
		return if same { Ok((Change::Unchanged, held)) } else { Err(SpanError::Rewritten) };
	}
	let problems = unfit(close, &held.started_at);
	if !problems.is_empty() {
		return Err(SpanError::Unfit(problems));
	}
	sqlx::query!(
		"INSERT INTO rust_work_span_event (subject_id, span_id, kind, at, stopped_by, answer, counted_seconds, kept_at)
		 VALUES (?1, ?2, 'closed', ?3, ?4, ?5, ?6, ?7)",
		subject,
		close.id,
		close.at,
		close.stopped_by,
		close.answer,
		close.counted_seconds,
		now
	)
	.execute(&mut *conn)
	.await?;
	let held = span(&mut *conn, subject, &close.id).await?.ok_or_else(|| sqlx::Error::RowNotFound)?;
	Ok((Change::Added, held))
}

/// One span's start and close, as the queries below read them.
struct Row {
	id: String,
	week: String,
	ticket: String,
	step: String,
	exercise: Option<i64>,
	points: String,
	device: String,
	started_at: String,
	kept_at: String,
	closed_at: Option<String>,
	stopped_by: Option<String>,
	answer: Option<String>,
	counted_seconds: Option<i64>,
	closed_kept_at: Option<String>,
}

impl Row {
	fn span(self) -> Result<Span, sqlx::Error> {
		let points: Vec<i64> = serde_json::from_str(&self.points).map_err(|err| sqlx::Error::Protocol(err.to_string()))?;
		let closed = match (self.closed_at, self.stopped_by, self.answer, self.counted_seconds, self.closed_kept_at) {
			(Some(at), Some(stopped_by), Some(answer), Some(counted_seconds), Some(kept_at)) => Some(Closed {
				at,
				stopped_by,
				answer,
				counted_seconds,
				kept_at,
			}),
			(None, None, None, None, None) => None,
			_ => return Err(sqlx::Error::Protocol(String::from("a span's close does not read back"))),
		};
		Ok(Span {
			id: self.id,
			week: self.week,
			ticket: self.ticket,
			step: self.step,
			exercise: self.exercise,
			points,
			device: self.device,
			started_at: self.started_at,
			kept_at: self.kept_at,
			closed,
		})
	}
}

/// `subject`'s span `id`, or `None` when home holds no start of it.
///
/// # Errors
/// Any `sqlx` failure, or a held span that no longer reads.
pub async fn span(conn: &mut SqliteConnection, subject: &str, id: &str) -> Result<Option<Span>, sqlx::Error> {
	let row = sqlx::query_as!(
		Row,
		r#"SELECT s.span_id AS id, s.week_id AS "week!", s.ticket AS "ticket!", s.step AS "step!", s.exercise, s.points AS "points!", s.device AS "device!",
		          s.at AS started_at, s.kept_at,
		          c.at AS "closed_at?", c.stopped_by AS "stopped_by?", c.answer AS "answer?", c.counted_seconds AS "counted_seconds?", c.kept_at AS "closed_kept_at?"
		   FROM rust_work_span_event AS s
		   LEFT JOIN rust_work_span_event AS c ON c.subject_id = s.subject_id AND c.span_id = s.span_id AND c.kind = 'closed'
		   WHERE s.subject_id = ?1 AND s.span_id = ?2 AND s.kind = 'started'"#,
		subject,
		id
	)
	.fetch_optional(&mut *conn)
	.await?;
	row.map(Row::span).transpose()
}

/// `subject`'s open span: its newest start, when that has no close.
///
/// No span starts while another is open, so no older start can be open.
///
/// # Errors
/// Any `sqlx` failure, or a held span that no longer reads.
pub async fn open(conn: &mut SqliteConnection, subject: &str) -> Result<Option<Span>, sqlx::Error> {
	let newest = sqlx::query_scalar!(
		"SELECT span_id FROM rust_work_span_event INDEXED BY rust_work_span_event_by_kind WHERE subject_id = ?1 AND kind = 'started' ORDER BY rowid DESC LIMIT 1",
		subject
	)
	.fetch_optional(&mut *conn)
	.await?;
	let Some(newest) = newest else {
		return Ok(None);
	};
	Ok(span(&mut *conn, subject, &newest).await?.filter(|span| span.closed.is_none()))
}

/// `subject`'s spans of `week`, in the order they started.
///
/// That is by the instant each start's `at` names, whatever its offset, so
/// read and sorted here rather than by the stored text. A week's spans are a
/// few a day.
///
/// # Errors
/// Any `sqlx` failure, or a held span that no longer reads.
pub async fn week_spans(conn: &mut SqliteConnection, subject: &str, week: &str) -> Result<Vec<Span>, sqlx::Error> {
	let rows = sqlx::query_as!(
		Row,
		r#"SELECT s.span_id AS id, s.week_id AS "week!", s.ticket AS "ticket!", s.step AS "step!", s.exercise, s.points AS "points!", s.device AS "device!",
		          s.at AS started_at, s.kept_at,
		          c.at AS "closed_at?", c.stopped_by AS "stopped_by?", c.answer AS "answer?", c.counted_seconds AS "counted_seconds?", c.kept_at AS "closed_kept_at?"
		   FROM rust_work_span_event AS s
		   LEFT JOIN rust_work_span_event AS c ON c.subject_id = s.subject_id AND c.span_id = s.span_id AND c.kind = 'closed'
		   WHERE s.subject_id = ?1 AND s.week_id = ?2 AND s.kind = 'started'"#,
		subject,
		week
	)
	.fetch_all(&mut *conn)
	.await?;
	let mut spans = rows.into_iter().map(Row::span).collect::<Result<Vec<_>, _>>()?;
	spans.sort_by(|a, b| (crate::time_order_key(&a.started_at), &a.id).cmp(&(crate::time_order_key(&b.started_at), &b.id)));
	Ok(spans)
}

#[cfg(test)]
mod tests {
	use super::{close, open, read_close, read_start, start, week_spans, SpanError};
	use crate::Change;
	use serde_json::{json, Value};
	use sqlx::sqlite::SqlitePoolOptions;
	use sqlx::SqlitePool;

	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");
	const A: &str = "subject-a";
	const B: &str = "subject-b";
	const NOW: &str = "2026-11-19T12:00:00.000Z";
	const ONE: &str = "11111111-1111-4111-8111-111111111111";
	const TWO: &str = "22222222-2222-4222-8222-222222222222";

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

	fn start_body(id: &str, at: &str) -> Value {
		json!({
			"id": id, "at": at, "week": "2026-w47", "ticket": "TB-101", "step": "academy",
			"exercise": 2, "points": [1, 3], "device": "Pixel 8",
		})
	}

	fn close_body(id: &str, at: &str, answer: &str, counted: i64) -> Value {
		json!({ "id": id, "at": at, "stoppedBy": "itself", "answer": answer, "countedSeconds": counted })
	}

	fn fields(problems: &[(String, &str)]) -> Vec<String> {
		problems.iter().map(|(field, _)| field.clone()).collect()
	}

	#[test]
	fn a_start_names_a_week_a_ticket_a_step_and_its_points() {
		let check = |body: Value| read_start(body.to_string().as_bytes(), NOW).map_err(|problems| fields(&problems));
		let at = "2026-11-19T10:00:00.000Z";
		assert!(check(start_body(ONE, at)).is_ok());
		let with = |key: &str, value: Value| {
			let mut body = start_body(ONE, at);
			body[key] = value;
			body
		};
		assert!(check(with("exercise", Value::Null)).is_ok(), "an exercise may be null");
		assert_eq!(check(with("id", json!("ONE"))).unwrap_err(), ["id"]);
		assert_eq!(check(with("id", json!("11111111-1111-4111-8111-11111111111A"))).unwrap_err(), ["id"], "lower case only");
		assert_eq!(check(with("at", json!("2026-11-19T12:30:00.000Z"))).unwrap_err(), ["at"], "ahead of home's clock");
		assert_eq!(check(with("week", json!("2026-47"))).unwrap_err(), ["week"]);
		assert_eq!(check(with("ticket", json!("tb-101"))).unwrap_err(), ["ticket"]);
		assert_eq!(check(with("step", json!("write"))).unwrap_err(), ["step"]);
		assert_eq!(check(with("exercise", json!(0))).unwrap_err(), ["exercise"]);
		assert_eq!(check(with("points", json!([]))).unwrap_err(), ["points"]);
		assert_eq!(check(with("points", json!([0]))).unwrap_err(), ["points"]);
		assert_eq!(check(with("points", json!([2, 2]))).unwrap_err(), ["points"]);
		assert_eq!(check(with("device", json!(" "))).unwrap_err(), ["device"]);
		assert_eq!(check(with("device", json!("x".repeat(65)))).unwrap_err(), ["device"]);
		assert_eq!(check(with("closed", json!(null))).unwrap_err(), ["body"], "nothing else");
	}

	#[test]
	fn a_close_says_who_stopped_it_and_what_was_worked() {
		let check = |body: Value| read_close(body.to_string().as_bytes(), NOW).map_err(|problems| fields(&problems));
		let at = "2026-11-19T11:00:00.000Z";
		assert!(check(close_body(ONE, at, "part", 600)).is_ok());
		assert!(check(close_body(ONE, at, "none", 0)).is_ok());
		assert_eq!(check(close_body(ONE, at, "none", 1)).unwrap_err(), ["countedSeconds"], "none counts nothing");
		assert_eq!(check(close_body(ONE, at, "part", -1)).unwrap_err(), ["countedSeconds"]);
		assert_eq!(check(close_body(ONE, at, "most", 1)).unwrap_err(), ["answer"]);
		let mut body = close_body(ONE, at, "all", 1);
		body["stoppedBy"] = json!("phone");
		assert_eq!(check(body).unwrap_err(), ["stoppedBy"]);
	}

	/// A start, then its close; each again changes nothing, another under the
	/// same key is refused; and a second start waits for the first's close.
	#[tokio::test]
	async fn a_span_starts_once_closes_once_and_one_is_open_at_a_time() {
		let pool = pool().await;
		let mut conn = pool.acquire().await.unwrap();
		let first = read_start(start_body(ONE, "2026-11-19T09:00:00.000Z").to_string().as_bytes(), NOW).unwrap();
		let (change, span) = start(&mut conn, A, &first, NOW).await.unwrap();
		assert_eq!((change, span.points.as_slice(), span.closed.is_none()), (Change::Added, [1, 3].as_slice(), true));
		assert_eq!(start(&mut conn, A, &first, NOW).await.unwrap().0, Change::Unchanged);
		let mut other = first.clone();
		other.step = String::from("voice");
		assert!(matches!(start(&mut conn, A, &other, NOW).await, Err(SpanError::Rewritten)));
		assert_eq!(open(&mut conn, A).await.unwrap().map(|span| span.id), Some(String::from(ONE)));
		assert_eq!(open(&mut conn, B).await.unwrap(), None, "another subject's span is not open for B");

		let second = read_start(start_body(TWO, "2026-11-19T10:00:00.000Z").to_string().as_bytes(), NOW).unwrap();
		match start(&mut conn, A, &second, NOW).await {
			Err(SpanError::Open(open)) => assert_eq!(open.id, ONE),
			other => panic!("{other:?}"),
		}
		assert_eq!(start(&mut conn, B, &second, NOW).await.unwrap().0, Change::Added, "one open per subject");

		// Two hours to the second, as the tracker stops itself.
		let closing = read_close(close_body(ONE, "2026-11-19T11:00:00.999Z", "all", 7_200).to_string().as_bytes(), NOW).unwrap();
		let (change, span) = close(&mut conn, A, &closing, NOW).await.unwrap();
		assert_eq!((change, span.closed.as_ref().map(|closed| closed.counted_seconds)), (Change::Added, Some(7_200)));
		assert_eq!(close(&mut conn, A, &closing, NOW).await.unwrap().0, Change::Unchanged);
		let mut other = closing.clone();
		other.answer = String::from("part");
		assert!(matches!(close(&mut conn, A, &other, NOW).await, Err(SpanError::Rewritten)));
		assert_eq!(open(&mut conn, A).await.unwrap(), None);
		assert_eq!(start(&mut conn, A, &second, NOW).await.unwrap().0, Change::Added);

		let unknown = read_close(close_body("33333333-3333-4333-8333-333333333333", NOW, "none", 0).to_string().as_bytes(), NOW).unwrap();
		assert!(matches!(close(&mut conn, A, &unknown, NOW).await, Err(SpanError::Unknown)));
		let ids: Vec<String> = week_spans(&mut conn, A, "2026-w47").await.unwrap().into_iter().map(|span| span.id).collect();
		assert_eq!(ids, [ONE, TWO]);
		assert!(week_spans(&mut conn, A, "2026-w48").await.unwrap().is_empty());
	}

	/// What the owner says they worked fits the span: at most its whole
	/// seconds, all of them for `all`; and it closes no earlier than it started.
	#[tokio::test]
	async fn a_close_fits_its_span() {
		let pool = pool().await;
		let mut conn = pool.acquire().await.unwrap();
		let first = read_start(start_body(ONE, "2026-11-19T09:00:00.000Z").to_string().as_bytes(), NOW).unwrap();
		start(&mut conn, A, &first, NOW).await.unwrap();
		for (at, answer, counted, field) in [
			("2026-11-19T09:10:00.000Z", "part", 601, "countedSeconds"),
			("2026-11-19T09:10:00.000Z", "all", 599, "countedSeconds"),
			("2026-11-19T08:59:59.000Z", "none", 0, "at"),
		] {
			let closing = read_close(close_body(ONE, at, answer, counted).to_string().as_bytes(), NOW).unwrap();
			match close(&mut conn, A, &closing, NOW).await {
				Err(SpanError::Unfit(problems)) => assert_eq!(fields(&problems), [field], "{answer} {counted}"),
				other => panic!("{other:?}"),
			}
		}
		// Whatever the offset: 10:10+01:00 is ten minutes after 09:00Z.
		let closing = read_close(close_body(ONE, "2026-11-19T10:10:00.000+01:00", "part", 600).to_string().as_bytes(), NOW).unwrap();
		assert_eq!(close(&mut conn, A, &closing, NOW).await.unwrap().0, Change::Added);
	}

	/// No row is ever updated, none deleted while its account stands, and a
	/// start beside an open span or a close with no start is refused by the
	/// table itself; the account takes them all.
	#[tokio::test]
	async fn the_spans_refuse_an_update_a_delete_and_a_second_open_and_leave_with_the_account() {
		let pool = pool().await;
		let mut conn = pool.acquire().await.unwrap();
		let first = read_start(start_body(ONE, "2026-11-19T09:00:00.000Z").to_string().as_bytes(), NOW).unwrap();
		start(&mut conn, A, &first, NOW).await.unwrap();
		for statement in [
			"UPDATE rust_work_span_event SET at = 'x'",
			"DELETE FROM rust_work_span_event",
			"INSERT INTO rust_work_span_event (subject_id, span_id, kind, at, week_id, ticket, step, points, device, kept_at)
			 VALUES ('subject-a', 'other', 'started', 'now', '2026-w47', 'TB-101', 'read', '[1]', 'd', 'now')",
			"INSERT INTO rust_work_span_event (subject_id, span_id, kind, at, stopped_by, answer, counted_seconds, kept_at)
			 VALUES ('subject-a', 'never', 'closed', 'now', 'you', 'none', 0, 'now')",
			"INSERT INTO rust_work_span_event (subject_id, span_id, kind, at, stopped_by, answer, counted_seconds, kept_at)
			 VALUES ('subject-a', '11111111-1111-4111-8111-111111111111', 'closed', 'now', 'you', 'none', 5, 'now')",
		] {
			assert!(sqlx::query(statement).execute(&mut *conn).await.is_err(), "{statement}");
		}
		sqlx::query("DELETE FROM account WHERE subject_id = ?1").bind(A).execute(&mut *conn).await.unwrap();
		let left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM rust_work_span_event").fetch_one(&mut *conn).await.unwrap();
		assert_eq!(left, 0);
	}
}

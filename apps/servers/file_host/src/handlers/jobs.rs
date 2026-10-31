//! Handlers for `routes::jobs`: the validate-and-answer half. The record's own
//! rules (append-only, what an event holds) are `jobs_repo`'s.
//!
//! **Refusals.** An event that breaks a rule is a `422` naming each field and
//! why, and a different event under an id the record holds is a `409`. Both
//! write nothing, so the phone can say which event home refused and keep it.
//!
//! **No audio.** An event names its voice notes by the phone's ids, and so
//! does a file for the mailbox. A note's audio reaches home by one route only,
//! `PUT /voice-notes/:id` (`handlers::voice`, docs/identity.md invariant 23);
//! nothing here takes it, and an id is text (invariant 16).
//!
//! **The deletion hold.** Each handler takes a [`SubjectId`] first and keeps
//! it until it returns, so a write holds off `DELETE /auth/account` as every
//! other subject-scoped write does (docs/identity.md, invariant 9).
//!
//! **No other writer.** [`put_event`] and a batch's events
//! (`handlers::sync`) are the only callers of `jobs_repo::put_event`; nothing
//! writes the record in the background.
//!
//! **Digests come from the mailbox copy.** [`get_digests`] serves the daily
//! job digests home read from the Jobs mailbox, which only the `mailbox`
//! poller writes (`mailbox_repo`, docs/identity.md invariant 19), and
//! [`get_runs`] the jobs routine's run records from the same copy. Both read,
//! and write nothing; `jobs_event` still has no writer but the two above.
//!
//! **Files for the mailbox are text.** [`put_file`] files the phone's
//! decisions or scouts file for the poller to hand to the mailbox, voice
//! notes named by id included: home transcribes each note and files the
//! transcript (`mailbox::pass`, invariant 19), and the routine reads it by
//! that id, so nothing in the file points at audio and no file is routed
//! through Drive (paulgsc/some-ui#2174: home is the default).

use crate::handlers::sharing::refusing;
use crate::subject::SubjectId;
use crate::FileHostError;
use axum::{
	body::Bytes,
	extract::{Path, Query, State},
	http::HeaderMap,
	Json,
};
use chrono::{NaiveDate, SecondsFormat, Utc};
use jobs_repo::{Event, JobsError, Record, REWRITTEN};
use serde::{Deserialize, Serialize};
use serde_json::{value::RawValue, Value};
use sha2::{Digest as _, Sha256};
use sharing_repo::Scope;
use sqlx::SqlitePool;
use tracing::instrument;

/// `PUT /jobs/events/:id`
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EventWritten {
	/// `added` or `unchanged` — `jobs_repo::Change`.
	pub change: &'static str,
	/// The event as the record holds it, which the phone compares with what
	/// it sent before it counts the event as home's.
	pub event: Event,
}

fn now() -> String {
	// Fixed width, as `handlers::aph`'s.
	Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn refusal(err: JobsError) -> FileHostError {
	match err {
		JobsError::Invalid(problems) => FileHostError::unprocessable_entity(problems),
		JobsError::Rewritten => FileHostError::Conflict(REWRITTEN),
		JobsError::Unreadable => FileHostError::OperationError("a stored Jobs event does not read back".to_owned()),
		JobsError::Storage(err) => FileHostError::Sqlite(err),
	}
}

/// `GET /jobs`
///
/// # Errors
/// 401 without a session, and 500 for a storage failure.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "jobs_record", skip_all, fields(otel.kind = "server"))]
pub async fn get_record(subject: SubjectId, State(db): State<SqlitePool>) -> Result<Json<Record>, FileHostError> {
	let mut conn = db.acquire().await?;
	jobs_repo::record(&mut conn, subject.as_str()).await.map(Json).map_err(refusal)
}

/// `PUT /jobs/events/:id`, the request body being the event itself.
/// `BEGIN IMMEDIATE`, so the read that decides and the write see one
/// snapshot.
///
/// # Errors
/// 401 without a session, 403 from an untrusted origin, 422 naming each
/// refused field, 409 for a different event under a held id or from a paired
/// phone that stopped sending the record home (`handlers::sharing::refusing`),
/// and 500 for a storage failure.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "jobs_event_put", skip_all, fields(otel.kind = "server"))]
pub async fn put_event(
	subject: SubjectId,
	State(db): State<SqlitePool>,
	headers: HeaderMap,
	Path(id): Path<String>,
	body: Bytes,
) -> Result<Json<EventWritten>, FileHostError> {
	let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
	refusing(&mut tx, subject.as_str(), &headers, Scope::Jobs).await?;
	let (change, event) = jobs_repo::put_event(&mut tx, subject.as_str(), &id, &body, &now()).await.map_err(refusal)?;
	tx.commit().await?;
	Ok(Json(EventWritten { change: change.as_str(), event }))
}

/// The most digests one answer holds: the phone's own cap on what one run
/// takes (`DIGESTS_PER_RUN` in paulgsc/some-ui `apps/www/src/lib/jobs/model.ts`).
pub const DIGESTS_PER_ANSWER: i64 = 7;

/// `GET /jobs/digests`' query: the newest day the phone holds, if any.
#[derive(Debug, Deserialize)]
pub struct DigestsQuery {
	after: Option<String>,
}

/// `GET /jobs/digests`
#[derive(Debug, Serialize)]
pub struct Digests {
	/// Oldest day first.
	pub digests: Vec<HeldDigest>,
}

#[derive(Debug, Serialize)]
pub struct HeldDigest {
	/// `YYYY-MM-DD`, always a real date.
	pub day: String,
	/// The digest file as the mailbox held it, verbatim.
	pub digest: Box<RawValue>,
}

/// Whether `text` is a day as the phone writes one: `YYYY-MM-DD`, a real date.
fn is_day(text: &str) -> bool {
	let shape = text.len() == 10
		&& text
			.bytes()
			.enumerate()
			.all(|(at, byte)| if at == 4 || at == 7 { byte == b'-' } else { byte.is_ascii_digit() });
	shape && NaiveDate::parse_from_str(text, "%Y-%m-%d").is_ok()
}

/// `GET /jobs/digests?after=YYYY-MM-DD`: the daily job digests home holds
/// for the caller, oldest first, at most [`DIGESTS_PER_ANSWER`]. With
/// `after`, the days after it; without, the newest days.
///
/// # Errors
/// 401 without a session, 422 for an `after` that is not a day, and 500 for
/// a storage failure or a held digest that does not read back.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "jobs_digests", skip_all, fields(otel.kind = "server"))]
pub async fn get_digests(subject: SubjectId, State(db): State<SqlitePool>, Query(query): Query<DigestsQuery>) -> Result<Json<Digests>, FileHostError> {
	if let Some(after) = &query.after {
		if !is_day(after) {
			return Err(FileHostError::unprocessable_entity([("after", "not a day as YYYY-MM-DD")]));
		}
	}
	let mut conn = db.acquire().await?;
	let held = mailbox_repo::digests_after(&mut conn, subject.as_str(), query.after.as_deref(), DIGESTS_PER_ANSWER)
		.await
		.map_err(|err| match err {
			mailbox_repo::MailboxError::Storage(err) => FileHostError::Sqlite(err),
			mailbox_repo::MailboxError::Rewritten => FileHostError::OperationError("a read cannot rewrite".to_owned()),
		})?;
	let digests = held
		.into_iter()
		.map(|held| {
			let text = String::from_utf8(held.body).map_err(|_| ())?;
			let digest = RawValue::from_string(text).map_err(|_| ())?;
			Ok(HeldDigest { day: held.day, digest })
		})
		.collect::<Result<Vec<_>, ()>>()
		.map_err(|()| FileHostError::OperationError("a held digest does not read back".to_owned()))?;
	Ok(Json(Digests { digests }))
}

/// The most run records one answer holds: a week of mornings, as many as the
/// phone keeps (`RUNS_KEPT` in paulgsc/some-ui `apps/www/src/lib/jobs/model.ts`).
pub const RUNS_PER_ANSWER: i64 = 7;

/// `GET /jobs/runs`
#[derive(Debug, Serialize)]
pub struct Runs {
	/// Newest first.
	pub runs: Vec<HeldRun>,
}

#[derive(Debug, Serialize)]
pub struct HeldRun {
	/// Its name in the mailbox, `run-jobs-<stamp>.json`.
	pub name: String,
	/// The run record as the mailbox held it, verbatim.
	pub run: Box<RawValue>,
}

/// `GET /jobs/runs`: the jobs routine's newest run records home holds.
///
/// The caller's, newest first, at most [`RUNS_PER_ANSWER`]: what each
/// morning run found and added, and what it couldn't do, a stopped run
/// included.
///
/// A held record that does not read back as JSON is left out and logged, so
/// one bad record never hides the rest.
///
/// # Errors
/// 401 without a session, and 500 for a storage failure.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "jobs_runs", skip_all, fields(otel.kind = "server"))]
pub async fn get_runs(subject: SubjectId, State(db): State<SqlitePool>) -> Result<Json<Runs>, FileHostError> {
	let mut conn = db.acquire().await?;
	let held = mailbox_repo::jobs_runs(&mut conn, subject.as_str(), RUNS_PER_ANSWER).await.map_err(|err| match err {
		mailbox_repo::MailboxError::Storage(err) => FileHostError::Sqlite(err),
		mailbox_repo::MailboxError::Rewritten => FileHostError::OperationError("a read cannot rewrite".to_owned()),
	})?;
	let runs = held
		.into_iter()
		.filter_map(|held| {
			// The poller kept only JSON (invariant 19), so a body that fails
			// here was written some other way: say so by name, never by body.
			let run = String::from_utf8(held.body)
				.map_err(|_| "utf8")
				.and_then(|text| RawValue::from_string(text).map_err(|_| "json"));
			match run {
				Ok(run) => Some(HeldRun { name: held.name, run }),
				Err(kind) => {
					tracing::error!(name = %held.name, kind, "a held jobs run record does not read back; left out");
					None
				}
			}
		})
		.collect();
	Ok(Json(Runs { runs }))
}

/// The longest FCM registration token taken: `jobs_notice_device`'s own
/// ceiling. Tokens are about 160 characters today.
pub const NOTICE_TOKEN_CEILING: usize = 4096;

/// `PUT` and `DELETE /jobs/notice`: one phone's FCM registration token.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NoticeDevice {
	pub token: String,
}

/// `PUT` and `DELETE /jobs/notice`
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NoticeChanged {
	/// `added` or `unchanged` for a `PUT`, `removed` or `unchanged` for a
	/// `DELETE`.
	pub change: &'static str,
}

/// The token in `body`, if it reads as one: `{ "token": … }`, printable
/// ASCII, no spaces, at most [`NOTICE_TOKEN_CEILING`]. Read from the bytes,
/// as the other Jobs writes are, so a phone's request needs no content type.
/// What it says is never logged (docs/identity.md invariant 24).
fn notice_token(body: &[u8]) -> Result<String, FileHostError> {
	let refused = || FileHostError::unprocessable_entity([("token", "not an FCM registration token")]);
	let device: NoticeDevice = serde_json::from_slice(body).map_err(|_| refused())?;
	let token = device.token;
	if token.is_empty() || token.len() > NOTICE_TOKEN_CEILING || !token.bytes().all(|byte| byte.is_ascii_graphic()) {
		return Err(refused());
	}
	Ok(token)
}

/// `PUT /jobs/notice`: tell this phone when a morning run's record arrives
/// (`jobs_notice`). Asking again changes nothing.
///
/// # Errors
/// 401 without a session, 403 from an untrusted origin, 422 for a token that
/// is not one, 409 from a paired phone that stopped sending Jobs home
/// (`handlers::sharing::refusing`), and 500 for a storage failure.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "jobs_notice_put", skip_all, fields(otel.kind = "server"))]
pub async fn put_notice(subject: SubjectId, State(db): State<SqlitePool>, headers: HeaderMap, body: Bytes) -> Result<Json<NoticeChanged>, FileHostError> {
	let token = notice_token(&body)?;
	let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
	refusing(&mut tx, subject.as_str(), &headers, Scope::Jobs).await?;
	let change = match notice_repo::register(&mut tx, subject.as_str(), &token, &now()).await? {
		notice_repo::Change::Added => "added",
		notice_repo::Change::Unchanged => "unchanged",
		// `SubjectId` found the account, so this is not reachable; if it
		// were, nothing was written.
		notice_repo::Change::NoAccount => return Err(FileHostError::OperationError("the account is gone".to_owned())),
	};
	tx.commit().await?;
	Ok(Json(NoticeChanged { change }))
}

/// `DELETE /jobs/notice`: stop telling this phone. Never refused for sharing:
/// a phone that stopped sending Jobs home can always stop being told.
///
/// # Errors
/// 401 without a session, 403 from an untrusted origin, 422 for a token that
/// is not one, and 500 for a storage failure.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "jobs_notice_delete", skip_all, fields(otel.kind = "server"))]
pub async fn delete_notice(subject: SubjectId, State(db): State<SqlitePool>, body: Bytes) -> Result<Json<NoticeChanged>, FileHostError> {
	let token = notice_token(&body)?;
	let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
	let removed = notice_repo::forget(&mut tx, subject.as_str(), &token).await?;
	tx.commit().await?;
	Ok(Json(NoticeChanged {
		change: if removed { "removed" } else { "unchanged" },
	}))
}

/// The most a filed file may hold: the Jobs record's own body ceiling.
pub const FILE_CEILING: usize = jobs_repo::EVENT_BODY_CEILING;

/// `PUT /jobs/files/:name`
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FileFiled {
	/// `added` or `unchanged`.
	pub change: &'static str,
}

/// The mailbox folder a file's name puts it in, and the schema it must
/// carry: `decisions-YYYY-MM-DD(-n).json` or `scouts-YYYYMMDDTHHMMSS.json`,
/// the mailbox's own patterns (paulgsc/some-ui `apps/mailbox`).
fn folder_of(name: &str) -> Option<(&'static str, &'static str)> {
	// The patterns are ASCII, and the byte offsets below assume it.
	if !name.is_ascii() {
		return None;
	}
	let digits = |text: &str, count: usize| text.len() == count && text.bytes().all(|byte| byte.is_ascii_digit());
	if let Some(rest) = name.strip_prefix("decisions-").and_then(|rest| rest.strip_suffix(".json")) {
		let (day, nth) = rest.split_at(rest.len().min(10));
		let nth_ok = nth.is_empty()
			|| nth
				.strip_prefix('-')
				.is_some_and(|n| !n.is_empty() && n.len() <= 6 && n.bytes().all(|byte| byte.is_ascii_digit()));
		return (is_day(day) && nth_ok).then_some(("decisions", "some-ui.job-decisions/1"));
	}
	let stamp = name.strip_prefix("scouts-")?.strip_suffix(".json")?;
	let (date, time) = stamp.split_once('T')?;
	(digits(date, 8) && digits(time, 6)).then_some(("scout", "some-ui.job-scouts/1"))
}

/// Reads a body that holds each object's members once. A duplicate member is
/// refused rather than read the way `serde_json::Value` does (the last one
/// wins): the bytes stored and handed on are the body as sent, so what is
/// checked (the `schema`) has to be every member those bytes hold.
struct UniqueMembers;

impl<'de> Deserialize<'de> for UniqueMembers {
	fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
		deserializer.deserialize_any(Self)
	}
}

impl<'de> serde::de::Visitor<'de> for UniqueMembers {
	type Value = Self;

	fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.write_str("JSON with no duplicate member")
	}

	fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> Result<Self, A::Error> {
		let mut seen = std::collections::HashSet::new();
		while let Some(key) = map.next_key::<String>()? {
			if !seen.insert(key) {
				return Err(serde::de::Error::custom("a duplicate member"));
			}
			map.next_value::<Self>()?;
		}
		Ok(Self)
	}

	fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<Self, A::Error> {
		while seq.next_element::<Self>()?.is_some() {}
		Ok(Self)
	}

	fn visit_bool<E>(self, _: bool) -> Result<Self, E> {
		Ok(Self)
	}

	fn visit_i64<E>(self, _: i64) -> Result<Self, E> {
		Ok(Self)
	}

	fn visit_u64<E>(self, _: u64) -> Result<Self, E> {
		Ok(Self)
	}

	fn visit_f64<E>(self, _: f64) -> Result<Self, E> {
		Ok(Self)
	}

	fn visit_str<E>(self, _: &str) -> Result<Self, E> {
		Ok(Self)
	}

	fn visit_unit<E>(self) -> Result<Self, E> {
		Ok(Self)
	}
}

/// `PUT /jobs/files/:name`, the body being the file itself.
///
/// A decisions or scouts file, filed as sent for the mailbox poller to hand
/// on; a voice note it names by id is text, and its transcript goes the same
/// way. `BEGIN IMMEDIATE`, so the read that decides and the write see one
/// snapshot.
///
/// # Errors
/// 401 without a session, 403 from an untrusted origin, 422 naming what is
/// wrong (`name`, `body` or `schema`), 409 for other bytes under a
/// filed name or from a paired phone that stopped sending the record home,
/// and 500 for a storage failure.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "jobs_file_put", skip_all, fields(otel.kind = "server"))]
pub async fn put_file(subject: SubjectId, State(db): State<SqlitePool>, headers: HeaderMap, Path(name): Path<String>, body: Bytes) -> Result<Json<FileFiled>, FileHostError> {
	let Some((folder, schema)) = folder_of(&name) else {
		return Err(FileHostError::unprocessable_entity([("name", "not a decisions or scouts file name")]));
	};
	if body.len() > FILE_CEILING {
		return Err(FileHostError::unprocessable_entity([("body", "over 256 KiB")]));
	}
	let Ok(file) = serde_json::from_slice::<Value>(&body) else {
		return Err(FileHostError::unprocessable_entity([("body", "not JSON")]));
	};
	if serde_json::from_slice::<UniqueMembers>(&body).is_err() {
		return Err(FileHostError::unprocessable_entity([("body", "an object holds a member twice")]));
	}
	if file.get("schema").and_then(Value::as_str) != Some(schema) {
		return Err(FileHostError::unprocessable_entity([("schema", "not the schema this file's name calls for")]));
	}
	let sha256 = hex::encode(Sha256::digest(&body));
	let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
	refusing(&mut tx, subject.as_str(), &headers, Scope::Jobs).await?;
	let change = mailbox_repo::file_outgoing(&mut tx, subject.as_str(), folder, &name, &sha256, &body, &now())
		.await
		.map_err(|err| match err {
			mailbox_repo::MailboxError::Rewritten => FileHostError::Conflict("this name already holds a different file: what home hands the mailbox is never replaced"),
			mailbox_repo::MailboxError::Storage(err) => FileHostError::Sqlite(err),
		})?;
	let change = match change {
		mailbox_repo::Change::Added => "added",
		mailbox_repo::Change::Unchanged => "unchanged",
		// `SubjectId` holds the deletion lock and found the account, so this
		// is not reachable; if it were, nothing was written.
		mailbox_repo::Change::NoAccount => return Err(FileHostError::OperationError("the account is gone".to_owned())),
	};
	tx.commit().await?;
	Ok(Json(FileFiled { change }))
}

#[cfg(test)]
mod tests {
	//! Through the real router, with a real phone session (a bearer token, as
	//! a paired phone sends) and a browser's.

	use crate::auth::{
		cookie::{Carrier, SessionToken},
		now, AuthContext,
	};
	use crate::routes::table::Module;
	use axum::{
		body::Body,
		extract::FromRef,
		http::{
			header::{AUTHORIZATION, COOKIE},
			Method, Request, StatusCode,
		},
		Router,
	};
	use serde_json::{json, Value};
	use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};
	use tower::ServiceExt;

	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");
	const A: &str = "subject-a";
	const B: &str = "subject-b";

	#[derive(Clone)]
	struct TestState {
		pool: SqlitePool,
		auth: AuthContext,
	}

	impl FromRef<TestState> for SqlitePool {
		fn from_ref(state: &TestState) -> Self {
			state.pool.clone()
		}
	}

	impl FromRef<TestState> for AuthContext {
		fn from_ref(state: &TestState) -> Self {
			state.auth.clone()
		}
	}

	fn module() -> Module<TestState> {
		crate::routes::jobs::jobs()
	}

	async fn state() -> TestState {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();
		TestState {
			auth: AuthContext::for_tests(pool.clone(), 100),
			pool,
		}
	}

	/// An account for `subject` and a live session on it, carried as `carrier`
	/// carries it: the header a request sends it in.
	async fn session(state: &TestState, subject: &str, carrier: Carrier) -> (axum::http::HeaderName, String) {
		sqlx::query("INSERT OR IGNORE INTO account (subject_id, user_handle) VALUES (?1, ?2)")
			.bind(subject)
			.bind(subject.as_bytes())
			.execute(&state.pool)
			.await
			.unwrap();
		let token = SessionToken::mint();
		let now = now();
		assert!(state.auth.repository().create_session(&token.hash(carrier), subject, now + 3_600, now).await.unwrap());
		match carrier {
			Carrier::Bearer => (AUTHORIZATION, String::from("Bearer ") + &token.encoded()),
			Carrier::Cookie => (COOKIE, token.set_cookie(3_600).to_str().unwrap().split(';').next().unwrap().to_owned()),
		}
	}

	fn app(state: TestState) -> Router {
		module().into_table_router().with_state(state)
	}

	async fn call(app: &Router, method: Method, path: &str, auth: Option<&(axum::http::HeaderName, String)>, body: &Value) -> (StatusCode, Value) {
		let mut request = Request::builder().method(method).uri(path);
		if let Some((name, value)) = auth {
			request = request.header(name, value);
		}
		let text = if body.is_null() { String::new() } else { body.to_string() };
		let response = app.clone().oneshot(request.body(Body::from(text)).unwrap()).await.unwrap();
		let status = response.status();
		let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
		(status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
	}

	fn passed(id: &str) -> Value {
		json!({
			"id": id, "at": "2026-10-18T22:10:00.000Z", "type": "passed",
			"day": "2026-10-18", "pickId": "avm-0b77", "fingerprint": "avmet applications|software engineer|remote",
			"reasons": ["seniority"], "note": "Clearance for most contracts", "voice": ["0b6f0f8e-1f2a-4c55-9d7e-3a1b2c3d4e5f"]
		})
	}

	#[tokio::test]
	async fn every_route_needs_a_session_and_writes_need_a_trusted_origin() {
		let state = state().await;
		let phone = session(&state, A, Carrier::Bearer).await;
		let app = app(state);
		let routes: Vec<(Method, String)> = module()
			.descriptors()
			.map(|route| {
				(
					Method::from_bytes(route.method.as_bytes()).unwrap(),
					route.path.replace(":id", "e1").replace(":name", "decisions-2026-10-23.json"),
				)
			})
			.collect();
		assert_eq!(routes.len(), 7, "{routes:?}");
		for (method, path) in routes {
			assert_eq!(call(&app, method.clone(), &path, None, &json!({})).await.0, StatusCode::UNAUTHORIZED, "{method} {path}");
		}
		let request = Request::builder()
			.method(Method::PUT)
			.uri("/jobs/events/e1")
			.header(&phone.0, &phone.1)
			.header("origin", "https://evil.app.test")
			.header("sec-fetch-site", "cross-site")
			.body(Body::from(passed("e1").to_string()))
			.unwrap();
		assert_eq!(app.clone().oneshot(request).await.unwrap().status(), StatusCode::FORBIDDEN);
	}

	/// The phone's whole round: an empty record, an event added, the same
	/// event unchanged, and the record read back. A browser session on the
	/// same account reads the same record.
	#[tokio::test]
	async fn a_paired_phone_keeps_its_record_and_reads_it_back() {
		let state = state().await;
		let phone = session(&state, A, Carrier::Bearer).await;
		let browser = session(&state, A, Carrier::Cookie).await;
		let app = app(state);

		assert_eq!(
			call(&app, Method::GET, "/jobs", Some(&phone), &Value::Null).await,
			(StatusCode::OK, json!({ "events": [] }))
		);
		assert_eq!(
			call(&app, Method::PUT, "/jobs/events/e1", Some(&phone), &passed("e1")).await,
			(StatusCode::OK, json!({ "change": "added", "event": passed("e1") }))
		);
		assert_eq!(call(&app, Method::PUT, "/jobs/events/e1", Some(&phone), &passed("e1")).await.1["change"], "unchanged");
		let record = json!({ "events": [passed("e1")] });
		assert_eq!(call(&app, Method::GET, "/jobs", Some(&phone), &Value::Null).await, (StatusCode::OK, record.clone()));
		assert_eq!(call(&app, Method::GET, "/jobs", Some(&browser), &Value::Null).await, (StatusCode::OK, record));
	}

	/// A write that breaks a rule is a 422 naming each field, a different
	/// event under a held id is a 409, and one subject's record is never
	/// another's.
	#[tokio::test]
	async fn refusals_name_their_fields_and_records_stay_apart() {
		let state = state().await;
		let a = session(&state, A, Carrier::Bearer).await;
		let b = session(&state, B, Carrier::Bearer).await;
		let app = app(state);
		assert_eq!(call(&app, Method::PUT, "/jobs/events/e1", Some(&a), &passed("e1")).await.0, StatusCode::OK);

		let named = |answer: (StatusCode, Value)| {
			assert_eq!(answer.0, StatusCode::UNPROCESSABLE_ENTITY, "{}", answer.1);
			let mut fields: Vec<String> = answer.1["error"]["details"].as_object().unwrap().keys().cloned().collect();
			fields.sort();
			fields
		};
		let mut wrong = passed("e2");
		wrong["day"] = json!("tomorrow");
		wrong["note"] = json!("x".repeat(2_001));
		assert_eq!(named(call(&app, Method::PUT, "/jobs/events/e2", Some(&a), &wrong).await), ["day", "note"]);
		assert_eq!(named(call(&app, Method::PUT, "/jobs/events/other", Some(&a), &passed("e3")).await), ["id"]);
		assert_eq!(named(call(&app, Method::PUT, "/jobs/events/e4", Some(&a), &json!("event")).await), ["body"]);

		let mut rewritten = passed("e1");
		rewritten["reasons"] = json!(["pay"]);
		let (status, answer) = call(&app, Method::PUT, "/jobs/events/e1", Some(&a), &rewritten).await;
		assert_eq!(status, StatusCode::CONFLICT, "{answer}");
		assert_eq!(answer["error"]["code"], "conflict");

		assert_eq!(
			call(&app, Method::GET, "/jobs", Some(&b), &Value::Null).await.1,
			json!({ "events": [] }),
			"B reads none of A's"
		);
		assert_eq!(
			call(&app, Method::GET, "/jobs", Some(&a), &Value::Null).await.1,
			json!({ "events": [passed("e1")] }),
			"nothing refused was written"
		);
	}

	/// A digest home read from the mailbox for `subject`, as the poller keeps
	/// one (`mailbox_repo::keep_item`).
	async fn hold(state: &TestState, subject: &str, day: &str) -> Value {
		let digest = json!({ "schema": "some-ui.job-digest/1", "day": day, "picks": [{ "id": String::from("p-") + day }] });
		sqlx::query("INSERT INTO mailbox_item (subject_id, folder, name, seq, sha256, size, body, pulled_at) VALUES (?1, 'brief', ?2, ?3, 's', 0, ?4, 'now')")
			.bind(subject)
			.bind(String::from("digest-") + day + ".json")
			.bind(day.replace('-', "").parse::<i64>().unwrap())
			.bind(digest.to_string().into_bytes())
			.execute(&state.pool)
			.await
			.unwrap();
		json!({ "day": day, "digest": digest })
	}

	fn days(answer: &Value) -> Vec<&str> {
		answer["digests"].as_array().unwrap().iter().map(|held| held["day"].as_str().unwrap()).collect()
	}

	#[tokio::test]
	async fn digests_needs_a_session() {
		let state = state().await;
		hold(&state, A, "2026-10-22").await;
		let app = app(state);
		assert_eq!(call(&app, Method::GET, "/jobs/digests", None, &Value::Null).await.0, StatusCode::UNAUTHORIZED);
	}

	#[tokio::test]
	async fn digests_answers_nothing_when_none_are_held() {
		let state = state().await;
		let phone = session(&state, A, Carrier::Bearer).await;
		let app = app(state);
		assert_eq!(
			call(&app, Method::GET, "/jobs/digests", Some(&phone), &Value::Null).await,
			(StatusCode::OK, json!({ "digests": [] }))
		);
	}

	/// The phone asks after the newest day it holds, and takes each digest
	/// verbatim: the body is the file the mailbox held, not a re-encoding.
	#[tokio::test]
	async fn digests_after_a_day_answers_only_later_days_oldest_first() {
		let state = state().await;
		let phone = session(&state, A, Carrier::Bearer).await;
		let mut held = Vec::new();
		for day in ["2026-10-23", "2026-10-21", "2026-10-22"] {
			held.push(hold(&state, A, day).await);
		}
		let app = app(state);
		let (status, answer) = call(&app, Method::GET, "/jobs/digests?after=2026-10-21", Some(&phone), &Value::Null).await;
		assert_eq!(status, StatusCode::OK);
		assert_eq!(answer, json!({ "digests": [held[2], held[0]] }));
		let newest = call(&app, Method::GET, "/jobs/digests?after=2026-10-23", Some(&phone), &Value::Null).await;
		assert_eq!(newest, (StatusCode::OK, json!({ "digests": [] })));
	}

	#[tokio::test]
	async fn digests_answers_at_most_seven() {
		let state = state().await;
		let phone = session(&state, A, Carrier::Bearer).await;
		for day in 10..=19 {
			hold(&state, A, &(String::from("2026-10-") + &day.to_string())).await;
		}
		let app = app(state);
		let (_, after) = call(&app, Method::GET, "/jobs/digests?after=2026-10-09", Some(&phone), &Value::Null).await;
		assert_eq!(
			days(&after),
			["2026-10-10", "2026-10-11", "2026-10-12", "2026-10-13", "2026-10-14", "2026-10-15", "2026-10-16"],
			"the oldest seven after the day"
		);
		let (_, newest) = call(&app, Method::GET, "/jobs/digests", Some(&phone), &Value::Null).await;
		assert_eq!(
			days(&newest),
			["2026-10-13", "2026-10-14", "2026-10-15", "2026-10-16", "2026-10-17", "2026-10-18", "2026-10-19"],
			"with no day, the newest seven"
		);
	}

	#[tokio::test]
	async fn digests_answers_only_the_callers_own() {
		let state = state().await;
		let a = session(&state, A, Carrier::Bearer).await;
		let b = session(&state, B, Carrier::Bearer).await;
		hold(&state, A, "2026-10-22").await;
		let app = app(state);
		assert_eq!(days(&call(&app, Method::GET, "/jobs/digests", Some(&a), &Value::Null).await.1), ["2026-10-22"]);
		assert_eq!(
			call(&app, Method::GET, "/jobs/digests", Some(&b), &Value::Null).await.1,
			json!({ "digests": [] }),
			"B reads none of A's"
		);
	}

	#[tokio::test]
	async fn digests_refuses_a_malformed_after() {
		let state = state().await;
		let phone = session(&state, A, Carrier::Bearer).await;
		let app = app(state);
		for after in ["", "2026-10-2", "2026-13-01", "2026-02-30", "20261022", "2026-10-22T00:00", "tomorrow"] {
			let (status, answer) = call(&app, Method::GET, &(String::from("/jobs/digests?after=") + after), Some(&phone), &Value::Null).await;
			assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{after}: {answer}");
			assert!(answer["error"]["details"]["after"].is_array(), "{answer}");
		}
	}

	/// A run record home read from the mailbox's `run` folder for `subject`,
	/// under `name`, as the poller keeps one.
	async fn hold_run(state: &TestState, subject: &str, name: &str, seq: i64, body: &[u8]) {
		sqlx::query("INSERT INTO mailbox_item (subject_id, folder, name, seq, sha256, size, body, pulled_at) VALUES (?1, 'run', ?2, ?3, 's', 0, ?4, 'now')")
			.bind(subject)
			.bind(name)
			.bind(seq)
			.bind(body)
			.execute(&state.pool)
			.await
			.unwrap();
	}

	fn run(outcome: &str) -> Value {
		json!({ "schema": "some-ui.routine-run/1", "routine": "jobs", "outcome": outcome, "problems": [] })
	}

	fn names(answer: &Value) -> Vec<&str> {
		answer["runs"].as_array().unwrap().iter().map(|held| held["name"].as_str().unwrap()).collect()
	}

	/// Only the jobs routine's records, only the caller's, newest first, each
	/// verbatim: the reflections routine's runs share the folder.
	#[tokio::test]
	async fn runs_answer_the_callers_jobs_runs_newest_first() {
		let state = state().await;
		let a = session(&state, A, Carrier::Bearer).await;
		let b = session(&state, B, Carrier::Bearer).await;
		hold_run(&state, A, "run-jobs-20261025T145200Z.json", 1, run("done").to_string().as_bytes()).await;
		hold_run(&state, A, "run-reflections-20261025T160454Z.json", 2, b"{\"routine\":\"reflections\"}").await;
		hold_run(&state, A, "run-jobs-20261025T170000Z.json", 3, run("stopped").to_string().as_bytes()).await;
		hold(&state, A, "2026-10-25").await;
		let app = app(state);
		let (status, answer) = call(&app, Method::GET, "/jobs/runs", Some(&a), &Value::Null).await;
		assert_eq!(status, StatusCode::OK);
		assert_eq!(
			answer,
			json!({ "runs": [
				{ "name": "run-jobs-20261025T170000Z.json", "run": run("stopped") },
				{ "name": "run-jobs-20261025T145200Z.json", "run": run("done") },
			] })
		);
		assert_eq!(
			call(&app, Method::GET, "/jobs/runs", Some(&b), &Value::Null).await.1,
			json!({ "runs": [] }),
			"B reads none of A's"
		);
	}

	#[tokio::test]
	async fn runs_answer_at_most_a_week_and_skip_one_that_does_not_read() {
		let state = state().await;
		let phone = session(&state, A, Carrier::Bearer).await;
		for seq in 1..=10 {
			hold_run(
				&state,
				A,
				&(String::from("run-jobs-") + &seq.to_string() + ".json"),
				seq,
				run("done").to_string().as_bytes(),
			)
			.await;
		}
		hold_run(&state, A, "run-jobs-11.json", 11, b"not json").await;
		let app = app(state);
		let (status, answer) = call(&app, Method::GET, "/jobs/runs", Some(&phone), &Value::Null).await;
		assert_eq!(status, StatusCode::OK);
		assert_eq!(
			names(&answer),
			[
				"run-jobs-10.json",
				"run-jobs-9.json",
				"run-jobs-8.json",
				"run-jobs-7.json",
				"run-jobs-6.json",
				"run-jobs-5.json"
			],
			"the newest seven, less the one that does not read"
		);
	}

	#[tokio::test]
	async fn a_phone_asks_to_be_told_once_and_can_stop() {
		let state = state().await;
		let a = session(&state, A, Carrier::Bearer).await;
		let b = session(&state, B, Carrier::Bearer).await;
		let app = app(state);
		let token = json!({ "token": "dK3x:APA91bH-phone_token.1" });
		assert_eq!(
			call(&app, Method::PUT, "/jobs/notice", Some(&a), &token).await,
			(StatusCode::OK, json!({ "change": "added" }))
		);
		assert_eq!(
			call(&app, Method::PUT, "/jobs/notice", Some(&a), &token).await,
			(StatusCode::OK, json!({ "change": "unchanged" }))
		);
		assert_eq!(
			call(&app, Method::DELETE, "/jobs/notice", Some(&b), &token).await,
			(StatusCode::OK, json!({ "change": "unchanged" })),
			"B cannot stop A's phone"
		);
		assert_eq!(
			call(&app, Method::DELETE, "/jobs/notice", Some(&a), &token).await,
			(StatusCode::OK, json!({ "change": "removed" }))
		);
		assert_eq!(
			call(&app, Method::DELETE, "/jobs/notice", Some(&a), &token).await,
			(StatusCode::OK, json!({ "change": "unchanged" }))
		);
	}

	#[tokio::test]
	async fn notice_refuses_what_is_not_a_token() {
		let state = state().await;
		let phone = session(&state, A, Carrier::Bearer).await;
		let app = app(state);
		let long = "x".repeat(super::NOTICE_TOKEN_CEILING + 1);
		for token in ["", "has a space", "line\nbreak", long.as_str()] {
			for method in [Method::PUT, Method::DELETE] {
				let (status, answer) = call(&app, method.clone(), "/jobs/notice", Some(&phone), &json!({ "token": token })).await;
				assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{method} {token:?}: {answer}");
				assert!(answer["error"]["details"]["token"].is_array(), "{answer}");
			}
		}
		let (status, _) = call(&app, Method::PUT, "/jobs/notice", Some(&phone), &json!({ "token": "t", "topics": [] })).await;
		assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "an unknown field is refused");
	}

	fn decisions(day: &str, voice: &[&str]) -> Value {
		json!({
			"schema": "some-ui.job-decisions/1", "day": day, "digestDay": day, "sharedAt": "2026-10-23T22:00:00.000Z",
			"picks": [{
				"day": day, "pickId": "blu-7f2a", "fingerprint": "bluesight|data analyst|remote", "id": "e1",
				"decision": "taken", "reasons": [], "note": "", "voice": [],
				"debrief": { "id": "e2", "minutes": 20, "friction": 2, "frictionChips": [], "what": "", "voice": voice, "result": "submitted" },
				"fit": []
			}],
			"disagreements": [], "outcomes": []
		})
	}

	async fn filed(state: &TestState) -> Vec<(String, String)> {
		sqlx::query_as("SELECT folder, name FROM mailbox_outgoing ORDER BY rowid")
			.fetch_all(&state.pool)
			.await
			.unwrap()
	}

	#[tokio::test]
	async fn files_take_a_file_once_and_never_replace_it() {
		let state = state().await;
		let phone = session(&state, A, Carrier::Bearer).await;
		let app = app(state.clone());
		let path = "/jobs/files/decisions-2026-10-23.json";
		assert_eq!(
			call(&app, Method::PUT, path, Some(&phone), &decisions("2026-10-23", &[])).await,
			(StatusCode::OK, json!({ "change": "added" }))
		);
		assert_eq!(
			call(&app, Method::PUT, path, Some(&phone), &decisions("2026-10-23", &[])).await.1,
			json!({ "change": "unchanged" })
		);
		let (status, answer) = call(&app, Method::PUT, path, Some(&phone), &decisions("2026-10-22", &[])).await;
		assert_eq!((status, &answer["error"]["code"]), (StatusCode::CONFLICT, &json!("conflict")), "{answer}");
		let scouts = json!({ "schema": "some-ui.job-scouts/1", "sharedAt": "2026-10-23T22:00:00.000Z", "scouts": [{ "voice": [] }] });
		assert_eq!(
			call(&app, Method::PUT, "/jobs/files/scouts-20261023T101500.json", Some(&phone), &scouts).await.0,
			StatusCode::OK
		);
		assert_eq!(
			filed(&state).await,
			[
				(String::from("decisions"), String::from("decisions-2026-10-23.json")),
				(String::from("scout"), String::from("scouts-20261023T101500.json")),
			]
		);
	}

	/// A voice note is named by id, and home holds its transcript for the
	/// routine (invariant 19), so a file naming one is filed as any other,
	/// its bytes as sent, for the pass to put in the mailbox's `decisions`
	/// folder (`mailbox::pass`'s `a_filed_file_is_handed_over_as_filed`).
	#[tokio::test]
	async fn files_take_a_file_that_names_a_voice_note() {
		let state = state().await;
		let phone = session(&state, A, Carrier::Bearer).await;
		let app = app(state.clone());
		let file = decisions("2026-10-23", &["0b6f0f8e-1f2a-4c55-9d7e-3a1b2c3d4e5f"]);
		assert_eq!(
			call(&app, Method::PUT, "/jobs/files/decisions-2026-10-23.json", Some(&phone), &file).await,
			(StatusCode::OK, json!({ "change": "added" }))
		);
		assert_eq!(filed(&state).await, [(String::from("decisions"), String::from("decisions-2026-10-23.json"))]);
		let body: Vec<u8> = sqlx::query_scalar("SELECT body FROM mailbox_outgoing").fetch_one(&state.pool).await.unwrap();
		assert_eq!(body, file.to_string().into_bytes(), "filed as sent, the note's id with it");
	}

	#[tokio::test]
	async fn files_refuse_a_wrong_name_schema_or_body() {
		let state = state().await;
		let phone = session(&state, A, Carrier::Bearer).await;
		let app = app(state.clone());
		let file = decisions("2026-10-23", &[]);
		for (path, body, field) in [
			("/jobs/files/digest-2026-10-23.json", file.clone(), "name"),
			("/jobs/files/decisions-2026-13-01.json", file.clone(), "name"),
			("/jobs/files/decisions-2026-10-23-x.json", file.clone(), "name"),
			("/jobs/files/scouts-20261023.json", file.clone(), "name"),
			("/jobs/files/decisions-2026-10-2%C3%A9.json", file.clone(), "name"),
			("/jobs/files/scouts-20261023T101500.json", file.clone(), "schema"),
			("/jobs/files/decisions-2026-10-23.json", json!("text"), "schema"),
			("/jobs/files/decisions-2026-10-23-2.json", json!({ "padding": "x".repeat(super::FILE_CEILING) }), "body"),
		] {
			let (status, answer) = call(&app, Method::PUT, path, Some(&phone), &body).await;
			assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{path}: {answer}");
			assert!(answer["error"]["details"][field].is_array(), "{path}: {answer}");
		}
		assert!(filed(&state).await.is_empty());
	}

	/// `Value` keeps the last of two members with one name, so a schema named
	/// by the first would pass a check of the parsed file and still be in
	/// the bytes stored and handed on. A duplicate member is refused.
	#[tokio::test]
	async fn files_refuse_a_member_named_twice() {
		let state = state().await;
		let phone = session(&state, A, Carrier::Bearer).await;
		let app = app(state.clone());
		let body = r#"{"schema":"some-ui.job-scouts/1","schema":"some-ui.job-decisions/1","picks":[]}"#;
		let request = Request::builder()
			.method(Method::PUT)
			.uri("/jobs/files/decisions-2026-10-23.json")
			.header(&phone.0, &phone.1)
			.body(Body::from(body))
			.unwrap();
		let response = app.clone().oneshot(request).await.unwrap();
		assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
		assert!(filed(&state).await.is_empty());
	}
}

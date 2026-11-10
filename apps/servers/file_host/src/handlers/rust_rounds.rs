//! Handlers for `routes::rust_rounds`: the validate-and-answer half.
//!
//! What a round must be to be kept, and the store's rules (append-only, programs
//! queued once, runs keyed to bytes and toolchain), are `rust_round_repo`'s.
//!
//! **Nothing here compiles, runs or asks the runner** (EX1, RP1). A `PUT`
//! stores the round and queues its programs in one transaction and answers;
//! `rust_rounds`' background pass records them, and a `GET` reads what it
//! kept. No request waits on a compile.
//!
//! **Refusals.** A round home cannot keep or run is a `422` naming each field
//! and why; another body under an id home holds is a `409`. Both write
//! nothing.
//!
//! **A round is its putter's; the code is nobody's** (docs/identity.md
//! invariant 28). Every handler takes a [`Paired`] phone, and reads and
//! writes rounds only under its subject: another subject's round under the
//! same id is a `404` to read and a round of its own to put. A round's runs
//! are found by its programs' bytes, so two subjects whose rounds share a
//! program share its run, which holds only what rustc printed. The
//! [`Paired`] extractor holds off account deletion while a handler runs, as
//! every subject-scoped write does (invariant 9). Neither a round nor a
//! program reaches a log line: the spans skip every argument.

use crate::handlers::db::activities::{etag_value, if_none_match_hits, not_modified};
use crate::subject::Paired;
use crate::FileHostError;
use axum::{
	body::{Body, Bytes},
	extract::{Path, State},
	http::{
		header::{CONTENT_TYPE, ETAG},
		HeaderMap, HeaderValue, StatusCode,
	},
	response::{IntoResponse, Response},
	Json,
};
use chrono::{SecondsFormat, Utc};
use rust_round_repo::{content_hash, Change, RoundEntry, RoundError, StoredRun, LIST_LIMIT, REWRITTEN};
use serde::Serialize;
use serde_json::value::RawValue;
use sqlx::SqlitePool;
use tracing::instrument;

/// `GET /rust-rounds`
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RoundList {
	/// The caller's newest rounds, newest first, at most
	/// [`LIST_LIMIT`].
	pub rounds: Vec<RoundEntry>,
}

/// `PUT /rust-rounds/:id`
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoundWritten {
	/// `added` or `unchanged`.
	pub change: &'static str,
	pub id: String,
	/// The hex SHA-256 of the round as home keeps it, which the phone
	/// compares with what it sent.
	pub content_hash: String,
}

/// `GET /rust-rounds/:id`
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoundWithRuns {
	pub id: String,
	pub content_hash: String,
	pub received_at: String,
	/// The round, byte for byte as home keeps it.
	pub round: Box<RawValue>,
	/// Each read and variant card's program, in card order, and every run
	/// kept for it.
	pub programs: Vec<ProgramRuns>,
}

/// One card's program and its runs.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgramRuns {
	pub card: String,
	/// The hex SHA-256 of the program as the runner is sent it.
	pub source_hash: String,
	/// Every run kept for these bytes, under any toolchain, in the order
	/// kept; empty until the pass records it.
	pub runs: Vec<RunKept>,
}

/// How one recording of a program ended.
///
/// Never when: a run is shared by every round that holds its program, whoever put it, so when it was kept
/// would tell one subject when another first put that program
/// (docs/identity.md invariant 28).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunKept {
	/// The runner's toolchain, `rustc -V` and the profile.
	pub toolchain: String,
	/// `recorded`; `refused` when the runner refused the program's request;
	/// `runner-lost` when the runner went away, or did not answer in time,
	/// while holding it, as many times as the pass allows (RP3).
	pub ended: String,
	/// For `refused`: the runner's reason (`malformed`, `too-large`).
	pub refusal: Option<String>,
	/// What rustc did, as some-ui's `RustcRunSchema` (RR1) reads it: for
	/// `refused` and `runner-lost`, a `stopped` run (`exit`).
	pub rustc: Box<RawValue>,
}

pub(crate) fn now() -> String {
	// Fixed width, as `handlers::voice`'.
	Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

pub(crate) fn unreadable() -> FileHostError {
	FileHostError::OperationError(String::from("a stored round or week does not read back"))
}

/// `GET /rust-rounds`
///
/// # Errors
/// 401 without a session, 403 for a browser's, 500 for a storage failure.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "rust_rounds", skip_all, fields(otel.kind = "server"))]
pub async fn list_rounds(paired: Paired, State(db): State<SqlitePool>) -> Result<Json<RoundList>, FileHostError> {
	let mut conn = db.acquire().await?;
	let rounds = rust_round_repo::rounds(&mut conn, paired.subject.as_str(), LIST_LIMIT).await?;
	Ok(Json(RoundList { rounds }))
}

/// `GET /rust-rounds/:id`: the round as home keeps it, and every run kept
/// for each of its programs. The `ETag` is the hash of the body, so it
/// changes when a run is kept; `If-None-Match` answers `304`.
///
/// # Errors
/// 401 without a session, 403 for a browser's, 404 for an id the caller's
/// subject does not hold (another subject's included), 500 for a storage
/// failure.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "rust_round", skip_all, fields(otel.kind = "server"))]
pub async fn get_round(paired: Paired, State(db): State<SqlitePool>, headers: HeaderMap, Path(id): Path<String>) -> Result<Response, FileHostError> {
	let mut conn = db.acquire().await?;
	let stored = rust_round_repo::round(&mut conn, paired.subject.as_str(), &id).await?.ok_or(FileHostError::NotFound)?;
	let read = rust_round_repo::round::read(stored.text.as_bytes()).map_err(|_| unreadable())?;
	let mut programs = Vec::with_capacity(read.programs.len());
	for program in read.programs {
		let (source_hash, runs) = runs_of(&mut conn, &program.source).await?;
		programs.push(ProgramRuns {
			card: program.card,
			source_hash,
			runs,
		});
	}
	let round = RoundWithRuns {
		id: stored.id,
		content_hash: stored.content_hash,
		received_at: stored.received_at,
		round: RawValue::from_string(stored.text).map_err(|_| unreadable())?,
		programs,
	};
	// Disallowed for tracing; this is the response body itself, hashed for
	// its tag.
	#[allow(clippy::disallowed_methods)]
	let body = serde_json::to_vec(&round)?;
	tagged(&headers, body)
}

/// The hash of `source` and every run kept for it, under any toolchain, in
/// the order kept: what a round's card and a week's ticket both show.
async fn runs_of(conn: &mut sqlx::SqliteConnection, source: &str) -> Result<(String, Vec<RunKept>), FileHostError> {
	let source_hash = content_hash(source.as_bytes());
	let runs = rust_round_repo::runs(conn, &source_hash)
		.await?
		.into_iter()
		.map(|run: StoredRun| {
			Ok(RunKept {
				toolchain: run.toolchain,
				ended: run.ended,
				refusal: run.refusal,
				rustc: RawValue::from_string(run.rustc).map_err(|_| unreadable())?,
			})
		})
		.collect::<Result<Vec<_>, FileHostError>>()?;
	Ok((source_hash, runs))
}

/// A JSON body with its `ETag`, the hash of the body, or `304` when
/// `If-None-Match` names it.
pub(crate) fn tagged(headers: &HeaderMap, body: Vec<u8>) -> Result<Response, FileHostError> {
	let etag = etag_value(&content_hash(&body))?;
	if if_none_match_hits(headers, &etag) {
		return Ok(not_modified(etag));
	}
	let mut response = Response::new(Body::from(body));
	response.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
	response.headers_mut().insert(ETAG, etag);
	Ok(response)
}

/// `GET /rust-weeks`
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WeekList {
	/// The weeks home holds, newest week first, at most [`LIST_LIMIT`]: the
	/// first is the current one. A file under a week's name that is not that
	/// week is left out.
	pub weeks: Vec<WeekEntry>,
}

/// A week home holds, as the list names it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WeekEntry {
	/// The ISO week it opens, `2026-w46`.
	pub id: String,
	/// The hex SHA-256 of the week as home keeps it.
	pub sha256: String,
	/// When home read it from the mailbox.
	pub received_at: String,
}

/// `GET /rust-weeks/:id`
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WeekWithRuns {
	pub id: String,
	pub sha256: String,
	pub received_at: String,
	/// The week, byte for byte as the routine put it.
	pub week: Box<RawValue>,
	/// Each ticket's program, in ticket order, and every run kept for it.
	pub tickets: Vec<TicketRuns>,
}

/// One ticket's program and its runs.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TicketRuns {
	/// The ticket's key, `QM-311`.
	pub ticket: String,
	/// The hex SHA-256 of the program as the runner is sent it.
	pub source_hash: String,
	/// Every run kept for these bytes, under any toolchain, in the order
	/// kept; empty until the pass records it.
	pub runs: Vec<RunKept>,
}

/// `GET /rust-weeks`: the weeks the routine put in the mailbox and home
/// read, under the caller's subject, newest first.
///
/// # Errors
/// 401 without a session, 403 for a browser's, 500 for a storage failure.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "rust_weeks", skip_all, fields(otel.kind = "server"))]
pub async fn list_weeks(paired: Paired, State(db): State<SqlitePool>) -> Result<Json<WeekList>, FileHostError> {
	let mut conn = db.acquire().await?;
	let held = mailbox_repo::rust_weeks(&mut conn, paired.subject.as_str()).await.map_err(mailbox_failure)?;
	let weeks = held
		.into_iter()
		.filter(|held| reads_as_named(&held.id, &held.body).is_some())
		.take(usize::try_from(LIST_LIMIT).unwrap_or(usize::MAX))
		.map(|held| WeekEntry {
			id: held.id,
			sha256: held.sha256,
			received_at: held.pulled_at,
		})
		.collect();
	Ok(Json(WeekList { weeks }))
}

/// `body` as the week its name says, `id`, or `None`, logged: a file under a
/// week's name that is not one is kept as any `brief` file is, and its
/// programs were never queued (`mailbox::pass::keep`), so it is no week.
/// Said by kind, never by body: what the routine put is the routine's.
pub(crate) fn reads_as_named(id: &str, body: &[u8]) -> Option<rust_round_repo::week::Week> {
	match rust_round_repo::week::read(body) {
		Ok(week) if week.id == id => Some(week),
		read => {
			let kind = if read.is_ok() { "misnamed" } else { "unreadable" };
			tracing::error!(kind, "a file held under a week's name is not that week; left out");
			None
		}
	}
}

/// `GET /rust-weeks/:id`: the week as home keeps it, and every run kept
/// for each ticket's program. The `ETag` is the hash of the body, so it
/// changes when a run is kept; `If-None-Match` answers `304`.
///
/// # Errors
/// 401 without a session, 403 for a browser's, 404 for a week the caller's
/// subject does not hold (a file under the week's name that is not that week
/// included), 500 for a storage failure.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "rust_week", skip_all, fields(otel.kind = "server"))]
pub async fn get_week(paired: Paired, State(db): State<SqlitePool>, headers: HeaderMap, Path(id): Path<String>) -> Result<Response, FileHostError> {
	if !rust_round_repo::week::is_week_id(&id) {
		return Err(FileHostError::NotFound);
	}
	let mut conn = db.acquire().await?;
	let held = mailbox_repo::rust_week(&mut conn, paired.subject.as_str(), &rust_round_repo::week::mailbox_name(&id))
		.await
		.map_err(mailbox_failure)?
		.ok_or(FileHostError::NotFound)?;
	let read = reads_as_named(&id, &held.body).ok_or(FileHostError::NotFound)?;
	let mut tickets = Vec::with_capacity(read.programs.len());
	for program in read.programs {
		let (source_hash, runs) = runs_of(&mut conn, &program.source).await?;
		tickets.push(TicketRuns {
			ticket: program.ticket,
			source_hash,
			runs,
		});
	}
	let text = String::from_utf8(held.body).map_err(|_| unreadable())?;
	let week = WeekWithRuns {
		id: read.id,
		sha256: held.sha256,
		received_at: held.pulled_at,
		week: RawValue::from_string(text).map_err(|_| unreadable())?,
		tickets,
	};
	// Disallowed for tracing; this is the response body itself, hashed for
	// its tag.
	#[allow(clippy::disallowed_methods)]
	let body = serde_json::to_vec(&week)?;
	tagged(&headers, body)
}

pub(crate) fn mailbox_failure(err: mailbox_repo::MailboxError) -> FileHostError {
	match err {
		mailbox_repo::MailboxError::Storage(err) => FileHostError::Sqlite(err),
		mailbox_repo::MailboxError::Rewritten => unreadable(),
	}
}

/// `PUT /rust-rounds/:id`, the body being the round's JSON as the tutor
/// wrote it: stored with its programs queued for the pass, in one
/// `BEGIN IMMEDIATE` transaction.
///
/// # Errors
/// 401 without a session, 403 for a browser's or from an untrusted origin,
/// 422 naming each refused field (`body`, `schema`, `id`, `cards`, a card's
/// field, and `id` when the path's id is not the round's), 409 for another
/// body under an id the caller's subject holds, 413 for a body longer than
/// one byte past `ROUND_CEILING` (refused before it is read whole, by the
/// route's `DefaultBodyLimit`), 500 for a storage failure.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "rust_round_put", skip_all, fields(otel.kind = "server"))]
pub async fn put_round(paired: Paired, State(db): State<SqlitePool>, Path(id): Path<String>, body: Bytes) -> Result<Response, FileHostError> {
	let round = rust_round_repo::round::read(&body).map_err(FileHostError::unprocessable_entity)?;
	if round.id != id {
		return Err(FileHostError::unprocessable_entity([("id", "must be the id in the path")]));
	}
	let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
	let change = rust_round_repo::put_round(&mut tx, paired.subject.as_str(), &round, &now())
		.await
		.map_err(|err| match err {
			RoundError::Rewritten => FileHostError::Conflict(REWRITTEN),
			RoundError::Storage(err) => FileHostError::Sqlite(err),
		})?;
	tx.commit().await?;
	let (status, change) = match change {
		Change::Added => (StatusCode::CREATED, "added"),
		Change::Unchanged => (StatusCode::OK, "unchanged"),
	};
	let written = RoundWritten {
		change,
		content_hash: content_hash(round.text.as_bytes()),
		id: round.id,
	};
	Ok((status, Json(written)).into_response())
}

#[cfg(test)]
mod tests {
	use crate::auth::{
		cookie::{Carrier, SessionToken},
		now, AuthContext,
	};
	use crate::routes::table::Module;
	use axum::{
		body::Body,
		extract::FromRef,
		http::{
			header::{AUTHORIZATION, CONTENT_TYPE, COOKIE, ETAG, IF_NONE_MATCH},
			HeaderName, Method, Request, StatusCode,
		},
		Router,
	};
	use rust_rounds_wire::protocol::{AskError, Request as Ask, Response as Answer};
	use rust_rounds_wire::recording::{CompileRecord, Outcome, Recording, RunRecord};
	use serde_json::{json, Value};
	use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};
	use std::sync::Arc;
	use std::time::Duration;
	use tokio_util::sync::CancellationToken;
	use tower::ServiceExt;

	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");
	const A: &str = "subject-a";
	const B: &str = "subject-b";
	const ROUND: &str = "/rust-rounds/indexing-01";
	const WEEK: &str = "/rust-weeks/2026-w46";

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
		crate::routes::rust_rounds::rust_rounds()
	}

	async fn state() -> TestState {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();
		TestState {
			auth: AuthContext::for_tests(pool.clone(), 100),
			pool,
		}
	}

	async fn session(state: &TestState, subject: &str, carrier: Carrier) -> (HeaderName, String) {
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

	async fn call(
		app: &Router,
		method: Method,
		path: &str,
		auth: Option<&(HeaderName, String)>,
		extra: Option<(HeaderName, &str)>,
		body: &[u8],
	) -> (StatusCode, Option<String>, Value) {
		let mut request = Request::builder().method(method).uri(path).header(CONTENT_TYPE, "application/json");
		if let Some((name, value)) = auth {
			request = request.header(name, value);
		}
		if let Some((name, value)) = extra {
			request = request.header(name, value);
		}
		let response = app.clone().oneshot(request.body(Body::from(body.to_vec())).unwrap()).await.unwrap();
		let status = response.status();
		let etag = response.headers().get(ETAG).map(|value| value.to_str().unwrap().to_owned());
		let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
		(status, etag, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
	}

	fn round(id: &str, program: &str) -> Vec<u8> {
		json!({
			"schema": "some-ui.rust-round/1",
			"id": id,
			"by": "claude-routine",
			"concept": "indexing",
			"edges": [],
			"cards": [
				{ "kind": "read", "id": "a", "program": program, "ask": "?", "choices": [], "expect": { "outcome": "panics" } },
				{ "kind": "variant", "id": "b", "of": "a", "hunk": { "line": 1, "replace": "fn main() { println!(\"b\"); }" } },
				{ "kind": "refine", "id": "c", "note": "n1", "nudge": "x", "aim": "apply", "say": "y" }
			]
		})
		.to_string()
		.into_bytes()
	}

	/// Every route answers only a paired phone: no session is `401`, and a
	/// browser's cookie session `403`.
	#[tokio::test]
	async fn every_route_answers_a_paired_phone_only() {
		let state = state().await;
		let browser = session(&state, A, Carrier::Cookie).await;
		let app = module().into_table_router().with_state(state);
		assert_eq!(module().descriptors().count(), 11);
		let body = round("indexing-01", "fn main() {}\n");
		for (method, path) in [
			(Method::GET, "/rust-rounds"),
			(Method::GET, ROUND),
			(Method::PUT, ROUND),
			(Method::GET, "/rust-weeks"),
			(Method::GET, WEEK),
			(Method::PUT, "/rust-weeks/2026-w46/sent"),
			(Method::PUT, "/rust-weeks/2026-w46/pastes/2026-11-10T08:00:00.000Z"),
			(Method::GET, "/rust-weeks/2026-w46/record"),
			(Method::PUT, "/rust-round-log/sessions/s1"),
			(Method::GET, "/rust-round-log/sessions"),
			(Method::GET, "/rust-round-log/sessions/s1"),
		] {
			assert_eq!(call(&app, method.clone(), path, None, None, &body).await.0, StatusCode::UNAUTHORIZED, "{method} {path}");
			assert_eq!(
				call(&app, method.clone(), path, Some(&browser), None, &body).await.0,
				StatusCode::FORBIDDEN,
				"{method} {path}"
			);
		}
	}

	/// `201` for a new round, `200` for the same bytes again, `409` for
	/// another body under its id, and `422` naming why for one home cannot
	/// keep; the first stays as it was.
	#[tokio::test]
	async fn a_put_answers_new_identical_rewritten_and_refused() {
		let state = state().await;
		let phone = session(&state, A, Carrier::Bearer).await;
		let app = module().into_table_router().with_state(state);
		let first = round("indexing-01", "fn main() {}\n");

		let (status, _, added) = call(&app, Method::PUT, ROUND, Some(&phone), None, &first).await;
		assert_eq!(status, StatusCode::CREATED);
		assert_eq!(added["change"], "added");
		assert_eq!(added["contentHash"], rust_round_repo::content_hash(&first));
		let (status, _, same) = call(&app, Method::PUT, ROUND, Some(&phone), None, &first).await;
		assert_eq!((status, same["change"].as_str()), (StatusCode::OK, Some("unchanged")));

		let (status, _, rewritten) = call(&app, Method::PUT, ROUND, Some(&phone), None, &round("indexing-01", "fn main() { }\n")).await;
		assert_eq!(status, StatusCode::CONFLICT, "{rewritten}");

		let (status, _, refused) = call(&app, Method::PUT, "/rust-rounds/other-01", Some(&phone), None, &first).await;
		assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
		assert!(refused.to_string().contains("must be the id in the path"), "{refused}");
		let mut wrong = round("indexing-02", "fn main() {}\n");
		wrong.extend(b"trailing");
		let (status, _, refused) = call(&app, Method::PUT, "/rust-rounds/indexing-02", Some(&phone), None, &wrong).await;
		assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
		assert!(refused.to_string().contains("is not one JSON object"), "{refused}");
		let bad_hunk = String::from_utf8(round("indexing-03", "fn main() {}\n")).unwrap().replace("\"line\":1", "\"line\":9");
		let (status, _, refused) = call(&app, Method::PUT, "/rust-rounds/indexing-03", Some(&phone), None, bad_hunk.as_bytes()).await;
		assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
		assert!(refused.to_string().contains("cards[1].hunk.line"), "{refused}");
		let ceiling = rust_round_repo::round::ROUND_CEILING;
		let (status, _, refused) = call(&app, Method::PUT, "/rust-rounds/big", Some(&phone), None, &vec![b' '; ceiling + 1]).await;
		assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "one byte past the ceiling is read, and named");
		assert!(refused.to_string().contains("larger than 32 KiB"), "{refused}");
		let (status, _, _) = call(&app, Method::PUT, "/rust-rounds/big", Some(&phone), None, &vec![b' '; ceiling + 2]).await;
		assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE, "past it, the body is refused before it is buffered");

		let (_, _, listed) = call(&app, Method::GET, "/rust-rounds", Some(&phone), None, b"").await;
		assert_eq!(listed["rounds"].as_array().unwrap().len(), 1, "only the stored round is listed");
		let (_, _, got) = call(&app, Method::GET, ROUND, Some(&phone), None, b"").await;
		assert_eq!(got["round"], serde_json::from_slice::<Value>(&first).unwrap(), "the first stays");
	}

	/// The runner, in memory: it records every program as printing `ok`.
	struct Prints;
	impl crate::rust_rounds::pass::Runner for Prints {
		fn exchange(&self, request: &Ask, _: Duration) -> Result<Answer, AskError> {
			let toolchain = String::from("rustc 1.97.0 (2d8144b78 2026-07-07); --edition 2024 -C opt-level=0");
			let Ask::Record { .. } = request else {
				return Ok(Answer::Toolchain { toolchain });
			};
			let recording = Recording {
				key: String::new(),
				toolchain,
				outcome: Outcome::Ok,
				note: None,
				compile: CompileRecord {
					diagnostics: Vec::new(),
					rendered: String::new(),
					rendered_bytes: 0,
					timed_out: false,
					exit_code: Some(0),
					signal: None,
				},
				run: Some(RunRecord {
					stdout: String::from("ok\n"),
					stdout_bytes: 3,
					stderr: String::new(),
					stderr_bytes: 0,
					cut: false,
					exit_code: Some(0),
					signal: None,
					milliseconds: 1,
				}),
			};
			Ok(Answer::Recorded {
				rustc: recording.rustc_run(),
				recording: Box::new(recording),
			})
		}
	}

	/// A round reads back with its programs, each with no runs until the pass
	/// records it; the `ETag` answers `304` until a run is kept, and changes
	/// then.
	#[tokio::test]
	async fn a_round_reads_back_with_its_runs_under_an_etag() {
		let state = state().await;
		let pool = state.pool.clone();
		let phone = session(&state, A, Carrier::Bearer).await;
		let app = module().into_table_router().with_state(state);
		let body = round("indexing-01", "fn main() {}\n");
		call(&app, Method::PUT, ROUND, Some(&phone), None, &body).await;

		let (status, etag, got) = call(&app, Method::GET, ROUND, Some(&phone), None, b"").await;
		assert_eq!(status, StatusCode::OK);
		let etag = etag.unwrap();
		assert_eq!(got["round"], serde_json::from_slice::<Value>(&body).unwrap());
		let cards: Vec<&str> = got["programs"].as_array().unwrap().iter().map(|program| program["card"].as_str().unwrap()).collect();
		assert_eq!(cards, ["a", "b"], "a refine card holds no program");
		assert!(got["programs"].as_array().unwrap().iter().all(|program| program["runs"] == json!([])));

		let (status, again, _) = call(&app, Method::GET, ROUND, Some(&phone), Some((IF_NONE_MATCH, &etag)), b"").await;
		assert_eq!((status, again.as_deref()), (StatusCode::NOT_MODIFIED, Some(etag.as_str())));

		let done = crate::rust_rounds::pass::run_once(
			&pool,
			&Arc::new(Prints),
			Duration::from_secs(600),
			crate::rust_rounds::pass::Waits::SERVER,
			&CancellationToken::new(),
			&|| String::from("now"),
		)
		.await
		.unwrap();
		assert_eq!(done.recorded, 2);

		let (status, newer, got) = call(&app, Method::GET, ROUND, Some(&phone), Some((IF_NONE_MATCH, &etag)), b"").await;
		assert_eq!(status, StatusCode::OK, "a kept run is a new tag");
		assert_ne!(newer.unwrap(), etag);
		let run = &got["programs"][1]["runs"][0];
		let keys: std::collections::BTreeSet<&str> = run.as_object().unwrap().keys().map(String::as_str).collect();
		assert_eq!(keys, ["ended", "refusal", "rustc", "toolchain"].into(), "a shared run says no time");
		assert_eq!((&run["ended"], &run["refusal"]), (&json!("recorded"), &json!(null)));
		assert_eq!(run["rustc"], json!({ "outcome": "prints", "toolchain": "1.97.0", "compileStderr": "", "stdout": "ok\n" }));
		assert_eq!(call(&app, Method::GET, "/rust-rounds/nope", Some(&phone), None, b"").await.0, StatusCode::NOT_FOUND);
	}

	/// A week as the mailbox pass keeps it for `subject`: in `brief`, its
	/// programs queued in the same transaction.
	async fn hold_week(pool: &SqlitePool, subject: &str, seq: i64, id: &str, programs: &[&str]) -> Value {
		let tickets: Vec<_> = programs
			.iter()
			.enumerate()
			.map(|(n, program)| json!({ "id": String::from("QM-31") + &n.to_string(), "program": program, "speakTo": ["a", "b"] }))
			.collect();
		let week = json!({ "schema": "some-ui.rust-week/1", "id": id, "company": { "name": "Quillmoss" }, "tickets": tickets });
		let body = week.to_string();
		hold_file(pool, subject, seq, &rust_round_repo::week::mailbox_name(id), &body).await;
		let mut conn = pool.acquire().await.unwrap();
		let read = rust_round_repo::week::read(body.as_bytes()).unwrap();
		rust_round_repo::queue_week(&mut conn, &read, "2026-11-09T08:00:00.000Z").await.unwrap();
		week
	}

	/// A file kept in `subject`'s `brief`, as the mailbox pass keeps one.
	async fn hold_file(pool: &SqlitePool, subject: &str, seq: i64, name: &str, body: &str) {
		let sha256 = rust_round_repo::content_hash(body.as_bytes());
		let item = mailbox_repo::ItemIn {
			folder: "brief",
			name,
			seq,
			sha256: &sha256,
			body: body.as_bytes(),
		};
		mailbox_repo::keep_item(&mut pool.acquire().await.unwrap(), subject, item, "2026-11-09T08:00:00.000Z")
			.await
			.unwrap();
	}

	/// The weeks home holds list newest first; one reads back with each
	/// ticket's program and its runs under an `ETag` that changes when a run
	/// is kept. Another subject's week, an unknown week and an id that is not
	/// a week are `404`s.
	#[tokio::test]
	async fn a_week_reads_back_with_each_tickets_runs_and_only_for_its_subject() {
		let state = state().await;
		let pool = state.pool.clone();
		let phone = session(&state, A, Carrier::Bearer).await;
		let theirs = session(&state, B, Carrier::Bearer).await;
		let app = module().into_table_router().with_state(state);
		hold_week(&pool, A, 1, "2026-w45", &["fn main() {}\n"]).await;
		let week = hold_week(&pool, A, 2, "2026-w46", &["fn main() {}\n", "fn main() { 1; }\n"]).await;
		// Kept as the pass keeps any `brief` file, but no week: unreadable,
		// and one whose body is another week's.
		hold_file(&pool, A, 3, "rust-week-2026-w49.json", r#"{"schema":"some-ui.rust-week/1","id":"2026-w49","tickets":[]}"#).await;
		let other = json!({ "schema": "some-ui.rust-week/1", "id": "2026-w48", "tickets": [{ "id": "QM-1", "program": "x" }] });
		hold_file(&pool, A, 4, "rust-week-2026-w47.json", &other.to_string()).await;

		let (status, _, list) = call(&app, Method::GET, "/rust-weeks", Some(&phone), None, b"").await;
		assert_eq!(status, StatusCode::OK);
		let ids: Vec<&str> = list["weeks"].as_array().unwrap().iter().map(|entry| entry["id"].as_str().unwrap()).collect();
		assert_eq!(ids, ["2026-w46", "2026-w45"], "newest week first, no file that is not its week");
		assert_eq!(list["weeks"][0]["receivedAt"], "2026-11-09T08:00:00.000Z");

		let (status, etag, got) = call(&app, Method::GET, WEEK, Some(&phone), None, b"").await;
		assert_eq!(status, StatusCode::OK);
		let etag = etag.unwrap();
		assert_eq!(got["week"], week);
		let tickets: Vec<&str> = got["tickets"].as_array().unwrap().iter().map(|ticket| ticket["ticket"].as_str().unwrap()).collect();
		assert_eq!(tickets, ["QM-310", "QM-311"]);
		assert!(got["tickets"].as_array().unwrap().iter().all(|ticket| ticket["runs"] == json!([])));

		let done = crate::rust_rounds::pass::run_once(
			&pool,
			&Arc::new(Prints),
			Duration::from_secs(600),
			crate::rust_rounds::pass::Waits::SERVER,
			&CancellationToken::new(),
			&|| String::from("now"),
		)
		.await
		.unwrap();
		assert_eq!(done.recorded, 2, "the two weeks hold two programs between them");

		let (status, newer, got) = call(&app, Method::GET, WEEK, Some(&phone), Some((IF_NONE_MATCH, &etag)), b"").await;
		assert_eq!(status, StatusCode::OK, "a kept run is a new tag");
		assert_ne!(newer.unwrap(), etag);
		assert_eq!(got["tickets"][1]["runs"][0]["rustc"]["stdout"], "ok\n");

		assert_eq!(call(&app, Method::GET, WEEK, Some(&theirs), None, b"").await.0, StatusCode::NOT_FOUND);
		assert_eq!(call(&app, Method::GET, "/rust-weeks", Some(&theirs), None, b"").await.2, json!({ "weeks": [] }));
		assert_eq!(call(&app, Method::GET, "/rust-weeks/2026-w47", Some(&phone), None, b"").await.0, StatusCode::NOT_FOUND);
		assert_eq!(call(&app, Method::GET, "/rust-weeks/indexing-01", Some(&phone), None, b"").await.0, StatusCode::NOT_FOUND);
		for not_a_week in ["/rust-weeks/2026-w49", "/rust-weeks/2026-w47"] {
			assert_eq!(call(&app, Method::GET, not_a_week, Some(&phone), None, b"").await.0, StatusCode::NOT_FOUND, "{not_a_week}");
		}
	}

	/// A round is its putter's: another subject lists none of it, reads its
	/// id as a `404`, and puts its own round under the same id; deleting an
	/// account takes its rounds and leaves the other's, with the run they
	/// share.
	#[tokio::test]
	async fn each_subject_sees_only_its_own_rounds_and_an_account_takes_its_own() {
		let state = state().await;
		let pool = state.pool.clone();
		let auth = state.auth.clone();
		let mine = session(&state, A, Carrier::Bearer).await;
		let theirs = session(&state, B, Carrier::Bearer).await;
		let app = module().into_table_router().with_state(state);
		let my_round = round("indexing-01", "fn main() {}\n");
		let their_round = round("indexing-01", "fn main() { let _ = 1; }\n");

		assert_eq!(call(&app, Method::PUT, ROUND, Some(&mine), None, &my_round).await.0, StatusCode::CREATED);
		let (_, _, listed) = call(&app, Method::GET, "/rust-rounds", Some(&theirs), None, b"").await;
		assert_eq!(listed["rounds"], json!([]), "another subject lists none of it");
		let (status, _, _) = call(&app, Method::GET, ROUND, Some(&theirs), None, b"").await;
		assert_eq!(status, StatusCode::NOT_FOUND, "another subject's id is not there");
		let (status, _, put) = call(&app, Method::PUT, ROUND, Some(&theirs), None, &their_round).await;
		assert_eq!((status, put["change"].as_str()), (StatusCode::CREATED, Some("added")), "its own round, not a 409");
		let (_, _, got) = call(&app, Method::GET, ROUND, Some(&theirs), None, b"").await;
		assert_eq!(got["round"], serde_json::from_slice::<Value>(&their_round).unwrap());
		let (_, _, got) = call(&app, Method::GET, ROUND, Some(&mine), None, b"").await;
		assert_eq!(got["round"], serde_json::from_slice::<Value>(&my_round).unwrap());

		let done = crate::rust_rounds::pass::run_once(
			&pool,
			&Arc::new(Prints),
			Duration::from_secs(600),
			crate::rust_rounds::pass::Waits::SERVER,
			&CancellationToken::new(),
			&|| String::from("now"),
		)
		.await
		.unwrap();
		assert_eq!(done.recorded, 3, "card b's program is one program for both");

		crate::handlers::auth::delete_subject(&auth, A).await.unwrap();
		let rounds: Vec<String> = sqlx::query_scalar("SELECT subject_id FROM rust_round").fetch_all(&pool).await.unwrap();
		assert_eq!(rounds, [B], "the account took its rounds, and only its own");
		let runs: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM rust_round_run").fetch_one(&pool).await.unwrap();
		assert_eq!(runs, 3, "the runs, which are nobody's, stay");
		let (_, _, got) = call(&app, Method::GET, ROUND, Some(&theirs), None, b"").await;
		assert_eq!(got["programs"][1]["runs"][0]["ended"], "recorded", "the shared run still reads for the other subject");
	}

	/// A week with its window, as the routine writes one: tickets `QM-1`,
	/// `QM-2` and `QM-3`.
	async fn hold_dated_week(pool: &SqlitePool, subject: &str, seq: i64, id: &str, opens: &str, due: &str) {
		let ticket = |key: &str| json!({ "id": key, "program": "fn main() {}\n" });
		let week = json!({
			"schema": "some-ui.rust-week/1", "id": id, "opens": opens, "due": due,
			"tickets": [ticket("QM-1"), ticket("QM-2"), ticket("QM-3")],
		});
		let body = week.to_string();
		hold_file(pool, subject, seq, &rust_round_repo::week::mailbox_name(id), &body).await;
		let read = rust_round_repo::week::read(body.as_bytes()).unwrap();
		rust_round_repo::queue_week(&mut pool.acquire().await.unwrap(), &read, "2026-09-28T08:00:00.000Z")
			.await
			.unwrap();
	}

	/// One of the phone's ticket opens: `week-ticket` left at `place`.
	async fn opened(pool: &SqlitePool, subject: &str, id: &str, at: &str, place: &str) {
		let event = json!({
			"family": "engagement", "id": id, "at": at, "build": null, "kind": "left",
			"pipeline": "rust-rounds", "affordance": "week-ticket", "surface": "apk",
			"startedAt": at, "place": place,
		});
		app_event_repo::put_event(&mut pool.acquire().await.unwrap(), subject, id, event.to_string().as_bytes(), at)
			.await
			.unwrap();
	}

	fn sent(at: &str) -> Vec<u8> {
		sent_of("2026-w40", at)
	}

	fn sent_of(week: &str, at: &str) -> Vec<u8> {
		json!({
			"week": week, "at": at,
			"answers": [{ "ticket": "QM-1", "take": "2026-w40:QM-1", "recordedAt": "2026-10-01T18:00:00.000Z" }],
			"passed": ["QM-2", "QM-3"],
		})
		.to_string()
		.into_bytes()
	}

	/// A week's send is a draft until its deadline and final at it, and none
	/// is taken once home's clock is past it; the record
	/// names each ticket answered or not, how often it was opened in the
	/// week, and the pastes; silence is a record too, and another subject's
	/// week is a `404`.
	#[tokio::test]
	async fn a_weeks_record_says_each_ticket_answered_or_not_and_how_often_opened() {
		const SENT: &str = "/rust-weeks/2026-w40/sent";
		const RECORD: &str = "/rust-weeks/2026-w40/record";
		let state = state().await;
		let pool = state.pool.clone();
		let phone = session(&state, A, Carrier::Bearer).await;
		let theirs = session(&state, B, Carrier::Bearer).await;
		let app = module().into_table_router().with_state(state);
		hold_dated_week(&pool, A, 1, "2026-w40", "2026-09-28T00:00:00-07:00", "2026-10-04T23:59:00-07:00").await;
		hold_dated_week(&pool, A, 2, "2026-w41", "2026-10-05T00:00:00-07:00", "2026-10-11T23:59:00-07:00").await;
		hold_dated_week(&pool, A, 3, "2026-w53", "2026-10-12T00:00:00-07:00", "2099-12-31T23:59:00-08:00").await;

		let late = sent("2026-10-05T07:00:00.000Z");
		let (status, _, refused) = call(&app, Method::PUT, SENT, Some(&phone), None, &late).await;
		assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "after 23:59 Pacific doesn't count");
		assert!(refused["error"]["details"]["at"].is_array(), "{refused}");
		// Sent before the deadline: the week's draft, as a send then left it.
		let on_time = sent("2026-10-05T06:58:00.000Z");
		sqlx::query(
			"INSERT INTO rust_week_draft (subject_id, week_id, sent_at, body, received_at) VALUES (?1, '2026-w40', '2026-10-05T06:58:00.000Z', ?2, '2026-10-05T06:58:01.000Z')",
		)
		.bind(A)
		.bind(String::from_utf8(on_time.clone()).unwrap())
		.execute(&pool)
		.await
		.unwrap();
		let (status, _, refused) = call(&app, Method::PUT, SENT, Some(&phone), None, &sent("2026-10-05T06:59:00.000Z")).await;
		assert_eq!(
			status,
			StatusCode::UNPROCESSABLE_ENTITY,
			"home's clock is past the deadline: no grace, even pressed before it"
		);
		assert!(refused["error"]["details"]["week"].is_array(), "{refused}");
		let finals: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM rust_week_final").fetch_one(&pool).await.unwrap();
		assert_eq!(finals, 1, "the refused send made the week final first");
		assert_eq!(
			call(&app, Method::PUT, SENT, Some(&theirs), None, &on_time).await.0,
			StatusCode::NOT_FOUND,
			"B holds no such week"
		);

		let paste = json!({ "at": "2026-10-02T09:00:00.000Z", "text": "error[E0502]: cannot borrow", "note": null }).to_string();
		let path = "/rust-weeks/2026-w40/pastes/2026-10-02T09%3A00%3A00.000Z";
		let (status, _, put) = call(&app, Method::PUT, path, Some(&phone), None, paste.as_bytes()).await;
		assert_eq!(
			(status, &put),
			(StatusCode::CREATED, &json!({ "change": "added", "paste": serde_json::from_str::<Value>(&paste).unwrap() }))
		);
		assert_eq!(call(&app, Method::PUT, path, Some(&phone), None, paste.as_bytes()).await.0, StatusCode::OK);

		opened(&pool, A, "o1", "2026-09-29T09:00:00.000Z", "ticket-1").await;
		opened(&pool, A, "o2", "2026-10-01T09:00:00.000Z", "ticket-1").await;
		opened(&pool, A, "o3", "2026-10-01T09:05:00.000Z", "ticket-3").await;
		opened(&pool, A, "o4", "2026-10-06T09:00:00.000Z", "ticket-2").await;
		opened(&pool, B, "o5", "2026-10-01T09:00:00.000Z", "ticket-2").await;

		let (status, _, record) = call(&app, Method::GET, RECORD, Some(&phone), None, b"").await;
		assert_eq!(status, StatusCode::OK);
		assert_eq!(
			record,
			json!({
				"week": "2026-w40", "opens": "2026-09-28T00:00:00-07:00", "due": "2026-10-04T23:59:00-07:00",
				"closed": true,
				"sent": { "at": "2026-10-05T06:58:00.000Z", "receivedAt": "2026-10-05T06:58:01.000Z", "draft": false },
				"tickets": [
					{ "ticket": "QM-1", "answered": true, "take": "2026-w40:QM-1", "recordedAt": "2026-10-01T18:00:00.000Z", "opens": 2 },
					{ "ticket": "QM-2", "answered": false, "take": null, "recordedAt": null, "opens": 0 },
					{ "ticket": "QM-3", "answered": false, "take": null, "recordedAt": null, "opens": 1 },
				],
				"unanswered": ["QM-2", "QM-3"],
				"pastes": [{ "at": "2026-10-02T09:00:00.000Z", "text": "error[E0502]: cannot borrow", "note": null, "receivedAt": record["pastes"][0]["receivedAt"] }],
			}),
			"the open after the deadline and B's are not counted"
		);

		let (_, _, silent) = call(&app, Method::GET, "/rust-weeks/2026-w41/record", Some(&phone), None, b"").await;
		assert_eq!((silent["closed"].as_bool(), &silent["sent"]), (Some(true), &Value::Null));
		assert_eq!(silent["unanswered"], json!(["QM-1", "QM-2", "QM-3"]), "nothing sent: every ticket a silence");
		let (_, _, open) = call(&app, Method::GET, "/rust-weeks/2026-w53/record", Some(&phone), None, b"").await;
		assert_eq!((&open["closed"], &open["sent"]), (&json!(false), &Value::Null));

		assert_eq!(call(&app, Method::GET, RECORD, Some(&theirs), None, b"").await.0, StatusCode::NOT_FOUND);
		assert_eq!(
			call(&app, Method::GET, "/rust-weeks/2026-w44/record", Some(&phone), None, b"").await.0,
			StatusCode::NOT_FOUND
		);
	}

	/// Before the deadline every send is a draft: a later one replaces it, an
	/// earlier one leaves it, the same value again is no change, another body
	/// at its moment is a `409`, and the record reads the draft as one.
	#[tokio::test]
	async fn a_weeks_send_is_a_draft_until_its_deadline() {
		const OPEN_SENT: &str = "/rust-weeks/2026-w53/sent";
		let state = state().await;
		let pool = state.pool.clone();
		let phone = session(&state, A, Carrier::Bearer).await;
		let app = module().into_table_router().with_state(state);
		hold_dated_week(&pool, A, 1, "2026-w53", "2026-10-12T00:00:00-07:00", "2099-12-31T23:59:00-08:00").await;

		let first = sent_of("2026-w53", "2026-10-20T09:00:00.000Z");
		let later = sent_of("2026-w53", "2026-10-21T09:00:00.000Z");
		let (status, _, put) = call(&app, Method::PUT, OPEN_SENT, Some(&phone), None, &first).await;
		assert_eq!(
			(status, &put),
			(StatusCode::CREATED, &json!({ "change": "added", "sent": serde_json::from_slice::<Value>(&first).unwrap() }))
		);
		let (status, _, put) = call(&app, Method::PUT, OPEN_SENT, Some(&phone), None, &later).await;
		assert_eq!((status, put["change"].as_str()), (StatusCode::OK, Some("replaced")));
		let (status, _, put) = call(&app, Method::PUT, OPEN_SENT, Some(&phone), None, &first).await;
		assert_eq!((status, put["change"].as_str()), (StatusCode::OK, Some("superseded")), "a late retry");
		assert_eq!(put["sent"], serde_json::from_slice::<Value>(&later).unwrap(), "answered with the draft that stands");
		let held: Value = serde_json::from_slice(&later).unwrap();
		let reordered = String::from("{ \"passed\": ")
			+ &held["passed"].to_string()
			+ ",\n \"answers\": "
			+ &held["answers"].to_string()
			+ ", \"at\": \"2026-10-21T09:00:00.000Z\", \"week\": \"2026-w53\" }";
		let (status, _, put) = call(&app, Method::PUT, OPEN_SENT, Some(&phone), None, reordered.as_bytes()).await;
		assert_eq!((status, put["change"].as_str()), (StatusCode::OK, Some("unchanged")), "the same value, keys reordered");
		let mut changed = held.clone();
		changed["answers"][0]["take"] = json!("2026-w53:QM-1:redo");
		assert_eq!(
			call(&app, Method::PUT, OPEN_SENT, Some(&phone), None, changed.to_string().as_bytes()).await.0,
			StatusCode::CONFLICT,
			"another body at the draft's moment"
		);
		let (_, _, open) = call(&app, Method::GET, "/rust-weeks/2026-w53/record", Some(&phone), None, b"").await;
		assert_eq!(
			(&open["sent"]["at"], &open["sent"]["draft"], &open["closed"]),
			(&json!("2026-10-21T09:00:00.000Z"), &json!(true), &json!(false))
		);
		assert_eq!(open["unanswered"], json!(["QM-2", "QM-3"]), "read from the draft");
	}

	/// A card-era session is kept as sent and read back byte for byte; it
	/// grows by revisions, and one that rewrites it is a `409`.
	#[tokio::test]
	async fn the_card_era_log_is_kept_as_sent_and_grows_by_revisions() {
		const PATH: &str = "/rust-round-log/sessions/s1";
		let state = state().await;
		let phone = session(&state, A, Carrier::Bearer).await;
		let theirs = session(&state, B, Carrier::Bearer).await;
		let app = module().into_table_router().with_state(state);
		let session_body = |cards: usize| {
			let card = |n: usize| {
				json!({ "round": "indexing-01", "card": String::from("c") + &n.to_string(), "at": "2026-10-30T08:00:00.000Z",
					"picked": null, "note": null, "refines": null, "skipped": true, "rustc": null })
			};
			let session = json!({
				"schema": "some-ui.rust-round-log/1", "id": "s1", "startedAt": "2026-10-30T07:59:00.000Z",
				"cards": (0..cards).map(card).collect::<Vec<_>>(), "pinned": [], "fromMyDesk": [], "tutorGuessedWrong": [],
			});
			session.to_string().replace(",\"", ", \"").into_bytes()
		};
		let (status, _, put) = call(&app, Method::PUT, PATH, Some(&phone), None, &session_body(1)).await;
		assert_eq!((status, &put), (StatusCode::CREATED, &json!({ "change": "added", "id": "s1", "revision": 1 })));
		let (status, _, put) = call(&app, Method::PUT, PATH, Some(&phone), None, &session_body(2)).await;
		assert_eq!((status, &put), (StatusCode::CREATED, &json!({ "change": "extended", "id": "s1", "revision": 2 })));
		assert_eq!(call(&app, Method::PUT, PATH, Some(&phone), None, &session_body(2)).await.0, StatusCode::OK);
		assert_eq!(call(&app, Method::PUT, PATH, Some(&phone), None, &session_body(1)).await.0, StatusCode::CONFLICT);
		assert_eq!(
			call(&app, Method::PUT, "/rust-round-log/sessions/s2", Some(&phone), None, &session_body(1)).await.0,
			StatusCode::UNPROCESSABLE_ENTITY,
			"the path names another session"
		);

		let request = Request::builder().uri(PATH).header(phone.0.clone(), phone.1.clone()).body(Body::empty()).unwrap();
		let response = app.clone().oneshot(request).await.unwrap();
		assert_eq!(response.status(), StatusCode::OK);
		let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
		assert_eq!(bytes.as_ref(), session_body(2).as_slice(), "read back byte for byte");
		let (_, _, list) = call(&app, Method::GET, "/rust-round-log/sessions", Some(&phone), None, b"").await;
		assert_eq!(list["sessions"].as_array().unwrap().len(), 1);
		assert_eq!(list["sessions"][0]["revision"], json!(2));
		assert_eq!(list["sessions"][0]["sha256"], json!(rust_round_repo::content_hash(&session_body(2))));
		let (_, _, other) = call(&app, Method::GET, "/rust-round-log/sessions", Some(&theirs), None, b"").await;
		assert_eq!(other["sessions"], json!([]));
		assert_eq!(call(&app, Method::GET, PATH, Some(&theirs), None, b"").await.0, StatusCode::NOT_FOUND);
	}
}

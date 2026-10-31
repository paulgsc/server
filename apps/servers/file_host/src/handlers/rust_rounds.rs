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

fn now() -> String {
	// Fixed width, as `handlers::voice`'.
	Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn unreadable() -> FileHostError {
	FileHostError::OperationError(String::from("a stored round does not read back"))
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
		let source_hash = content_hash(program.source.as_bytes());
		let runs = rust_round_repo::runs(&mut conn, &source_hash)
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
	let etag = etag_value(&content_hash(&body))?;
	if if_none_match_hits(&headers, &etag) {
		return Ok(not_modified(etag));
	}
	let mut response = Response::new(Body::from(body));
	response.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
	response.headers_mut().insert(ETAG, etag);
	Ok(response)
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
		assert_eq!(module().descriptors().count(), 3);
		let body = round("indexing-01", "fn main() {}\n");
		for (method, path) in [(Method::GET, "/rust-rounds"), (Method::GET, ROUND), (Method::PUT, ROUND)] {
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
}

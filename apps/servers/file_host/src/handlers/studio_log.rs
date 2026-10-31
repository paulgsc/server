//! Handlers for `routes::studio_log`: the validate-and-answer half. The
//! record's own rules (append-only, what an event or a report holds) are
//! `studio_log_repo`'s.
//!
//! **Refusals.** A write that breaks one of those rules is a `422` naming
//! each field and why, and a different body under a held id is a `409`. Both
//! write nothing, so the phone can say which entry home refused and keep it.
//!
//! **The deletion hold.** Each handler takes a [`SubjectId`] first and keeps
//! it until it returns, so a write holds off `DELETE /auth/account` as every
//! other subject-scoped write does (docs/identity.md, invariant 9).
//!
//! **Nothing typed reaches a log.** A report's `cause` may be typed by the
//! owner. Every handler here is `#[instrument(skip_all)]` and records only
//! `otel.kind` (invariant 6), which `nothing_a_report_says_reaches_a_log_line`
//! checks.
//!
//! **No other writer.** [`put_event`] and [`put_report`] are the only callers
//! of `studio_log_repo`'s puts; nothing writes the record in the background.

use crate::subject::SubjectId;
use crate::FileHostError;
use axum::{
	body::Bytes,
	extract::{Path, State},
	Json,
};
use chrono::{SecondsFormat, Utc};
use serde::Serialize;
use sqlx::SqlitePool;
use studio_log_repo::{Event, LogError, Report, Reports, SittingEvents, Sittings, REWRITTEN, UNREADABLE};
use tracing::instrument;

/// `PUT /studio/sittings/:sitting/events/:id`
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EventWritten {
	/// `added` or `unchanged` — `studio_log_repo::Change`.
	pub change: &'static str,
	/// The event as the record holds it, which the phone compares with what
	/// it sent before it lets its own copy go.
	pub event: Event,
}

/// `PUT /studio/friction/:id`
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReportWritten {
	/// `added` or `unchanged` — `studio_log_repo::Change`.
	pub change: &'static str,
	/// The report as the record holds it.
	pub report: Report,
}

fn now() -> String {
	// Fixed width, as `handlers::jobs`'s.
	Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn refusal(err: LogError) -> FileHostError {
	match err {
		LogError::Invalid(problems) => FileHostError::unprocessable_entity(problems),
		LogError::Rewritten => FileHostError::Conflict(REWRITTEN),
		LogError::Unreadable => FileHostError::OperationError(UNREADABLE.to_owned()),
		LogError::Storage(err) => FileHostError::Sqlite(err),
	}
}

/// `GET /studio/sittings`
///
/// # Errors
/// 401 without a session, and 500 for a storage failure.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "studio_sittings", skip_all, fields(otel.kind = "server"))]
pub async fn get_sittings(subject: SubjectId, State(db): State<SqlitePool>) -> Result<Json<Sittings>, FileHostError> {
	let mut conn = db.acquire().await?;
	studio_log_repo::sittings(&mut conn, subject.as_str()).await.map(Json).map_err(refusal)
}

/// `GET /studio/sittings/:sitting/events`: an empty list for a sitting home
/// has never seen.
///
/// # Errors
/// 401 without a session, and 500 for a storage failure.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "studio_sitting_events", skip_all, fields(otel.kind = "server"))]
pub async fn get_sitting_events(subject: SubjectId, State(db): State<SqlitePool>, Path(sitting): Path<String>) -> Result<Json<SittingEvents>, FileHostError> {
	let mut conn = db.acquire().await?;
	studio_log_repo::sitting_events(&mut conn, subject.as_str(), &sitting).await.map(Json).map_err(refusal)
}

/// `PUT /studio/sittings/:sitting/events/:id`, the request body being the
/// event itself. `BEGIN IMMEDIATE`, so the reads that decide and the write
/// see one snapshot.
///
/// # Errors
/// 401 without a session, 403 from an untrusted origin, 422 naming each
/// refused field, 409 for a different event under a held id, and 500 for a
/// storage failure.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "studio_sitting_event_put", skip_all, fields(otel.kind = "server"))]
pub async fn put_event(
	subject: SubjectId,
	State(db): State<SqlitePool>,
	Path((sitting, id)): Path<(String, String)>,
	body: Bytes,
) -> Result<Json<EventWritten>, FileHostError> {
	let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
	let (change, event) = studio_log_repo::put_event(&mut tx, subject.as_str(), &sitting, &id, &body, &now()).await.map_err(refusal)?;
	tx.commit().await?;
	Ok(Json(EventWritten { change: change.as_str(), event }))
}

/// `GET /studio/friction`
///
/// # Errors
/// 401 without a session, and 500 for a storage failure.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "studio_friction", skip_all, fields(otel.kind = "server"))]
pub async fn get_reports(subject: SubjectId, State(db): State<SqlitePool>) -> Result<Json<Reports>, FileHostError> {
	let mut conn = db.acquire().await?;
	studio_log_repo::reports(&mut conn, subject.as_str()).await.map(Json).map_err(refusal)
}

/// `PUT /studio/friction/:id`, the request body being the report itself.
/// `BEGIN IMMEDIATE`, as [`put_event`].
///
/// # Errors
/// 401 without a session, 403 from an untrusted origin, 422 naming each
/// refused field, 409 for a different report under a held id, and 500 for a
/// storage failure.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "studio_friction_put", skip_all, fields(otel.kind = "server"))]
pub async fn put_report(subject: SubjectId, State(db): State<SqlitePool>, Path(id): Path<String>, body: Bytes) -> Result<Json<ReportWritten>, FileHostError> {
	let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
	let (change, report) = studio_log_repo::put_report(&mut tx, subject.as_str(), &id, &body, &now()).await.map_err(refusal)?;
	tx.commit().await?;
	Ok(Json(ReportWritten { change: change.as_str(), report }))
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
	const S: &str = "4f0c2a8e-7b1d-4c3e-9a55-0d1e2f3a4b5c";

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
		crate::routes::studio_log::studio_log()
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

	fn events(sitting: &str) -> String {
		String::from("/studio/sittings/") + sitting + "/events"
	}

	fn event_path(sitting: &str, id: &str) -> String {
		events(sitting) + "/" + id
	}

	fn report_path(id: &str) -> String {
		String::from("/studio/friction/") + id
	}

	fn opened(id: &str) -> Value {
		json!({ "id": id, "at": "2026-10-30T09:00:00.000Z", "sitting": S, "kind": "record", "type": "sitting_opened", "screen": null })
	}

	fn entered(id: &str) -> Value {
		json!({ "id": id, "at": "2026-10-30T09:01:00.000Z", "sitting": S, "kind": "record", "type": "screen_entered", "screen": "record.shot-list" })
	}

	fn report(id: &str) -> Value {
		json!({ "id": id, "at": "2026-10-30T09:30:00.000Z", "sitting": S, "kind": "record", "how": "some_snags", "cause": "the mic clipped", "shot": "ep04:s3", "voice": "n1" })
	}

	#[tokio::test]
	async fn every_route_needs_a_session_and_writes_need_a_trusted_origin() {
		let state = state().await;
		let phone = session(&state, A, Carrier::Bearer).await;
		let app = app(state);
		let routes: Vec<(Method, String)> = module()
			.descriptors()
			.map(|route| (Method::from_bytes(route.method.as_bytes()).unwrap(), route.path.replace(":sitting", S).replace(":id", "e1")))
			.collect();
		assert_eq!(routes.len(), 5, "{routes:?}");
		for (method, path) in routes {
			assert_eq!(call(&app, method.clone(), &path, None, &json!({})).await.0, StatusCode::UNAUTHORIZED, "{method} {path}");
		}
		for (path, body) in [(event_path(S, "e1"), opened("e1")), (report_path("f1"), report("f1"))] {
			let request = Request::builder()
				.method(Method::PUT)
				.uri(path)
				.header(&phone.0, &phone.1)
				.header("origin", "https://evil.app.test")
				.header("sec-fetch-site", "cross-site")
				.body(Body::from(body.to_string()))
				.unwrap();
			assert_eq!(app.clone().oneshot(request).await.unwrap().status(), StatusCode::FORBIDDEN);
		}
	}

	/// The phone's whole round: an empty sitting, its events added, one sent
	/// again unchanged, the sitting read back, the list of sittings, and a
	/// friction report sent twice and read back. A browser session on the
	/// same account reads the same record.
	#[tokio::test]
	async fn a_paired_phone_sends_a_sitting_and_its_friction_and_reads_them_back() {
		let state = state().await;
		let phone = session(&state, A, Carrier::Bearer).await;
		let browser = session(&state, A, Carrier::Cookie).await;
		let app = app(state);

		assert_eq!(
			call(&app, Method::GET, &events(S), Some(&phone), &Value::Null).await,
			(StatusCode::OK, json!({ "sitting": S, "events": [] }))
		);
		assert_eq!(
			call(&app, Method::GET, "/studio/sittings", Some(&phone), &Value::Null).await,
			(StatusCode::OK, json!({ "sittings": [] }))
		);
		assert_eq!(
			call(&app, Method::GET, "/studio/friction", Some(&phone), &Value::Null).await,
			(StatusCode::OK, json!({ "reports": [] }))
		);
		assert_eq!(
			call(&app, Method::PUT, &event_path(S, "e1"), Some(&phone), &opened("e1")).await,
			(StatusCode::OK, json!({ "change": "added", "event": opened("e1") }))
		);
		assert_eq!(
			call(&app, Method::PUT, &event_path(S, "e2"), Some(&phone), &entered("e2")).await,
			(StatusCode::OK, json!({ "change": "added", "event": entered("e2") }))
		);
		assert_eq!(call(&app, Method::PUT, &event_path(S, "e1"), Some(&phone), &opened("e1")).await.1["change"], "unchanged");

		let held = json!({ "sitting": S, "events": [opened("e1"), entered("e2")] });
		assert_eq!(call(&app, Method::GET, &events(S), Some(&phone), &Value::Null).await, (StatusCode::OK, held.clone()));
		assert_eq!(call(&app, Method::GET, &events(S), Some(&browser), &Value::Null).await, (StatusCode::OK, held));
		assert_eq!(
			call(&app, Method::GET, "/studio/sittings", Some(&browser), &Value::Null).await,
			(
				StatusCode::OK,
				json!({ "sittings": [{ "sitting": S, "kind": "record", "firstAt": "2026-10-30T09:00:00.000Z", "lastAt": "2026-10-30T09:01:00.000Z", "events": 2 }] })
			)
		);

		assert_eq!(
			call(&app, Method::PUT, &report_path("f1"), Some(&phone), &report("f1")).await,
			(StatusCode::OK, json!({ "change": "added", "report": report("f1") }))
		);
		assert_eq!(
			call(&app, Method::PUT, &report_path("f1"), Some(&phone), &report("f1")).await,
			(StatusCode::OK, json!({ "change": "unchanged", "report": report("f1") }))
		);
		assert_eq!(
			call(&app, Method::GET, "/studio/friction", Some(&browser), &Value::Null).await,
			(StatusCode::OK, json!({ "reports": [report("f1")] }))
		);
	}

	/// A write that breaks a rule is a 422 naming each field, a different
	/// body under a held id is a 409, and one subject's record is never
	/// another's.
	#[tokio::test]
	async fn refusals_name_their_fields_and_records_stay_apart() {
		let state = state().await;
		let a = session(&state, A, Carrier::Bearer).await;
		let b = session(&state, B, Carrier::Bearer).await;
		let app = app(state);
		assert_eq!(call(&app, Method::PUT, &event_path(S, "e1"), Some(&a), &opened("e1")).await.0, StatusCode::OK);
		assert_eq!(call(&app, Method::PUT, &report_path("f1"), Some(&a), &report("f1")).await.0, StatusCode::OK);

		let named = |answer: (StatusCode, Value)| {
			assert_eq!(answer.0, StatusCode::UNPROCESSABLE_ENTITY, "{}", answer.1);
			let mut fields: Vec<String> = answer.1["error"]["details"].as_object().unwrap().keys().cloned().collect();
			fields.sort();
			fields
		};
		let mut wrong = entered("e2");
		wrong["at"] = json!("yesterday");
		wrong["screen"] = Value::Null;
		assert_eq!(named(call(&app, Method::PUT, &event_path(S, "e2"), Some(&a), &wrong).await), ["at", "screen"]);
		let mut watch = entered("e3");
		watch["kind"] = json!("watch");
		assert_eq!(
			named(call(&app, Method::PUT, &event_path(S, "e3"), Some(&a), &watch).await),
			["kind"],
			"a sitting keeps one kind"
		);
		assert_eq!(named(call(&app, Method::PUT, &event_path(S, "other"), Some(&a), &entered("e4")).await), ["id"]);
		assert_eq!(named(call(&app, Method::PUT, &event_path("s-other", "e5"), Some(&a), &entered("e5")).await), ["sitting"]);
		assert_eq!(named(call(&app, Method::PUT, &event_path(S, "e6"), Some(&a), &json!("event")).await), ["body"]);
		let mut long = report("f2");
		long["cause"] = json!("x".repeat(201));
		long["voice"] = json!("a/b");
		assert_eq!(named(call(&app, Method::PUT, &report_path("f2"), Some(&a), &long).await), ["cause", "voice"]);

		let mut rewritten = opened("e1");
		rewritten["type"] = json!("sitting_closed");
		let (status, answer) = call(&app, Method::PUT, &event_path(S, "e1"), Some(&a), &rewritten).await;
		assert_eq!(status, StatusCode::CONFLICT, "{answer}");
		assert_eq!(answer["error"]["code"], "conflict");
		let mut rewritten = report("f1");
		rewritten["how"] = json!("rough");
		assert_eq!(call(&app, Method::PUT, &report_path("f1"), Some(&a), &rewritten).await.0, StatusCode::CONFLICT);

		assert_eq!(
			call(&app, Method::GET, &events(S), Some(&b), &Value::Null).await.1,
			json!({ "sitting": S, "events": [] }),
			"B reads none of A's"
		);
		assert_eq!(call(&app, Method::GET, "/studio/sittings", Some(&b), &Value::Null).await.1, json!({ "sittings": [] }));
		assert_eq!(call(&app, Method::GET, "/studio/friction", Some(&b), &Value::Null).await.1, json!({ "reports": [] }));
		assert_eq!(
			call(&app, Method::GET, &events(S), Some(&a), &Value::Null).await.1,
			json!({ "sitting": S, "events": [opened("e1")] }),
			"nothing refused was written"
		);
		assert_eq!(
			call(&app, Method::GET, "/studio/friction", Some(&a), &Value::Null).await.1,
			json!({ "reports": [report("f1")] })
		);
	}

	/// docs/identity.md invariant 6: a report's cause may be typed by the
	/// owner, and neither it nor anything else a write carries reaches a log
	/// line or span field, refused or kept.
	#[tokio::test]
	async fn nothing_a_report_says_reaches_a_log_line() {
		const SAID: &str = "kept-losing-my-place-zq7";
		let state = state().await;
		let phone = session(&state, A, Carrier::Bearer).await;
		let app = app(state);
		let (captured, _guard) = crate::privacy::capture_where(crate::auth::loggable);
		let mut said = report("f1");
		said["cause"] = json!(SAID);
		assert_eq!(call(&app, Method::PUT, &report_path("f1"), Some(&phone), &said).await.0, StatusCode::OK);
		said["how"] = json!("rough");
		assert_eq!(call(&app, Method::PUT, &report_path("f1"), Some(&phone), &said).await.0, StatusCode::CONFLICT);
		said["shot"] = json!(SAID.to_owned() + " with a space");
		assert_eq!(call(&app, Method::PUT, &report_path("f2"), Some(&phone), &said).await.0, StatusCode::UNPROCESSABLE_ENTITY);
		assert_eq!(call(&app, Method::GET, "/studio/friction", Some(&phone), &Value::Null).await.0, StatusCode::OK);

		let lines = captured.lines();
		assert!(lines.iter().any(|line| line.contains("otel.kind")), "no handler span was captured: {lines:#?}");
		for line in &lines {
			assert!(!line.contains(SAID), "what a report says reached a log line: {line}");
		}
	}
}

//! Handlers for `routes::leetype_log`: the validate-and-answer half. The
//! record's own rules (append-only, what an event holds and what it may not)
//! are `leetype_log_repo`'s.
//!
//! **Refusals.** An event that breaks one of those rules is a `422` naming
//! each field and why, and a different event under an id the session holds
//! is a `409`. Both write nothing, so the phone can say which event home
//! refused and keep it.
//!
//! **The deletion hold.** Each handler takes a [`SubjectId`] first and keeps
//! it until it returns, so a write holds off `DELETE /auth/account` as every
//! other subject-scoped write does (docs/identity.md, invariant 9).
//!
//! **No other writer.** [`put_event`] is the only caller of
//! `leetype_log_repo::put_event`; nothing writes the record in the background.

use crate::subject::SubjectId;
use crate::FileHostError;
use axum::{
	body::Bytes,
	extract::{Path, State},
	Json,
};
use chrono::{SecondsFormat, Utc};
use leetype_log_repo::{Event, LogError, SessionEvents, Sessions, REWRITTEN, UNREADABLE};
use serde::Serialize;
use sqlx::SqlitePool;
use tracing::instrument;

/// `PUT /leetype/sessions/:session/events/:id`
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EventWritten {
	/// `added` or `unchanged` — `leetype_log_repo::Change`.
	pub change: &'static str,
	/// The event as the record holds it, which the phone compares with what
	/// it sent before it lets its own copy go.
	pub event: Event,
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

/// `GET /leetype/sessions`
///
/// # Errors
/// 401 without a session, and 500 for a storage failure.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "leetype_sessions", skip_all, fields(otel.kind = "server"))]
pub async fn get_sessions(subject: SubjectId, State(db): State<SqlitePool>) -> Result<Json<Sessions>, FileHostError> {
	let mut conn = db.acquire().await?;
	leetype_log_repo::sessions(&mut conn, subject.as_str()).await.map(Json).map_err(refusal)
}

/// `GET /leetype/sessions/:session/events`: an empty list for a session home
/// has never seen.
///
/// # Errors
/// 401 without a session, and 500 for a storage failure.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "leetype_session_events", skip_all, fields(otel.kind = "server"))]
pub async fn get_session_events(subject: SubjectId, State(db): State<SqlitePool>, Path(session): Path<String>) -> Result<Json<SessionEvents>, FileHostError> {
	let mut conn = db.acquire().await?;
	leetype_log_repo::session_events(&mut conn, subject.as_str(), &session).await.map(Json).map_err(refusal)
}

/// `PUT /leetype/sessions/:session/events/:id`, the request body being the
/// event itself. `BEGIN IMMEDIATE`, so the read that decides and the write
/// see one snapshot.
///
/// # Errors
/// 401 without a session, 403 from an untrusted origin, 422 naming each
/// refused field, 409 for a different event under a held id, and 500 for a
/// storage failure.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "leetype_session_event_put", skip_all, fields(otel.kind = "server"))]
pub async fn put_event(
	subject: SubjectId,
	State(db): State<SqlitePool>,
	Path((session, id)): Path<(String, String)>,
	body: Bytes,
) -> Result<Json<EventWritten>, FileHostError> {
	let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
	let (change, event) = leetype_log_repo::put_event(&mut tx, subject.as_str(), &session, &id, &body, &now())
		.await
		.map_err(refusal)?;
	tx.commit().await?;
	Ok(Json(EventWritten { change: change.as_str(), event }))
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
		crate::routes::leetype_log::leetype_log()
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

	fn events(session: &str) -> String {
		String::from("/leetype/sessions/") + session + "/events"
	}

	fn event_path(session: &str, id: &str) -> String {
		events(session) + "/" + id
	}

	fn started(id: &str) -> Value {
		json!({ "id": id, "at": "2026-10-20T09:00:00.000Z", "session": S, "type": "started", "file": "raft-1.json", "form": "full", "declaredMinutes": 25 })
	}

	/// A spoken answer in its home form: no ticks.
	fn spoken(id: &str) -> Value {
		json!({
			"id": id, "at": "2026-10-20T09:12:00.000Z", "session": S, "type": "spoken", "file": "raft-1.json", "screen": "say-why",
			"answer": { "as": "recorded", "seconds": 41.2, "strong": true }
		})
	}

	#[tokio::test]
	async fn every_route_needs_a_session_and_writes_need_a_trusted_origin() {
		let state = state().await;
		let phone = session(&state, A, Carrier::Bearer).await;
		let app = app(state);
		let routes: Vec<(Method, String)> = module()
			.descriptors()
			.map(|route| (Method::from_bytes(route.method.as_bytes()).unwrap(), route.path.replace(":session", S).replace(":id", "e1")))
			.collect();
		assert_eq!(routes.len(), 3, "{routes:?}");
		for (method, path) in routes {
			assert_eq!(call(&app, method.clone(), &path, None, &json!({})).await.0, StatusCode::UNAUTHORIZED, "{method} {path}");
		}
		let request = Request::builder()
			.method(Method::PUT)
			.uri(event_path(S, "e1"))
			.header(&phone.0, &phone.1)
			.header("origin", "https://evil.app.test")
			.header("sec-fetch-site", "cross-site")
			.body(Body::from(started("e1").to_string()))
			.unwrap();
		assert_eq!(app.clone().oneshot(request).await.unwrap().status(), StatusCode::FORBIDDEN);
	}

	/// The phone's whole round: an empty session, its events added, one sent
	/// again unchanged, the session read back, and the list of sessions. A
	/// browser session on the same account reads the same record.
	#[tokio::test]
	async fn a_paired_phone_sends_a_session_and_reads_it_back() {
		let state = state().await;
		let phone = session(&state, A, Carrier::Bearer).await;
		let browser = session(&state, A, Carrier::Cookie).await;
		let app = app(state);

		assert_eq!(
			call(&app, Method::GET, &events(S), Some(&phone), &Value::Null).await,
			(StatusCode::OK, json!({ "session": S, "events": [] }))
		);
		assert_eq!(
			call(&app, Method::GET, "/leetype/sessions", Some(&phone), &Value::Null).await,
			(StatusCode::OK, json!({ "sessions": [] }))
		);
		assert_eq!(
			call(&app, Method::PUT, &event_path(S, "e1"), Some(&phone), &started("e1")).await,
			(StatusCode::OK, json!({ "change": "added", "event": started("e1") }))
		);
		assert_eq!(
			call(&app, Method::PUT, &event_path(S, "e2"), Some(&phone), &spoken("e2")).await,
			(StatusCode::OK, json!({ "change": "added", "event": spoken("e2") }))
		);
		assert_eq!(call(&app, Method::PUT, &event_path(S, "e1"), Some(&phone), &started("e1")).await.1["change"], "unchanged");

		let held = json!({ "session": S, "events": [started("e1"), spoken("e2")] });
		assert_eq!(call(&app, Method::GET, &events(S), Some(&phone), &Value::Null).await, (StatusCode::OK, held.clone()));
		assert_eq!(call(&app, Method::GET, &events(S), Some(&browser), &Value::Null).await, (StatusCode::OK, held));
		assert_eq!(
			call(&app, Method::GET, "/leetype/sessions", Some(&browser), &Value::Null).await,
			(
				StatusCode::OK,
				json!({ "sessions": [{ "session": S, "file": "raft-1.json", "firstAt": "2026-10-20T09:00:00.000Z", "lastAt": "2026-10-20T09:12:00.000Z", "events": 2 }] })
			)
		);
	}

	/// The phone's `rubric-shown` (a take's rubric shown for a round) is
	/// stored and served back as the JSON value sent. Key order is not kept,
	/// so it is sent here in serde's key order and the text compares too. One
	/// with a field the phone does not send is still refused, and nothing of
	/// it written.
	#[tokio::test]
	async fn a_shown_rubric_reads_back_as_sent_in_serdes_key_order() {
		let state = state().await;
		let phone = session(&state, A, Carrier::Bearer).await;
		let app = app(state);
		let sent =
			String::from(r#"{"type":"rubric-shown","id":"e1","at":"2026-10-20T09:14:00.000Z","session":""#) + S + r#"","file":"raft-1.json","screen":"r2","take":"say-why"}"#;
		let request = Request::builder()
			.method(Method::PUT)
			.uri(event_path(S, "e1"))
			.header(&phone.0, &phone.1)
			.body(Body::from(sent.clone()))
			.unwrap();
		let response = app.clone().oneshot(request).await.unwrap();
		assert_eq!(response.status(), StatusCode::OK);
		let written = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
		assert_eq!(String::from_utf8(written.to_vec()).unwrap(), String::from(r#"{"change":"added","event":"#) + &sent + "}");

		let request = Request::builder().uri(events(S)).header(&phone.0, &phone.1).body(Body::empty()).unwrap();
		let response = app.clone().oneshot(request).await.unwrap();
		assert_eq!(response.status(), StatusCode::OK);
		let held = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
		assert_eq!(
			String::from_utf8(held.to_vec()).unwrap(),
			String::from(r#"{"session":""#) + S + r#"","events":["# + &sent + "]}",
			"the same value, in serde's key order"
		);

		let mut extra: Value = serde_json::from_str(&sent).unwrap();
		extra["id"] = json!("e2");
		extra["covers"] = json!(["r2"]);
		let (status, answer) = call(&app, Method::PUT, &event_path(S, "e2"), Some(&phone), &extra).await;
		assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{answer}");
		assert_eq!(answer["error"]["details"].as_object().unwrap().keys().collect::<Vec<_>>(), ["body"]);
		assert_eq!(
			call(&app, Method::GET, &events(S), Some(&phone), &Value::Null).await.1["events"].as_array().unwrap().len(),
			1
		);
	}

	/// A write that breaks a rule is a 422 naming each field, a soundbite
	/// included, a different event under a held id is a 409, and one
	/// subject's record is never another's.
	#[tokio::test]
	async fn refusals_name_their_fields_and_records_stay_apart() {
		let state = state().await;
		let a = session(&state, A, Carrier::Bearer).await;
		let b = session(&state, B, Carrier::Bearer).await;
		let app = app(state);
		assert_eq!(call(&app, Method::PUT, &event_path(S, "e1"), Some(&a), &started("e1")).await.0, StatusCode::OK);

		let named = |answer: (StatusCode, Value)| {
			assert_eq!(answer.0, StatusCode::UNPROCESSABLE_ENTITY, "{}", answer.1);
			let mut fields: Vec<String> = answer.1["error"]["details"].as_object().unwrap().keys().cloned().collect();
			fields.sort();
			fields
		};
		let mut whole = spoken("e2");
		whole["answer"] = json!({ "as": "typed", "ticks": ["names the term"], "typed": "Because…", "strong": true });
		assert_eq!(
			named(call(&app, Method::PUT, &event_path(S, "e2"), Some(&a), &whole).await),
			["answer.ticks", "answer.typed"],
			"soundbites stay off home"
		);
		let mut wrong = started("e3");
		wrong["at"] = json!("yesterday");
		wrong["declaredMinutes"] = json!(0);
		assert_eq!(named(call(&app, Method::PUT, &event_path(S, "e3"), Some(&a), &wrong).await), ["at", "declaredMinutes"]);
		assert_eq!(named(call(&app, Method::PUT, &event_path(S, "other"), Some(&a), &started("e4")).await), ["id"]);
		assert_eq!(named(call(&app, Method::PUT, &event_path("s-other", "e5"), Some(&a), &started("e5")).await), ["session"]);
		assert_eq!(named(call(&app, Method::PUT, &event_path(S, "e6"), Some(&a), &json!("event")).await), ["body"]);

		let mut rewritten = started("e1");
		rewritten["form"] = json!("short");
		let (status, answer) = call(&app, Method::PUT, &event_path(S, "e1"), Some(&a), &rewritten).await;
		assert_eq!(status, StatusCode::CONFLICT, "{answer}");
		assert_eq!(answer["error"]["code"], "conflict");

		assert_eq!(
			call(&app, Method::GET, &events(S), Some(&b), &Value::Null).await.1,
			json!({ "session": S, "events": [] }),
			"B reads none of A's"
		);
		assert_eq!(call(&app, Method::GET, "/leetype/sessions", Some(&b), &Value::Null).await.1, json!({ "sessions": [] }));
		assert_eq!(
			call(&app, Method::GET, &events(S), Some(&a), &Value::Null).await.1,
			json!({ "session": S, "events": [started("e1")] }),
			"nothing refused was written"
		);
	}
}

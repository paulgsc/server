//! Handlers for `routes::jobs`: the validate-and-answer half. The record's own
//! rules (append-only, what an event holds) are `jobs_repo`'s.
//!
//! **Refusals.** An event that breaks a rule is a `422` naming each field and
//! why, and a different event under an id the record holds is a `409`. Both
//! write nothing, so the phone can say which event home refused and keep it.
//!
//! **No audio.** An event names its voice notes by the phone's ids; the notes
//! stay on the phone and in the owner's Drive, and no route here takes them.
//!
//! **The deletion hold.** Each handler takes a [`SubjectId`] first and keeps
//! it until it returns, so a write holds off `DELETE /auth/account` as every
//! other subject-scoped write does (docs/identity.md, invariant 9).
//!
//! **No other writer.** [`put_event`] is the only caller of
//! `jobs_repo::put_event`; nothing writes the record in the background.

use crate::subject::SubjectId;
use crate::FileHostError;
use axum::{
	body::Bytes,
	extract::{Path, State},
	Json,
};
use chrono::{SecondsFormat, Utc};
use jobs_repo::{Event, JobsError, Record, REWRITTEN};
use serde::Serialize;
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
/// refused field, 409 for a different event under a held id, and 500 for a
/// storage failure.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "jobs_event_put", skip_all, fields(otel.kind = "server"))]
pub async fn put_event(subject: SubjectId, State(db): State<SqlitePool>, Path(id): Path<String>, body: Bytes) -> Result<Json<EventWritten>, FileHostError> {
	let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
	let (change, event) = jobs_repo::put_event(&mut tx, subject.as_str(), &id, &body, &now()).await.map_err(refusal)?;
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
			.map(|route| (Method::from_bytes(route.method.as_bytes()).unwrap(), route.path.replace(":id", "e1")))
			.collect();
		assert_eq!(routes.len(), 2, "{routes:?}");
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
}

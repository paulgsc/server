//! Handlers for `routes::app_events`: the validate-and-answer half. What an
//! event holds, and that the record is append-only, are `app_event_repo`'s.
//!
//! **Refusals.** A write that breaks one of those rules is a `422` naming
//! each field and why, and a different body under a held id is a `409`.
//! Both write nothing, so the phone keeps the event and can say home
//! refused it.
//!
//! **The deletion hold.** Each handler takes a [`SubjectId`] first and keeps
//! it until it returns, so a write holds off `DELETE /auth/account` as every
//! other subject-scoped write does (docs/identity.md, invariant 9).
//!
//! **Nothing an event says reaches a log.** Every handler here is
//! `#[instrument(skip_all)]` and records only `otel.kind` (invariant 6),
//! which `nothing_an_event_says_reaches_a_log_line` checks.
//!
//! **No other writer.** [`put_event`] is the only caller of
//! `app_event_repo::put_event`; nothing writes the record in the background.

use crate::subject::SubjectId;
use crate::FileHostError;
use app_event_repo::{Event, EventError, Summary, REWRITTEN, UNREADABLE};
use axum::{
	body::Bytes,
	extract::{Path, Query, State},
	Json,
};
use chrono::{Duration, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tracing::instrument;

/// The window a summary reads when none is asked for, in days.
pub const SUMMARY_DAYS: i64 = 7;

/// The longest window a summary reads, in days.
pub const SUMMARY_DAYS_MAX: i64 = 90;

/// `PUT /app-events/:id`
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EventWritten {
	/// `added` or `unchanged` — `app_event_repo::Change`.
	pub change: &'static str,
	/// The event as the record holds it, which the phone compares with what
	/// it sent before it lets its own copy go.
	pub event: Event,
}

/// `GET /app-events/summary`'s query: how many days back, 1 to
/// [`SUMMARY_DAYS_MAX`].
#[derive(Debug, Deserialize)]
pub struct SummaryQuery {
	days: Option<String>,
}

fn now() -> String {
	// Fixed width, as `handlers::jobs`'s.
	Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn refusal(err: EventError) -> FileHostError {
	match err {
		EventError::Invalid(problems) => FileHostError::unprocessable_entity(problems),
		EventError::Rewritten => FileHostError::Conflict(REWRITTEN),
		EventError::Unreadable => FileHostError::OperationError(UNREADABLE.to_owned()),
		EventError::Storage(err) => FileHostError::Sqlite(err),
	}
}

/// `PUT /app-events/:id`, the request body being the event itself. `BEGIN
/// IMMEDIATE`, so the read that decides and the write see one snapshot.
///
/// # Errors
/// 401 without a session, 403 from an untrusted origin, 422 naming each
/// refused field, 409 for a different event under a held id, and 500 for a
/// storage failure.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "app_event_put", skip_all, fields(otel.kind = "server"))]
pub async fn put_event(subject: SubjectId, State(db): State<SqlitePool>, Path(id): Path<String>, body: Bytes) -> Result<Json<EventWritten>, FileHostError> {
	let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
	let (change, event) = app_event_repo::put_event(&mut tx, subject.as_str(), &id, &body, &now()).await.map_err(refusal)?;
	tx.commit().await?;
	Ok(Json(EventWritten { change: change.as_str(), event }))
}

/// `GET /app-events/summary?days=N`: a summary of the caller's last `N` days.
///
/// Its events per family (`N` is [`SUMMARY_DAYS`] when unsaid), their
/// outcomes per process family and screen, and each process family's last
/// outcome that worked and that failed (paulgsc/some-ui CK8, #2388, reads
/// it).
///
/// # Errors
/// 401 without a session, 422 for `days` outside 1 to [`SUMMARY_DAYS_MAX`],
/// and 500 for a storage failure or a held event that does not read back.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "app_event_summary", skip_all, fields(otel.kind = "server"))]
pub async fn get_summary(subject: SubjectId, State(db): State<SqlitePool>, Query(query): Query<SummaryQuery>) -> Result<Json<Summary>, FileHostError> {
	let days = match query.days.as_deref() {
		None => SUMMARY_DAYS,
		Some(text) => match text.parse::<i64>() {
			Ok(days) if (1..=SUMMARY_DAYS_MAX).contains(&days) => days,
			Ok(_) | Err(_) => return Err(FileHostError::unprocessable_entity([("days", "a whole number of days from 1 to 90")])),
		},
	};
	let since = (Utc::now() - Duration::days(days)).to_rfc3339_opts(SecondsFormat::Millis, true);
	let mut conn = db.acquire().await?;
	app_event_repo::summary(&mut conn, subject.as_str(), &since).await.map(Json).map_err(refusal)
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
	use chrono::{Duration, SecondsFormat, Utc};
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
		crate::routes::app_events::app_events()
	}

	async fn state() -> TestState {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();
		TestState {
			auth: AuthContext::for_tests(pool.clone(), 100),
			pool,
		}
	}

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

	fn path(id: &str) -> String {
		String::from("/app-events/") + id
	}

	/// An hour ago, so it sits inside every window.
	fn recent() -> String {
		(Utc::now() - Duration::hours(1)).to_rfc3339_opts(SecondsFormat::Millis, true)
	}

	fn outcome(id: &str, at: &str, ending: &str, went: &str) -> Value {
		json!({
			"family": "outcome", "id": id, "at": at, "build": "42a72b958",
			"process": "phone-recording", "screen": "jobs.pass", "ending": ending, "went": went,
			"readyMs": 250, "durationMs": 4_000, "peak": 0.4
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
			.uri(path("e1"))
			.header(&phone.0, &phone.1)
			.header("origin", "https://evil.app.test")
			.header("sec-fetch-site", "cross-site")
			.body(Body::from(outcome("e1", &recent(), "kept", "worked").to_string()))
			.unwrap();
		assert_eq!(app.clone().oneshot(request).await.unwrap().status(), StatusCode::FORBIDDEN);
	}

	/// The phone's round: an event sent, sent again unchanged, a different
	/// body under its id refused, and the summary read from a browser on the
	/// same account; nothing of A's reaches B.
	#[tokio::test]
	async fn a_paired_phone_sends_its_outcomes_and_the_summary_counts_them() {
		let state = state().await;
		let phone = session(&state, A, Carrier::Bearer).await;
		let browser = session(&state, A, Carrier::Cookie).await;
		let other = session(&state, B, Carrier::Bearer).await;
		let app = app(state);
		let at = recent();
		let kept = outcome("e1", &at, "kept", "worked");
		assert_eq!(
			call(&app, Method::PUT, &path("e1"), Some(&phone), &kept).await,
			(StatusCode::OK, json!({ "change": "added", "event": kept }))
		);
		assert_eq!(call(&app, Method::PUT, &path("e1"), Some(&phone), &kept).await.1["change"], "unchanged");
		let refused = outcome("e2", &at, "blocked-denied", "failed");
		assert_eq!(call(&app, Method::PUT, &path("e2"), Some(&phone), &refused).await.0, StatusCode::OK);

		let rewritten = outcome("e1", &at, "kept", "failed");
		let (status, answer) = call(&app, Method::PUT, &path("e1"), Some(&phone), &rewritten).await;
		assert_eq!(status, StatusCode::CONFLICT, "{answer}");

		let (status, summary) = call(&app, Method::GET, "/app-events/summary", Some(&browser), &Value::Null).await;
		assert_eq!(status, StatusCode::OK, "{summary}");
		assert_eq!(summary["families"], json!([{ "family": "outcome", "events": 2 }]));
		assert_eq!(
			summary["outcomes"],
			json!([{ "process": "phone-recording", "screen": "jobs.pass", "worked": 1, "failed": 1, "stopped": 0, "readyMs": { "median": 250, "max": 250, "of": 2 } }])
		);
		assert_eq!(summary["last"][0]["worked"], json!({ "at": at, "screen": "jobs.pass", "ending": "kept" }));
		assert_eq!(summary["last"][0]["failed"]["ending"], "blocked-denied");
		assert_eq!(summary["partial"], false);

		let (_, theirs) = call(&app, Method::GET, "/app-events/summary?days=90", Some(&other), &Value::Null).await;
		assert_eq!((theirs["families"].clone(), theirs["outcomes"].clone()), (json!([]), json!([])), "B reads none of A's");
	}

	#[tokio::test]
	async fn refusals_name_their_fields() {
		let state = state().await;
		let phone = session(&state, A, Carrier::Bearer).await;
		let app = app(state);
		let named = |answer: (StatusCode, Value)| {
			assert_eq!(answer.0, StatusCode::UNPROCESSABLE_ENTITY, "{}", answer.1);
			let mut fields: Vec<String> = answer.1["error"]["details"].as_object().unwrap().keys().cloned().collect();
			fields.sort();
			fields
		};
		let mut wrong = outcome("e1", "yesterday", "kept", "worked");
		wrong["screen"] = json!("The pass screen");
		assert_eq!(named(call(&app, Method::PUT, &path("e1"), Some(&phone), &wrong).await), ["at", "screen"]);
		assert_eq!(
			named(call(&app, Method::PUT, &path("e9"), Some(&phone), &outcome("e1", &recent(), "kept", "worked")).await),
			["id"]
		);
		assert_eq!(named(call(&app, Method::PUT, &path("e1"), Some(&phone), &json!("event")).await), ["body"]);
		for days in ["0", "91", "week"] {
			let answer = call(&app, Method::GET, &(String::from("/app-events/summary?days=") + days), Some(&phone), &Value::Null).await;
			assert_eq!(named(answer), ["days"], "{days}");
		}
		let (_, summary) = call(&app, Method::GET, "/app-events/summary", Some(&phone), &Value::Null).await;
		assert_eq!(summary["families"], json!([]), "nothing refused was written");
	}

	/// docs/identity.md invariant 6: nothing a write carries reaches a log
	/// line or span field, kept or refused.
	#[tokio::test]
	async fn nothing_an_event_says_reaches_a_log_line() {
		const SAID: &str = "kept-losing-my-place-zq7";
		let state = state().await;
		let phone = session(&state, A, Carrier::Bearer).await;
		let app = app(state);
		let (captured, _guard) = crate::privacy::capture_where(crate::auth::loggable);
		let said = outcome("e1", &recent(), SAID, "worked");
		assert_eq!(call(&app, Method::PUT, &path("e1"), Some(&phone), &said).await.0, StatusCode::OK);
		let mut refused = said.clone();
		refused["screen"] = json!(SAID.to_owned() + " with a space");
		assert_eq!(call(&app, Method::PUT, &path("e2"), Some(&phone), &refused).await.0, StatusCode::UNPROCESSABLE_ENTITY);
		assert_eq!(call(&app, Method::GET, "/app-events/summary", Some(&phone), &Value::Null).await.0, StatusCode::OK);

		let lines = captured.lines();
		assert!(lines.iter().any(|line| line.contains("otel.kind")), "no handler span was captured: {lines:#?}");
		for line in &lines {
			assert!(!line.contains(SAID), "what an event says reached a log line: {line}");
		}
	}
}

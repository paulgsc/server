//! Handlers for `routes::sharing`: a paired phone's choices of what it keeps
//! sending home. The record's own rules (append-only, two times per choice)
//! are `sharing_repo`'s.
//!
//! **The deletion hold.** Each handler takes a [`PairedDevice`], whose
//! [`crate::subject::SubjectId`] it keeps until it returns, as every other
//! subject-scoped write does (docs/identity.md, invariant 9).
//!
//! **No other writer.** [`put_choice`] is the only caller of
//! `sharing_repo::choose`. What reads the choices is each phone write's check
//! ([`refusing`]).

use crate::subject::{paired_device, PairedDevice};
use crate::FileHostError;
use axum::{
	extract::{Path, State},
	http::HeaderMap,
	Json,
};
use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use sharing_repo::{Change, Scope};
use sqlx::{SqliteConnection, SqlitePool};
use tracing::instrument;

/// What a stopped write is refused with: the phone says "Sending to home is
/// off", and keeps what it holds.
pub const STOPPED: &str = "this phone stopped sending it to home";

/// A pairing's choices: what it keeps sending now and since when, and every
/// change, in the order home heard them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Choices {
	pub kept: Vec<Kept>,
	pub changes: Vec<Change>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Kept {
	pub scope: Scope,
	pub since: String,
}

/// `PUT /sharing/:scope`
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Choice {
	pub kept: bool,
}

fn now() -> String {
	// Fixed width, as `handlers::jobs`'s.
	Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn choices_of(changes: Vec<Change>) -> Choices {
	let kept = Scope::ALL
		.into_iter()
		.filter_map(|scope| {
			let latest = changes.iter().rev().find(|change| change.scope == scope)?;
			latest.kept.then(|| Kept { scope, since: latest.at.clone() })
		})
		.collect();
	Choices { kept, changes }
}

/// `GET /sharing`
///
/// # Errors
/// 401 without a session, 403 for a browser's, and 500 for a storage failure.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "sharing_choices", skip_all, fields(otel.kind = "server"))]
pub async fn get_choices(paired: PairedDevice, State(db): State<SqlitePool>) -> Result<Json<Choices>, FileHostError> {
	let mut conn = db.acquire().await?;
	let changes = sharing_repo::changes(&mut conn, paired.subject.as_str(), paired.device()).await?;
	Ok(Json(choices_of(changes)))
}

/// `PUT /sharing/:scope`, `{ "kept": true }` to keep sending `scope` from now
/// on, `false` to stop; choosing what already holds changes nothing.
///
/// # Errors
/// 401 without a session, 403 for a browser's or from an untrusted origin,
/// 422 for an unknown scope or a body that is not a choice, and 500 for a
/// storage failure.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "sharing_choose", skip_all, fields(otel.kind = "server"))]
pub async fn put_choice(paired: PairedDevice, State(db): State<SqlitePool>, Path(scope): Path<String>, body: axum::body::Bytes) -> Result<Json<Choices>, FileHostError> {
	let scope = Scope::named(&scope).ok_or_else(|| FileHostError::unprocessable_entity([("scope", "is sessions, shelf, aph or jobs")]))?;
	let choice: Choice = serde_json::from_slice(&body).map_err(|_| FileHostError::unprocessable_entity([("body", "is { \"kept\": true } or { \"kept\": false }")]))?;
	let (subject, device) = (paired.subject.as_str(), paired.device());
	let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
	sharing_repo::choose(&mut tx, subject, device, scope, choice.kept, &now()).await?;
	let changes = sharing_repo::changes(&mut tx, subject, device).await?;
	tx.commit().await?;
	Ok(Json(choices_of(changes)))
}

/// Refuses a paired phone's write of `scope` its person stopped
/// (`sharing_repo::refuses`); a browser's write is never refused here. Run it
/// on the write's own transaction, before the write.
///
/// Only aph and the Jobs record call it: their phone writes come from a
/// standing choice alone. Sessions and the shelf also go home by a tap of
/// Settings' one-time send, which no choice covers, so they are not checked.
///
/// # Errors
/// 409 ([`STOPPED`]) when stopped, and 500 for a storage failure.
pub async fn refusing(conn: &mut SqliteConnection, subject: &str, headers: &HeaderMap, scope: Scope) -> Result<(), FileHostError> {
	let Some(device) = paired_device(headers) else {
		return Ok(());
	};
	if sharing_repo::refuses(conn, subject, &device, scope).await? {
		return Err(FileHostError::Conflict(STOPPED));
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	//! Through the real router, with real phone sessions (bearer tokens, as a
	//! paired phone sends) and a browser's, and the Jobs record's write as
	//! the one a choice gates.

	use crate::auth::{
		cookie::{Carrier, SessionToken},
		now, AuthContext,
	};
	use axum::{
		body::Body,
		extract::FromRef,
		http::{
			header::{AUTHORIZATION, COOKIE},
			HeaderName, Method, Request, StatusCode,
		},
		Router,
	};
	use serde_json::{json, Value};
	use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};
	use tower::ServiceExt;

	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");
	const A: &str = "subject-a";

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

	fn app(state: TestState) -> Router {
		crate::routes::sharing::sharing()
			.into_table_router()
			.merge(crate::routes::jobs::jobs().into_table_router())
			.with_state(state)
	}

	async fn call(app: &Router, method: Method, path: &str, auth: Option<&(HeaderName, String)>, body: &Value) -> (StatusCode, Value) {
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

	fn event(id: &str) -> Value {
		json!({
			"id": id, "at": "2026-10-19T08:00:00.000Z", "type": "taken",
			"day": "2026-10-19", "pickId": "blu-7f2a", "fingerprint": "bluesight|data analyst|remote"
		})
	}

	async fn send_jobs(app: &Router, phone: &(HeaderName, String), id: &str) -> StatusCode {
		call(app, Method::PUT, &(String::from("/jobs/events/") + id), Some(phone), &event(id)).await.0
	}

	#[tokio::test]
	async fn only_a_paired_phone_makes_or_reads_a_choice() {
		let state = state().await;
		let browser = session(&state, A, Carrier::Cookie).await;
		let app = app(state);
		assert_eq!(call(&app, Method::GET, "/sharing", None, &Value::Null).await.0, StatusCode::UNAUTHORIZED);
		assert_eq!(call(&app, Method::GET, "/sharing", Some(&browser), &Value::Null).await.0, StatusCode::FORBIDDEN);
		assert_eq!(
			call(&app, Method::PUT, "/sharing/jobs", Some(&browser), &json!({ "kept": true })).await.0,
			StatusCode::FORBIDDEN
		);
	}

	#[tokio::test]
	async fn a_choice_reads_back_and_refusals_name_what_is_wrong() {
		let state = state().await;
		let phone = session(&state, A, Carrier::Bearer).await;
		let app = app(state);
		assert_eq!(
			call(&app, Method::GET, "/sharing", Some(&phone), &Value::Null).await,
			(StatusCode::OK, json!({ "kept": [], "changes": [] }))
		);
		let (status, kept) = call(&app, Method::PUT, "/sharing/jobs", Some(&phone), &json!({ "kept": true })).await;
		assert_eq!(status, StatusCode::OK);
		assert_eq!(kept["kept"][0]["scope"], "jobs");
		assert_eq!(kept["changes"].as_array().unwrap().len(), 1);
		let again = call(&app, Method::PUT, "/sharing/jobs", Some(&phone), &json!({ "kept": true })).await.1;
		assert_eq!(again, kept, "keeping what is kept changes nothing");

		let named = |answer: (StatusCode, Value)| {
			assert_eq!(answer.0, StatusCode::UNPROCESSABLE_ENTITY, "{}", answer.1);
			answer.1["error"]["details"].as_object().unwrap().keys().cloned().collect::<Vec<String>>()
		};
		assert_eq!(
			named(call(&app, Method::PUT, "/sharing/reflections", Some(&phone), &json!({ "kept": true })).await),
			["scope"]
		);
		assert_eq!(named(call(&app, Method::PUT, "/sharing/aph", Some(&phone), &json!({ "kept": "yes" })).await), ["body"]);
	}

	/// What a phone that told home its choices sends is taken only while it is
	/// kept; a phone that never told home any, and a browser, are refused
	/// nothing; and each pairing's choices are its own.
	#[tokio::test]
	async fn home_takes_a_phones_writes_only_while_they_are_kept() {
		let state = state().await;
		let phone = session(&state, A, Carrier::Bearer).await;
		let other_phone = session(&state, A, Carrier::Bearer).await;
		let browser = session(&state, A, Carrier::Cookie).await;
		let app = app(state);

		assert_eq!(send_jobs(&app, &phone, "e1").await, StatusCode::OK, "told home nothing yet");
		call(&app, Method::PUT, "/sharing/aph", Some(&phone), &json!({ "kept": true })).await;
		let (status, refused) = call(&app, Method::PUT, "/jobs/events/e2", Some(&phone), &event("e2")).await;
		assert_eq!(status, StatusCode::CONFLICT, "told home, but never kept the Jobs record");
		assert_eq!(refused["error"]["message"], super::STOPPED);

		call(&app, Method::PUT, "/sharing/jobs", Some(&phone), &json!({ "kept": true })).await;
		assert_eq!(send_jobs(&app, &phone, "e2").await, StatusCode::OK);
		call(&app, Method::PUT, "/sharing/jobs", Some(&phone), &json!({ "kept": false })).await;
		assert_eq!(send_jobs(&app, &phone, "e3").await, StatusCode::CONFLICT, "stopped");

		assert_eq!(send_jobs(&app, &other_phone, "e3").await, StatusCode::OK, "another pairing's choices are its own");
		assert_eq!(send_jobs(&app, &browser, "e4").await, StatusCode::OK, "a browser makes no choice and is refused none");
		let held = call(&app, Method::GET, "/jobs", Some(&browser), &Value::Null).await.1;
		assert_eq!(held["events"].as_array().unwrap().len(), 4, "what home took before the stop stays");
	}
}

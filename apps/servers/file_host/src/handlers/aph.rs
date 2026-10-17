//! Handlers for `routes::aph`: the validate-and-answer half. The record's own
//! rules (append-only, what a revision may change, one plain entry per
//! checkpoint a day) are `aph_repo`'s.
//!
//! **Refusals.** A write that breaks a rule is a `422` naming each field and
//! why, and writes nothing, so the phone can say which entry home refused and
//! keep it.
//!
//! **The deletion hold.** Each handler takes a [`SubjectId`] first and keeps
//! it until it returns, so a write holds off `DELETE /auth/account` as every
//! other subject-scoped write does (docs/identity.md, invariant 9).
//!
//! **No other writer.** [`put_entry`] and [`put_settings`] are the only
//! callers of `aph_repo::put_entry` and `::put_settings`; nothing writes the
//! record in the background.

use crate::subject::SubjectId;
use crate::FileHostError;
use aph_repo::{AphError, Entry, Record, Settings};
use axum::{
	body::Bytes,
	extract::{Path, State},
	Json,
};
use chrono::{SecondsFormat, Utc};
use serde::Serialize;
use sqlx::SqlitePool;
use tracing::instrument;

/// `PUT /aph/entries/:id`
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EntryWritten {
	/// `kept`, `revised` or `unchanged` — `aph_repo::Change`.
	pub change: &'static str,
	/// The entry as it now stands, which the phone compares with what it sent
	/// before it lets go of its own copy.
	pub entry: Entry,
}

/// `PUT /aph/settings`
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SettingsWritten {
	pub change: &'static str,
	pub settings: Settings,
}

fn now() -> String {
	// Fixed width, as `handlers::shelf`'s.
	Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn refusal(err: AphError) -> FileHostError {
	match err {
		AphError::Invalid(problems) => FileHostError::unprocessable_entity(problems),
		AphError::Unreadable => FileHostError::OperationError("a stored aph revision does not read back".to_owned()),
		AphError::Storage(err) => FileHostError::Sqlite(err),
	}
}

/// This subject's record as it stands.
pub(crate) async fn current(db: &SqlitePool, subject: &str) -> Result<Record, FileHostError> {
	aph_repo::record(&mut *db.acquire().await?, subject).await.map_err(refusal)
}

/// Writes `body` as entry `id`'s next revision. `BEGIN IMMEDIATE`, so the
/// reads that decide and the write see one snapshot.
pub(crate) async fn write_entry(db: &SqlitePool, subject: &str, id: &str, body: &[u8], now: &str) -> Result<EntryWritten, FileHostError> {
	let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
	let (change, entry) = aph_repo::put_entry(&mut tx, subject, id, body, now).await.map_err(refusal)?;
	tx.commit().await?;
	Ok(EntryWritten { change: change.as_str(), entry })
}

/// Writes `body` as the next settings revision, as [`write_entry`].
pub(crate) async fn write_settings(db: &SqlitePool, subject: &str, body: &[u8], now: &str) -> Result<SettingsWritten, FileHostError> {
	let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
	let (change, settings) = aph_repo::put_settings(&mut tx, subject, body, now).await.map_err(refusal)?;
	tx.commit().await?;
	Ok(SettingsWritten {
		change: change.as_str(),
		settings,
	})
}

/// `GET /aph`
///
/// # Errors
/// 401 without a session, and 500 for a storage failure.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "aph_record", skip_all, fields(otel.kind = "server"))]
pub async fn get_record(subject: SubjectId, State(db): State<SqlitePool>) -> Result<Json<Record>, FileHostError> {
	current(&db, subject.as_str()).await.map(Json)
}

/// `PUT /aph/entries/:id`, the request body being the entry itself.
///
/// # Errors
/// 401 without a session, 403 from an untrusted origin, 422 naming each
/// refused field, and 500 for a storage failure.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "aph_entry_put", skip_all, fields(otel.kind = "server"))]
pub async fn put_entry(subject: SubjectId, State(db): State<SqlitePool>, Path(id): Path<String>, body: Bytes) -> Result<Json<EntryWritten>, FileHostError> {
	write_entry(&db, subject.as_str(), &id, &body, &now()).await.map(Json)
}

/// `PUT /aph/settings`, the request body being the settings.
///
/// # Errors
/// 401 without a session, 403 from an untrusted origin, 422 naming each
/// refused field, and 500 for a storage failure.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "aph_settings_put", skip_all, fields(otel.kind = "server"))]
pub async fn put_settings(subject: SubjectId, State(db): State<SqlitePool>, body: Bytes) -> Result<Json<SettingsWritten>, FileHostError> {
	write_settings(&db, subject.as_str(), &body, &now()).await.map(Json)
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
		crate::routes::aph::aph()
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

	fn settings() -> Value {
		json!({
			"checkpoints": [
				{ "id": "7", "label": "7:00", "minutes": 420, "goal": 4100 },
				{ "id": "12", "label": "12:00", "minutes": 720, "goal": 5100 }
			],
			"labels": ["no-bs"], "step": 100, "usualLow": 3000, "usualHigh": 6500, "tolerance": 100,
			"since": "2026-09-14"
		})
	}

	fn entry(id: &str) -> Value {
		json!({
			"id": id, "day": "2026-10-17", "checkpoint": "7", "time": null,
			"mine": { "value": 4300, "approx": true }, "theirs": null, "goal": 4100,
			"labels": [], "note": null, "review": null
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
		assert_eq!(routes.len(), 3, "{routes:?}");
		for (method, path) in routes {
			assert_eq!(call(&app, method.clone(), &path, None, &json!({})).await.0, StatusCode::UNAUTHORIZED, "{method} {path}");
		}
		let request = Request::builder()
			.method(Method::PUT)
			.uri("/aph/settings")
			.header(&phone.0, &phone.1)
			.header("origin", "https://evil.app.test")
			.header("sec-fetch-site", "cross-site")
			.body(Body::from(settings().to_string()))
			.unwrap();
		assert_eq!(app.clone().oneshot(request).await.unwrap().status(), StatusCode::FORBIDDEN);
	}

	/// The phone's whole round: an empty record, the settings, an entry kept,
	/// the same entry unchanged, their figure in as a revision, and the record
	/// read back as it stands. A browser session on the same account reads the
	/// same record.
	#[tokio::test]
	async fn a_paired_phone_keeps_its_record_and_reads_it_back() {
		let state = state().await;
		let phone = session(&state, A, Carrier::Bearer).await;
		let browser = session(&state, A, Carrier::Cookie).await;
		let app = app(state);

		assert_eq!(
			call(&app, Method::GET, "/aph", Some(&phone), &Value::Null).await,
			(StatusCode::OK, json!({ "settings": null, "entries": [] }))
		);
		assert_eq!(
			call(&app, Method::PUT, "/aph/settings", Some(&phone), &settings()).await,
			(StatusCode::OK, json!({ "change": "kept", "settings": settings() }))
		);
		let mut logged = entry("e1");
		assert_eq!(
			call(&app, Method::PUT, "/aph/entries/e1", Some(&phone), &logged).await,
			(StatusCode::OK, json!({ "change": "kept", "entry": logged }))
		);
		assert_eq!(call(&app, Method::PUT, "/aph/entries/e1", Some(&phone), &logged).await.1["change"], "unchanged");
		logged["theirs"] = json!({ "value": 4250 });
		assert_eq!(
			call(&app, Method::PUT, "/aph/entries/e1", Some(&phone), &logged).await,
			(StatusCode::OK, json!({ "change": "revised", "entry": logged }))
		);
		let record = json!({ "settings": settings(), "entries": [logged] });
		assert_eq!(call(&app, Method::GET, "/aph", Some(&phone), &Value::Null).await, (StatusCode::OK, record.clone()));
		assert_eq!(call(&app, Method::GET, "/aph", Some(&browser), &Value::Null).await, (StatusCode::OK, record));
	}

	/// A write that breaks a rule is a 422 naming each field, and one subject's
	/// record is never another's.
	#[tokio::test]
	async fn refusals_name_their_fields_and_records_stay_apart() {
		let state = state().await;
		let a = session(&state, A, Carrier::Bearer).await;
		let b = session(&state, B, Carrier::Bearer).await;
		let app = app(state);
		call(&app, Method::PUT, "/aph/settings", Some(&a), &settings()).await;
		let mut logged = entry("e1");
		logged["theirs"] = json!({ "value": 4250 });
		assert_eq!(call(&app, Method::PUT, "/aph/entries/e1", Some(&a), &logged).await.0, StatusCode::OK);

		let named = |answer: (StatusCode, Value)| {
			assert_eq!(answer.0, StatusCode::UNPROCESSABLE_ENTITY, "{}", answer.1);
			let mut fields: Vec<String> = answer.1["error"]["details"].as_object().unwrap().keys().cloned().collect();
			fields.sort();
			fields
		};
		let mut rewritten = logged.clone();
		rewritten["day"] = json!("2026-10-16");
		rewritten["theirs"] = json!(null);
		assert_eq!(named(call(&app, Method::PUT, "/aph/entries/e1", Some(&a), &rewritten).await), ["day", "theirs"]);
		assert_eq!(
			named(call(&app, Method::PUT, "/aph/entries/e2", Some(&a), &entry("e2")).await),
			["labels"],
			"a second plain figure"
		);
		assert_eq!(named(call(&app, Method::PUT, "/aph/entries/other", Some(&a), &entry("e3")).await), ["id"]);
		assert_eq!(named(call(&app, Method::PUT, "/aph/settings", Some(&a), &json!("settings")).await), ["body"]);

		assert_eq!(
			call(&app, Method::GET, "/aph", Some(&b), &Value::Null).await.1,
			json!({ "settings": null, "entries": [] }),
			"B reads none of A's"
		);
		assert_eq!(
			call(&app, Method::GET, "/aph", Some(&a), &Value::Null).await.1["entries"],
			json!([logged]),
			"nothing refused was written"
		);
	}
}

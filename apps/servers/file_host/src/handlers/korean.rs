//! Handlers for `routes::korean`: the validate-and-answer half. The record's
//! own rules (append-only, a removal as a revision, content never parsed)
//! are `korean_repo`'s.
//!
//! **Refusals.** A write with an unknown kind, a key that is not plain or a
//! body that is not a JSON object or array within the ceiling is a `422`
//! naming each field and why, and writes nothing, so the phone can say which
//! entry home refused and keep it.
//!
//! **The deletion hold.** Each handler takes a [`SubjectId`] first and keeps
//! it until it returns, so a write holds off `DELETE /auth/account` as every
//! other subject-scoped write does (docs/identity.md, invariant 9).
//!
//! **No other writer.** [`put_entry`] and [`delete_entry`], and a batch's
//! items (`handlers::sync`), are the only callers of `korean_repo::put` and
//! `::remove`; nothing writes the record in the background.

use crate::subject::SubjectId;
use crate::FileHostError;
use axum::{
	body::Bytes,
	extract::{Path, State},
	Json,
};
use chrono::{SecondsFormat, Utc};
use korean_repo::{Entry, Kind, KoreanError};
use serde::Serialize;
use sqlx::SqlitePool;
use tracing::instrument;

/// `GET /korean/:kind`
#[derive(Debug, Serialize)]
pub struct Entries {
	/// Each standing entry of the kind, in the order its key was first
	/// written.
	pub entries: Vec<Entry>,
}

/// `PUT` and `DELETE /korean/:kind/:key`
#[derive(Debug, Serialize)]
pub struct Written {
	/// `kept`, `revised`, `removed` or `unchanged` — `korean_repo::Change`.
	pub change: &'static str,
	/// The entry as it now stands, which the phone compares with what it sent
	/// before it lets go of its own copy: its body is the bytes sent, or null
	/// once removed. Null for a removal of a key never written.
	pub entry: Option<Entry>,
}

fn now() -> String {
	// Fixed width, as `handlers::aph`'s.
	Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

pub(crate) fn refusal(err: KoreanError) -> FileHostError {
	match err {
		KoreanError::Invalid(problems) => FileHostError::unprocessable_entity(problems),
		KoreanError::Unreadable => FileHostError::OperationError("a stored Korean revision does not read back".to_owned()),
		KoreanError::Storage(err) => FileHostError::Sqlite(err),
	}
}

/// Writes `body` as the next revision of `kind`'s `key`, or, with no body,
/// removes it. `BEGIN IMMEDIATE`, so the read that decides and the write see
/// one snapshot.
async fn write(db: &SqlitePool, subject: &str, kind: &str, key: &str, body: Option<&[u8]>) -> Result<Written, FileHostError> {
	let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
	let now = now();
	let (change, entry) = match body {
		Some(body) => korean_repo::put(&mut tx, subject, kind, key, body, &now).await.map(|(change, entry)| (change, Some(entry))),
		None => korean_repo::remove(&mut tx, subject, kind, key, &now).await,
	}
	.map_err(refusal)?;
	tx.commit().await?;
	Ok(Written { change: change.as_str(), entry })
}

/// `GET /korean/:kind`: every entry of the kind that stands for the caller.
///
/// # Errors
/// 401 without a session, 404 for a kind that is not one of
/// `korean_repo::Kind`'s, and 500 for a storage failure.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "korean_entries", skip_all, fields(otel.kind = "server"))]
pub async fn get_entries(subject: SubjectId, State(db): State<SqlitePool>, Path(kind): Path<String>) -> Result<Json<Entries>, FileHostError> {
	let kind = Kind::named(&kind).ok_or(FileHostError::NotFound)?;
	let mut conn = db.acquire().await?;
	let entries = korean_repo::standing(&mut conn, subject.as_str(), kind).await.map_err(refusal)?;
	Ok(Json(Entries { entries }))
}

/// `PUT /korean/:kind/:key`, the request body being the entry itself.
///
/// # Errors
/// 401 without a session, 403 from an untrusted origin, 422 naming each
/// refused field, and 500 for a storage failure.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "korean_entry_put", skip_all, fields(otel.kind = "server"))]
pub async fn put_entry(subject: SubjectId, State(db): State<SqlitePool>, Path((kind, key)): Path<(String, String)>, body: Bytes) -> Result<Json<Written>, FileHostError> {
	write(&db, subject.as_str(), &kind, &key, Some(&body)).await.map(Json)
}

/// `DELETE /korean/:kind/:key`: a revision that removes it, the earlier ones
/// kept.
///
/// # Errors
/// As [`put_entry`]'s, with `body` never refused.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "korean_entry_delete", skip_all, fields(otel.kind = "server"))]
pub async fn delete_entry(subject: SubjectId, State(db): State<SqlitePool>, Path((kind, key)): Path<(String, String)>) -> Result<Json<Written>, FileHostError> {
	write(&db, subject.as_str(), &kind, &key, None).await.map(Json)
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
			header::{AUTHORIZATION, CONTENT_TYPE, COOKIE},
			HeaderName, Method, Request, StatusCode,
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
		crate::routes::korean::korean()
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
		module().into_table_router().with_state(state)
	}

	async fn call(app: &Router, method: Method, path: &str, auth: Option<&(HeaderName, String)>, body: &str) -> (StatusCode, Value) {
		let mut request = Request::builder().method(method).uri(path).header(CONTENT_TYPE, "application/json");
		if let Some((name, value)) = auth {
			request = request.header(name, value);
		}
		let response = app.clone().oneshot(request.body(Body::from(body.to_owned())).unwrap()).await.unwrap();
		let status = response.status();
		let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
		(status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
	}

	/// `answer`'s entry, or each standing entry, without when it was written.
	fn undated(mut value: Value) -> Value {
		let undate = |entry: &mut Value| {
			assert!(entry.as_object_mut().unwrap().remove("writtenAt").is_some_and(|at| at.is_string()), "{entry}");
		};
		if let Some(entries) = value.get_mut("entries").and_then(Value::as_array_mut) {
			entries.iter_mut().for_each(undate);
		}
		if let Some(entry) = value.get_mut("entry").filter(|entry| entry.is_object()) {
			undate(entry);
		}
		value
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
					route.path.replace(":kind", "set").replace(":key", "cafe"),
				)
			})
			.collect();
		assert_eq!(routes.len(), 3, "{routes:?}");
		for (method, path) in routes {
			assert_eq!(call(&app, method.clone(), &path, None, "{}").await.0, StatusCode::UNAUTHORIZED, "{method} {path}");
		}
		let request = Request::builder()
			.method(Method::PUT)
			.uri("/korean/set/cafe")
			.header(&phone.0, &phone.1)
			.header("origin", "https://evil.app.test")
			.header("sec-fetch-site", "cross-site")
			.body(Body::from("{}"))
			.unwrap();
		assert_eq!(app.clone().oneshot(request).await.unwrap().status(), StatusCode::FORBIDDEN);
	}

	/// The phone's whole round: nothing held, a set kept, the same bytes
	/// unchanged, a revision, a removal, and the kind read back as it stands
	/// from a browser on the same account too.
	#[tokio::test]
	async fn a_paired_phone_keeps_its_record_and_reads_it_back() {
		let state = state().await;
		let phone = session(&state, A, Carrier::Bearer).await;
		let browser = session(&state, A, Carrier::Cookie).await;
		let app = app(state);

		assert_eq!(call(&app, Method::GET, "/korean/set", Some(&phone), "").await, (StatusCode::OK, json!({ "entries": [] })));
		let first = json!({ "deck": { "id": "cafe" }, "addedAt": "t0", "openedAt": null });
		let (status, kept) = call(&app, Method::PUT, "/korean/set/cafe", Some(&phone), &first.to_string()).await;
		assert_eq!(
			(status, undated(kept)),
			(StatusCode::OK, json!({ "change": "kept", "entry": { "key": "cafe", "revision": 1, "body": first } }))
		);
		assert_eq!(call(&app, Method::PUT, "/korean/set/cafe", Some(&phone), &first.to_string()).await.1["change"], "unchanged");
		let opened = json!({ "deck": { "id": "cafe" }, "addedAt": "t0", "openedAt": "t1" });
		assert_eq!(call(&app, Method::PUT, "/korean/set/cafe", Some(&phone), &opened.to_string()).await.1["change"], "revised");
		call(&app, Method::PUT, "/korean/set/bakery", Some(&phone), "{}").await;
		call(&app, Method::PUT, "/korean/pace/w-1", Some(&phone), r#"{"factor":1,"seen":2}"#).await;

		let (status, removed) = call(&app, Method::DELETE, "/korean/set/bakery", Some(&phone), "").await;
		assert_eq!(
			(status, undated(removed)),
			(StatusCode::OK, json!({ "change": "removed", "entry": { "key": "bakery", "revision": 2, "body": null } }))
		);
		assert_eq!(
			call(&app, Method::DELETE, "/korean/set/never", Some(&phone), "").await,
			(StatusCode::OK, json!({ "change": "unchanged", "entry": null }))
		);

		let sets = json!({ "entries": [{ "key": "cafe", "revision": 2, "body": opened }] });
		assert_eq!(undated(call(&app, Method::GET, "/korean/set", Some(&phone), "").await.1), sets);
		assert_eq!(undated(call(&app, Method::GET, "/korean/set", Some(&browser), "").await.1), sets);
		assert_eq!(
			undated(call(&app, Method::GET, "/korean/pace", Some(&phone), "").await.1),
			json!({ "entries": [{ "key": "w-1", "revision": 1, "body": { "factor": 1, "seen": 2 } }] })
		);
		assert_eq!(call(&app, Method::GET, "/korean/words", Some(&phone), "").await.0, StatusCode::NOT_FOUND, "an unknown kind");
	}

	/// A refused write is a 422 naming each field, and one subject's record is
	/// never another's.
	#[tokio::test]
	async fn refusals_name_their_fields_and_records_stay_apart() {
		let state = state().await;
		let pool = state.pool.clone();
		let a = session(&state, A, Carrier::Bearer).await;
		let b = session(&state, B, Carrier::Bearer).await;
		let app = app(state);
		assert_eq!(call(&app, Method::PUT, "/korean/tally/s-1", Some(&a), r#"{"shown":1}"#).await.0, StatusCode::OK);

		let named = |answer: (StatusCode, Value)| {
			assert_eq!(answer.0, StatusCode::UNPROCESSABLE_ENTITY, "{}", answer.1);
			let mut fields: Vec<String> = answer.1["error"]["details"].as_object().unwrap().keys().cloned().collect();
			fields.sort();
			fields
		};
		assert_eq!(named(call(&app, Method::PUT, "/korean/words/.x", Some(&a), "\"text\"").await), ["body", "key", "kind"]);
		assert_eq!(named(call(&app, Method::PUT, "/korean/tally/s-1", Some(&a), "{ not json").await), ["body"]);
		assert_eq!(named(call(&app, Method::DELETE, "/korean/words/s-1", Some(&a), "").await), ["kind"]);

		assert_eq!(
			call(&app, Method::GET, "/korean/tally", Some(&b), "").await.1,
			json!({ "entries": [] }),
			"B reads none of A's"
		);
		assert_eq!(
			call(&app, Method::DELETE, "/korean/tally/s-1", Some(&b), "").await.1["change"],
			"unchanged",
			"nor removes it"
		);
		let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM korean_entry").fetch_one(&pool).await.unwrap();
		assert_eq!(rows, 1, "nothing refused was written");
	}
}

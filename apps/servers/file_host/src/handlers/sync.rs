//! Handlers for `routes::sync`: a kept record's writes in one request.
//!
//! Each item is answered as its own route would answer it, and the record's
//! changes are read on from a mark. A collection is one record's own storage
//! behind [`Collection`]: no store is shared between records, and nothing
//! here keeps a time per item.
//!
//! **One transaction per batch.** `BEGIN IMMEDIATE`, so every item and the
//! choice it is checked against see one snapshot. An item home refuses (a
//! `422` naming its fields, or a `409`) writes nothing and the rest go on;
//! a storage failure ends the whole batch, and none of it is kept.
//!
//! **The deletion hold.** Each handler takes a [`SubjectId`] first and keeps
//! it until it returns (docs/identity.md, invariant 9).

use crate::handlers::sharing::refusing;
use crate::subject::SubjectId;
use crate::FileHostError;
use aph_repo::AphError;
use axum::{
	body::Bytes,
	extract::{Path, Query, State},
	http::HeaderMap,
	Json,
};
use chrono::{SecondsFormat, Utc};
use jobs_repo::{JobsError, REWRITTEN};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sharing_repo::Scope;
use sqlx::{SqliteConnection, SqlitePool};
use std::collections::BTreeMap;
use tracing::instrument;

/// The most items one batch carries.
pub const BATCH_CEILING: usize = 100;

/// The most items one read answers with.
const READ_CEILING: i64 = 200;

/// How a batch item ended, as its own route would have answered.
enum Outcome {
	/// Stored, or already held: the route's `change`.
	Written(&'static str),
	/// A `422`: each refused field and why.
	Refused(Vec<(&'static str, &'static str)>),
	/// A `409`: what it conflicts with.
	Conflict(&'static str),
}

/// One record's storage, as the sync routes reach it.
trait Collection {
	/// What its person keeps sending when this record goes home.
	const SCOPE: Scope;
	/// Where a read left off.
	type Mark: Default;

	async fn put(conn: &mut SqliteConnection, subject: &str, id: &str, body: &[u8], now: &str) -> Result<Outcome, FileHostError>;

	async fn since(conn: &mut SqliteConnection, subject: &str, after: &Self::Mark, limit: i64) -> Result<(Vec<Value>, Self::Mark), FileHostError>;

	/// The mark as `next` gives it, and back.
	fn encode(mark: &Self::Mark) -> String;
	fn decode(text: &str) -> Option<Self::Mark>;
}

/// `parts` joined by `|`, which no id or time here holds.
fn joined(parts: &[&str]) -> String {
	parts.join("|")
}

fn values<T: Serialize>(items: &[T]) -> Result<Vec<Value>, FileHostError> {
	items.iter().map(|item| serde_json::to_value(item).map_err(FileHostError::from)).collect()
}

struct Jobs;

impl Collection for Jobs {
	const SCOPE: Scope = Scope::Jobs;
	type Mark = jobs_repo::Mark;

	async fn put(conn: &mut SqliteConnection, subject: &str, id: &str, body: &[u8], now: &str) -> Result<Outcome, FileHostError> {
		match jobs_repo::put_event(conn, subject, id, body, now).await {
			Ok((change, _)) => Ok(Outcome::Written(change.as_str())),
			Err(JobsError::Invalid(problems)) => Ok(Outcome::Refused(problems)),
			Err(JobsError::Rewritten) => Ok(Outcome::Conflict(REWRITTEN)),
			Err(JobsError::Unreadable) => Err(FileHostError::OperationError("a stored Jobs event does not read back".to_owned())),
			Err(JobsError::Storage(err)) => Err(FileHostError::Sqlite(err)),
		}
	}

	async fn since(conn: &mut SqliteConnection, subject: &str, after: &Self::Mark, limit: i64) -> Result<(Vec<Value>, Self::Mark), FileHostError> {
		match jobs_repo::since(conn, subject, after, limit).await {
			Ok((events, next)) => Ok((values(&events)?, next)),
			Err(JobsError::Storage(err)) => Err(FileHostError::Sqlite(err)),
			Err(_) => Err(FileHostError::OperationError("a stored Jobs event does not read back".to_owned())),
		}
	}

	fn encode(mark: &Self::Mark) -> String {
		joined(&[&mark.written_at, &mark.event_id])
	}

	fn decode(text: &str) -> Option<Self::Mark> {
		let (written_at, event_id) = text.split_once('|')?;
		Some(jobs_repo::Mark {
			written_at: written_at.to_owned(),
			event_id: event_id.to_owned(),
		})
	}
}

struct Aph;

impl Collection for Aph {
	const SCOPE: Scope = Scope::Aph;
	type Mark = aph_repo::Mark;

	async fn put(conn: &mut SqliteConnection, subject: &str, id: &str, body: &[u8], now: &str) -> Result<Outcome, FileHostError> {
		match aph_repo::put_entry(conn, subject, id, body, now).await {
			Ok((change, _)) => Ok(Outcome::Written(change.as_str())),
			Err(AphError::Invalid(problems)) => Ok(Outcome::Refused(problems)),
			Err(AphError::Unreadable) => Err(FileHostError::OperationError("a stored aph revision does not read back".to_owned())),
			Err(AphError::Storage(err)) => Err(FileHostError::Sqlite(err)),
		}
	}

	async fn since(conn: &mut SqliteConnection, subject: &str, after: &Self::Mark, limit: i64) -> Result<(Vec<Value>, Self::Mark), FileHostError> {
		match aph_repo::since(conn, subject, after, limit).await {
			Ok((entries, next)) => Ok((values(&entries)?, next)),
			Err(AphError::Storage(err)) => Err(FileHostError::Sqlite(err)),
			Err(_) => Err(FileHostError::OperationError("a stored aph revision does not read back".to_owned())),
		}
	}

	fn encode(mark: &Self::Mark) -> String {
		joined(&[&mark.written_at, &mark.entry_id, &mark.revision.to_string()])
	}

	fn decode(text: &str) -> Option<Self::Mark> {
		let (rest, revision) = text.rsplit_once('|')?;
		let (written_at, entry_id) = rest.split_once('|')?;
		Some(aph_repo::Mark {
			written_at: written_at.to_owned(),
			entry_id: entry_id.to_owned(),
			revision: revision.parse().ok()?,
		})
	}
}

/// `POST /sync/:collection/batch`
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Batch {
	pub items: Vec<Item>,
}

/// One write: the id its own route takes in the path, and its body.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Item {
	pub id: String,
	pub body: Value,
}

/// One item's answer: the status its own route would have answered, and that
/// route's `change`, or the fields refused and why.
#[derive(Debug, Serialize)]
pub struct Answer {
	pub id: String,
	pub status: u16,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub change: Option<&'static str>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub refused: Option<BTreeMap<&'static str, Vec<&'static str>>>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub conflict: Option<&'static str>,
}

#[derive(Debug, Serialize)]
pub struct Written {
	pub results: Vec<Answer>,
}

/// `GET /sync/:collection?since=`
#[derive(Debug, Deserialize)]
pub struct Since {
	pub since: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct Read {
	/// Each as its own record holds it, in the order it reached home.
	pub items: Vec<Value>,
	/// What to send as `since` to read on; the same mark when nothing came.
	pub next: String,
}

fn now() -> String {
	// Fixed width, as `handlers::jobs`'s.
	Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

async fn write<C: Collection>(db: &SqlitePool, subject: &str, headers: &HeaderMap, batch: Batch) -> Result<Written, FileHostError> {
	let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
	refusing(&mut tx, subject, headers, C::SCOPE).await?;
	let now = now();
	let mut results = Vec::with_capacity(batch.items.len());
	for Item { id, body } in batch.items {
		let bytes = body.to_string();
		let answer = match C::put(&mut tx, subject, &id, bytes.as_bytes(), &now).await? {
			Outcome::Written(change) => Answer {
				id,
				status: 200,
				change: Some(change),
				refused: None,
				conflict: None,
			},
			Outcome::Refused(problems) => {
				let mut refused: BTreeMap<&'static str, Vec<&'static str>> = BTreeMap::new();
				for (field, why) in problems {
					refused.entry(field).or_default().push(why);
				}
				Answer {
					id,
					status: 422,
					change: None,
					refused: Some(refused),
					conflict: None,
				}
			}
			Outcome::Conflict(why) => Answer {
				id,
				status: 409,
				change: None,
				refused: None,
				conflict: Some(why),
			},
		};
		results.push(answer);
	}
	tx.commit().await?;
	Ok(Written { results })
}

async fn read<C: Collection>(db: &SqlitePool, subject: &str, since: Option<&str>) -> Result<Read, FileHostError> {
	let after = match since {
		None => C::Mark::default(),
		Some(text) => C::decode(text).ok_or_else(|| FileHostError::unprocessable_entity([("since", "is a `next` this route answered")]))?,
	};
	let mut conn = db.acquire().await?;
	let (items, next) = C::since(&mut conn, subject, &after, READ_CEILING).await?;
	Ok(Read { items, next: C::encode(&next) })
}

/// `POST /sync/:collection/batch`, `{ "items": [{ "id", "body" }, …] }`, at
/// most [`BATCH_CEILING`] of them.
///
/// # Errors
/// 401 without a session, 403 from an untrusted origin, 404 for an unknown
/// collection, 409 from a paired phone that stopped sending this record,
/// 422 for a body that is not a batch or holds too many items, and 500 for
/// a storage failure, which keeps none of the batch.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "sync_batch", skip_all, fields(otel.kind = "server"))]
pub async fn write_batch(
	subject: SubjectId,
	State(db): State<SqlitePool>,
	headers: HeaderMap,
	Path(collection): Path<String>,
	body: Bytes,
) -> Result<Json<Written>, FileHostError> {
	let batch: Batch = serde_json::from_slice(&body).map_err(|_| FileHostError::unprocessable_entity([("body", "is { \"items\": [{ \"id\", \"body\" }] }")]))?;
	if batch.items.len() > BATCH_CEILING {
		return Err(FileHostError::unprocessable_entity([("items", "are at most 100")]));
	}
	let subject = subject.as_str();
	match collection.as_str() {
		"jobs" => write::<Jobs>(&db, subject, &headers, batch).await.map(Json),
		"aph" => write::<Aph>(&db, subject, &headers, batch).await.map(Json),
		_ => Err(FileHostError::NotFound),
	}
}

/// `GET /sync/:collection?since=`: what was written after `since` (a `next`
/// this route answered; none for the start), at most 200 items.
///
/// # Errors
/// 401 without a session, 404 for an unknown collection, 422 for a `since`
/// this route did not answer, and 500 for a storage failure.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "sync_since", skip_all, fields(otel.kind = "server"))]
pub async fn read_since(subject: SubjectId, State(db): State<SqlitePool>, Path(collection): Path<String>, Query(query): Query<Since>) -> Result<Json<Read>, FileHostError> {
	let since = query.since.as_deref();
	match collection.as_str() {
		"jobs" => read::<Jobs>(&db, subject.as_str(), since).await.map(Json),
		"aph" => read::<Aph>(&db, subject.as_str(), since).await.map(Json),
		_ => Err(FileHostError::NotFound),
	}
}

#[cfg(test)]
mod tests {
	//! Through the real router, with real phone sessions (bearer tokens, as a
	//! paired phone sends) and the sharing routes for the choices a batch is
	//! checked against.

	use crate::auth::{
		cookie::{Carrier, SessionToken},
		now, AuthContext,
	};
	use axum::{
		body::Body,
		extract::FromRef,
		http::{header::AUTHORIZATION, HeaderName, Method, Request, StatusCode},
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

	async fn state() -> TestState {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();
		TestState {
			auth: AuthContext::for_tests(pool.clone(), 100),
			pool,
		}
	}

	async fn phone(state: &TestState, subject: &str) -> (HeaderName, String) {
		sqlx::query("INSERT OR IGNORE INTO account (subject_id, user_handle) VALUES (?1, ?2)")
			.bind(subject)
			.bind(subject.as_bytes())
			.execute(&state.pool)
			.await
			.unwrap();
		let token = SessionToken::mint();
		let now = now();
		assert!(state
			.auth
			.repository()
			.create_session(&token.hash(Carrier::Bearer), subject, now + 3_600, now)
			.await
			.unwrap());
		(AUTHORIZATION, String::from("Bearer ") + &token.encoded())
	}

	fn app(state: TestState) -> Router {
		crate::routes::sync::sync()
			.into_table_router()
			.merge(crate::routes::sharing::sharing().into_table_router())
			.with_state(state)
	}

	async fn call(app: &Router, method: Method, path: &str, auth: &(HeaderName, String), body: &Value) -> (StatusCode, Value) {
		let text = if body.is_null() { String::new() } else { body.to_string() };
		let request = Request::builder().method(method).uri(path).header(&auth.0, &auth.1).body(Body::from(text)).unwrap();
		let response = app.clone().oneshot(request).await.unwrap();
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

	#[tokio::test]
	async fn a_batch_answers_each_item_as_its_own_route_would_and_reads_back_from_a_mark() {
		let state = state().await;
		let a = phone(&state, A).await;
		let b = phone(&state, B).await;
		let app = app(state);

		let mut wrong = event("e3");
		wrong["day"] = json!("tomorrow");
		let mut rewritten = event("e1");
		rewritten["type"] = json!("resume-ready");
		let batch = json!({ "items": [
			{ "id": "e1", "body": event("e1") },
			{ "id": "e2", "body": event("e2") },
			{ "id": "e3", "body": wrong },
		] });
		let (status, written) = call(&app, Method::POST, "/sync/jobs/batch", &a, &batch).await;
		assert_eq!(status, StatusCode::OK, "{written}");
		let statuses: Vec<_> = written["results"].as_array().unwrap().iter().map(|r| (r["id"].clone(), r["status"].clone())).collect();
		assert_eq!(statuses, [(json!("e1"), json!(200)), (json!("e2"), json!(200)), (json!("e3"), json!(422))]);
		assert_eq!(written["results"][2]["refused"]["day"].as_array().unwrap().len(), 1);

		let again = json!({ "items": [{ "id": "e1", "body": event("e1") }, { "id": "e1", "body": rewritten }] });
		let (_, answered) = call(&app, Method::POST, "/sync/jobs/batch", &a, &again).await;
		assert_eq!(answered["results"][0]["change"], "unchanged");
		assert_eq!(answered["results"][1]["status"], 409);

		let (status, first) = call(&app, Method::GET, "/sync/jobs", &a, &Value::Null).await;
		assert_eq!(status, StatusCode::OK);
		assert_eq!(first["items"], json!([event("e1"), event("e2")]));
		let next = first["next"].as_str().unwrap().to_owned();
		let (_, rest) = call(
			&app,
			Method::GET,
			&(String::from("/sync/jobs?since=") + &next.replace('|', "%7C").replace(':', "%3A")),
			&a,
			&Value::Null,
		)
		.await;
		assert_eq!(rest["items"], json!([]));
		assert_eq!(rest["next"], json!(next), "an empty read keeps its mark");

		assert_eq!(call(&app, Method::GET, "/sync/jobs", &b, &Value::Null).await.1["items"], json!([]), "B reads none of A's");
		assert_eq!(call(&app, Method::GET, "/sync/mood", &a, &Value::Null).await.0, StatusCode::NOT_FOUND);
		assert_eq!(
			call(&app, Method::GET, "/sync/jobs?since=garbage", &a, &Value::Null).await.0,
			StatusCode::UNPROCESSABLE_ENTITY
		);
	}

	#[tokio::test]
	async fn a_batch_is_refused_whole_once_its_phone_stopped_sending_it() {
		let state = state().await;
		let a = phone(&state, A).await;
		let app = app(state);
		call(&app, Method::PUT, "/sharing/jobs", &a, &json!({ "kept": true })).await;
		let batch = json!({ "items": [{ "id": "e1", "body": event("e1") }] });
		assert_eq!(call(&app, Method::POST, "/sync/jobs/batch", &a, &batch).await.0, StatusCode::OK);
		call(&app, Method::PUT, "/sharing/jobs", &a, &json!({ "kept": false })).await;
		let late = json!({ "items": [{ "id": "e2", "body": event("e2") }] });
		assert_eq!(call(&app, Method::POST, "/sync/jobs/batch", &a, &late).await.0, StatusCode::CONFLICT);
		assert_eq!(call(&app, Method::GET, "/sync/jobs", &a, &Value::Null).await.1["items"], json!([event("e1")]));

		let entry = json!({
			"id": "a1", "day": "2026-10-17", "checkpoint": null, "time": null,
			"mine": { "value": 4300, "approx": true }, "theirs": null, "goal": null,
			"labels": [], "note": null, "review": null
		});
		let (status, kept) = call(&app, Method::POST, "/sync/aph/batch", &a, &json!({ "items": [{ "id": "a1", "body": entry }] })).await;
		assert_eq!((status, kept["results"][0]["status"].clone()), (StatusCode::OK, json!(200)), "{kept}");
		assert_eq!(call(&app, Method::GET, "/sync/aph", &a, &Value::Null).await.1["items"], json!([entry]));

		let too_many: Vec<Value> = (0..=super::BATCH_CEILING).map(|i| json!({ "id": i.to_string(), "body": {} })).collect();
		assert_eq!(
			call(&app, Method::POST, "/sync/aph/batch", &a, &json!({ "items": too_many })).await.0,
			StatusCode::UNPROCESSABLE_ENTITY
		);
	}
}

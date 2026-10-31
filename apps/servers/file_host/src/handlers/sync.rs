//! Handlers for `routes::sync`: a kept record's writes in one request.
//!
//! Each item is answered as its own route would answer it, from the same
//! bytes, by the same repository call. A collection is one record's own
//! storage behind [`Collection`]: no store is shared between records, and
//! nothing new is stored.
//!
//! **One transaction per batch.** `BEGIN IMMEDIATE`, so every item sees one
//! snapshot. An item home refuses (a
//! `422` naming its fields, or a `409`) writes nothing and the rest go on;
//! a storage failure ends the whole batch, and none of it is kept.
//!
//! **The deletion hold.** The handler takes a [`SubjectId`] first and keeps
//! it until it returns (docs/identity.md, invariant 9).

use crate::subject::SubjectId;
use crate::FileHostError;
use aph_repo::AphError;
use axum::{
	body::Bytes,
	extract::{Path, State},
	Json,
};
use chrono::{SecondsFormat, Utc};
use jobs_repo::{JobsError, REWRITTEN};
use korean_repo::KoreanError;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use sqlx::{SqliteConnection, SqlitePool};
use std::collections::BTreeMap;
use tracing::instrument;

/// The most items one batch carries.
pub(crate) const BATCH_CEILING: usize = 100;

/// How a batch item ended, as its own route would have answered.
enum Outcome {
	/// Stored, or already held: the route's `change`, and what the record now
	/// holds under the id, which the phone compares with what it sent.
	Written(&'static str, Box<RawValue>),
	/// A `422`: each refused field and why.
	Refused(Vec<(&'static str, &'static str)>),
	/// A `409`: what it conflicts with.
	Conflict(&'static str),
}

/// One record's storage, as a batch reaches it.
trait Collection {
	async fn put(conn: &mut SqliteConnection, subject: &str, id: &str, body: &[u8], now: &str) -> Result<Outcome, FileHostError>;
}

struct Jobs;

impl Collection for Jobs {
	async fn put(conn: &mut SqliteConnection, subject: &str, id: &str, body: &[u8], now: &str) -> Result<Outcome, FileHostError> {
		match jobs_repo::put_event(conn, subject, id, body, now).await {
			Ok((change, event)) => Ok(Outcome::Written(change.as_str(), serde_json::value::to_raw_value(&event)?)),
			Err(JobsError::Invalid(problems)) => Ok(Outcome::Refused(problems)),
			Err(JobsError::Rewritten) => Ok(Outcome::Conflict(REWRITTEN)),
			Err(JobsError::Unreadable) => Err(FileHostError::OperationError("a stored Jobs event does not read back".to_owned())),
			Err(JobsError::Storage(err)) => Err(FileHostError::Sqlite(err)),
		}
	}
}

struct Aph;

impl Collection for Aph {
	async fn put(conn: &mut SqliteConnection, subject: &str, id: &str, body: &[u8], now: &str) -> Result<Outcome, FileHostError> {
		match aph_repo::put_entry(conn, subject, id, body, now).await {
			Ok((change, entry)) => Ok(Outcome::Written(change.as_str(), serde_json::value::to_raw_value(&entry)?)),
			Err(AphError::Invalid(problems)) => Ok(Outcome::Refused(problems)),
			Err(AphError::Unreadable) => Err(FileHostError::OperationError("a stored aph revision does not read back".to_owned())),
			Err(AphError::Storage(err)) => Err(FileHostError::Sqlite(err)),
		}
	}
}

struct Korean;

impl Collection for Korean {
	/// `id` is `<kind>/<key>`, the route's path, and a `null` body is a
	/// removal, as `DELETE /korean/:kind/:key` answers it.
	async fn put(conn: &mut SqliteConnection, subject: &str, id: &str, body: &[u8], now: &str) -> Result<Outcome, FileHostError> {
		let Some((kind, key)) = id.split_once('/') else {
			return Ok(Outcome::Refused(vec![("id", "is <kind>/<key>")]));
		};
		let written = if body == b"null" {
			korean_repo::remove(conn, subject, kind, key, now).await
		} else {
			korean_repo::put(conn, subject, kind, key, body, now).await.map(|(change, entry)| (change, Some(entry)))
		};
		match written {
			Ok((change, entry)) => Ok(Outcome::Written(change.as_str(), serde_json::value::to_raw_value(&entry)?)),
			Err(KoreanError::Invalid(problems)) => Ok(Outcome::Refused(problems)),
			Err(KoreanError::Unreadable) => Err(FileHostError::OperationError("a stored Korean revision does not read back".to_owned())),
			Err(KoreanError::Storage(err)) => Err(FileHostError::Sqlite(err)),
		}
	}
}

/// `POST /sync/:collection/batch`
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Batch {
	pub items: Vec<Item>,
}

/// One write: the id its own route takes in the path, and its body, kept
/// as the bytes that arrived so it is read exactly as that route reads it.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Item {
	pub id: String,
	pub body: Box<RawValue>,
}

/// One item's answer: the status its own route would have answered, and that
/// route's `change` with what the record holds, or the fields refused and
/// why, or the conflict.
#[derive(Debug, Serialize)]
pub struct Answer {
	pub id: String,
	pub status: u16,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub change: Option<&'static str>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub held: Option<Box<RawValue>>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub refused: Option<BTreeMap<&'static str, Vec<&'static str>>>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub conflict: Option<&'static str>,
}

#[derive(Debug, Serialize)]
pub struct Written {
	pub results: Vec<Answer>,
}

fn now() -> String {
	// Fixed width, as `handlers::jobs`'s.
	Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn answer(id: String, outcome: Outcome) -> Answer {
	let blank = Answer {
		id,
		status: 200,
		change: None,
		held: None,
		refused: None,
		conflict: None,
	};
	match outcome {
		Outcome::Written(change, held) => Answer {
			change: Some(change),
			held: Some(held),
			..blank
		},
		Outcome::Refused(problems) => {
			let mut refused: BTreeMap<&'static str, Vec<&'static str>> = BTreeMap::new();
			for (field, why) in problems {
				refused.entry(field).or_default().push(why);
			}
			Answer {
				status: 422,
				refused: Some(refused),
				..blank
			}
		}
		Outcome::Conflict(why) => Answer {
			status: 409,
			conflict: Some(why),
			..blank
		},
	}
}

async fn write<C: Collection>(db: &SqlitePool, subject: &str, body: &[u8]) -> Result<Written, FileHostError> {
	let batch: Batch = serde_json::from_slice(body).map_err(|_| FileHostError::unprocessable_entity([("body", "is { \"items\": [{ \"id\", \"body\" }] }")]))?;
	if batch.items.len() > BATCH_CEILING {
		return Err(FileHostError::unprocessable_entity([("items", "are at most 100")]));
	}
	let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
	let now = now();
	let mut results = Vec::with_capacity(batch.items.len());
	for Item { id, body } in batch.items {
		let outcome = C::put(&mut tx, subject, &id, body.get().as_bytes(), &now).await?;
		results.push(answer(id, outcome));
	}
	tx.commit().await?;
	Ok(Written { results })
}

/// `POST /sync/:collection/batch`, `{ "items": [{ "id", "body" }, …] }`.
///
/// At most [`BATCH_CEILING`] items; `:collection` is `jobs` (events, as
/// `PUT /jobs/events/:id`), `aph` (entries, as `PUT /aph/entries/:id`) or
/// `korean` (entries, each `id` a `<kind>/<key>`, as
/// `PUT /korean/:kind/:key`, or `DELETE` for a `null` body).
///
/// # Errors
/// 401 without a session, 403 from an untrusted origin, 404 for an unknown
/// collection, 413 for a body over axum's 2 MiB default (about 14 of the largest Jobs
/// events, so a client sizes a batch by bytes as well as by count), 422 for
/// a body that is not a batch or holds too many items, and 500 for a
/// storage failure, which keeps none of the batch.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "sync_batch", skip_all, fields(otel.kind = "server"))]
pub async fn write_batch(subject: SubjectId, State(db): State<SqlitePool>, Path(collection): Path<String>, body: Bytes) -> Result<Json<Written>, FileHostError> {
	let subject = subject.as_str();
	match collection.as_str() {
		"jobs" => write::<Jobs>(&db, subject, &body).await.map(Json),
		"aph" => write::<Aph>(&db, subject, &body).await.map(Json),
		"korean" => write::<Korean>(&db, subject, &body).await.map(Json),
		_ => Err(FileHostError::NotFound),
	}
}

#[cfg(test)]
mod tests {
	//! Through the real router, with real phone sessions (bearer tokens, as a
	//! paired phone sends).

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
		crate::routes::sync::sync().into_table_router().with_state(state)
	}

	async fn call(app: &Router, method: Method, path: &str, auth: &(HeaderName, String), body: &str) -> (StatusCode, Value) {
		let request = Request::builder()
			.method(method)
			.uri(path)
			.header(&auth.0, &auth.1)
			.body(Body::from(body.to_owned()))
			.unwrap();
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
	async fn a_batch_answers_each_item_as_its_own_route_would() {
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
			{ "id": "e3", "body": wrong },
			{ "id": "e2", "body": event("e2") },
		] });
		let (status, written) = call(&app, Method::POST, "/sync/jobs/batch", &a, &batch.to_string()).await;
		assert_eq!(status, StatusCode::OK, "{written}");
		let statuses: Vec<_> = written["results"].as_array().unwrap().iter().map(|r| (r["id"].clone(), r["status"].clone())).collect();
		assert_eq!(
			statuses,
			[(json!("e1"), json!(200)), (json!("e3"), json!(422)), (json!("e2"), json!(200))],
			"the rest go on"
		);
		assert_eq!(written["results"][0]["held"], event("e1"), "what the record holds");
		assert_eq!(written["results"][1]["refused"]["day"].as_array().unwrap().len(), 1);
		let fixed = json!({ "items": [{ "id": "e3", "body": event("e3") }] });
		let (_, kept) = call(&app, Method::POST, "/sync/jobs/batch", &a, &fixed.to_string()).await;
		assert_eq!(kept["results"][0]["change"], "added", "a refused item kept nothing");

		let again = json!({ "items": [{ "id": "e1", "body": event("e1") }, { "id": "e1", "body": rewritten }] });
		let (_, answered) = call(&app, Method::POST, "/sync/jobs/batch", &a, &again.to_string()).await;
		assert_eq!(answered["results"][0]["change"], "unchanged");
		assert_eq!(answered["results"][1]["status"], 409);

		// The bytes as sent: a duplicate key is refused as its own route refuses it.
		let doubled = String::from(r#"{ "items": [{ "id": "e4", "body": {"id":"e4","#) + &event("e4").to_string()[1..] + "}] }";
		let (_, refused) = call(&app, Method::POST, "/sync/jobs/batch", &a, &doubled).await;
		assert_eq!(refused["results"][0]["status"], 422, "{refused}");

		let (status, b_wrote) = call(&app, Method::POST, "/sync/jobs/batch", &b, &json!({ "items": [] }).to_string()).await;
		assert_eq!((status, b_wrote), (StatusCode::OK, json!({ "results": [] })));
		assert_eq!(call(&app, Method::POST, "/sync/mood/batch", &a, &batch.to_string()).await.0, StatusCode::NOT_FOUND);
	}

	#[tokio::test]
	async fn an_aph_batch_writes_entries_and_a_batch_holds_at_most_a_hundred() {
		let state = state().await;
		let a = phone(&state, A).await;
		let app = app(state);
		let entry = |id: &str| {
			json!({
				"id": id, "day": "2026-10-17", "checkpoint": null, "time": null,
				"mine": { "value": 4300, "approx": true }, "theirs": null, "goal": null,
				"labels": [], "note": null, "review": null
			})
		};
		let items =
			|n: usize| -> Value { json!({ "items": (0..n).map(|i| { let id = String::from("a") + &i.to_string(); json!({ "id": id, "body": entry(&id) }) }).collect::<Vec<_>>() }) };
		let (status, kept) = call(&app, Method::POST, "/sync/aph/batch", &a, &items(super::BATCH_CEILING).to_string()).await;
		assert_eq!(status, StatusCode::OK, "{kept}");
		assert_eq!(kept["results"].as_array().unwrap().len(), super::BATCH_CEILING);
		assert_eq!(kept["results"][0]["held"], entry("a0"));
		assert_eq!(
			call(&app, Method::POST, "/sync/aph/batch", &a, &items(super::BATCH_CEILING + 1).to_string()).await.0,
			StatusCode::UNPROCESSABLE_ENTITY
		);
	}

	/// A Korean batch answers each item as `PUT` (or, for a `null` body,
	/// `DELETE`) `/korean/:kind/:key` would, and refuses an id that names no
	/// kind and key.
	#[tokio::test]
	async fn a_korean_batch_writes_and_removes_entries_by_kind_and_key() {
		let state = state().await;
		let a = phone(&state, A).await;
		let app = app(state);
		let batch = json!({ "items": [
			{ "id": "set/cafe", "body": { "deck": { "id": "cafe" } } },
			{ "id": "pace/w-1", "body": { "factor": 1 } },
			{ "id": "set/cafe", "body": null },
			{ "id": "cafe", "body": {} },
			{ "id": "words/w-1", "body": {} },
		] });
		let (status, written) = call(&app, Method::POST, "/sync/korean/batch", &a, &batch.to_string()).await;
		assert_eq!(status, StatusCode::OK, "{written}");
		let results = written["results"].as_array().unwrap();
		let said: Vec<(u64, &Value)> = results.iter().map(|result| (result["status"].as_u64().unwrap(), &result["change"])).collect();
		assert_eq!(
			said,
			[
				(200, &json!("kept")),
				(200, &json!("kept")),
				(200, &json!("removed")),
				(422, &Value::Null),
				(422, &Value::Null)
			]
		);
		assert_eq!(
			results[0]["held"]["body"],
			json!({ "deck": { "id": "cafe" } }),
			"what home holds, to compare with what was sent"
		);
		assert_eq!(results[2]["held"]["body"], Value::Null);
		assert_eq!(results[3]["refused"], json!({ "id": ["is <kind>/<key>"] }));
		assert!(results[4]["refused"]["kind"].is_array());
	}

	/// What home holds is answered as the bytes it kept, as the item's own
	/// route answers it: never re-read as a `Value`, which would reformat a
	/// body and refuse a number it cannot hold.
	#[tokio::test]
	async fn a_korean_batch_answers_each_body_byte_for_byte() {
		let state = state().await;
		let a = phone(&state, A).await;
		let app = app(state);
		let body = r#"{ "n" : 1e400,  "n" : 2 }"#;
		let batch = String::from(r#"{ "items": [{ "id": "record/it", "body": "#) + body + " }] }";
		let request = Request::builder()
			.method(Method::POST)
			.uri("/sync/korean/batch")
			.header(&a.0, &a.1)
			.body(Body::from(batch))
			.unwrap();
		let response = app.oneshot(request).await.unwrap();
		assert_eq!(response.status(), StatusCode::OK);
		let text = String::from_utf8(axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap().to_vec()).unwrap();
		assert!(text.contains(&(String::from(r#""body":"#) + body)), "{text}");
	}

	#[tokio::test]
	async fn a_batch_needs_a_session_from_a_trusted_origin() {
		let state = state().await;
		let a = phone(&state, A).await;
		let app = app(state);
		let request = |auth: Option<&(HeaderName, String)>, origin: Option<&str>| {
			let mut request = Request::builder().method(Method::POST).uri("/sync/jobs/batch");
			if let Some((name, value)) = auth {
				request = request.header(name, value);
			}
			if let Some(origin) = origin {
				request = request.header("origin", origin);
			}
			request.body(Body::from(json!({ "items": [] }).to_string())).unwrap()
		};
		assert_eq!(app.clone().oneshot(request(None, None)).await.unwrap().status(), StatusCode::UNAUTHORIZED);
		assert_eq!(
			app.clone().oneshot(request(Some(&a), Some("https://evil.app.test"))).await.unwrap().status(),
			StatusCode::FORBIDDEN
		);
	}
}

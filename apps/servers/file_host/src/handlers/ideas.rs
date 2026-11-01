//! Handlers for `routes::ideas`: the validate-and-answer half. The record's
//! own rules (append-only, what a send holds, what the lists hold) are
//! `idea_repo`'s.
//!
//! **Refusals.** A write that breaks one of those rules is a `422` naming
//! each field and why. A send naming a voice note that is not the subject's
//! own `studio` note on home, or one the owner set aside, is a `409` naming
//! each such note by id under `details.notes`, so the phone can send that
//! take first, or have that answer said again, and try again. A
//! different bundle under a held `sentAt`, and a new revision of an idea set
//! aside, are each a `409`. Every refusal writes nothing.
//!
//! **The deletion hold.** Each handler takes a [`SubjectId`] first and keeps
//! it until it returns, so a write holds off `DELETE /auth/account` as every
//! other subject-scoped write does (docs/identity.md, invariant 9).
//!
//! **Nothing said reaches a log.** A title, a typed answer and what home heard
//! in a note are the owner's own words. Every handler here is
//! `#[instrument(skip_all)]` and records only `otel.kind` (invariant 6), and
//! refusals carry fixed words and ids, never a value sent;
//! `nothing_an_idea_says_reaches_a_log_line` checks it.

use crate::subject::SubjectId;
use crate::FileHostError;
use axum::{
	body::Bytes,
	extract::{Path, State},
	http::StatusCode,
	Json,
};
use chrono::{SecondsFormat, Utc};
use idea_repo::{Idea, IdeaError, Ideas, List, Lists, SetAside, NOTES_NOT_HELD, REWRITTEN, SET_ASIDE, UNREADABLE};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use std::borrow::Cow;
use std::collections::HashMap;
use tracing::instrument;

/// `PUT /ideas/:id`
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IdeaWritten {
	/// `added` (a new revision) or `unchanged` — `idea_repo::Change`.
	pub change: &'static str,
	/// The idea as home now holds it, which the phone compares with what it
	/// sent before it lets its own copy go.
	pub idea: Idea,
}

/// `PUT /ideas/:id/furnishing`
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FurnishingWritten {
	/// `added` or `unchanged`.
	pub change: &'static str,
	/// The mark as it now stands.
	pub furnishing: bool,
}

/// `PUT` and `DELETE /idea-lists/{topics,keywords}/:name`
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ListWritten {
	/// `added` or `unchanged`.
	pub change: &'static str,
	#[serde(flatten)]
	pub lists: Lists,
}

/// `PUT /ideas/:id/furnishing`'s body.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Furnishing {
	furnishing: bool,
}

fn now() -> String {
	// Fixed width, as `handlers::jobs`'s.
	Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn refusal(err: IdeaError) -> FileHostError {
	match err {
		IdeaError::Invalid(problems) => FileHostError::unprocessable_entity(problems),
		IdeaError::Rewritten => FileHostError::Conflict(REWRITTEN),
		IdeaError::SetAside => FileHostError::Conflict(SET_ASIDE),
		IdeaError::NotesNotHeld(notes) => FileHostError::ConflictNaming {
			reason: NOTES_NOT_HELD,
			details: HashMap::from([(Cow::Borrowed("notes"), notes.into_iter().map(Cow::Owned).collect())]),
		},
		IdeaError::Unreadable => FileHostError::OperationError(UNREADABLE.to_owned()),
		IdeaError::Storage(err) => FileHostError::Sqlite(err),
	}
}

/// `GET /ideas`
///
/// # Errors
/// 401 without a session, and 500 for a storage failure.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "ideas_get", skip_all, fields(otel.kind = "server"))]
pub async fn get_ideas(subject: SubjectId, State(db): State<SqlitePool>) -> Result<Json<Ideas>, FileHostError> {
	let mut conn = db.acquire().await?;
	idea_repo::ideas(&mut conn, subject.as_str()).await.map(Json).map_err(refusal)
}

/// `GET /ideas/:id`
///
/// # Errors
/// 401 without a session, 404 for an idea the caller's subject does not hold
/// (another subject's included), and 500 for a storage failure.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "idea_get", skip_all, fields(otel.kind = "server"))]
pub async fn get_idea(subject: SubjectId, State(db): State<SqlitePool>, Path(id): Path<String>) -> Result<Json<Idea>, FileHostError> {
	let mut conn = db.acquire().await?;
	idea_repo::idea(&mut conn, subject.as_str(), &id)
		.await
		.map_err(refusal)?
		.map(Json)
		.ok_or(FileHostError::NotFound)
}

/// `PUT /ideas/:id`, the request body being the send itself. `BEGIN
/// IMMEDIATE`, so the reads that decide and the write see one snapshot.
///
/// # Errors
/// 401 without a session, 403 from an untrusted origin, 422 naming each
/// refused field, 409 naming each voice note home does not hold as the
/// subject's `studio` note or holds set aside, 409 for a different bundle under a held `sentAt`
/// or an idea set aside, and 500 for a storage failure.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "idea_put", skip_all, fields(otel.kind = "server"))]
pub async fn put_idea(subject: SubjectId, State(db): State<SqlitePool>, Path(id): Path<String>, body: Bytes) -> Result<Json<IdeaWritten>, FileHostError> {
	let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
	let (change, idea) = idea_repo::put_idea(&mut tx, subject.as_str(), &id, &body, &now()).await.map_err(refusal)?;
	tx.commit().await?;
	Ok(Json(IdeaWritten { change: change.as_str(), idea }))
}

/// `PUT /ideas/:id/furnishing`, the body `{ "furnishing": true | false }`.
///
/// Whether the owner is still furnishing the idea. The latest mark stands and
/// every change is kept; a mark that is already the latest writes nothing.
/// It is the owner's alone: nothing reads it for a routine or the mailbox.
///
/// # Errors
/// 401 without a session, 403 from an untrusted origin, 422 naming
/// `furnishing` for any other body, 404 for an idea the caller's subject does
/// not hold, and 500 for a storage failure.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "idea_furnishing_put", skip_all, fields(otel.kind = "server"))]
pub async fn put_furnishing(subject: SubjectId, State(db): State<SqlitePool>, Path(id): Path<String>, body: Bytes) -> Result<Json<FurnishingWritten>, FileHostError> {
	let Ok(Furnishing { furnishing }) = serde_json::from_slice::<Furnishing>(&body) else {
		return Err(FileHostError::unprocessable_entity([("furnishing", "must be the body's one field, true or false")]));
	};
	let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
	let change = idea_repo::mark_furnishing(&mut tx, subject.as_str(), &id, furnishing, &now()).await.map_err(refusal)?;
	tx.commit().await?;
	let change = change.ok_or(FileHostError::NotFound)?;
	Ok(Json(FurnishingWritten {
		change: change.as_str(),
		furnishing,
	}))
}

/// `PUT /ideas/:id/set-aside`: takes the idea out of the list for good.
///
/// A mark, never a delete: every revision stays, and `GET /ideas/:id` still
/// reads it. The first mark stands, so a repeat is a `204` that changes
/// nothing. No body, in either direction.
///
/// # Errors
/// 401 without a session, 403 from an untrusted origin, 404 for an idea the
/// caller's subject does not hold, and 500 for a storage failure.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "idea_set_aside_put", skip_all, fields(otel.kind = "server"))]
pub async fn put_set_aside(subject: SubjectId, State(db): State<SqlitePool>, Path(id): Path<String>) -> Result<StatusCode, FileHostError> {
	let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
	let marked = idea_repo::set_aside(&mut tx, subject.as_str(), &id, &now()).await.map_err(refusal)?;
	tx.commit().await?;
	match marked {
		SetAside::Marked | SetAside::AlreadyMarked => Ok(StatusCode::NO_CONTENT),
		SetAside::NotHeld => Err(FileHostError::NotFound),
	}
}

/// `GET /idea-lists`
///
/// # Errors
/// 401 without a session, and 500 for a storage failure.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "idea_lists_get", skip_all, fields(otel.kind = "server"))]
pub async fn get_lists(subject: SubjectId, State(db): State<SqlitePool>) -> Result<Json<Lists>, FileHostError> {
	let mut conn = db.acquire().await?;
	idea_repo::lists(&mut conn, subject.as_str()).await.map(Json).map_err(refusal)
}

async fn mark(subject: &SubjectId, db: &SqlitePool, list: List, name: &str, listed: bool) -> Result<Json<ListWritten>, FileHostError> {
	let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
	let (change, lists) = idea_repo::mark_entry(&mut tx, subject.as_str(), list, name, listed, &now()).await.map_err(refusal)?;
	tx.commit().await?;
	Ok(Json(ListWritten { change: change.as_str(), lists }))
}

/// `PUT /idea-lists/topics/:name`: adds the topic. One already on is
/// unchanged.
///
/// # Errors
/// 401 without a session, 403 from an untrusted origin, 422 naming `name`
/// for one that is not an entry or a list already holding 100, and 500 for a
/// storage failure.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "idea_topic_put", skip_all, fields(otel.kind = "server"))]
pub async fn put_topic(subject: SubjectId, State(db): State<SqlitePool>, Path(name): Path<String>) -> Result<Json<ListWritten>, FileHostError> {
	mark(&subject, &db, List::Topic, &name, true).await
}

/// `DELETE /idea-lists/topics/:name`: takes the topic off, as a mark. The
/// ideas that use it are unchanged. One not on is unchanged.
///
/// # Errors
/// As [`put_topic`], but for the cap.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "idea_topic_delete", skip_all, fields(otel.kind = "server"))]
pub async fn delete_topic(subject: SubjectId, State(db): State<SqlitePool>, Path(name): Path<String>) -> Result<Json<ListWritten>, FileHostError> {
	mark(&subject, &db, List::Topic, &name, false).await
}

/// `PUT /idea-lists/keywords/:name`: adds the keyword, as [`put_topic`].
///
/// # Errors
/// As [`put_topic`].
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "idea_keyword_put", skip_all, fields(otel.kind = "server"))]
pub async fn put_keyword(subject: SubjectId, State(db): State<SqlitePool>, Path(name): Path<String>) -> Result<Json<ListWritten>, FileHostError> {
	mark(&subject, &db, List::Keyword, &name, true).await
}

/// `DELETE /idea-lists/keywords/:name`: takes the keyword off, as
/// [`delete_topic`].
///
/// # Errors
/// As [`delete_topic`].
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "idea_keyword_delete", skip_all, fields(otel.kind = "server"))]
pub async fn delete_keyword(subject: SubjectId, State(db): State<SqlitePool>, Path(name): Path<String>) -> Result<Json<ListWritten>, FileHostError> {
	mark(&subject, &db, List::Keyword, &name, false).await
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
		crate::routes::ideas::ideas()
	}

	async fn state() -> TestState {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();
		TestState {
			auth: AuthContext::for_tests(pool.clone(), 100),
			pool,
		}
	}

	/// An account for `subject`, a `studio` note `n1` and a reflection note
	/// `r1` on it, and a live session carried as `carrier` carries it.
	async fn session(state: &TestState, subject: &str, carrier: Carrier) -> (axum::http::HeaderName, String) {
		sqlx::query("INSERT OR IGNORE INTO account (subject_id, user_handle) VALUES (?1, ?2)")
			.bind(subject)
			.bind(subject.as_bytes())
			.execute(&state.pool)
			.await
			.unwrap();
		for (note, kind) in [("n1", "studio"), ("r1", "reflection")] {
			sqlx::query(
				"INSERT OR IGNORE INTO voice_note (subject_id, note_id, of_kind, of_ref, recorded_at, content_type, size, sha256, stored_at)
				 VALUES (?1, ?2, ?3, 'idea:i1:what', 'then', 'audio/webm', 1, 's', 'now')",
			)
			.bind(subject)
			.bind(note)
			.bind(kind)
			.execute(&state.pool)
			.await
			.unwrap();
		}
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

	fn send(id: &str) -> Value {
		json!({
			"id": id,
			"createdAt": "2026-11-01T09:00:00.000Z",
			"sentAt": "2026-11-01T09:10:00.000Z",
			"title": "Why the mic clips",
			"topic": "Studio",
			"format": "short",
			"keywords": ["voice", "how it works"],
			"questions": 1,
			"answers": [{ "question": "what", "note": "n1" }, { "question": "takeaway", "typed": "Keep it a hand away." }]
		})
	}

	fn held(id: &str) -> Value {
		json!({
			"id": id, "createdAt": "2026-11-01T09:00:00.000Z", "revision": 1, "sentAt": "2026-11-01T09:10:00.000Z",
			"title": "Why the mic clips", "topic": "Studio", "format": "short", "keywords": ["how it works", "voice"], "questions": 1,
			"answers": [
				{ "question": "what", "note": "n1", "typed": null, "heard": null, "setAside": false },
				{ "question": "takeaway", "note": null, "typed": "Keep it a hand away.", "heard": null, "setAside": false }
			],
			"furnishing": false, "setAside": false
		})
	}

	fn idea_path(id: &str) -> String {
		String::from("/ideas/") + id
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
					route.path.replace(":id", "i1").replace(":name", "Cooking"),
				)
			})
			.collect();
		assert_eq!(routes.len(), 10, "{routes:?}");
		for (method, path) in &routes {
			assert_eq!(call(&app, method.clone(), path, None, &json!({})).await.0, StatusCode::UNAUTHORIZED, "{method} {path}");
		}
		for (method, path) in routes.into_iter().filter(|(method, _)| *method != Method::GET) {
			let request = Request::builder()
				.method(method.clone())
				.uri(&path)
				.header(&phone.0, &phone.1)
				.header("origin", "https://evil.app.test")
				.header("sec-fetch-site", "cross-site")
				.body(Body::from(send("i1").to_string()))
				.unwrap();
			assert_eq!(app.clone().oneshot(request).await.unwrap().status(), StatusCode::FORBIDDEN, "{method} {path}");
		}
	}

	/// The phone's whole round: the lists, an idea sent and sent again, read
	/// back alone and in the list, marked still furnishing, set aside and so
	/// left out of the list, and the lists edited. A browser session on the
	/// same account reads the same record.
	#[tokio::test]
	async fn a_paired_phone_sends_an_idea_and_reads_it_back() {
		let state = state().await;
		let phone = session(&state, A, Carrier::Bearer).await;
		let browser = session(&state, A, Carrier::Cookie).await;
		let app = app(state);

		assert_eq!(
			call(&app, Method::GET, "/ideas", Some(&phone), &Value::Null).await,
			(StatusCode::OK, json!({ "ideas": [], "more": false }))
		);
		let seeds = json!({
			"topics": ["Studio", "LeetType", "Korean", "Playoff Delta", "Rust Rounds"],
			"keywords": ["home", "voice", "phone", "mistakes", "how it works", "behind the scenes"]
		});
		assert_eq!(call(&app, Method::GET, "/idea-lists", Some(&phone), &Value::Null).await, (StatusCode::OK, seeds));

		assert_eq!(
			call(&app, Method::PUT, &idea_path("i1"), Some(&phone), &send("i1")).await,
			(StatusCode::OK, json!({ "change": "added", "idea": held("i1") }))
		);
		assert_eq!(
			call(&app, Method::PUT, &idea_path("i1"), Some(&phone), &send("i1")).await,
			(StatusCode::OK, json!({ "change": "unchanged", "idea": held("i1") }))
		);
		assert_eq!(call(&app, Method::GET, &idea_path("i1"), Some(&browser), &Value::Null).await, (StatusCode::OK, held("i1")));
		assert_eq!(
			call(&app, Method::GET, "/ideas", Some(&browser), &Value::Null).await,
			(StatusCode::OK, json!({ "ideas": [held("i1")], "more": false }))
		);

		let furnishing = idea_path("i1") + "/furnishing";
		assert_eq!(
			call(&app, Method::PUT, &furnishing, Some(&phone), &json!({ "furnishing": true })).await,
			(StatusCode::OK, json!({ "change": "added", "furnishing": true }))
		);
		assert_eq!(
			call(&app, Method::PUT, &furnishing, Some(&phone), &json!({ "furnishing": true })).await.1["change"],
			"unchanged"
		);
		assert_eq!(call(&app, Method::GET, &idea_path("i1"), Some(&phone), &Value::Null).await.1["furnishing"], true);

		let mut second = send("i2");
		second["createdAt"] = json!("2026-11-01T10:00:00.000Z");
		assert_eq!(call(&app, Method::PUT, &idea_path("i2"), Some(&phone), &second).await.0, StatusCode::OK);
		let ids = |answer: (StatusCode, Value)| answer.1["ideas"].as_array().unwrap().iter().map(|idea| idea["id"].clone()).collect::<Vec<_>>();
		assert_eq!(ids(call(&app, Method::GET, "/ideas", Some(&phone), &Value::Null).await), ["i2", "i1"], "newest begun first");

		let set_aside = idea_path("i2") + "/set-aside";
		assert_eq!(call(&app, Method::PUT, &set_aside, Some(&phone), &Value::Null).await.0, StatusCode::NO_CONTENT);
		assert_eq!(
			call(&app, Method::PUT, &set_aside, Some(&phone), &Value::Null).await.0,
			StatusCode::NO_CONTENT,
			"again, unchanged"
		);
		assert_eq!(ids(call(&app, Method::GET, "/ideas", Some(&phone), &Value::Null).await), ["i1"], "set aside, left out");
		assert_eq!(call(&app, Method::GET, &idea_path("i2"), Some(&phone), &Value::Null).await.1["setAside"], true, "but kept");

		let (status, written) = call(&app, Method::PUT, "/idea-lists/topics/Home%20cooking", Some(&phone), &Value::Null).await;
		assert_eq!((status, &written["change"]), (StatusCode::OK, &json!("added")));
		assert_eq!(written["topics"].as_array().unwrap().last().unwrap(), "Home cooking", "a name with a space, decoded");
		let (status, written) = call(&app, Method::DELETE, "/idea-lists/keywords/voice", Some(&phone), &Value::Null).await;
		assert_eq!((status, &written["change"]), (StatusCode::OK, &json!("added")), "taking off is a mark added");
		assert!(!written["keywords"].as_array().unwrap().contains(&json!("voice")));
		assert_eq!(
			call(&app, Method::DELETE, "/idea-lists/keywords/voice", Some(&phone), &Value::Null).await.1["change"],
			"unchanged"
		);
		assert_eq!(
			call(&app, Method::GET, &idea_path("i1"), Some(&phone), &Value::Null).await.1["keywords"],
			json!(["how it works", "voice"]),
			"the idea that uses it is unchanged"
		);
		assert_eq!(
			call(&app, Method::PUT, "/idea-lists/keywords/voice", Some(&browser), &Value::Null).await.1["change"],
			"added"
		);
	}

	/// A write that breaks a rule is a 422 naming each field, a voice note
	/// home lacks is a 409 naming it, and one subject's record is never
	/// another's.
	#[tokio::test]
	async fn refusals_name_their_fields_and_records_stay_apart() {
		let state = state().await;
		let a = session(&state, A, Carrier::Bearer).await;
		let b = session(&state, B, Carrier::Bearer).await;
		let app = app(state);
		assert_eq!(call(&app, Method::PUT, &idea_path("i1"), Some(&a), &send("i1")).await.0, StatusCode::OK);

		let named = |answer: (StatusCode, Value)| {
			assert_eq!(answer.0, StatusCode::UNPROCESSABLE_ENTITY, "{}", answer.1);
			let mut fields: Vec<String> = answer.1["error"]["details"].as_object().unwrap().keys().cloned().collect();
			fields.sort();
			fields
		};
		let mut bad = send("i2");
		bad["title"] = json!("x".repeat(81));
		bad["format"] = json!("reel");
		bad["answers"] = json!([{ "question": "takeaway", "typed": "y".repeat(1_001) }]);
		assert_eq!(named(call(&app, Method::PUT, &idea_path("i2"), Some(&a), &bad).await), ["answers", "format", "title"]);
		let mut off_list = send("i2");
		off_list["topic"] = json!("Cooking");
		off_list["keywords"] = json!(["lighting"]);
		assert_eq!(named(call(&app, Method::PUT, &idea_path("i2"), Some(&a), &off_list).await), ["keywords", "topic"]);
		assert_eq!(named(call(&app, Method::PUT, &idea_path("other"), Some(&a), &send("i2")).await), ["id"]);
		assert_eq!(named(call(&app, Method::PUT, &idea_path("i2"), Some(&a), &json!("idea")).await), ["body"]);
		assert_eq!(
			named(call(&app, Method::PUT, &(idea_path("i1") + "/furnishing"), Some(&a), &json!({ "furnishing": "yes" })).await),
			["furnishing"]
		);
		assert_eq!(named(call(&app, Method::PUT, "/idea-lists/topics/%20padded", Some(&a), &Value::Null).await), ["name"]);

		let mut unheld = send("i2");
		unheld["answers"] = json!([{ "question": "what", "note": "r1" }, { "question": "spark", "note": "never-sent" }]);
		let (status, answer) = call(&app, Method::PUT, &idea_path("i2"), Some(&a), &unheld).await;
		assert_eq!(status, StatusCode::CONFLICT, "{answer}");
		assert_eq!(answer["error"]["code"], "conflict");
		assert_eq!(answer["error"]["details"], json!({ "notes": ["r1", "never-sent"] }), "each named by id");
		assert_eq!(
			call(&app, Method::PUT, &idea_path("i2"), Some(&b), &send("i2")).await.0,
			StatusCode::OK,
			"B's own n1 is B's to name"
		);

		let mut rewritten = send("i1");
		rewritten["format"] = json!("video");
		assert_eq!(
			call(&app, Method::PUT, &idea_path("i1"), Some(&a), &rewritten).await.0,
			StatusCode::CONFLICT,
			"a held send is one bundle"
		);
		assert_eq!(
			call(&app, Method::PUT, &(idea_path("i1") + "/set-aside"), Some(&a), &Value::Null).await.0,
			StatusCode::NO_CONTENT
		);
		rewritten["sentAt"] = json!("2026-11-01T12:00:00.000Z");
		assert_eq!(
			call(&app, Method::PUT, &idea_path("i1"), Some(&a), &rewritten).await.0,
			StatusCode::CONFLICT,
			"set aside for good"
		);

		for path in [idea_path("i1"), idea_path("nope")] {
			assert_eq!(call(&app, Method::GET, &path, Some(&b), &Value::Null).await.0, StatusCode::NOT_FOUND, "{path}");
			assert_eq!(
				call(&app, Method::PUT, &(path.clone() + "/furnishing"), Some(&b), &json!({ "furnishing": true })).await.0,
				StatusCode::NOT_FOUND
			);
			assert_eq!(call(&app, Method::PUT, &(path + "/set-aside"), Some(&b), &Value::Null).await.0, StatusCode::NOT_FOUND);
		}
		let ids: Vec<Value> = call(&app, Method::GET, "/ideas", Some(&b), &Value::Null).await.1["ideas"]
			.as_array()
			.unwrap()
			.iter()
			.map(|idea| idea["id"].clone())
			.collect();
		assert_eq!(ids, ["i2"], "B lists only B's");
		assert_eq!(
			call(&app, Method::GET, "/ideas", Some(&a), &Value::Null).await.1,
			json!({ "ideas": [], "more": false }),
			"nothing refused was written"
		);
	}

	/// docs/identity.md invariant 6: a title and a typed answer are the
	/// owner's own words, and neither reaches a log line or span field,
	/// refused or kept.
	#[tokio::test]
	async fn nothing_an_idea_says_reaches_a_log_line() {
		const SAID: &str = "a-title-only-i-wrote-zq7";
		let state = state().await;
		let phone = session(&state, A, Carrier::Bearer).await;
		let app = app(state);
		let (captured, _guard) = crate::privacy::capture_where(crate::auth::loggable);
		let mut said = send("i1");
		said["title"] = json!(SAID);
		said["answers"][1]["typed"] = json!(SAID);
		assert_eq!(call(&app, Method::PUT, &idea_path("i1"), Some(&phone), &said).await.0, StatusCode::OK);
		said["format"] = json!("video");
		assert_eq!(call(&app, Method::PUT, &idea_path("i1"), Some(&phone), &said).await.0, StatusCode::CONFLICT);
		said["title"] = json!(SAID.repeat(10));
		assert_eq!(call(&app, Method::PUT, &idea_path("i2"), Some(&phone), &said).await.0, StatusCode::UNPROCESSABLE_ENTITY);
		assert_eq!(call(&app, Method::GET, "/ideas", Some(&phone), &Value::Null).await.0, StatusCode::OK);

		let lines = captured.lines();
		assert!(lines.iter().any(|line| line.contains("otel.kind")), "no handler span was captured: {lines:#?}");
		for line in &lines {
			assert!(!line.contains(SAID), "what an idea says reached a log line: {line}");
		}
	}
}

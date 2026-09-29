//! Handlers for `routes::shelf` (#387): the validate-and-answer half.
//!
//! See that module's doc comment for the surface. The table's own rules (the
//! cap, the body check, idempotence by content hash) are `learner_shelf_repo`'s.
//!
//! **Refusals.** A read of something that cannot be on the shelf (an
//! unknown `:activity`, a key that is not a plain key) is a `404`, the same
//! answer as a key nobody kept: nothing can be stored there. A write
//! (`PUT`, `DELETE`) naming one is a `422` naming the field (`activity`,
//! `key`), because a write aimed at a place that cannot exist is a client bug
//! worth naming, and a `DELETE` answering `204` for a misspelt activity would
//! hide it. A body over the ceiling, not UTF-8, or not a JSON object or array
//! is a `422` on `body`. A new key on a full shelf is a `409`.
//!
//! **The deletion hold.** Each handler takes a [`SubjectId`] as its first
//! argument and keeps it until it returns, so a write holds off
//! `DELETE /auth/account` exactly as every other subject-scoped write does
//! (docs/identity.md, invariant 9).
//!
//! **No other writer.** These two handlers ([`put_item`], [`delete_item`])
//! are the only callers of `learner_shelf_repo::put` and `::delete`;
//! nothing syncs to the shelf in the background.

use crate::subject::SubjectId;
use crate::FileHostError;
use axum::{
	body::{Body, Bytes},
	extract::{Path, State},
	http::{header::CONTENT_TYPE, HeaderValue, StatusCode},
	response::{IntoResponse, Response},
	Json,
};
use chrono::{SecondsFormat, Utc};
use learner_shelf_repo::{is_plain_key, validate, Activity, Problem, ShelfEntry, ShelfError, KEY_PROBLEM, SHELF_CAP};
use serde::Serialize;
use sqlx::SqlitePool;
use tracing::instrument;

/// `GET /shelf/:activity`
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Listing {
	/// Oldest kept first, then by key. Never a body.
	pub items: Vec<ShelfEntry>,
	/// [`SHELF_CAP`], so the client can say how much room is left without
	/// hard-coding it.
	pub cap: i64,
}

/// `PUT /shelf/:activity/:key`
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Written {
	/// `kept`, `replaced` or `unchanged` — `learner_shelf_repo::Change`.
	pub change: &'static str,
	pub item: ShelfEntry,
}

const UNKNOWN_ACTIVITY: Problem = ("activity", "must be `topik` or `leetype`");
const NOT_A_KEY: Problem = ("key", KEY_PROBLEM);

/// The 409 a full shelf answers.
const SHELF_FULL: &str = "the shelf is full: delete an item before keeping another";

/// A read's target: a missing activity or a malformed key is simply absent.
fn read_target(activity: &str) -> Result<Activity, FileHostError> {
	Activity::parse(activity).ok_or(FileHostError::NotFound)
}

/// A write's target: each malformed part is a `422` naming its field.
fn write_target(activity: &str, key: &str) -> Result<Activity, Vec<Problem>> {
	let parsed = Activity::parse(activity);
	let mut problems = Vec::new();
	if parsed.is_none() {
		problems.push(UNKNOWN_ACTIVITY);
	}
	if !is_plain_key(key) {
		problems.push(NOT_A_KEY);
	}
	parsed.filter(|_| problems.is_empty()).ok_or(problems)
}

fn now() -> String {
	// Fixed width, so `ORDER BY saved_at` is chronological as text.
	Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// This subject's shelf for `activity`.
pub(crate) async fn listing(db: &SqlitePool, subject: &str, activity: &str) -> Result<Listing, FileHostError> {
	let activity = read_target(activity)?;
	let items = learner_shelf_repo::list(&mut *db.acquire().await?, subject, activity).await?;
	Ok(Listing { items, cap: SHELF_CAP })
}

/// This subject's kept body under `key`, verbatim, or `NotFound`.
pub(crate) async fn item(db: &SqlitePool, subject: &str, activity: &str, key: &str) -> Result<String, FileHostError> {
	let activity = read_target(activity)?;
	if !is_plain_key(key) {
		return Err(FileHostError::NotFound);
	}
	learner_shelf_repo::body(&mut *db.acquire().await?, subject, activity, key)
		.await?
		.ok_or(FileHostError::NotFound)
}

/// Keep `body` under `key` on this subject's shelf, as of `now`.
///
/// Every problem with the target and the body is found before the
/// transaction, so a refusal takes no write lock. `BEGIN IMMEDIATE`, so the
/// read that tells kept, replaced and unchanged apart and the write see one
/// snapshot (the cap itself is a condition of the insert).
pub(crate) async fn keep(db: &SqlitePool, subject: &str, activity: &str, key: &str, body: &[u8], now: &str) -> Result<Written, FileHostError> {
	let target = write_target(activity, key);
	let checked = validate(key, body);
	let activity = match (target, checked) {
		(Ok(activity), Ok(())) => activity,
		(target, checked) => {
			let mut problems = target.err().unwrap_or_default();
			problems.extend(checked.err().unwrap_or_default().into_iter().filter(|problem| *problem != NOT_A_KEY));
			return Err(FileHostError::unprocessable_entity(problems));
		}
	};
	let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
	let put = learner_shelf_repo::put(&mut tx, subject, activity, key, body, now).await.map_err(|err| match err {
		ShelfError::Invalid(problems) => FileHostError::unprocessable_entity(problems),
		ShelfError::Full => FileHostError::Conflict(SHELF_FULL),
		ShelfError::Storage(err) => FileHostError::Sqlite(err),
	})?;
	tx.commit().await?;
	Ok(Written {
		change: put.change.as_str(),
		item: put.entry,
	})
}

/// Remove `key` from this subject's shelf, if it is there.
pub(crate) async fn remove(db: &SqlitePool, subject: &str, activity: &str, key: &str) -> Result<(), FileHostError> {
	let activity = write_target(activity, key).map_err(FileHostError::unprocessable_entity)?;
	learner_shelf_repo::delete(&mut *db.acquire().await?, subject, activity, key).await?;
	Ok(())
}

/// `GET /shelf/:activity`
///
/// # Errors
/// 401 without a session, 404 for an activity the shelf does not serve, and
/// 500 for a storage failure.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "shelf_list", skip_all, fields(otel.kind = "server"))]
pub async fn list_items(subject: SubjectId, State(db): State<SqlitePool>, Path(activity): Path<String>) -> Result<Json<Listing>, FileHostError> {
	listing(&db, subject.as_str(), &activity).await.map(Json)
}

/// `GET /shelf/:activity/:key`
///
/// The body exactly as it was kept, as `application/json`. Absent, another
/// subject's, or unaddressable: a JSON `404`.
///
/// # Errors
/// 401 without a session, 404 as above, and 500 for a storage failure.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "shelf_item", skip_all, fields(otel.kind = "server"))]
pub async fn get_item(subject: SubjectId, State(db): State<SqlitePool>, Path((activity, key)): Path<(String, String)>) -> Result<Response, FileHostError> {
	let body = item(&db, subject.as_str(), &activity, &key).await?;
	let mut response = Response::new(Body::from(body));
	response.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
	Ok(response)
}

/// `PUT /shelf/:activity/:key`, the request body being the item itself.
///
/// # Errors
/// 401 without a session, 403 from an untrusted origin, 422 naming each
/// refused field, 409 for a new key on a full shelf, and 500 for a storage
/// failure.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "shelf_put", skip_all, fields(otel.kind = "server"))]
pub async fn put_item(subject: SubjectId, State(db): State<SqlitePool>, Path((activity, key)): Path<(String, String)>, body: Bytes) -> Result<Json<Written>, FileHostError> {
	keep(&db, subject.as_str(), &activity, &key, &body, &now()).await.map(Json)
}

/// `DELETE /shelf/:activity/:key` — `204` whether or not it was kept.
///
/// # Errors
/// 401 without a session, 403 from an untrusted origin, 422 naming a refused
/// activity or key, and 500 for a storage failure.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "shelf_delete", skip_all, fields(otel.kind = "server"))]
pub async fn delete_item(subject: SubjectId, State(db): State<SqlitePool>, Path((activity, key)): Path<(String, String)>) -> Result<Response, FileHostError> {
	remove(&db, subject.as_str(), &activity, &key).await?;
	Ok(StatusCode::NO_CONTENT.into_response())
}

#[cfg(test)]
mod tests {
	//! Through the real router, with real sessions (as `auth::operator`'s
	//! tests make them): each of #387's hard constraints is pinned here or in
	//! `learner_shelf_repo`'s own tests.

	use crate::auth::{cookie::SessionToken, now, AuthContext};
	use crate::routes::table::Module;
	use axum::{
		body::Body,
		extract::FromRef,
		http::{
			header::{CONTENT_TYPE, COOKIE},
			Method, Request, StatusCode,
		},
		Router,
	};
	use learner_shelf_repo::{SHELF_BODY_CEILING, SHELF_CAP};
	use publication_repo::PublicationRepository;
	use serde_json::{json, Value};
	use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};
	use std::collections::BTreeMap;
	use tower::ServiceExt;

	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");
	const A: &str = "subject-a";
	const B: &str = "subject-b";

	/// What the shelf's handlers need, and nothing that connects to NATS.
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
		crate::routes::shelf::shelf()
	}

	async fn state() -> TestState {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();
		TestState {
			auth: AuthContext::for_tests(pool.clone(), 100),
			pool,
		}
	}

	/// An account for `subject` and a live session on it, as the cookie a
	/// browser would send.
	async fn signed_in(state: &TestState, subject: &str) -> String {
		sqlx::query("INSERT INTO account (subject_id, user_handle) VALUES (?1, ?2)")
			.bind(subject)
			.bind(subject.as_bytes())
			.execute(&state.pool)
			.await
			.unwrap();
		let token = SessionToken::mint();
		let now = now();
		assert!(state.auth.repository().create_session(&token.hash(), subject, now + 3_600, now).await.unwrap());
		token.set_cookie(3_600).to_str().unwrap().split(';').next().unwrap().to_owned()
	}

	fn app(state: TestState) -> Router {
		module().into_table_router().with_state(state)
	}

	struct Answer {
		status: StatusCode,
		content_type: Option<String>,
		text: String,
	}

	impl Answer {
		fn json(&self) -> Value {
			serde_json::from_str(&self.text).unwrap()
		}
	}

	async fn call(app: &Router, method: Method, path: &str, cookie: Option<&str>, body: &[u8]) -> Answer {
		let mut request = Request::builder().method(method).uri(path);
		if let Some(cookie) = cookie {
			request = request.header(COOKIE, cookie);
		}
		let response = app.clone().oneshot(request.body(Body::from(body.to_vec())).unwrap()).await.unwrap();
		let status = response.status();
		let content_type = response.headers().get(CONTENT_TYPE).map(|value| value.to_str().unwrap().to_owned());
		let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
		Answer {
			status,
			content_type,
			text: String::from_utf8(bytes.to_vec()).unwrap(),
		}
	}

	async fn keys(app: &Router, cookie: &str, activity: &str) -> Vec<String> {
		let listed = call(app, Method::GET, &(String::from("/shelf/") + activity), Some(cookie), b"").await;
		assert_eq!(listed.status, StatusCode::OK, "{}", listed.text);
		listed.json()["items"]
			.as_array()
			.unwrap()
			.iter()
			.map(|item| item["key"].as_str().unwrap().to_owned())
			.collect()
	}

	/// Every registered shelf route, with its captures filled in.
	fn requests() -> Vec<(Method, String)> {
		module()
			.descriptors()
			.map(|route| {
				let path: Vec<&str> = route
					.path
					.split('/')
					.map(|segment| match segment {
						":activity" => "topik",
						":key" => "k",
						other => other,
					})
					.collect();
				(Method::from_bytes(route.method.as_bytes()).unwrap(), path.join("/"))
			})
			.collect()
	}

	/// #387's "blocked on auth": no route answers without a passkey session,
	/// and no write is accepted from an untrusted origin, whatever the
	/// session.
	#[tokio::test]
	async fn every_route_needs_a_session_and_writes_need_a_trusted_origin() {
		let state = state().await;
		let cookie = signed_in(&state, A).await;
		let app = app(state);
		let requests = requests();
		assert_eq!(requests.len(), 4, "{requests:?}");
		for (method, path) in requests {
			let answer = call(&app, method.clone(), &path, None, b"{}").await;
			assert_eq!(answer.status, StatusCode::UNAUTHORIZED, "{method} {path} without a session");
		}
		for method in [Method::PUT, Method::DELETE] {
			let request = Request::builder()
				.method(method.clone())
				.uri("/shelf/topik/k")
				.header(COOKIE, &cookie)
				.header("origin", "https://evil.app.test")
				.header("sec-fetch-site", "same-site")
				.body(Body::from("{}"))
				.unwrap();
			assert_eq!(app.clone().oneshot(request).await.unwrap().status(), StatusCode::FORBIDDEN, "{method}");
		}
	}

	/// The wire shapes, end to end: kept, listed, served verbatim, unchanged
	/// for the same bytes, replaced for new ones, deleted, and deleted again.
	#[tokio::test]
	async fn an_item_is_kept_listed_served_verbatim_replaced_and_deleted() {
		let state = state().await;
		let cookie = signed_in(&state, A).await;
		let app = app(state);
		let bytes = "{ \"lesson\":  {\"q\": [1, 2]} }\n";
		let hash = curriculum_repo::content_hash(bytes.as_bytes());

		assert_eq!(
			call(&app, Method::GET, "/shelf/topik", Some(&cookie), b"").await.json(),
			json!({ "items": [], "cap": SHELF_CAP })
		);

		let kept = call(&app, Method::PUT, "/shelf/topik/lesson-1", Some(&cookie), bytes.as_bytes()).await;
		assert_eq!(kept.status, StatusCode::OK, "{}", kept.text);
		let kept = kept.json();
		let saved_at = kept["item"]["savedAt"].as_str().unwrap().to_owned();
		assert_eq!(kept, json!({ "change": "kept", "item": { "key": "lesson-1", "contentHash": hash, "savedAt": saved_at } }));
		assert!(saved_at.ends_with('Z') && saved_at.len() == "2026-09-29T00:00:00.000Z".len(), "{saved_at}");

		let listed = call(&app, Method::GET, "/shelf/topik", Some(&cookie), b"").await.json();
		assert_eq!(listed, json!({ "items": [kept["item"]], "cap": SHELF_CAP }));
		assert_eq!(
			call(&app, Method::GET, "/shelf/leetype", Some(&cookie), b"").await.json()["items"],
			json!([]),
			"keyed by activity"
		);

		let served = call(&app, Method::GET, "/shelf/topik/lesson-1", Some(&cookie), b"").await;
		assert_eq!((served.status, served.content_type.as_deref()), (StatusCode::OK, Some("application/json")));
		assert_eq!(served.text, bytes, "verbatim, never re-serialised");

		let again = call(&app, Method::PUT, "/shelf/topik/lesson-1", Some(&cookie), bytes.as_bytes()).await.json();
		assert_eq!(again, json!({ "change": "unchanged", "item": kept["item"] }), "same bytes move nothing");

		let replaced = call(&app, Method::PUT, "/shelf/topik/lesson-1", Some(&cookie), b"[\"v2\"]").await.json();
		assert_eq!(replaced["change"], "replaced");
		assert_eq!(replaced["item"]["contentHash"], curriculum_repo::content_hash(b"[\"v2\"]"));
		assert_eq!(call(&app, Method::GET, "/shelf/topik/lesson-1", Some(&cookie), b"").await.text, "[\"v2\"]");

		for _ in 0..2 {
			let deleted = call(&app, Method::DELETE, "/shelf/topik/lesson-1", Some(&cookie), b"").await;
			assert_eq!((deleted.status, deleted.text.as_str()), (StatusCode::NO_CONTENT, ""), "idempotent");
		}
		let gone = call(&app, Method::GET, "/shelf/topik/lesson-1", Some(&cookie), b"").await;
		assert_eq!(gone.status, StatusCode::NOT_FOUND);
		assert_eq!(gone.json()["error"]["code"], "not_found", "a JSON 404, not a page");
	}

	/// #387, "not readable across subjects": with two real sessions, B cannot
	/// list, read or delete A's item, and B keeping the same key keeps B's own.
	/// A's key is a 404 to B, the answer for a key nobody kept.
	#[tokio::test]
	async fn one_subject_can_neither_list_read_nor_delete_anothers_items() {
		let state = state().await;
		let a = signed_in(&state, A).await;
		let b = signed_in(&state, B).await;
		let app = app(state);
		assert_eq!(call(&app, Method::PUT, "/shelf/topik/mine", Some(&a), b"{\"a\":1}").await.status, StatusCode::OK);

		assert!(keys(&app, &b, "topik").await.is_empty());
		let read = call(&app, Method::GET, "/shelf/topik/mine", Some(&b), b"").await;
		let absent = call(&app, Method::GET, "/shelf/topik/nobody-kept-this", Some(&b), b"").await;
		assert_eq!((read.status, read.text), (absent.status, absent.text), "indistinguishable from absent");
		assert_eq!(call(&app, Method::DELETE, "/shelf/topik/mine", Some(&b), b"").await.status, StatusCode::NO_CONTENT);
		assert_eq!(
			call(&app, Method::GET, "/shelf/topik/mine", Some(&a), b"").await.text,
			"{\"a\":1}",
			"A's item survived B's delete"
		);

		let theirs = call(&app, Method::PUT, "/shelf/topik/mine", Some(&b), b"{\"b\":2}").await.json();
		assert_eq!(theirs["change"], "kept", "B's own row, not a replace of A's");
		assert_eq!(call(&app, Method::GET, "/shelf/topik/mine", Some(&a), b"").await.text, "{\"a\":1}");
		assert_eq!(call(&app, Method::GET, "/shelf/topik/mine", Some(&b), b"").await.text, "{\"b\":2}");
	}

	/// #387, "over the cap, the write is refused, not evicted": the 21st new
	/// key is a 409 and the first 20 are all still there, bodies included.
	#[tokio::test]
	async fn the_21st_new_key_is_a_409_and_the_first_20_remain() {
		let state = state().await;
		let cookie = signed_in(&state, A).await;
		let app = app(state);
		let mut expected = Vec::new();
		for i in 0..SHELF_CAP {
			let key = String::from("lesson-") + &i.to_string();
			let body = String::from("{\"k\":\"") + &key + "\"}";
			assert_eq!(
				call(&app, Method::PUT, &(String::from("/shelf/topik/") + &key), Some(&cookie), body.as_bytes())
					.await
					.status,
				StatusCode::OK
			);
			expected.push((key, body));
		}
		let refused = call(&app, Method::PUT, "/shelf/topik/one-more", Some(&cookie), b"{}").await;
		assert_eq!(refused.status, StatusCode::CONFLICT, "{}", refused.text);
		assert_eq!(refused.json()["error"]["code"], "conflict");
		assert!(refused.json()["error"]["message"].as_str().unwrap().contains("shelf is full"), "{}", refused.text);

		let mut listed = keys(&app, &cookie, "topik").await;
		listed.sort();
		let mut wanted: Vec<String> = expected.iter().map(|(key, _)| key.clone()).collect();
		wanted.sort();
		assert_eq!(listed, wanted, "nothing evicted, nothing added");
		for (key, body) in &expected {
			assert_eq!(&call(&app, Method::GET, &(String::from("/shelf/topik/") + key), Some(&cookie), b"").await.text, body);
		}
		assert_eq!(
			call(&app, Method::PUT, "/shelf/topik/lesson-0", Some(&cookie), b"[]").await.json()["change"],
			"replaced",
			"a kept key can still be replaced at the cap"
		);
	}

	/// Unknown activity: 404 to a read, 422 to a write. A malformed key
	/// likewise. The body must be a JSON object or array under the ceiling.
	/// Nothing refused is written.
	#[tokio::test]
	async fn refusals_name_their_field_and_write_nothing() {
		let state = state().await;
		let cookie = signed_in(&state, A).await;
		let pool = state.pool.clone();
		let app = app(state);

		for path in ["/shelf/honeycomb", "/shelf/honeycomb/k", "/shelf/topik/k.json"] {
			assert_eq!(call(&app, Method::GET, path, Some(&cookie), b"").await.status, StatusCode::NOT_FOUND, "{path}");
		}
		let over = vec![b' '; SHELF_BODY_CEILING + 1];
		for (method, path, body, fields) in [
			(Method::PUT, "/shelf/honeycomb/k", &b"{}"[..], &["activity"][..]),
			(Method::PUT, "/shelf/topik/k.json", b"{}", &["key"]),
			(Method::PUT, "/shelf/Topik/.k", b"nope", &["activity", "body", "key"]),
			(Method::PUT, "/shelf/topik/k", b"not json", &["body"]),
			(Method::PUT, "/shelf/topik/k", b"\"a string\"", &["body"]),
			(Method::PUT, "/shelf/topik/k", b"", &["body"]),
			(Method::PUT, "/shelf/topik/k", &over, &["body"]),
			(Method::DELETE, "/shelf/honeycomb/k", b"", &["activity"]),
			(Method::DELETE, "/shelf/topik/k.json", b"", &["key"]),
		] {
			let answer = call(&app, method.clone(), path, Some(&cookie), body).await;
			assert_eq!(answer.status, StatusCode::UNPROCESSABLE_ENTITY, "{method} {path}: {}", answer.text);
			let details = answer.json()["error"]["details"].clone();
			let mut named: Vec<&str> = details.as_object().unwrap().keys().map(String::as_str).collect();
			named.sort_unstable();
			assert_eq!(named, fields, "{method} {path}: {details}");
		}
		let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM learner_shelf").fetch_one(&pool).await.unwrap();
		assert_eq!(rows, 0);
	}

	/// Every table but `learner_shelf`, each rendered as its sorted rows.
	async fn every_other_table(pool: &SqlitePool) -> BTreeMap<String, String> {
		let tables: Vec<String> = sqlx::query_scalar("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' AND name <> 'learner_shelf' ORDER BY name")
			.fetch_all(pool)
			.await
			.unwrap();
		let mut rendered = BTreeMap::new();
		for table in tables {
			let columns: Vec<String> = sqlx::query_scalar("SELECT name FROM pragma_table_info(?)").bind(&table).fetch_all(pool).await.unwrap();
			let row: Vec<String> = columns.iter().map(|column| String::from("quote(\"") + column + "\")").collect();
			let query = String::from("SELECT COALESCE(group_concat(r, char(10)), '') FROM (SELECT ") + &row.join(" || ',' || ") + " AS r FROM \"" + &table + "\" ORDER BY 1)";
			let rows: String = sqlx::query_scalar(&query).fetch_one(pool).await.unwrap();
			rendered.insert(table, rows);
		}
		rendered
	}

	/// #387, "never in `curriculum`, never in `curriculum_publication`": a
	/// shelf write changes no other table (so no corpus row, no publication,
	/// no engagement epoch or watermark), and detection finds nothing new
	/// after it, even when the kept key is a corpus lesson's key.
	#[tokio::test]
	async fn a_shelf_write_moves_no_epoch_and_touches_no_other_table() {
		let state = state().await;
		let cookie = signed_in(&state, A).await;
		let pool = state.pool.clone();
		let app = app(state);

		sqlx::query(
			"INSERT INTO curriculum (key, activity_id, display_name, description, batch_count, total_questions, total_messages, published_at, version, content_hash, body)
			 VALUES ('lesson-1', 'topik', 'n', 'd', 1, 1, 1, 'now', 1, 'h', '{}')",
		)
		.execute(&pool)
		.await
		.unwrap();
		let publications = PublicationRepository::new(pool.clone());
		publications.detect_activity_publications("now").await.unwrap();
		publications.detect_curriculum_publications("now").await.unwrap();
		let epoch = publications.newest().await.unwrap().unwrap().id;
		sqlx::query("INSERT INTO engagement_gate (subject_id, eligible_at, curriculum_epoch) VALUES (?1, 'later', ?2)")
			.bind(A)
			.bind(epoch)
			.execute(&pool)
			.await
			.unwrap();
		let before = every_other_table(&pool).await;
		assert!(before["curriculum_publication"].contains("'lesson-1'"), "sanity: the corpus lesson was published");

		for (method, path, body) in [
			(Method::PUT, "/shelf/topik/lesson-1", &b"{\"mine\":true}"[..]),
			(Method::PUT, "/shelf/topik/lesson-1", b"{\"mine\":2}"),
			(Method::PUT, "/shelf/leetype/round-1", b"{\"id\":\"round-1\"}"),
			(Method::PUT, "/shelf/topik/lesson-2", b"[]"),
			(Method::DELETE, "/shelf/topik/lesson-2", b""),
		] {
			assert!(call(&app, method.clone(), path, Some(&cookie), body).await.status.is_success(), "{method} {path}");
		}

		assert_eq!(every_other_table(&pool).await, before, "a shelf write touched another table");
		assert_eq!(publications.detect_activity_publications("later").await.unwrap(), 0);
		assert_eq!(publications.detect_curriculum_publications("later").await.unwrap(), 0);
		assert_eq!(publications.newest().await.unwrap().unwrap().id, epoch, "the epoch did not move");
		let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM learner_shelf").fetch_one(&pool).await.unwrap();
		assert_eq!(rows, 2, "sanity: the writes landed on the shelf");
	}
}

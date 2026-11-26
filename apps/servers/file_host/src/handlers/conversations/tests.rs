use super::{post, previews_of};
use crate::auth::{
	cookie::{Carrier, SessionToken},
	now, AuthContext,
};
use crate::conversations::Folder;
use crate::routes::table::Module;
use axum::{
	body::{to_bytes, Body},
	extract::FromRef,
	http::{
		header::{AUTHORIZATION, CONTENT_TYPE},
		Method, Request, StatusCode,
	},
	response::Response,
	Router,
};
use serde_json::{json, Value};
use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};
use tower::ServiceExt;

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
	crate::routes::conversations::conversations()
}

async fn state() -> TestState {
	let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
	crate::schema::MIGRATOR.run(&pool).await.unwrap();
	TestState {
		auth: AuthContext::for_tests(pool.clone(), 100),
		pool,
	}
}

async fn bearer(state: &TestState) -> String {
	sqlx::query("INSERT OR IGNORE INTO account (subject_id, user_handle) VALUES ('subject-a', x'61')")
		.execute(&state.pool)
		.await
		.unwrap();
	let token = SessionToken::mint();
	let now = now();
	assert!(state
		.auth
		.repository()
		.create_session(&token.hash(Carrier::Bearer), "subject-a", now + 3_600, now)
		.await
		.unwrap());
	String::from("Bearer ") + &token.encoded()
}

async fn send(app: &Router, method: Method, path: &str, auth: Option<&str>, body: Option<&Value>) -> (StatusCode, Value) {
	let mut request = Request::builder().method(method).uri(path);
	if let Some(auth) = auth {
		request = request.header(AUTHORIZATION, auth);
	}
	if body.is_some() {
		request = request.header(CONTENT_TYPE, "application/json");
	}
	let body = body.map_or_else(Body::empty, |body| Body::from(body.to_string()));
	let response = app.clone().oneshot(request.body(body).unwrap()).await.unwrap();
	answer(response).await
}

async fn answer(response: Response) -> (StatusCode, Value) {
	let status = response.status();
	let json = response
		.headers()
		.get(axum::http::header::CONTENT_TYPE)
		.is_some_and(|kind| kind.as_bytes().starts_with(b"application/json"));
	let bytes = to_bytes(response.into_body(), 1 << 20).await.unwrap();
	let value = if json { serde_json::from_slice(&bytes).unwrap() } else { Value::Null };
	(status, value)
}

fn topic() -> Value {
	json!({ "event": "topic", "by": "owner", "at": "2026-11-26T10:00:00.000Z", "topic": { "said": "why the record is a log" } })
}

fn green_lit() -> Vec<Value> {
	vec![
		topic(),
		json!({ "event": "version", "by": "agent", "at": "2026-11-26T10:02:00.000Z", "version": {
			"n": 1,
			"speakers": [{ "id": "tin", "character": "tin-owlet", "voice": { "kind": "kokoro" } }],
			"turns": [{ "id": "t1", "speaker": "tin", "voice": "kokoro", "said": "a line" }],
			"answers": []
		} }),
		json!({ "event": "heard", "by": "home", "at": "2026-11-26T10:04:00.000Z", "version": 1, "ms": 60_000 }),
		json!({ "event": "green-light", "by": "owner", "at": "2026-11-26T10:05:00.000Z", "version": 1 }),
	]
}

fn approve(preview: &str, variant: &str, rule: &str) -> Value {
	json!({
		"event": "approve",
		"by": "owner",
		"at": "2026-11-26T10:06:00.000Z",
		"preview": preview,
		"offered": ["a", "b"],
		"recommended": "a",
		"chose": { "kind": "variant", "variant": variant, "rule": rule }
	})
}

async fn rows(pool: &SqlitePool) -> i64 {
	sqlx::query_scalar("SELECT COUNT(*) FROM conversation_event").fetch_one(pool).await.unwrap()
}

#[tokio::test]
async fn every_route_needs_a_session_and_an_id_that_is_none_is_not_found() {
	let state = state().await;
	let phone = bearer(&state).await;
	let app = module().into_table_router().with_state(state);
	assert_eq!(module().descriptors().count(), 4);
	for (method, path) in [
		(Method::GET, "/conversations/c"),
		(Method::GET, "/conversations/c/previews"),
		(Method::GET, "/conversations/c/previews/p/a/video"),
		(Method::POST, "/conversations/c/events"),
	] {
		assert_eq!(send(&app, method, path, None, Some(&topic())).await.0, StatusCode::UNAUTHORIZED, "{path}");
	}
	for (method, path) in [
		(Method::GET, "/conversations/Not_An_Id"),
		(Method::GET, "/conversations/c"),
		(Method::GET, "/conversations/c/previews/p/a/video"),
		(Method::POST, "/conversations/-c/events"),
	] {
		assert_eq!(send(&app, method, path, Some(&phone), Some(&topic())).await.0, StatusCode::NOT_FOUND, "{path}");
	}
}

#[tokio::test]
async fn an_event_is_kept_once_and_the_record_reads_back_as_sent() {
	let state = state().await;
	let phone = bearer(&state).await;
	let app = module().into_table_router().with_state(state.clone());
	for (at, event) in green_lit().iter().enumerate() {
		assert_eq!(
			send(&app, Method::POST, "/conversations/c/events", Some(&phone), Some(event)).await,
			(StatusCode::CREATED, json!({ "kept": at, "events": at + 1 }))
		);
	}
	let again = send(&app, Method::POST, "/conversations/c/events", Some(&phone), Some(&green_lit()[3])).await;
	assert_eq!(again, (StatusCode::OK, json!({ "kept": 3, "events": 4 })), "a retried send keeps nothing twice");
	let (status, record) = send(&app, Method::GET, "/conversations/c", Some(&phone), None).await;
	assert_eq!(status, StatusCode::OK);
	assert_eq!(record, json!({ "schema": "some-ui.conversation/2", "id": "c", "events": green_lit() }));
	assert_eq!(rows(&state.pool).await, 4);
}

#[tokio::test]
async fn a_refused_event_answers_why_and_writes_nothing() {
	let state = state().await;
	let phone = bearer(&state).await;
	let app = module().into_table_router().with_state(state.clone());
	send(&app, Method::POST, "/conversations/c/events", Some(&phone), Some(&topic())).await;
	let lit = json!({ "event": "green-light", "by": "owner", "at": "2026-11-26T10:05:00.000Z", "version": 1 });
	assert_eq!(
		send(&app, Method::POST, "/conversations/c/events", Some(&phone), Some(&lit)).await,
		(
			StatusCode::CONFLICT,
			json!({ "refused": { "why": "out-of-phase", "phase": "drafting", "event": "green-light" }, "at": 1 })
		)
	);
	let mut extra = topic();
	extra["value"] = json!(3);
	assert_eq!(
		send(&app, Method::POST, "/conversations/c/events", Some(&phone), Some(&extra)).await.0,
		StatusCode::UNPROCESSABLE_ENTITY
	);
	let mut old_read = json!({ "event": "point", "by": "owner", "at": "2026-11-26T10:01:00.000Z", "take": "take-1", "basis": 0 });
	assert_eq!(
		send(&app, Method::POST, "/conversations/c/events", Some(&phone), Some(&old_read)).await,
		(StatusCode::CONFLICT, json!({ "refused": { "why": "stale", "basis": 0, "seq": 1 }, "at": 1 })),
		"a write against an older log is refused by number"
	);
	old_read["basis"] = json!(1);
	old_read["at"] = json!("2020-01-01T00:00:00.000Z");
	assert_eq!(
		send(&app, Method::POST, "/conversations/c/events", Some(&phone), Some(&old_read)).await,
		(StatusCode::CREATED, json!({ "kept": 1, "events": 2 })),
		"a writer's clock is never compared"
	);
	assert_eq!(rows(&state.pool).await, 2);
}

#[tokio::test]
async fn an_approval_is_kept_only_for_a_preview_home_offers_as_it_offers_it() {
	let state = state().await;
	let dir = tempfile::tempdir().unwrap();
	let folder = Folder(dir.path().to_owned());
	crate::conversations::tests::render(dir.path(), "p-close", &[("a", "calm-close"), ("b", "lively-close")], "a");
	for event in green_lit() {
		assert_eq!(
			post(&state.pool, "subject-a", &folder, "c", event.to_string().as_bytes()).await.unwrap().status(),
			StatusCode::CREATED
		);
	}
	let unknown = answer(
		post(&state.pool, "subject-a", &folder, "c", approve("p-open", "a", "warm").to_string().as_bytes())
			.await
			.unwrap(),
	)
	.await;
	assert_eq!(
		unknown,
		(StatusCode::CONFLICT, json!({ "refused": { "why": "preview-unknown", "preview": "p-open" }, "at": 4 }))
	);
	let unoffered = answer(
		post(&state.pool, "subject-a", &folder, "c", approve("p-close", "c", "loud").to_string().as_bytes())
			.await
			.unwrap(),
	)
	.await;
	assert_eq!(
		unoffered,
		(
			StatusCode::CONFLICT,
			json!({ "refused": { "why": "preview-variant", "preview": "p-close", "variant": "c" }, "at": 4 })
		),
		"the record's step refuses a variant the event never offered"
	);
	let other_rule = answer(
		post(&state.pool, "subject-a", &folder, "c", approve("p-close", "b", "calm-close").to_string().as_bytes())
			.await
			.unwrap(),
	)
	.await;
	assert_eq!(
		other_rule,
		(StatusCode::CONFLICT, json!({ "refused": { "why": "preview-offer", "preview": "p-close" }, "at": 4 }))
	);
	assert_eq!(rows(&state.pool).await, 4, "a refused approval writes nothing");
	let picked = post(&state.pool, "subject-a", &folder, "c", approve("p-close", "b", "lively-close").to_string().as_bytes())
		.await
		.unwrap();
	assert_eq!(answer(picked).await, (StatusCode::CREATED, json!({ "kept": 4, "events": 5 })));
	let theirs = answer(post(&state.pool, "subject-b", &folder, "c", green_lit()[0].to_string().as_bytes()).await.unwrap()).await;
	assert_eq!(theirs.0, StatusCode::CREATED, "subject B's own conversation c");
	assert!(
		previews_of(&state.pool, "subject-b", &folder, "c").await.unwrap().previews.is_empty(),
		"A's previews are none of B's"
	);
	let listed = previews_of(&state.pool, "subject-a", &folder, "c").await.unwrap();
	let shown = serde_json::to_value(&listed).unwrap();
	assert_eq!(
		shown["previews"][0]["approved"],
		json!({ "chose": { "kind": "variant", "variant": "b", "rule": "lively-close" } })
	);
	assert_eq!(shown["previews"][0]["variants"][1]["video"], json!("/api/v1/conversations/c/previews/p-close/b/video"));
}

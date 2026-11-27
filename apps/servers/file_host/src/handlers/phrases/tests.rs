use crate::auth::{
	cookie::{Carrier, SessionToken},
	now, AuthContext,
};
use crate::meaning::MeaningContext;
use crate::phrases::{
	tests::{Words, SHELF, VOCABULARY},
	Phrases,
};
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
use std::sync::Arc;
use tower::ServiceExt;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");
const A: &str = "subject-a";
const B: &str = "subject-b";
const HUNCH: &str = "0b9c1f0e-5a3f-4c1e-9d6b-2f1d8e7a6c5b-1";

#[derive(Clone)]
struct TestState {
	pool: SqlitePool,
	auth: AuthContext,
	meaning: Option<MeaningContext>,
}

impl FromRef<TestState> for Option<MeaningContext> {
	fn from_ref(state: &TestState) -> Self {
		state.meaning.clone()
	}
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

/// The meaning index over the test shelf, by [`Words`]' model.
fn meaning() -> MeaningContext {
	let model = Words(VOCABULARY);
	let phrases = Phrases::from_text(SHELF, &model).unwrap();
	MeaningContext::new(Arc::new(model), Arc::new(phrases))
}

async fn state(meaning: Option<MeaningContext>) -> TestState {
	let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
	MIGRATOR.run(&pool).await.unwrap();
	TestState {
		auth: AuthContext::for_tests(pool.clone(), 100),
		pool,
		meaning,
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
	crate::routes::phrases::phrases().into_table_router().with_state(state)
}

async fn look_up(app: &Router, auth: Option<&(HeaderName, String)>, body: &str) -> (StatusCode, Value) {
	let mut request = Request::builder().method(Method::POST).uri("/phrases/lookup").header(CONTENT_TYPE, "application/json");
	if let Some((name, value)) = auth {
		request = request.header(name, value);
	}
	let response = app.clone().oneshot(request.body(Body::from(body.to_owned())).unwrap()).await.unwrap();
	let status = response.status();
	let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
	(status, serde_json::from_slice(&bytes).unwrap())
}

fn asked(words: &str, hunch: &str) -> String {
	json!({"words": words, "hunch": hunch}).to_string()
}

async fn kept(pool: &SqlitePool) -> Vec<(String, String, String)> {
	sqlx::query_as("SELECT subject_id, hunch, words FROM phrase_hunch ORDER BY subject_id, hunch")
		.fetch_all(pool)
		.await
		.unwrap()
}

/// The story's "done when": a hunch an intent says is a hit with its
/// phrase, and keeps nothing.
#[tokio::test]
async fn a_hunch_an_intent_says_is_a_hit_and_keeps_nothing() {
	let state = state(Some(meaning())).await;
	let phone = session(&state, A, Carrier::Bearer).await;
	let app = app(state.clone());
	let (status, answer) = look_up(&app, Some(&phone), &asked("  Where my phone ", HUNCH)).await;
	assert_eq!((status, answer), (StatusCode::OK, json!({"kind": "hit", "phrases": ["ph.where-is-my-phone"]})));
	let (status, answer) = look_up(&app, Some(&phone), &asked("my phone", HUNCH)).await;
	assert_eq!(
		(status, answer),
		(StatusCode::OK, json!({"kind": "hit", "phrases": ["ph.where-is-my-phone", "ph.lost-my-phone"]})),
		"close runners-up come too, closest first"
	);
	assert!(kept(&state.pool).await.is_empty());
}

/// The story's other "done when": a hunch near nothing answers `kept` and
/// is in the store, once however often it is sent.
#[tokio::test]
async fn a_miss_is_kept_in_the_answer_once_however_often_it_is_sent() {
	let state = state(Some(meaning())).await;
	let phone = session(&state, A, Carrier::Bearer).await;
	let other = session(&state, B, Carrier::Bearer).await;
	let app = app(state.clone());
	for _ in 0..2 {
		let (status, answer) = look_up(&app, Some(&phone), &asked(" how do I say I'm sleepy ", HUNCH)).await;
		assert_eq!((status, answer), (StatusCode::OK, json!({"kind": "kept"})));
	}
	let (status, answer) = look_up(&app, Some(&other), &asked("rain", HUNCH)).await;
	assert_eq!((status, answer), (StatusCode::OK, json!({"kind": "kept"})), "near, but below the threshold");
	assert_eq!(
		kept(&state.pool).await,
		[(A.into(), HUNCH.into(), "how do I say I'm sleepy".into()), (B.into(), HUNCH.into(), "rain".into())],
		"one each, trimmed, each subject's own"
	);
}

#[tokio::test]
async fn a_lookup_out_of_shape_is_refused_by_what_is_wrong_and_keeps_nothing() {
	let state = state(Some(meaning())).await;
	let phone = session(&state, A, Carrier::Bearer).await;
	let app = app(state.clone());
	let long = "a".repeat(201);
	for (body, field) in [
		(asked("", HUNCH), "words"),
		(asked("   ", HUNCH), "words"),
		(asked(&long, HUNCH), "words"),
		(asked("w", ""), "hunch"),
		(asked("w", "a b"), "hunch"),
		(asked("w", &"a".repeat(65)), "hunch"),
		(String::from("{\"words\": \"w\"}"), "body"),
		(String::from("{\"words\": \"w\", \"hunch\": \"h\", \"target\": \"ph.its-okay\"}"), "body"),
		(String::from("not json"), "body"),
	] {
		let (status, answer) = look_up(&app, Some(&phone), &body).await;
		assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}: {answer}");
		assert!(answer.to_string().contains(field), "{body} names {field}: {answer}");
	}
	assert!(kept(&state.pool).await.is_empty());
}

#[tokio::test]
async fn only_a_paired_phone_looks_up() {
	let state = state(Some(meaning())).await;
	let browser = session(&state, A, Carrier::Cookie).await;
	let app = app(state.clone());
	assert_eq!(look_up(&app, None, &asked("w", HUNCH)).await.0, StatusCode::UNAUTHORIZED);
	assert_eq!(look_up(&app, Some(&browser), &asked("w", HUNCH)).await.0, StatusCode::FORBIDDEN);
	assert!(kept(&state.pool).await.is_empty());
}

/// Off, home can tell no hunch's phrases, so it answers neither and keeps
/// nothing: the phone reads the `503` as unreached and sends it again.
#[tokio::test]
async fn with_the_meaning_index_off_home_answers_unavailable_and_keeps_nothing() {
	let state = state(None).await;
	let phone = session(&state, A, Carrier::Bearer).await;
	let app = app(state.clone());
	let (status, answer) = look_up(&app, Some(&phone), &asked("how do I say I'm sleepy", HUNCH)).await;
	assert_eq!(
		(status, &answer["error"]["code"]),
		(StatusCode::SERVICE_UNAVAILABLE, &json!("feature_not_configured")),
		"{answer}"
	);
	assert!(kept(&state.pool).await.is_empty());
}

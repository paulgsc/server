//! Handlers for `routes::study_notes`: the validate-and-answer half. The
//! record's own rules (append-only, content never parsed) are
//! `study_note_repo`'s.
//!
//! **Refusals.** A note with an unknown kind, an id that breaks the rule or a
//! body that is not a JSON object within the ceiling is a `422` naming each
//! field and why, and other bytes under an id home holds are a `409`. Both
//! write nothing, so the phone can say which note home refused and keep it.
//!
//! **Nothing said reaches a log line.** The span skips every argument, and a
//! refusal names fields with fixed words, never what the note says: a
//! `LeetType` answer can carry what the owner typed.
//!
//! **The deletion hold.** The handler takes a [`SubjectId`] first and keeps
//! it until it returns, so a write holds off `DELETE /auth/account` as every
//! other subject-scoped write does (docs/identity.md, invariant 9).
//!
//! **No other writer.** [`put_note`] is the only caller of
//! `study_note_repo::put`; nothing writes the record in the background.

use crate::subject::SubjectId;
use crate::FileHostError;
use axum::{
	body::Bytes,
	extract::{Path, State},
	Json,
};
use chrono::{SecondsFormat, Utc};
use serde::Serialize;
use sqlx::SqlitePool;
use study_note_repo::{Note, NoteError, REWRITTEN};
use tracing::instrument;

/// `PUT /study-notes/:kind/:id`
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NoteWritten {
	/// `added` or `unchanged` — `study_note_repo::Change`.
	pub change: &'static str,
	/// The note as home holds it, its SHA-256 included, which the phone
	/// compares with what it sent before it counts the note as home's.
	pub note: Note,
}

fn now() -> String {
	// Fixed width, as `handlers::voice`'s.
	Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn refusal(err: NoteError) -> FileHostError {
	match err {
		NoteError::Invalid(problems) => FileHostError::unprocessable_entity(problems),
		NoteError::Rewritten => FileHostError::Conflict(REWRITTEN),
		NoteError::Storage(err) => FileHostError::Sqlite(err),
	}
}

/// `PUT /study-notes/:kind/:id`, the request body being the note itself.
/// `BEGIN IMMEDIATE`, so the read that decides and the write see one
/// snapshot.
///
/// # Errors
/// 401 without a session, 403 from an untrusted origin, 422 naming each
/// refused field (`kind`, `id`, `body`), 409 for other bytes under a held
/// id, and 500 for a storage failure.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "study_note_put", skip_all, fields(otel.kind = "server"))]
pub async fn put_note(subject: SubjectId, State(db): State<SqlitePool>, Path((kind, id)): Path<(String, String)>, body: Bytes) -> Result<Json<NoteWritten>, FileHostError> {
	let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
	let (change, note) = study_note_repo::put(&mut tx, subject.as_str(), &kind, &id, &body, &now()).await.map_err(refusal)?;
	tx.commit().await?;
	Ok(Json(NoteWritten { change: change.as_str(), note }))
}

#[cfg(test)]
mod tests {
	//! Through the real router, with real phone sessions (bearer tokens, as a
	//! paired phone sends) and a browser's.

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
	const NOTE: &str = "/study-notes/leetype/n1";
	const ANSWER: &str = r#"{"id":"n1","prompt":"Why a map?","ticks":["O(1) lookup"],"typed":"a secret only the owner typed"}"#;

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
		crate::routes::study_notes::study_notes()
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

	async fn put(app: &Router, path: &str, auth: Option<&(HeaderName, String)>, body: &str) -> (StatusCode, Value) {
		let mut request = Request::builder().method(Method::PUT).uri(path).header(CONTENT_TYPE, "application/json");
		if let Some((name, value)) = auth {
			request = request.header(name, value);
		}
		let response = app.clone().oneshot(request.body(Body::from(body.to_owned())).unwrap()).await.unwrap();
		let status = response.status();
		let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
		(status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
	}

	#[tokio::test]
	async fn the_route_needs_a_session_and_a_trusted_origin() {
		let state = state().await;
		let phone = session(&state, A, Carrier::Bearer).await;
		let app = app(state);
		assert_eq!(module().descriptors().count(), 1);
		assert_eq!(put(&app, NOTE, None, ANSWER).await.0, StatusCode::UNAUTHORIZED);
		let request = Request::builder()
			.method(Method::PUT)
			.uri(NOTE)
			.header(&phone.0, &phone.1)
			.header(CONTENT_TYPE, "application/json")
			.header("origin", "https://evil.app.test")
			.header("sec-fetch-site", "cross-site")
			.body(Body::from(ANSWER))
			.unwrap();
		assert_eq!(app.clone().oneshot(request).await.unwrap().status(), StatusCode::FORBIDDEN);
	}

	/// The phone's round: a note added, the same note unchanged, its bytes
	/// kept as sent, and a browser session on the same account seeing the
	/// same note as held.
	#[tokio::test]
	async fn a_paired_phone_sends_a_note_home_once() {
		let state = state().await;
		let pool = state.pool.clone();
		let phone = session(&state, A, Carrier::Bearer).await;
		let browser = session(&state, A, Carrier::Cookie).await;
		let app = app(state);
		let held = |change: &str| {
			json!({
				"change": change,
				"note": {
					"kind": "leetype", "id": "n1", "size": ANSWER.len(),
					"sha256": curriculum_repo::content_hash(ANSWER.as_bytes())
				}
			})
		};

		let (status, answer) = put(&app, NOTE, Some(&phone), ANSWER).await;
		assert_eq!(status, StatusCode::OK, "{answer}");
		assert_eq!(answer, held("added"));
		assert_eq!(put(&app, NOTE, Some(&phone), ANSWER).await, (StatusCode::OK, held("unchanged")));
		assert_eq!(put(&app, NOTE, Some(&browser), ANSWER).await.1["change"], "unchanged");
		let body: String = sqlx::query_scalar("SELECT body FROM study_note WHERE subject_id = ?1")
			.bind(A)
			.fetch_one(&pool)
			.await
			.unwrap();
		assert_eq!(body, ANSWER);
	}

	/// A note that breaks a rule is a 422 naming each field, other bytes under
	/// a held id are a 409, and one subject's notes are never another's.
	#[tokio::test]
	async fn refusals_name_their_fields_and_notes_stay_apart() {
		let state = state().await;
		let pool = state.pool.clone();
		let a = session(&state, A, Carrier::Bearer).await;
		let b = session(&state, B, Carrier::Bearer).await;
		let app = app(state);
		assert_eq!(put(&app, NOTE, Some(&a), ANSWER).await.0, StatusCode::OK);

		let (status, answer) = put(&app, "/study-notes/diary/n%20two", Some(&a), "[1]").await;
		assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{answer}");
		let mut fields: Vec<String> = answer["error"]["details"].as_object().unwrap().keys().cloned().collect();
		fields.sort();
		assert_eq!(fields, ["body", "id", "kind"]);
		assert!(!answer.to_string().contains("secret"), "a refusal never quotes the note");

		let (status, answer) = put(&app, NOTE, Some(&a), r#"{"id":"n1","ticks":[]}"#).await;
		assert_eq!(status, StatusCode::CONFLICT, "{answer}");
		assert_eq!(answer["error"]["code"], "conflict");

		assert_eq!(put(&app, NOTE, Some(&b), r#"{"other":true}"#).await.1["change"], "added", "B's ids are B's");
		let held: Vec<(String, String)> = sqlx::query_as("SELECT subject_id, body FROM study_note ORDER BY subject_id")
			.fetch_all(&pool)
			.await
			.unwrap();
		assert_eq!(
			held,
			[(A.to_owned(), ANSWER.to_owned()), (B.to_owned(), String::from(r#"{"other":true}"#))],
			"nothing refused was written"
		);
	}
}

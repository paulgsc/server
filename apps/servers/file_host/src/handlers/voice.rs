//! Handlers for `routes::voice`: the validate-and-answer half. The notes' own
//! rules (append-only, what a note holds, where its audio sits) are
//! `voice_repo`'s.
//!
//! **Refusals.** A note that breaks a rule is a `422` naming each field and
//! why, and other bytes or another description under an id home holds are a
//! `409`. Both write nothing, so the phone can say which note home refused
//! and keep it.
//!
//! **The one way audio reaches home** (docs/identity.md invariant 23):
//! [`put_note`] is the only caller of `voice_repo::put_note`, and nothing
//! writes `voice_note` or `voice_note_audio` in the background. Neither the
//! audio nor its hash reaches a log line: the span skips every argument.
//!
//! **The deletion hold.** The handler takes a [`SubjectId`] first and keeps
//! it until it returns, so a write holds off `DELETE /auth/account` as every
//! other subject-scoped write does (docs/identity.md, invariant 9).
//!
//! **No standing choice yet.** Unlike aph's and the Jobs record's writes,
//! this one is not checked against what a paired phone's person chose to
//! keep sending (`handlers::sharing::refusing`): `sharing_choice` has no
//! scope for voice notes. The phone's own opt-in (paulgsc/some-ui#2038) adds
//! one, and this handler then calls `refusing` with it.

use crate::subject::SubjectId;
use crate::FileHostError;
use axum::{
	body::Bytes,
	extract::{Path, Query, State},
	http::{header::CONTENT_TYPE, HeaderMap},
	Json,
};
use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tracing::instrument;
use voice_repo::{Note, Said, VoiceError, REWRITTEN};

/// `PUT /voice-notes/:id`'s query: what the phone says the note is.
#[derive(Debug, Deserialize)]
pub struct NoteQuery {
	of: Option<String>,
	#[serde(rename = "ref")]
	of_ref: Option<String>,
	at: Option<String>,
}

/// `PUT /voice-notes/:id`
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NoteWritten {
	/// `added` or `unchanged` — `voice_repo::Change`.
	pub change: &'static str,
	/// The note as home holds it, its SHA-256 included, which the phone
	/// compares with what it sent before it counts the note as home's.
	pub note: Note,
}

fn now() -> String {
	// Fixed width, as `handlers::jobs`'.
	Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn refusal(err: VoiceError) -> FileHostError {
	match err {
		VoiceError::Rewritten => FileHostError::Conflict(REWRITTEN),
		VoiceError::Unreadable => FileHostError::OperationError("a stored voice note does not read back".to_owned()),
		VoiceError::Storage(err) => FileHostError::Sqlite(err),
	}
}

/// `PUT /voice-notes/:id?of=&ref=&at=`, the request body being the audio
/// itself and its `Content-Type` its container. `BEGIN IMMEDIATE`, so the
/// read that decides and both writes see one snapshot.
///
/// # Errors
/// 401 without a session, 403 from an untrusted origin, 422 naming each
/// refused field (`id`, `of`, `ref`, `at`, `contentType`, `body`), 409 for
/// another note under a held id, 413 for a body past the server's limit, and
/// 500 for a storage failure.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "voice_note_put", skip_all, fields(otel.kind = "server"))]
pub async fn put_note(
	subject: SubjectId,
	State(db): State<SqlitePool>,
	headers: HeaderMap,
	Path(id): Path<String>,
	Query(query): Query<NoteQuery>,
	body: Bytes,
) -> Result<Json<NoteWritten>, FileHostError> {
	let said = Said {
		of: query.of.as_deref(),
		of_ref: query.of_ref.as_deref(),
		at: query.at.as_deref(),
		content_type: headers.get(CONTENT_TYPE).and_then(|value| value.to_str().ok()),
	};
	let note = voice_repo::describe(&id, said, &body).map_err(FileHostError::unprocessable_entity)?;
	let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
	let change = voice_repo::put_note(&mut tx, subject.as_str(), &note, &body, &now()).await.map_err(refusal)?;
	tx.commit().await?;
	Ok(Json(NoteWritten { change: change.as_str(), note }))
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
	const NOTE: &str = "/voice-notes/n1?of=reflection&ref=session-7f2a&at=2026-10-24T20:10:00-07:00";
	const TAKE: &[u8] = b"opus frames";
	/// SHA-256 of `TAKE`.
	const TAKE_SHA256: &str = "23db732c0eb6cefc9a7cccf43093477df991eb223245bb88e8221a6bd59837a8";

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
		crate::routes::voice::voice()
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

	async fn put(app: &Router, path: &str, auth: Option<&(HeaderName, String)>, content_type: &str, body: &[u8]) -> (StatusCode, Value) {
		let mut request = Request::builder().method(Method::PUT).uri(path).header(CONTENT_TYPE, content_type);
		if let Some((name, value)) = auth {
			request = request.header(name, value);
		}
		let response = app.clone().oneshot(request.body(Body::from(body.to_vec())).unwrap()).await.unwrap();
		let status = response.status();
		let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
		(status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
	}

	fn held(change: &str) -> Value {
		json!({
			"change": change,
			"note": {
				"id": "n1", "of": "reflection", "ref": "session-7f2a", "at": "2026-10-25T03:10:00.000Z",
				"contentType": "audio/webm", "size": TAKE.len(), "sha256": TAKE_SHA256
			}
		})
	}

	#[tokio::test]
	async fn the_route_needs_a_session_and_a_trusted_origin() {
		let state = state().await;
		let phone = session(&state, A, Carrier::Bearer).await;
		let app = app(state);
		assert_eq!(module().descriptors().count(), 1);
		assert_eq!(put(&app, NOTE, None, "audio/webm", TAKE).await.0, StatusCode::UNAUTHORIZED);
		let request = Request::builder()
			.method(Method::PUT)
			.uri(NOTE)
			.header(&phone.0, &phone.1)
			.header(CONTENT_TYPE, "audio/webm")
			.header("origin", "https://evil.app.test")
			.header("sec-fetch-site", "cross-site")
			.body(Body::from(TAKE))
			.unwrap();
		assert_eq!(app.clone().oneshot(request).await.unwrap().status(), StatusCode::FORBIDDEN);
	}

	/// The phone's round: a note added, the same note unchanged, its audio
	/// kept byte for byte, and a browser session on the same account seeing
	/// the same note as held.
	#[tokio::test]
	async fn a_paired_phone_sends_a_note_home_once() {
		let state = state().await;
		let pool = state.pool.clone();
		let phone = session(&state, A, Carrier::Bearer).await;
		let browser = session(&state, A, Carrier::Cookie).await;
		let app = app(state);

		let (status, answer) = put(&app, NOTE, Some(&phone), "audio/webm;codecs=opus", TAKE).await;
		assert_eq!(status, StatusCode::OK, "{answer}");
		assert_eq!(answer, held("added"));
		assert_eq!(put(&app, NOTE, Some(&phone), "audio/webm", TAKE).await, (StatusCode::OK, held("unchanged")));
		assert_eq!(put(&app, NOTE, Some(&browser), "audio/webm", TAKE).await.1["change"], "unchanged");

		let audio: Vec<u8> = sqlx::query_scalar("SELECT body FROM voice_note_audio WHERE subject_id = ?1 AND note_id = 'n1'")
			.bind(A)
			.fetch_one(&pool)
			.await
			.unwrap();
		assert_eq!(audio, TAKE);
	}

	/// A note that breaks a rule is a 422 naming each field, another note
	/// under a held id is a 409, and one subject's notes are never another's.
	#[tokio::test]
	async fn refusals_name_their_fields_and_notes_stay_apart() {
		let state = state().await;
		let pool = state.pool.clone();
		let a = session(&state, A, Carrier::Bearer).await;
		let b = session(&state, B, Carrier::Bearer).await;
		let app = app(state);
		assert_eq!(put(&app, NOTE, Some(&a), "audio/webm", TAKE).await.0, StatusCode::OK);

		let named = |answer: (StatusCode, Value)| {
			assert_eq!(answer.0, StatusCode::UNPROCESSABLE_ENTITY, "{}", answer.1);
			let mut fields: Vec<String> = answer.1["error"]["details"].as_object().unwrap().keys().cloned().collect();
			fields.sort();
			fields
		};
		assert_eq!(
			named(put(&app, "/voice-notes/n2?of=diary&at=soon", Some(&a), "video/mp4", b"").await),
			["at", "body", "contentType", "of", "ref"]
		);
		assert_eq!(
			named(
				put(
					&app,
					"/voice-notes/n2?of=jobs&ref=pick-1&at=2026-10-25T03:10:00Z",
					Some(&a),
					"audio/ogg",
					&vec![0; voice_repo::NOTE_CEILING + 1]
				)
				.await
			),
			["body"]
		);

		let (status, answer) = put(&app, NOTE, Some(&a), "audio/webm", b"another take").await;
		assert_eq!(status, StatusCode::CONFLICT, "{answer}");
		assert_eq!(answer["error"]["code"], "conflict");

		assert_eq!(put(&app, NOTE, Some(&b), "audio/webm", b"another take").await.1["change"], "added", "B's ids are B's");
		let held: Vec<(String, i64)> = sqlx::query_as("SELECT subject_id, size FROM voice_note ORDER BY subject_id")
			.fetch_all(&pool)
			.await
			.unwrap();
		let take = i64::try_from(TAKE.len()).unwrap();
		assert_eq!(held, [(A.to_owned(), take), (B.to_owned(), 12)], "nothing refused was written");
	}
}

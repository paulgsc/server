//! Handlers for `routes::episode_notes`: the owner's notes on the feeds'
//! episodes (SFEED6: paulgsc/some-ui#2745).
//!
//! `PUT /feeds/:feed/notes/:id` keeps each note the phone sends home
//! (`episode_note_repo`, docs/identity.md invariant 38), append-only, and in
//! the same transaction files it for the mailbox's `feedback` folder
//! (`mailbox::pass::feedback_file`), where the routine reads it. Home files
//! no issue from a note (SFEED decision 6). A spoken note names its take by
//! voice note id only, and what home heard in that take reaches the routine
//! as its transcript (`transcript`), never through here.
//!
//! Nothing of a note reaches a log: each handler records only `otel.kind`.

use crate::feeds::Feed;
use crate::subject::SubjectId;
use crate::FileHostError;
use axum::{
	body::Bytes,
	extract::{Path, State},
	Json,
};
use chrono::{SecondsFormat, Utc};
use serde::Serialize;
use sha2::{Digest as _, Sha256};
use sqlx::SqlitePool;
use tracing::instrument;

fn feed(name: &str) -> Result<Feed, FileHostError> {
	Feed::parse(name).ok_or(FileHostError::NotFound)
}

/// `GET /feeds/:feed/notes`
#[derive(Debug, Serialize)]
pub struct Notes {
	/// Oldest written first.
	pub notes: Vec<episode_note_repo::Held>,
}

/// `GET /feeds/:feed/notes`: every note the caller left on the feed's
/// episodes, oldest written first.
///
/// # Errors
/// 401 without a session, 404 for a feed home does not render, and 500 for
/// a storage failure.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "episode_notes_get", skip_all, fields(otel.kind = "server"))]
pub async fn get_notes(subject: SubjectId, State(db): State<SqlitePool>, Path(name): Path<String>) -> Result<Json<Notes>, FileHostError> {
	let feed = feed(&name)?;
	let mut conn = db.acquire().await?;
	let notes = episode_note_repo::notes(&mut conn, subject.as_str(), feed.as_str()).await?;
	Ok(Json(Notes { notes }))
}

/// `PUT /feeds/:feed/notes/:id`
#[derive(Debug, Serialize)]
pub struct NoteKept {
	/// `added` or `unchanged` — `episode_note_repo::Change`.
	pub change: &'static str,
	/// The note as home holds it, which the phone compares with what it sent
	/// before it counts it as home's.
	pub note: episode_note_repo::Note,
}

fn refusal(err: episode_note_repo::NoteError) -> FileHostError {
	match err {
		episode_note_repo::NoteError::Invalid(problems) => FileHostError::unprocessable_entity(problems),
		episode_note_repo::NoteError::Rewritten => FileHostError::Conflict(episode_note_repo::REWRITTEN),
		episode_note_repo::NoteError::Storage(err) => FileHostError::Sqlite(err),
	}
}

/// `PUT /feeds/:feed/notes/:id`, the request body being the note itself, in
/// the phone's shape, with `take` (a voice note id, or `null`) when it was
/// said.
///
/// Kept in `episode_note`, and, when it is new, filed in the same
/// `BEGIN IMMEDIATE` transaction for the mailbox's `feedback` folder, so a
/// note home holds is always on its way to the routine and none is filed
/// twice. The same note again changes nothing.
///
/// # Errors
/// 401 without a session, 403 from an untrusted origin, 404 for a feed home
/// does not render, 422 naming each refused field (a category outside the
/// set or out of its order, a moment or a time that is not one), 409 for a
/// different note under a held id, and 500 for a storage failure.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "episode_notes_put", skip_all, fields(otel.kind = "server"))]
pub async fn put_note(subject: SubjectId, State(db): State<SqlitePool>, Path((name, id)): Path<(String, String)>, body: Bytes) -> Result<Json<NoteKept>, FileHostError> {
	let feed = feed(&name)?;
	let now = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
	let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
	let (change, note) = episode_note_repo::put_note(&mut tx, subject.as_str(), feed.as_str(), &id, &body, &now)
		.await
		.map_err(refusal)?;
	if change == episode_note_repo::Change::Added {
		let (name, bytes) = crate::mailbox::pass::feedback_file(&note);
		let sha256 = hex::encode(Sha256::digest(&bytes));
		let filed = mailbox_repo::file_outgoing(&mut tx, subject.as_str(), mailbox_repo::FEEDBACK, &name, &sha256, &bytes, &now)
			.await
			.map_err(|err| match err {
				mailbox_repo::MailboxError::Rewritten => FileHostError::Conflict("the mailbox's copy of this note already holds another"),
				mailbox_repo::MailboxError::Storage(err) => FileHostError::Sqlite(err),
			})?;
		if filed == mailbox_repo::Change::NoAccount {
			// `SubjectId` found the account, so this is not reachable; if it
			// were, the transaction is dropped and nothing is written.
			return Err(FileHostError::OperationError("the account is gone".to_owned()));
		}
	}
	tx.commit().await?;
	Ok(Json(NoteKept { change: change.as_str(), note }))
}

#[cfg(test)]
mod tests {
	use crate::auth::{
		cookie::{Carrier, SessionToken},
		now, AuthContext,
	};
	use axum::{
		body::Body,
		extract::FromRef,
		http::{header::AUTHORIZATION, Method, Request, StatusCode},
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

	async fn session(state: &TestState, subject: &str) -> String {
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
		String::from("Bearer ") + &token.encoded()
	}

	async fn send(state: TestState, auth: Option<&str>, method: Method, uri: &str, body: Option<&Value>) -> (StatusCode, Value) {
		let app: Router = crate::routes::episode_notes::episode_notes().into_table_router().with_state(state);
		let mut request = Request::builder().method(method).uri(uri).header(axum::http::header::CONTENT_TYPE, "application/json");
		if let Some(auth) = auth {
			request = request.header(AUTHORIZATION, auth);
		}
		let body = body.map_or_else(Body::empty, |body| Body::from(body.to_string()));
		let response = app.oneshot(request.body(body).unwrap()).await.unwrap();
		let status = response.status();
		let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
		(status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
	}

	async fn put(state: &TestState, auth: Option<&str>, feed: &str, note: &Value) -> (StatusCode, Value) {
		let uri = String::from("/feeds/") + feed + "/notes/" + note["id"].as_str().unwrap();
		send(state.clone(), auth, Method::PUT, &uri, Some(note)).await
	}

	async fn get(state: &TestState, auth: Option<&str>, feed: &str) -> (StatusCode, Value) {
		send(state.clone(), auth, Method::GET, &(String::from("/feeds/") + feed + "/notes"), None).await
	}

	fn note(id: &str, categories: &[&str]) -> Value {
		json!({ "id": id, "feed": "call", "episode": "2026-10-30", "atMs": 61_000, "categories": categories, "writtenAt": "2026-11-08T08:15:00.000Z" })
	}

	/// docs/identity.md invariant 38: kept once, answered as the phone keeps
	/// it, filed once for the mailbox, and never replaced.
	#[tokio::test]
	async fn a_note_is_kept_once_and_filed_once_for_the_mailbox() {
		let state = state().await;
		let phone = session(&state, A).await;
		let mut sent = note("n1", &["shallow", "voice"]);
		sent["take"] = json!("take-7");
		assert_eq!(put(&state, Some(&phone), "call", &sent).await, (StatusCode::OK, json!({ "change": "added", "note": sent })));
		assert_eq!(
			put(&state, Some(&phone), "call", &sent).await,
			(StatusCode::OK, json!({ "change": "unchanged", "note": sent }))
		);
		let (status, _) = put(&state, Some(&phone), "call", &note("n1", &["shallow"])).await;
		assert_eq!(status, StatusCode::CONFLICT, "a different note under a held id");

		let mut conn = state.pool.acquire().await.unwrap();
		let pending = mailbox_repo::pending_outgoing(&mut conn, A, 10).await.unwrap();
		assert_eq!(pending.len(), 1, "filed once");
		assert_eq!((pending[0].folder.as_str(), pending[0].name.as_str()), ("feedback", "feedback-n1.json"));
		let file: Value = serde_json::from_slice(&pending[0].body).unwrap();
		assert_eq!(
			file,
			json!({
				"schema": "some-ui.episode-note/1",
				"note": "n1",
				"feed": "call",
				"episode": "2026-10-30",
				"atMs": 61_000,
				"categories": ["shallow", "voice"],
				"take": "take-7",
				"writtenAt": "2026-11-08T08:15:00.000Z",
			})
		);
		assert!(mailbox_repo::pending_outgoing(&mut conn, B, 10).await.unwrap().is_empty(), "only the caller's");
	}

	#[tokio::test]
	async fn notes_answer_the_callers_own_on_one_feed_oldest_first() {
		let state = state().await;
		let (phone, other) = (session(&state, A).await, session(&state, B).await);
		let mut late = note("late", &["tooLong"]);
		late["writtenAt"] = json!("2026-11-08T09:00:00.000Z");
		assert_eq!(put(&state, Some(&phone), "call", &late).await.0, StatusCode::OK);
		assert_eq!(put(&state, Some(&phone), "call", &note("early", &["shallow"])).await.0, StatusCode::OK);
		let mut pd = note("pd", &["overlap"]);
		pd["feed"] = json!("playoff-delta");
		pd["episode"] = json!("2026-SF-08");
		assert_eq!(put(&state, Some(&phone), "playoff-delta", &pd).await.0, StatusCode::OK);
		assert_eq!(put(&state, Some(&other), "call", &note("theirs", &["pacing"])).await.0, StatusCode::OK);

		let (status, body) = get(&state, Some(&phone), "call").await;
		assert_eq!(status, StatusCode::OK);
		let notes = body["notes"].as_array().unwrap();
		assert_eq!(notes.iter().map(|note| note["id"].as_str().unwrap()).collect::<Vec<_>>(), ["early", "late"]);
		let mut keys: Vec<&str> = notes[0].as_object().unwrap().keys().map(String::as_str).collect();
		keys.sort_unstable();
		assert_eq!(keys, ["atMs", "categories", "episode", "id", "storedAt", "take", "writtenAt"]);
		assert_eq!(notes[0]["take"], Value::Null);
		let (_, pd) = get(&state, Some(&phone), "playoff-delta").await;
		assert_eq!(pd["notes"][0]["id"], "pd");
	}

	#[tokio::test]
	async fn a_refused_note_writes_and_files_nothing() {
		let state = state().await;
		let phone = session(&state, A).await;
		assert_eq!(put(&state, None, "call", &note("n1", &["shallow"])).await.0, StatusCode::UNAUTHORIZED);
		assert_eq!(get(&state, None, "call").await.0, StatusCode::UNAUTHORIZED);
		assert_eq!(put(&state, Some(&phone), "podcast", &note("n1", &["shallow"])).await.0, StatusCode::NOT_FOUND);
		assert_eq!(get(&state, Some(&phone), "podcast").await.0, StatusCode::NOT_FOUND);
		let mut early = note("n3", &["shallow"]);
		early["atMs"] = json!(-5);
		let mut no_time = note("n4", &["shallow"]);
		no_time["writtenAt"] = json!("yesterday");
		for bad in [note("n1", &["voice", "shallow"]), note("n2", &["boring"]), early, no_time] {
			let (status, body) = put(&state, Some(&phone), "call", &bad).await;
			assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{bad}: {body}");
		}
		assert_eq!(
			put(&state, Some(&phone), "playoff-delta", &note("n5", &["shallow"])).await.0,
			StatusCode::UNPROCESSABLE_ENTITY,
			"the feed the path names"
		);
		let mut conn = state.pool.acquire().await.unwrap();
		assert!(mailbox_repo::pending_outgoing(&mut conn, A, 10).await.unwrap().is_empty());
		drop(conn);
		let held: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM episode_note").fetch_one(&state.pool).await.unwrap();
		assert_eq!(held, 0);
	}
}

//! Handlers for `routes::routine`: the files the reflections routine left for
//! the phone through the Jobs mailbox, as home keeps them (`mailbox_item`,
//! docs/identity.md invariant 19).
//!
//! **Read only.** The `mailbox` poller is the files' one writer; nothing here
//! writes, and the phone keeps its own record of which it opened.

use crate::subject::SubjectId;
use crate::FileHostError;
use axum::{extract::State, Json};
use serde::Serialize;
use serde_json::value::RawValue;
use sqlx::SqlitePool;
use tracing::instrument;

/// The most files one answer holds: what one look at the phone's "From the
/// routine" list can show.
pub const FILES_PER_ANSWER: i64 = 20;

/// `GET /routine/files`
#[derive(Debug, Serialize)]
pub struct Files {
	/// Newest first.
	pub files: Vec<HeldFile>,
}

#[derive(Debug, Serialize)]
pub struct HeldFile {
	/// `scene` (a word set pair) or `drama`.
	pub folder: String,
	/// Its name in the mailbox, which the phone remembers once opened.
	pub name: String,
	/// The file as the mailbox held it, verbatim.
	pub file: Box<RawValue>,
}

/// `GET /routine/files`: the reflections routine's newest word sets and
/// dramas home holds for the caller, newest first, at most
/// [`FILES_PER_ANSWER`].
///
/// # Errors
/// 401 without a session, and 500 for a storage failure or a held file that
/// does not read back.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "routine_files", skip_all, fields(otel.kind = "server"))]
pub async fn get_files(subject: SubjectId, State(db): State<SqlitePool>) -> Result<Json<Files>, FileHostError> {
	let mut conn = db.acquire().await?;
	let held = mailbox_repo::routine_files(&mut conn, subject.as_str(), FILES_PER_ANSWER).await.map_err(|err| match err {
		mailbox_repo::MailboxError::Storage(err) => FileHostError::Sqlite(err),
		mailbox_repo::MailboxError::Rewritten => FileHostError::OperationError("a read cannot rewrite".to_owned()),
	})?;
	let files = held
		.into_iter()
		.map(|held| {
			let text = String::from_utf8(held.body).map_err(|_| ())?;
			let file = RawValue::from_string(text).map_err(|_| ())?;
			Ok(HeldFile {
				folder: held.folder,
				name: held.name,
				file,
			})
		})
		.collect::<Result<Vec<_>, ()>>()
		.map_err(|()| FileHostError::OperationError("a held routine file does not read back".to_owned()))?;
	Ok(Json(Files { files }))
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

	async fn hold(state: &TestState, subject: &str, folder: &str, seq: i64, file: &Value) {
		let name = String::from(folder) + "-" + &seq.to_string() + ".json";
		sqlx::query("INSERT INTO mailbox_item (subject_id, folder, name, seq, sha256, size, body, pulled_at) VALUES (?1, ?2, ?3, ?4, 's', 0, ?5, 'now')")
			.bind(subject)
			.bind(folder)
			.bind(name)
			.bind(seq)
			.bind(file.to_string().into_bytes())
			.execute(&state.pool)
			.await
			.unwrap();
	}

	async fn get(state: TestState, auth: Option<&str>) -> (StatusCode, Value) {
		let app: Router = crate::routes::routine::routine().into_table_router().with_state(state);
		let mut request = Request::builder().method(Method::GET).uri("/routine/files");
		if let Some(auth) = auth {
			request = request.header(AUTHORIZATION, auth);
		}
		let response = app.oneshot(request.body(Body::empty()).unwrap()).await.unwrap();
		let status = response.status();
		let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
		(status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
	}

	#[tokio::test]
	async fn files_need_a_session() {
		assert_eq!(get(state().await, None).await.0, StatusCode::UNAUTHORIZED);
	}

	/// Only the routine's word sets and dramas, only the caller's, newest
	/// first, each verbatim.
	#[tokio::test]
	async fn files_answer_the_callers_own_scenes_and_dramas_newest_first() {
		let state = state().await;
		let phone = session(&state, A).await;
		let scene = json!({ "schema": "some-ui.routine-scene/1", "decks": [] });
		let drama = json!({ "schema": "some-ui.routine-drama/1", "tree": {} });
		hold(&state, A, "scene", 3, &scene).await;
		hold(&state, A, "drama", 5, &drama).await;
		hold(&state, A, "brief", 9, &json!({ "schema": "some-ui.job-digest/1" })).await;
		hold(&state, B, "scene", 7, &scene).await;
		let (status, answer) = get(state, Some(&phone)).await;
		assert_eq!(status, StatusCode::OK);
		assert_eq!(
			answer,
			json!({ "files": [
				{ "folder": "drama", "name": "drama-5.json", "file": drama },
				{ "folder": "scene", "name": "scene-3.json", "file": scene },
			] })
		);
	}

	#[tokio::test]
	async fn files_answer_at_most_twenty() {
		let state = state().await;
		let phone = session(&state, A).await;
		for seq in 1..=25 {
			hold(&state, A, "scene", seq, &json!({ "seq": seq })).await;
		}
		let (_, answer) = get(state, Some(&phone)).await;
		let files = answer["files"].as_array().unwrap();
		assert_eq!(files.len(), 20);
		assert_eq!((files[0]["file"]["seq"].as_i64(), files[19]["file"]["seq"].as_i64()), (Some(25), Some(6)));
	}
}

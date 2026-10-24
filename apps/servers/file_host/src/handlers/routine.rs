//! Handlers for `routes::routine`: what the reflections routine left for the
//! phone through the Jobs mailbox.
//!
//! Its files and the record of each of its runs, as home keeps them
//! (`mailbox_item`, docs/identity.md invariant 19).
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

/// The most run records one answer holds: a month of daily runs, so a phone
/// away from home that long still learns of every one.
pub const RUNS_PER_ANSWER: i64 = 31;

/// `GET /routine/files`
#[derive(Debug, Serialize)]
pub struct Files {
	/// Newest first.
	pub files: Vec<HeldFile>,
	/// What each of the routine's runs found and added, newest first.
	pub runs: Vec<HeldFile>,
	/// When home answered (ISO-8601 UTC): the phone keeps the runs it saw,
	/// and tells a routine that went quiet from a look made before it was due.
	pub at: String,
}

#[derive(Debug, Serialize)]
pub struct HeldFile {
	/// `scene` (a word set pair), `drama` or `run` (one run's record).
	pub folder: String,
	/// Its name in the mailbox, which the phone remembers once opened.
	pub name: String,
	/// The file as the mailbox held it, verbatim.
	pub file: Box<RawValue>,
}

/// `GET /routine/files`: the reflections routine's newest files and runs.
///
/// Its word sets and dramas home holds for the caller, newest first, at
/// most [`FILES_PER_ANSWER`], and its run records, at most
/// [`RUNS_PER_ANSWER`].
///
/// A held file that does not read back as JSON is left out and logged, so
/// one bad file never hides the rest.
///
/// # Errors
/// 401 without a session, and 500 for a storage failure.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "routine_files", skip_all, fields(otel.kind = "server"))]
pub async fn get_files(subject: SubjectId, State(db): State<SqlitePool>) -> Result<Json<Files>, FileHostError> {
	let mut conn = db.acquire().await?;
	let files = mailbox_repo::routine_files(&mut conn, subject.as_str(), FILES_PER_ANSWER).await.map_err(storage)?;
	let runs = mailbox_repo::routine_runs(&mut conn, subject.as_str(), RUNS_PER_ANSWER).await.map_err(storage)?;
	Ok(Json(Files {
		files: readable(files),
		runs: readable(runs),
		at: crate::mailbox::pass::now(),
	}))
}

fn storage(err: mailbox_repo::MailboxError) -> FileHostError {
	match err {
		mailbox_repo::MailboxError::Storage(err) => FileHostError::Sqlite(err),
		mailbox_repo::MailboxError::Rewritten => FileHostError::OperationError("a read cannot rewrite".to_owned()),
	}
}

/// The held files that read back as JSON, in order.
fn readable(held: Vec<mailbox_repo::RoutineFile>) -> Vec<HeldFile> {
	held
		.into_iter()
		.filter_map(|held| {
			// The poller kept only JSON (invariant 19), so a body that fails
			// here was written some other way: say so, by kind, never by body.
			let file = String::from_utf8(held.body)
				.map_err(|_| "utf8")
				.and_then(|text| RawValue::from_string(text).map_err(|_| "json"));
			match file {
				Ok(file) => Some(HeldFile {
					folder: held.folder,
					name: held.name,
					file,
				}),
				Err(kind) => {
					tracing::error!(folder = %held.folder, kind, "a held routine file does not read back; left out");
					None
				}
			}
		})
		.collect()
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
	/// first, each verbatim; its run records apart, the same way.
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
		let run = json!({ "schema": "some-ui.routine-run/1", "added": [] });
		hold(&state, A, "run", 4, &run).await;
		hold(&state, A, "run", 6, &run).await;
		hold(&state, B, "run", 8, &run).await;
		let (status, mut answer) = get(state, Some(&phone)).await;
		assert_eq!(status, StatusCode::OK);
		let at = answer.as_object_mut().unwrap().remove("at").unwrap();
		assert!(chrono::DateTime::parse_from_rfc3339(at.as_str().unwrap()).is_ok(), "{at}");
		assert_eq!(
			answer,
			json!({
				"files": [
					{ "folder": "drama", "name": "drama-5.json", "file": drama },
					{ "folder": "scene", "name": "scene-3.json", "file": scene },
				],
				"runs": [
					{ "folder": "run", "name": "run-6.json", "file": run },
					{ "folder": "run", "name": "run-4.json", "file": run },
				],
			})
		);
	}

	#[tokio::test]
	async fn a_file_that_does_not_read_back_is_left_out() {
		let state = state().await;
		let phone = session(&state, A).await;
		hold(&state, A, "scene", 1, &json!({ "ok": 1 })).await;
		for (seq, body) in [(2, &b"\xff"[..]), (3, &b"not json"[..])] {
			sqlx::query("INSERT INTO mailbox_item (subject_id, folder, name, seq, sha256, size, body, pulled_at) VALUES (?1, 'drama', ?2, ?3, 's', 1, ?4, 'now')")
				.bind(A)
				.bind(String::from("bad-") + &seq.to_string() + ".json")
				.bind(seq)
				.bind(body)
				.execute(&state.pool)
				.await
				.unwrap();
		}
		let (status, answer) = get(state, Some(&phone)).await;
		assert_eq!(
			(status, &answer["files"][0]["name"], answer["files"].as_array().unwrap().len()),
			(StatusCode::OK, &json!("scene-1.json"), 1)
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

	#[tokio::test]
	async fn runs_answer_at_most_a_month_apart_from_files() {
		let state = state().await;
		let phone = session(&state, A).await;
		hold(&state, A, "scene", 1, &json!({ "seq": 1 })).await;
		for seq in 2..=40 {
			hold(&state, A, "run", seq, &json!({ "seq": seq })).await;
		}
		let (_, answer) = get(state, Some(&phone)).await;
		let runs = answer["runs"].as_array().unwrap();
		assert_eq!(runs.len(), 31);
		assert_eq!((runs[0]["file"]["seq"].as_i64(), runs[30]["file"]["seq"].as_i64()), (Some(40), Some(10)));
		assert_eq!(answer["files"].as_array().unwrap().len(), 1, "runs take no file's place");
	}
}

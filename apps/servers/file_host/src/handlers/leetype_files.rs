//! Handlers for `routes::leetype_files`: the `LeetType` session files the
//! routine left for the phone through the Jobs mailbox (paulgsc/some-ui#2026).
//!
//! The files home keeps from the mailbox's `leetype` folder (`mailbox_item`,
//! docs/identity.md invariant 19), each `some-ui.leetype-session/1`: the
//! session file the phone plays, wrapped with its schema.
//!
//! **Read only.** The `mailbox` poller is the files' one writer; nothing here
//! writes. The phone never uploads a session file: they come down from the
//! routine.
//!
//! **One file at a time.** The list names each file with its title, one line
//! and hash; the phone fetches the file it plays by name, with the hash as
//! its `ETag`, so a file it already holds costs a `304`.

use crate::handlers::db::activities::{etag_value, if_none_match_hits, not_modified};
use crate::subject::SubjectId;
use crate::FileHostError;
use axum::{
	extract::{Path, State},
	http::{
		header::{CONTENT_TYPE, ETAG},
		HeaderMap, HeaderValue,
	},
	response::{IntoResponse, Response},
	Json,
};
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use sqlx::SqlitePool;
use tracing::instrument;

/// The most files one list holds, newest first: far more than a study
/// thread's sessions.
pub const FILES_LISTED: i64 = 200;

/// What a file in the `leetype` folder is (paulgsc/some-ui `apps/mailbox`).
pub const SESSION_SCHEMA: &str = "some-ui.leetype-session/1";

/// `GET /leetype/session-files`
#[derive(Debug, Serialize)]
pub struct SessionFiles {
	/// Newest first.
	pub files: Vec<Listed>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Listed {
	/// Its name in the mailbox, which the phone fetches it by.
	pub name: String,
	/// The session file's own `title` and `oneLine`.
	pub title: String,
	pub one_line: String,
	/// Lowercase hex SHA-256 of the file as the mailbox held it: the `ETag`
	/// `GET /leetype/session-files/:name` answers with.
	pub sha256: String,
	/// When home read it from the mailbox.
	pub kept_at: String,
}

/// A held file: its schema and the session file inside, verbatim.
#[derive(Deserialize)]
struct Envelope<'a> {
	schema: String,
	#[serde(borrow)]
	file: &'a RawValue,
}

/// What the list shows of a session file.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Summary {
	title: String,
	one_line: String,
}

/// The session file inside `body` and its summary, or why it is not one, by
/// kind: never by body.
fn session_file(body: &[u8]) -> Result<(&RawValue, Summary), &'static str> {
	let text = std::str::from_utf8(body).map_err(|_| "utf8")?;
	let envelope: Envelope<'_> = serde_json::from_str(text).map_err(|_| "json")?;
	if envelope.schema != SESSION_SCHEMA {
		return Err("schema");
	}
	let summary: Summary = serde_json::from_str(envelope.file.get()).map_err(|_| "summary")?;
	if summary.title.trim().is_empty() || summary.one_line.trim().is_empty() {
		return Err("summary");
	}
	Ok((envelope.file, summary))
}

fn storage(err: mailbox_repo::MailboxError) -> FileHostError {
	match err {
		mailbox_repo::MailboxError::Storage(err) => FileHostError::Sqlite(err),
		mailbox_repo::MailboxError::Rewritten => FileHostError::OperationError("a read cannot rewrite".to_owned()),
	}
}

/// `GET /leetype/session-files`: the session files home holds for the
/// caller, newest first, at most [`FILES_LISTED`].
///
/// A held file that is not a session file (not JSON, another schema, or no
/// title or one line) is left out and logged, so one bad file never hides
/// the rest.
///
/// # Errors
/// 401 without a session, and 500 for a storage failure.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "leetype_session_files", skip_all, fields(otel.kind = "server"))]
pub async fn get_files(subject: SubjectId, State(db): State<SqlitePool>) -> Result<Json<SessionFiles>, FileHostError> {
	let mut conn = db.acquire().await?;
	let held = mailbox_repo::leetype_files(&mut conn, subject.as_str(), FILES_LISTED).await.map_err(storage)?;
	let files = held
		.into_iter()
		.filter_map(|held| match session_file(&held.body) {
			Ok((_, summary)) => Some(Listed {
				name: held.name,
				title: summary.title,
				one_line: summary.one_line,
				sha256: held.sha256,
				kept_at: held.pulled_at,
			}),
			Err(kind) => {
				// The poller kept only JSON (invariant 19); what the routine
				// put inside is the routine's. Said by kind, never by body.
				tracing::error!(folder = mailbox_repo::LEETYPE, kind, "a held LeetType session file does not read back; left out");
				None
			}
		})
		.collect();
	Ok(Json(SessionFiles { files }))
}

/// `GET /leetype/session-files/:name`: the session file itself, as the
/// routine wrote it, with the held file's SHA-256 as its `ETag`; `304` when
/// `If-None-Match` names it.
///
/// # Errors
/// 401 without a session, 404 for a name home holds no session file under
/// (or one that does not read back as one, which is logged), and 500 for a
/// storage failure.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "leetype_session_file", skip_all, fields(otel.kind = "server"))]
pub async fn get_file(subject: SubjectId, State(db): State<SqlitePool>, Path(name): Path<String>, headers: HeaderMap) -> Result<Response, FileHostError> {
	let mut conn = db.acquire().await?;
	let held = mailbox_repo::leetype_file(&mut conn, subject.as_str(), &name)
		.await
		.map_err(storage)?
		.ok_or(FileHostError::NotFound)?;
	let etag = etag_value(&held.sha256)?;
	if if_none_match_hits(&headers, &etag) {
		return Ok(not_modified(etag));
	}
	let file = match session_file(&held.body) {
		Ok((file, _)) => file.get().to_owned(),
		Err(kind) => {
			tracing::error!(folder = mailbox_repo::LEETYPE, kind, "a held LeetType session file does not read back; not served");
			return Err(FileHostError::NotFound);
		}
	};
	let mut response = file.into_response();
	response.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
	response.headers_mut().insert(ETAG, etag);
	Ok(response)
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
		http::{
			header::{AUTHORIZATION, CONTENT_TYPE, ETAG, IF_NONE_MATCH},
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

	/// The session file a routine writes, as the phone plays it. Spaced and
	/// ordered as written, so a test can tell it is served verbatim.
	fn played(title: &str) -> String {
		String::from(r#"{ "title": ""#)
			+ title
			+ r#"", "oneLine": "Why a leader steps down.", "session": { "kind": "incident", "budgetMinutes": 25 }, "start": "a", "screens": {} }"#
	}

	async fn hold(state: &TestState, subject: &str, folder: &str, (seq, name): (i64, &str), body: &[u8]) {
		sqlx::query("INSERT INTO mailbox_item (subject_id, folder, name, seq, sha256, size, body, pulled_at) VALUES (?1, ?2, ?3, ?4, ?5, 0, ?6, '2026-10-30T09:00:00.000Z')")
			.bind(subject)
			.bind(folder)
			.bind(name)
			.bind(seq)
			.bind(String::from("sha-") + name)
			.bind(body)
			.execute(&state.pool)
			.await
			.unwrap();
	}

	async fn hold_file(state: &TestState, subject: &str, (seq, name): (i64, &str), title: &str) {
		let body = String::from(r#"{"schema":"some-ui.leetype-session/1","file":"#) + &played(title) + "}";
		hold(state, subject, "leetype", (seq, name), body.as_bytes()).await;
	}

	async fn get(state: TestState, uri: &str, headers: &[(axum::http::HeaderName, &str)]) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
		let app: Router = crate::routes::leetype_files::leetype_files().into_table_router().with_state(state);
		let mut request = Request::builder().method(Method::GET).uri(uri);
		for (name, value) in headers {
			request = request.header(name, *value);
		}
		let response = app.oneshot(request.body(Body::empty()).unwrap()).await.unwrap();
		let (status, headers) = (response.status(), response.headers().clone());
		let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
		(status, headers, bytes.to_vec())
	}

	#[tokio::test]
	async fn both_need_a_session() {
		let state = state().await;
		assert_eq!(get(state.clone(), "/leetype/session-files", &[]).await.0, StatusCode::UNAUTHORIZED);
		assert_eq!(get(state, "/leetype/session-files/t1-01-raft.json", &[]).await.0, StatusCode::UNAUTHORIZED);
	}

	/// Only the `leetype` folder, only the caller's, newest first, each by its
	/// own title and one line; one that is not a session file is left out.
	#[tokio::test]
	async fn the_list_names_the_callers_own_session_files_newest_first() {
		let state = state().await;
		let phone = session(&state, A).await;
		hold_file(&state, A, (3, "t1-01-raft.json"), "Raft").await;
		hold_file(&state, A, (5, "t1-02-paxos.json"), "Paxos").await;
		hold_file(&state, B, (7, "t1-03-zab.json"), "Zab").await;
		hold(&state, A, "scene", (8, "scene-1.json"), br#"{"schema":"some-ui.routine-scene/1"}"#).await;
		hold(&state, A, "leetype", (9, "t1-04-other.json"), br#"{"schema":"some-ui.routine-drama/1","file":{}}"#).await;
		hold(
			&state,
			A,
			"leetype",
			(10, "t1-05-bare.json"),
			br#"{"schema":"some-ui.leetype-session/1","file":{"title":"","oneLine":"x"}}"#,
		)
		.await;
		hold(&state, A, "leetype", (11, "t1-06-bytes.json"), b"\xff").await;
		let (status, _, body) = get(state, "/leetype/session-files", &[(AUTHORIZATION, &phone)]).await;
		assert_eq!(status, StatusCode::OK);
		assert_eq!(
			serde_json::from_slice::<Value>(&body).unwrap(),
			json!({ "files": [
				{ "name": "t1-02-paxos.json", "title": "Paxos", "oneLine": "Why a leader steps down.", "sha256": "sha-t1-02-paxos.json", "keptAt": "2026-10-30T09:00:00.000Z" },
				{ "name": "t1-01-raft.json", "title": "Raft", "oneLine": "Why a leader steps down.", "sha256": "sha-t1-01-raft.json", "keptAt": "2026-10-30T09:00:00.000Z" },
			] })
		);
	}

	/// The session file itself, byte for byte as the routine wrote it inside
	/// the held file, with the held file's hash as its `ETag`.
	#[tokio::test]
	async fn one_file_is_served_verbatim_with_its_hash_as_etag_and_304_when_held() {
		let state = state().await;
		let phone = session(&state, A).await;
		hold_file(&state, A, (3, "t1-01-raft.json"), "Raft").await;
		hold_file(&state, B, (4, "t1-02-paxos.json"), "Paxos").await;
		let uri = "/leetype/session-files/t1-01-raft.json";
		let (status, headers, body) = get(state.clone(), uri, &[(AUTHORIZATION, &phone)]).await;
		assert_eq!(status, StatusCode::OK);
		assert_eq!(String::from_utf8(body).unwrap(), played("Raft"), "verbatim");
		assert_eq!(headers[ETAG], "\"sha-t1-01-raft.json\"");
		assert_eq!(headers[CONTENT_TYPE], "application/json");

		let (status, headers, body) = get(state.clone(), uri, &[(AUTHORIZATION, &phone), (IF_NONE_MATCH, "\"sha-t1-01-raft.json\"")]).await;
		assert_eq!((status, body.is_empty()), (StatusCode::NOT_MODIFIED, true));
		assert_eq!(headers[ETAG], "\"sha-t1-01-raft.json\"");
		let (status, ..) = get(state.clone(), uri, &[(AUTHORIZATION, &phone), (IF_NONE_MATCH, "\"an-older-one\"")]).await;
		assert_eq!(status, StatusCode::OK, "another hash is a miss");

		for missing in ["/leetype/session-files/t1-02-paxos.json", "/leetype/session-files/t9-99-never.json"] {
			assert_eq!(
				get(state.clone(), missing, &[(AUTHORIZATION, &phone)]).await.0,
				StatusCode::NOT_FOUND,
				"{missing}: B's, or none"
			);
		}
		hold(&state, A, "leetype", (5, "t1-03-broken.json"), b"{\"schema\":\"some-ui.leetype-session/1\"}").await;
		assert_eq!(
			get(state, "/leetype/session-files/t1-03-broken.json", &[(AUTHORIZATION, &phone)]).await.0,
			StatusCode::NOT_FOUND,
			"a held file that is not a session file is not served"
		);
	}
}

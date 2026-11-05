//! Handlers for `routes::studio_videos`: the validate-and-answer half. The
//! record's own rules (append-only, what a DNA holds, which takes it may
//! name) are `studio_video_repo`'s.
//!
//! **Refusals.** A DNA that breaks one of those rules is a `422` naming each
//! field and why. A DNA whose takes name a voice note that is not the
//! subject's own `studio` note on home, or one the owner set aside, is a
//! `422` naming `takes`, with each such note by id under `details.notes`, so
//! the phone can say which take has not reached home, send it, and queue
//! again. Other bytes under a queued revision are a `409`. Every refusal
//! writes nothing.
//!
//! **The deletion hold.** Each handler takes a [`SubjectId`] first and keeps
//! it until it returns, so a write holds off `DELETE /auth/account` as every
//! other subject-scoped write does (docs/identity.md, invariant 9).
//!
//! **Nothing the DNA says reaches a log.** A title, a shape and a video's
//! data are the owner's own. Every handler here is `#[instrument(skip_all)]`
//! and records only `otel.kind` (invariant 6), and refusals carry fixed words
//! and note ids, never a value sent; `nothing_a_dna_says_reaches_a_log_line`
//! checks it.
//!
//! **No other writer, and never filed.** [`put_queued`] is the one caller of
//! `studio_video_repo::queue`, and nothing reads the record for a routine or
//! the mailbox in this slice (invariant 36).

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
use std::borrow::Cow;
use studio_video_repo::{DnaError, Queued, Videos, REWRITTEN, TAKES_NOT_HELD};
use tracing::instrument;

/// `PUT /studio/videos/:video/queued/:rev`
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct QueuedWritten {
	/// `added` or `unchanged` — `studio_video_repo::Change`.
	pub change: &'static str,
	/// The video as home now lists it, its stage included.
	pub video: Queued,
}

fn now() -> String {
	// Fixed width, as `handlers::jobs`'s.
	Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn refusal(err: DnaError) -> FileHostError {
	match err {
		DnaError::Invalid(problems) => FileHostError::unprocessable_entity(problems),
		DnaError::TakesNotHeld(notes) => FileHostError::unprocessable_entity(
			std::iter::once((Cow::Borrowed("takes"), Cow::Borrowed(TAKES_NOT_HELD))).chain(notes.into_iter().map(|note| (Cow::Borrowed("notes"), Cow::Owned(note)))),
		),
		DnaError::Rewritten => FileHostError::Conflict(REWRITTEN),
		DnaError::Storage(err) => FileHostError::Sqlite(err),
	}
}

/// `GET /studio/videos`
///
/// # Errors
/// 401 without a session, and 500 for a storage failure.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "studio_videos_get", skip_all, fields(otel.kind = "server"))]
pub async fn get_videos(subject: SubjectId, State(db): State<SqlitePool>) -> Result<Json<Videos>, FileHostError> {
	let mut conn = db.acquire().await?;
	studio_video_repo::videos(&mut conn, subject.as_str()).await.map(Json).map_err(refusal)
}

/// `PUT /studio/videos/:video/queued/:rev`, the request body being the DNA
/// itself. `BEGIN IMMEDIATE`, so the reads that decide and the write see one
/// snapshot.
///
/// # Errors
/// 401 without a session, 403 from an untrusted origin, 422 naming each
/// refused field, 422 naming `takes` and each voice note home does not hold
/// as the subject's `studio` note or holds set aside, 409 for other bytes
/// under a queued revision, 413 for a body longer than one byte past
/// `DNA_CEILING` (refused before it is read whole, by the route's
/// `DefaultBodyLimit`), and 500 for a storage failure.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "studio_video_queued_put", skip_all, fields(otel.kind = "server"))]
pub async fn put_queued(
	subject: SubjectId,
	State(db): State<SqlitePool>,
	Path((video, rev)): Path<(String, String)>,
	body: Bytes,
) -> Result<Json<QueuedWritten>, FileHostError> {
	let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
	let (change, queued) = studio_video_repo::queue(&mut tx, subject.as_str(), &video, &rev, &body, &now()).await.map_err(refusal)?;
	tx.commit().await?;
	Ok(Json(QueuedWritten {
		change: change.as_str(),
		video: queued,
	}))
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
	use studio_video_repo::DNA_CEILING;
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
		crate::routes::studio_videos::studio_videos()
	}

	async fn state() -> TestState {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();
		TestState {
			auth: AuthContext::for_tests(pool.clone(), 100),
			pool,
		}
	}

	/// An account for `subject`, its `studio` notes `n1` and `n2` and a
	/// reflection note `r1`, and a live session carried as `carrier` carries it.
	async fn session(state: &TestState, subject: &str, carrier: Carrier) -> (axum::http::HeaderName, String) {
		sqlx::query("INSERT OR IGNORE INTO account (subject_id, user_handle) VALUES (?1, ?2)")
			.bind(subject)
			.bind(subject.as_bytes())
			.execute(&state.pool)
			.await
			.unwrap();
		for (note, kind) in [("n1", "studio"), ("n2", "studio"), ("r1", "reflection")] {
			sqlx::query(
				"INSERT OR IGNORE INTO voice_note (subject_id, note_id, of_kind, of_ref, recorded_at, content_type, size, sha256, stored_at)
				 VALUES (?1, ?2, ?3, 'p1:division:t1', 'then', 'audio/webm', 1, 's', 'now')",
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

	async fn call(app: &Router, method: Method, path: &str, auth: Option<&(axum::http::HeaderName, String)>, body: String) -> (StatusCode, Value) {
		let mut request = Request::builder().method(method).uri(path);
		if let Some((name, value)) = auth {
			request = request.header(name, value);
		}
		let response = app.clone().oneshot(request.body(Body::from(body)).unwrap()).await.unwrap();
		let status = response.status();
		let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
		(status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
	}

	fn dna(video: &str, rev: i64, queued_at: &str) -> Value {
		json!({
			"schema": "some-ui.video-dna/1",
			"video": video,
			"rev": rev,
			"title": "Playoff Delta · 2026 week 8",
			"queuedAt": queued_at,
			"template": { "kind": "show-format", "id": "playoff-delta", "version": 2 },
			"takes": [{ "scene": "division", "note": "n1" }, { "scene": "outro", "note": "n2" }],
			"shape": { "scenes": [{ "id": "division" }, { "id": "outro" }] },
			"data": { "pin": { "period": "2026-w8" } }
		})
	}

	fn path(video: &str, rev: i64) -> String {
		String::from("/studio/videos/") + video + "/queued/" + &rev.to_string()
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
					route.path.replace(":video", "v1").replace(":rev", "1"),
				)
			})
			.collect();
		assert_eq!(routes.len(), 2, "{routes:?}");
		for (method, path) in &routes {
			assert_eq!(call(&app, method.clone(), path, None, String::new()).await.0, StatusCode::UNAUTHORIZED, "{method} {path}");
		}
		let request = Request::builder()
			.method(Method::PUT)
			.uri(path("v1", 1))
			.header(&phone.0, &phone.1)
			.header("origin", "https://evil.app.test")
			.header("sec-fetch-site", "cross-site")
			.body(Body::from(dna("v1", 1, "2026-11-05T17:02:11.123Z").to_string()))
			.unwrap();
		assert_eq!(app.clone().oneshot(request).await.unwrap().status(), StatusCode::FORBIDDEN);
	}

	/// The phone's round: a DNA queued, queued again, read back in the list
	/// with its stage, and a later revision beside it. A browser session on
	/// the same account reads the same record.
	#[tokio::test]
	async fn a_paired_phone_queues_a_video_and_reads_where_it_stands() {
		let state = state().await;
		let phone = session(&state, A, Carrier::Bearer).await;
		let browser = session(&state, A, Carrier::Cookie).await;
		let app = app(state);

		assert_eq!(
			call(&app, Method::GET, "/studio/videos", Some(&phone), String::new()).await,
			(StatusCode::OK, json!({ "videos": [] }))
		);
		let first = json!({
			"video": "v1", "rev": 1, "title": "Playoff Delta · 2026 week 8",
			"queuedAt": "2026-11-05T17:02:11.123Z", "stage": "queued"
		});
		let sent = dna("v1", 1, "2026-11-05T17:02:11.123Z").to_string();
		let (status, written) = call(&app, Method::PUT, &path("v1", 1), Some(&phone), sent.clone()).await;
		assert_eq!((status, &written["change"]), (StatusCode::OK, &json!("added")), "{written}");
		let stored_at = written["video"]["storedAt"].clone();
		let mut held = first.clone();
		held["storedAt"] = stored_at;
		assert_eq!(written["video"], held);
		assert_eq!(
			call(&app, Method::PUT, &path("v1", 1), Some(&phone), sent).await,
			(StatusCode::OK, json!({ "change": "unchanged", "video": held.clone() }))
		);

		let (status, _) = call(&app, Method::PUT, &path("v1", 2), Some(&phone), dna("v1", 2, "2026-11-05T18:00:00.000Z").to_string()).await;
		assert_eq!(status, StatusCode::OK);
		let (status, listed) = call(&app, Method::GET, "/studio/videos", Some(&browser), String::new()).await;
		assert_eq!(status, StatusCode::OK);
		let entries: Vec<(Value, Value, Value)> = listed["videos"]
			.as_array()
			.unwrap()
			.iter()
			.map(|video| (video["video"].clone(), video["rev"].clone(), video["stage"].clone()))
			.collect();
		assert_eq!(
			entries,
			[(json!("v1"), json!(2), json!("queued")), (json!("v1"), json!(1), json!("queued"))],
			"newest queued first"
		);
		assert_eq!(listed["videos"][1], held);
	}

	/// A DNA that breaks a rule is a 422 naming each field, a take home does
	/// not hold a 422 naming `takes` and each note, other bytes under a
	/// queued revision a 409, a body past the ceiling a 422 or a 413, and one
	/// subject's record is never another's.
	#[tokio::test]
	async fn refusals_name_their_fields_and_records_stay_apart() {
		let state = state().await;
		let a = session(&state, A, Carrier::Bearer).await;
		let b = session(&state, B, Carrier::Bearer).await;
		let app = app(state);
		let sent = dna("v1", 1, "2026-11-05T17:02:11.123Z");
		assert_eq!(call(&app, Method::PUT, &path("v1", 1), Some(&a), sent.to_string()).await.0, StatusCode::OK);

		let named = |answer: (StatusCode, Value)| {
			assert_eq!(answer.0, StatusCode::UNPROCESSABLE_ENTITY, "{}", answer.1);
			let mut fields: Vec<String> = answer.1["error"]["details"].as_object().unwrap().keys().cloned().collect();
			fields.sort();
			fields
		};
		let mut bad = dna("v2", 1, "soon");
		bad["schema"] = json!("some-ui.video-dna/2");
		bad["title"] = json!("x".repeat(201));
		bad["shape"] = json!([]);
		assert_eq!(
			named(call(&app, Method::PUT, &path("v2", 1), Some(&a), bad.to_string()).await),
			["queuedAt", "schema", "shape", "title"]
		);
		assert_eq!(
			named(call(&app, Method::PUT, &path("other", 1), Some(&a), dna("v2", 1, "2026-11-05T17:02:11.123Z").to_string()).await),
			["video"]
		);
		assert_eq!(
			named(call(&app, Method::PUT, &path("v2", 2), Some(&a), dna("v2", 1, "2026-11-05T17:02:11.123Z").to_string()).await),
			["rev"]
		);
		assert_eq!(
			named(
				call(
					&app,
					Method::PUT,
					"/studio/videos/v2/queued/one",
					Some(&a),
					dna("v2", 1, "2026-11-05T17:02:11.123Z").to_string()
				)
				.await
			),
			["rev"]
		);
		assert_eq!(named(call(&app, Method::PUT, &path("v2", 1), Some(&a), String::from("\"dna\"")).await), ["body"]);
		let mut padded = dna("v2", 1, "2026-11-05T17:02:11.123Z").to_string();
		padded.push_str(&" ".repeat(DNA_CEILING + 1 - padded.len()));
		assert_eq!(
			named(call(&app, Method::PUT, &path("v2", 1), Some(&a), padded.clone()).await),
			["body"],
			"one byte past the ceiling"
		);
		padded.push(' ');
		assert_eq!(
			call(&app, Method::PUT, &path("v2", 1), Some(&a), padded).await.0,
			StatusCode::PAYLOAD_TOO_LARGE,
			"and longer, not read whole"
		);

		let mut unheld = dna("v2", 1, "2026-11-05T17:02:11.123Z");
		unheld["takes"] = json!([{ "scene": "division", "note": "r1" }, { "scene": "outro", "note": "never-sent" }, { "scene": "recap", "note": "n1" }]);
		let (status, answer) = call(&app, Method::PUT, &path("v2", 1), Some(&a), unheld.to_string()).await;
		assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{answer}");
		assert_eq!(answer["error"]["details"]["notes"], json!(["r1", "never-sent"]), "each named by id");
		assert_eq!(answer["error"]["details"]["takes"].as_array().unwrap().len(), 1);
		assert_eq!(
			call(&app, Method::PUT, &path("v1", 1), Some(&b), sent.to_string()).await.0,
			StatusCode::OK,
			"B's own takes are B's to name"
		);

		let mut rewritten = sent.clone();
		rewritten["title"] = json!("Another title");
		let (status, answer) = call(&app, Method::PUT, &path("v1", 1), Some(&a), rewritten.to_string()).await;
		assert_eq!(
			(status, &answer["error"]["code"]),
			(StatusCode::CONFLICT, &json!("conflict")),
			"a queued revision is one DNA"
		);

		for (who, expected) in [(&a, 1), (&b, 1)] {
			assert_eq!(
				call(&app, Method::GET, "/studio/videos", Some(who), String::new()).await.1["videos"]
					.as_array()
					.unwrap()
					.len(),
				expected,
				"each lists only its own, and nothing refused was written"
			);
		}
	}

	/// docs/identity.md invariants 6 and 36: a title, a shape and a video's
	/// data are the owner's own, and none reaches a log line or span field,
	/// refused or kept.
	#[tokio::test]
	async fn nothing_a_dna_says_reaches_a_log_line() {
		const SAID: &str = "a-title-only-i-wrote-zq9";
		let state = state().await;
		let phone = session(&state, A, Carrier::Bearer).await;
		let app = app(state);
		let (captured, _guard) = crate::privacy::capture_where(crate::auth::loggable);
		let mut said = dna("v1", 1, "2026-11-05T17:02:11.123Z");
		said["title"] = json!(SAID);
		said["shape"] = json!({ "line": SAID });
		said["data"] = json!({ "fact": SAID });
		assert_eq!(call(&app, Method::PUT, &path("v1", 1), Some(&phone), said.to_string()).await.0, StatusCode::OK);
		said["data"] = json!({ "fact": SAID, "more": SAID });
		assert_eq!(call(&app, Method::PUT, &path("v1", 1), Some(&phone), said.to_string()).await.0, StatusCode::CONFLICT);
		said["title"] = json!(SAID.repeat(10));
		said["takes"] = json!([{ "scene": "division", "note": "never-sent" }]);
		assert_eq!(
			call(&app, Method::PUT, &path("v1", 2), Some(&phone), said.to_string()).await.0,
			StatusCode::UNPROCESSABLE_ENTITY
		);
		said["title"] = json!(SAID);
		said["rev"] = json!(2);
		assert_eq!(
			call(&app, Method::PUT, &path("v1", 2), Some(&phone), said.to_string()).await.0,
			StatusCode::UNPROCESSABLE_ENTITY
		);
		assert_eq!(call(&app, Method::GET, "/studio/videos", Some(&phone), String::new()).await.0, StatusCode::OK);

		let lines = captured.lines();
		assert!(lines.iter().any(|line| line.contains("otel.kind")), "no handler span was captured: {lines:#?}");
		for line in &lines {
			assert!(!line.contains(SAID), "what a DNA says reached a log line: {line}");
		}
	}
}

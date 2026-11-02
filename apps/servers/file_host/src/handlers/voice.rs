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
//! writes `voice_note`, `voice_note_audio` or `game_bite` in the background.
//! [`put_set_aside`] is likewise the only caller of
//! `voice_repo::set_aside::set_aside`, the one writer of
//! `voice_note_set_aside`; it writes no note.
//! Neither the audio, nor its hash, nor a bite's strip or context line
//! reaches a log line: the span skips every argument, and a refusal names
//! fields with fixed words, never what was said.
//!
//! **The one way audio leaves home** (docs/identity.md invariant 23, the
//! owner's D4 on paulgsc/some-ui#2237): [`get_audio`] is the only caller of
//! `voice_repo::audio`, and serves a Rust Rounds note's audio, and no other
//! kind's, to a [`Paired`] phone of the subject that holds it. Neither the
//! audio nor its hash reaches a log line: its span skips every argument.
//!
//! **The deletion hold.** The handler takes a [`SubjectId`] first and keeps
//! it until it returns, so a write holds off `DELETE /auth/account` as every
//! other subject-scoped write does (docs/identity.md, invariant 9).

use crate::handlers::db::activities::{etag_value, if_none_match_hits, not_modified};
use crate::subject::{Paired, SubjectId};
use crate::FileHostError;
use axum::{
	body::Bytes,
	extract::{Path, Query, State},
	http::{
		header::{CONTENT_TYPE, ETAG},
		HeaderMap, StatusCode,
	},
	response::{IntoResponse, Response},
	Json,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tracing::instrument;
use voice_repo::{
	set_aside::SetAside,
	transcript::{Hearing, Transcript},
	Note, Of, Said, SaidBite, VoiceError, REWRITTEN,
};

/// `PUT /voice-notes/:id`'s query: what the phone says the note is.
#[derive(Debug, Deserialize)]
pub struct NoteQuery {
	of: Option<String>,
	#[serde(rename = "ref")]
	of_ref: Option<String>,
	at: Option<String>,
	/// A game bite's own fields, as `voice_repo::SaidBite` lists them: for
	/// `of=game` only.
	scenario: Option<String>,
	how: Option<String>,
	seq: Option<String>,
	strip: Option<String>,
	context: Option<String>,
	watching: Option<String>,
	started: Option<String>,
	offset: Option<String>,
	mode: Option<String>,
	catalog: Option<String>,
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
/// itself and its `Content-Type` its container.
///
/// A game bite (`of=game`, `ref` the game's id) also says its bite's fields
/// (`voice_repo::SaidBite`), stored beside the note in the same transaction.
/// A Studio take (`of=studio`, `ref` the take as `<project>:<shot>:<take>`)
/// says nothing more, as a reflection does; a redo is a new note. A take a
/// list of Drive's takes home already holds names is recorded as on Drive
/// in the same transaction (`voice_repo::on_drive::match_take`,
/// paulgsc/some-ui#2416).
///
/// `BEGIN IMMEDIATE`, so the read that decides and both writes see one
/// snapshot.
///
/// # Errors
/// 401 without a session, 403 from an untrusted origin, 422 naming each
/// refused field (`id`, `of`, `ref`, `at`, `contentType`, `body`, and a
/// bite's fields: missing or wrong for a game note, said at all for any
/// other), 409 for
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
		bite: SaidBite {
			scenario: query.scenario.as_deref(),
			how: query.how.as_deref(),
			seq: query.seq.as_deref(),
			strip: query.strip.as_deref(),
			context: query.context.as_deref(),
			watching: query.watching.as_deref(),
			started: query.started.as_deref(),
			offset: query.offset.as_deref(),
			mode: query.mode.as_deref(),
			catalog: query.catalog.as_deref(),
		},
	};
	let note = voice_repo::describe(&id, said, &body).map_err(FileHostError::unprocessable_entity)?;
	let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
	let change = voice_repo::put_note(&mut tx, subject.as_str(), &note, &body, &now()).await.map_err(refusal)?;
	tx.commit().await?;
	Ok(Json(NoteWritten { change: change.as_str(), note }))
}

/// One take home knows Drive holds too.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TakeOnDrive {
	/// The phone's id for the take.
	pub note: String,
	/// When the build-journal session that first listed it there looked.
	pub listed_at: String,
}

/// `GET /voice-notes/on-drive`
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TakesOnDrive {
	/// Each asked-about take home knows Drive holds, in the order asked.
	pub on_drive: Vec<TakeOnDrive>,
}

/// `GET /voice-notes/on-drive?ids=<id>,<id>`
///
/// Which of the phone's Studio takes home knows Drive holds too
/// (paulgsc/some-ui#2401, HF11), from the build-journal session's lists in
/// the mailbox (`voice_repo::on_drive`).
///
/// Reads only; it writes nothing and sends nothing. The ids are the phone's,
/// as `PUT /voice-notes/:id` took them; one home does not know as on Drive,
/// or does not hold, is simply not in the answer.
///
/// The query is read as its pairs, so `ids` said twice is refused as any
/// other wrong `ids` is, not by the extractor; other keys are ignored.
///
/// # Errors
/// 401 without a session, 403 from an untrusted origin, 422 for no ids, more
/// than `voice_repo::on_drive::MAX_IDS`, one that is not a note's id, or
/// `ids` said more than once, and 500 for a storage failure.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "voice_on_drive_get", skip_all, fields(otel.kind = "server"))]
pub async fn get_on_drive(subject: SubjectId, State(db): State<SqlitePool>, Query(query): Query<Vec<(String, String)>>) -> Result<Json<TakesOnDrive>, FileHostError> {
	let mut said = query.iter().filter(|(key, _)| key == "ids").map(|(_, value)| value.as_str());
	let asked = said.next().unwrap_or_default();
	let ids: Vec<&str> = asked.split(',').filter(|id| !id.is_empty()).collect();
	if said.next().is_some() || ids.is_empty() || ids.len() > voice_repo::on_drive::MAX_IDS || !ids.iter().all(|id| voice_repo::is_note_id(id)) {
		let max = voice_repo::on_drive::MAX_IDS.to_string();
		let message = String::from("between 1 and ") + &max + " of the phone's note ids, comma-separated, said once";
		return Err(FileHostError::unprocessable_entity([("ids", message)]));
	}
	let mut conn = db.acquire().await?;
	let found = voice_repo::on_drive::on_drive(&mut conn, subject.as_str(), &ids).await?;
	Ok(Json(TakesOnDrive {
		on_drive: found
			.into_iter()
			.map(|take| TakeOnDrive {
				note: take.note,
				listed_at: take.listed_at,
			})
			.collect(),
	}))
}

/// `GET /voice-notes/:id/heard`
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NoteHeard {
	/// Null while home holds the note and has not heard it yet.
	pub heard: Option<Transcript>,
}

/// `GET /voice-notes/:id/heard`
///
/// What home heard in the caller's note `id` (paulgsc/some-ui#2428, MIC10):
/// the newest transcript that heard speech, or, when none did, the newest
/// that found the note silent, as empty text (`voice_repo::transcript::heard`).
/// `heard` is null while no model has ended the note, and stays null for one
/// every attempt failed on, or one but a Jobs note set aside
/// (`PUT /voice-notes/:id/set-aside`) before any model ended it, since the
/// transcriber skips those; the phone keeps the take either way.
///
/// Reads only; it writes nothing, sends nothing, and reads no audio and no
/// word. The text never reaches a log line: the span skips every argument.
///
/// # Errors
/// 401 without a session, 403 from an untrusted origin, 404 for a note the
/// caller's subject does not hold (another subject's included), and 500 for
/// a storage failure.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "voice_heard_get", skip_all, fields(otel.kind = "server"))]
pub async fn get_heard(subject: SubjectId, State(db): State<SqlitePool>, Path(id): Path<String>) -> Result<Json<NoteHeard>, FileHostError> {
	let mut conn = db.acquire().await?;
	let heard = match voice_repo::transcript::heard(&mut conn, subject.as_str(), &id).await.map_err(refusal)? {
		Hearing::NotHeld => return Err(FileHostError::NotFound),
		Hearing::Waiting => None,
		Hearing::Heard(transcript) => Some(transcript),
	};
	Ok(Json(NoteHeard { heard }))
}

/// `GET /voice-notes/:id/audio`
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteAudio {
	/// The phone's id for the note.
	pub id: String,
	/// Its container, one of `voice_repo::CONTENT_TYPES`.
	pub content_type: &'static str,
	/// The lowercase hex SHA-256 of its bytes, which the phone checks before
	/// it plays or keeps them.
	pub sha256: String,
	/// Its bytes as the phone sent them, as standard base64.
	pub audio: String,
}

/// `GET /voice-notes/:id/audio`
///
/// The caller's Rust Rounds note `id` back from home (paulgsc/some-ui#2237,
/// RR6; the owner's D4, 2026-10-29: home gives a note's audio back to the
/// phone, for refine cards and replay), so a refine card can play an earlier
/// note the phone has let go once home held it.
///
/// The audio comes inside JSON, as base64 beside its container and SHA-256,
/// because the phone reads every answer from home as text
/// (paulgsc/some-ui `lib/device-backend/native-http`). Its `ETag` is the
/// note's SHA-256: a note is never rewritten, so the tag never changes, and
/// `If-None-Match` naming it answers `304`.
///
/// Reads only; it writes nothing and sends nothing elsewhere.
///
/// # Errors
/// 401 without a session, 403 for a browser's (only a paired phone reads a
/// note's audio), 404 for a note the caller's subject does not hold
/// (another subject's included) and for one said for anything but
/// `rust-round`, and 500 for a storage failure.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "voice_audio_get", skip_all, fields(otel.kind = "server"))]
pub async fn get_audio(paired: Paired, State(db): State<SqlitePool>, headers: HeaderMap, Path(id): Path<String>) -> Result<Response, FileHostError> {
	let mut conn = db.acquire().await?;
	let held = voice_repo::audio(&mut conn, paired.subject.as_str(), &id, Of::RustRound)
		.await
		.map_err(refusal)?
		.ok_or(FileHostError::NotFound)?;
	let etag = etag_value(&held.sha256)?;
	if if_none_match_hits(&headers, &etag) {
		return Ok(not_modified(etag));
	}
	let mut response = Json(NoteAudio {
		id,
		content_type: held.content_type,
		sha256: held.sha256,
		audio: STANDARD.encode(&held.body),
	})
	.into_response();
	response.headers_mut().insert(ETAG, etag);
	Ok(response)
}

/// `PUT /voice-notes/:id/set-aside`'s query: when the owner let the note go.
#[derive(Debug, Deserialize)]
pub struct SetAsideQuery {
	at: Option<String>,
}

/// `PUT /voice-notes/:id/set-aside?at=`
///
/// The owner let the caller's note `id` go on the phone (the owner's D3 on
/// paulgsc/some-ui epic #2368, MIC2 paulgsc/some-ui#2372): home sets it
/// aside and keeps it (`voice_repo::set_aside`). The note, its audio and any
/// transcript stay; the transcriber's queue, the transcripts filed for the
/// mailbox and the reflections index skip it from then on, unless it is a
/// Jobs note, which a decisions or scouts file naming it still waits on.
///
/// Idempotent: the first mark stands, so a repeat (a retry, or another
/// device's) is a `204` that changes nothing, its time included. There is
/// no route that un-sets a mark: the phone has no undo for Let go.
///
/// `BEGIN IMMEDIATE`, so the write and the read that tells an earlier mark
/// from no note see one snapshot. No body, in either direction.
///
/// # Errors
/// 401 without a session, 403 from an untrusted origin, 422 naming `at` when
/// it is missing or not an RFC 3339 instant, 404 for a note the caller's
/// subject does not hold (another subject's included), and 500 for a storage
/// failure.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "voice_set_aside_put", skip_all, fields(otel.kind = "server"))]
pub async fn put_set_aside(subject: SubjectId, State(db): State<SqlitePool>, Path(id): Path<String>, Query(query): Query<SetAsideQuery>) -> Result<StatusCode, FileHostError> {
	let Some(at) = query.at.as_deref().and_then(voice_repo::set_aside::when) else {
		return Err(FileHostError::unprocessable_entity([("at", "not an RFC 3339 instant")]));
	};
	let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
	let marked = voice_repo::set_aside::set_aside(&mut tx, subject.as_str(), &id, &at, &now()).await.map_err(refusal)?;
	tx.commit().await?;
	match marked {
		SetAside::Marked | SetAside::AlreadyMarked => Ok(StatusCode::NO_CONTENT),
		SetAside::NotHeld => Err(FileHostError::NotFound),
	}
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
		assert_eq!(module().descriptors().count(), 5);
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

	/// A Studio take is sent home as any note is, and is held said for
	/// `studio`; its own rules are `voice_repo`'s tests.
	#[tokio::test]
	async fn a_studio_take_reaches_home_as_a_note() {
		const TAKE_PATH: &str = "/voice-notes/t1?of=studio&ref=build-journal-ep04:s3:2&at=2026-10-30T18:20:00Z";
		let state = state().await;
		let pool = state.pool.clone();
		let phone = session(&state, A, Carrier::Bearer).await;
		let app = app(state);

		let (status, answer) = put(&app, TAKE_PATH, Some(&phone), "audio/mp4", TAKE).await;
		assert_eq!(status, StatusCode::OK, "{answer}");
		assert_eq!(
			answer,
			json!({
				"change": "added",
				"note": {
					"id": "t1", "of": "studio", "ref": "build-journal-ep04:s3:2", "at": "2026-10-30T18:20:00.000Z",
					"contentType": "audio/mp4", "size": TAKE.len(), "sha256": TAKE_SHA256
				}
			})
		);
		let held: (String, String) = sqlx::query_as("SELECT of_kind, of_ref FROM voice_note WHERE subject_id = ?1 AND note_id = 't1'")
			.bind(A)
			.fetch_one(&pool)
			.await
			.unwrap();
		assert_eq!(held, (String::from("studio"), String::from("build-journal-ep04:s3:2")));
	}

	async fn get(app: &Router, path: &str, auth: Option<&(HeaderName, String)>) -> (StatusCode, Value) {
		let mut request = Request::builder().method(Method::GET).uri(path);
		if let Some((name, value)) = auth {
			request = request.header(name, value);
		}
		let response = app.clone().oneshot(request.body(Body::empty()).unwrap()).await.unwrap();
		let status = response.status();
		let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
		(status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
	}

	/// HF11 (paulgsc/some-ui#2401): the phone reads which of its takes home
	/// knows Drive holds, its own only, and the id route still takes a note.
	#[tokio::test]
	async fn the_phone_reads_which_of_its_takes_home_knows_drive_holds() {
		let state = state().await;
		let pool = state.pool.clone();
		let phone = session(&state, A, Carrier::Bearer).await;
		let other = session(&state, B, Carrier::Bearer).await;
		let app = app(state);
		for path in [
			"/voice-notes/t1?of=studio&ref=ep04:s1:t1&at=2026-10-30T18:20:00Z",
			"/voice-notes/t2?of=studio&ref=ep04:s2:t2&at=2026-10-30T18:21:00Z",
		] {
			let body: &[u8] = if path.contains("t1?") { TAKE } else { b"other frames" };
			assert_eq!(put(&app, path, Some(&phone), "audio/webm", body).await.0, StatusCode::OK);
		}
		let sha256 = vec![String::from(TAKE_SHA256)];
		let listed = voice_repo::on_drive::Listed {
			at: "2026-11-01T09:15:00.000Z",
			name: "drive-takes-20261101T091500Z.json",
			sha256: &sha256,
		};
		let mut conn = pool.acquire().await.unwrap();
		assert_eq!(voice_repo::on_drive::record(&mut conn, A, listed, "now").await.unwrap(), 1);
		drop(conn);

		let (status, answer) = get(&app, "/voice-notes/on-drive?ids=t2,t1,nope", Some(&phone)).await;
		assert_eq!(status, StatusCode::OK, "{answer}");
		assert_eq!(answer, json!({ "onDrive": [{ "note": "t1", "listedAt": "2026-11-01T09:15:00.000Z" }] }));
		// Another subject's read finds none of A's.
		assert_eq!(get(&app, "/voice-notes/on-drive?ids=t1", Some(&other)).await.1, json!({ "onDrive": [] }));
		assert_eq!(get(&app, "/voice-notes/on-drive?ids=t1", None).await.0, StatusCode::UNAUTHORIZED);
		let many = vec!["t"; 101].join(",");
		for bad in ["", "?ids=", "?ids=a/b", "?ids=,,", "?ids=t1&ids=t2", "?ids=t1,on-drive"] {
			let path = String::from("/voice-notes/on-drive") + bad;
			assert_eq!(get(&app, &path, Some(&phone)).await.0, StatusCode::UNPROCESSABLE_ENTITY, "{bad}");
		}
		let path = String::from("/voice-notes/on-drive?ids=") + &many;
		let (status, refused) = get(&app, &path, Some(&phone)).await;
		assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
		let said = String::from("between 1 and ") + &voice_repo::on_drive::MAX_IDS.to_string() + " of the phone's note ids";
		assert!(refused["error"]["details"]["ids"][0].as_str().unwrap().starts_with(&said), "{refused}");
	}

	/// One attempt by `model` on `subject`'s note `note`, ended at `at`.
	async fn hear(pool: &SqlitePool, subject: &str, note: &str, model: &str, outcome: voice_repo::transcript::Outcome, at: &str) {
		let attempt = voice_repo::transcript::Attempt {
			model,
			engine: "whisper.cpp 1.8.3",
			outcome,
			duration_ms: Some(1_200),
			took_ms: 900,
		};
		let mut conn = pool.acquire().await.unwrap();
		let kept = voice_repo::transcript::keep_attempt(&mut conn, subject, note, &attempt, at).await.unwrap();
		assert!(matches!(kept, voice_repo::transcript::Kept::Added(_)), "{kept:?}");
	}

	fn said(text: &str, language: &str) -> voice_repo::transcript::Outcome {
		voice_repo::transcript::Outcome::Heard {
			language: language.to_owned(),
			language_p: 0.97,
			text: text.to_owned(),
			words: Vec::new(),
		}
	}

	/// MIC10 (paulgsc/some-ui#2428): the phone reads back what home heard in
	/// its own note: null until a model ends it (a failure is not an ending),
	/// then the newest transcript that heard speech, which a later silent
	/// ending does not hide; another subject's note is not found, as an
	/// unknown one is.
	#[tokio::test]
	async fn the_phone_reads_back_what_home_heard_in_its_note() {
		use voice_repo::transcript::Outcome;
		const SMALL: &str = "ggml-small-q5_1.bin";
		const MEDIUM: &str = "ggml-medium-q5_0.bin";
		let state = state().await;
		let pool = state.pool.clone();
		let phone = session(&state, A, Carrier::Bearer).await;
		let other = session(&state, B, Carrier::Bearer).await;
		let app = app(state);
		assert_eq!(put(&app, NOTE, Some(&phone), "audio/webm", TAKE).await.0, StatusCode::OK);

		assert_eq!(get(&app, "/voice-notes/n1/heard", None).await.0, StatusCode::UNAUTHORIZED);
		let (status, unknown) = get(&app, "/voice-notes/n9/heard", Some(&phone)).await;
		assert_eq!(status, StatusCode::NOT_FOUND, "{unknown}");
		assert_eq!(get(&app, "/voice-notes/on-drive/heard", Some(&phone)).await.0, StatusCode::NOT_FOUND);

		assert_eq!(get(&app, "/voice-notes/n1/heard", Some(&phone)).await, (StatusCode::OK, json!({ "heard": null })));
		hear(&pool, A, "n1", SMALL, Outcome::Deadline, "2026-10-25T16:00:00.000Z").await;
		assert_eq!(
			get(&app, "/voice-notes/n1/heard", Some(&phone)).await.1,
			json!({ "heard": null }),
			"a failed attempt heard nothing"
		);

		hear(&pool, A, "n1", SMALL, said("a small idea", "en"), "2026-10-25T16:05:00.000Z").await;
		let small = json!({
			"heard": { "text": "a small idea", "language": "en", "model": SMALL, "heardAt": "2026-10-25T16:05:00.000Z" }
		});
		assert_eq!(get(&app, "/voice-notes/n1/heard", Some(&phone)).await, (StatusCode::OK, small));

		hear(&pool, A, "n1", MEDIUM, said("a smaller idea", "en"), "2026-10-26T09:00:00.000Z").await;
		let medium = json!({
			"heard": { "text": "a smaller idea", "language": "en", "model": MEDIUM, "heardAt": "2026-10-26T09:00:00.000Z" }
		});
		assert_eq!(get(&app, "/voice-notes/n1/heard", Some(&phone)).await.1, medium, "the newest transcript is read");
		hear(&pool, A, "n1", "ggml-large.bin", Outcome::Silent, "2026-10-27T09:00:00.000Z").await;
		assert_eq!(get(&app, "/voice-notes/n1/heard", Some(&phone)).await.1, medium, "words are not hidden by a later silence");

		let (status, hidden) = get(&app, "/voice-notes/n1/heard", Some(&other)).await;
		assert_eq!(status, StatusCode::NOT_FOUND, "another subject's note is not theirs to read: {hidden}");
		assert!(!hidden.to_string().contains("idea"), "{hidden}");

		// A note home found silent, and nothing else, reads as heard with no words.
		assert_eq!(put(&app, NOTE, Some(&other), "audio/webm", TAKE).await.0, StatusCode::OK);
		hear(&pool, B, "n1", SMALL, Outcome::Silent, "2026-10-25T17:00:00.000Z").await;
		assert_eq!(
			get(&app, "/voice-notes/n1/heard", Some(&other)).await.1,
			json!({ "heard": { "text": "", "language": null, "model": SMALL, "heardAt": "2026-10-25T17:00:00.000Z" } })
		);
	}

	/// MIC2 (paulgsc/some-ui#2372, D3): Let go sets the caller's own note aside
	/// and keeps it. A repeat is a 204 that leaves the first mark's time; an
	/// unknown note and another subject's are a 404; a missing or wrong `at`
	/// is a 422; and the route needs a session and a trusted origin.
	#[tokio::test]
	async fn the_phone_sets_its_own_note_aside_once_and_home_keeps_it() {
		const SET_ASIDE: &str = "/voice-notes/n1/set-aside?at=2026-11-01T02:15:00-07:00";
		let state = state().await;
		let pool = state.pool.clone();
		let phone = session(&state, A, Carrier::Bearer).await;
		let other = session(&state, B, Carrier::Bearer).await;
		let app = app(state);
		assert_eq!(put(&app, NOTE, Some(&phone), "audio/webm", TAKE).await.0, StatusCode::OK);
		let marks = || async {
			sqlx::query_as::<_, (String, String, String)>("SELECT subject_id, note_id, set_aside_at FROM voice_note_set_aside ORDER BY subject_id")
				.fetch_all(&pool)
				.await
				.unwrap()
		};

		assert_eq!(put(&app, SET_ASIDE, None, "text/plain", b"").await.0, StatusCode::UNAUTHORIZED);
		let request = Request::builder()
			.method(Method::PUT)
			.uri(SET_ASIDE)
			.header(&phone.0, &phone.1)
			.header("origin", "https://evil.app.test")
			.header("sec-fetch-site", "cross-site")
			.body(Body::empty())
			.unwrap();
		assert_eq!(app.clone().oneshot(request).await.unwrap().status(), StatusCode::FORBIDDEN);
		assert!(marks().await.is_empty(), "nothing refused was written");

		let (status, refused) = put(&app, SET_ASIDE, Some(&other), "text/plain", b"").await;
		assert_eq!(status, StatusCode::NOT_FOUND, "another subject's note is not theirs to set aside: {refused}");
		assert_eq!(
			put(&app, "/voice-notes/n9/set-aside?at=2026-11-01T09:15:00Z", Some(&phone), "text/plain", b"").await.0,
			StatusCode::NOT_FOUND
		);
		assert_eq!(
			put(&app, "/voice-notes/on-drive/set-aside?at=2026-11-01T09:15:00Z", Some(&phone), "text/plain", b"")
				.await
				.0,
			StatusCode::NOT_FOUND
		);
		for bad in ["/voice-notes/n1/set-aside", "/voice-notes/n1/set-aside?at=soon"] {
			let (status, refused) = put(&app, bad, Some(&phone), "text/plain", b"").await;
			assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{bad}");
			assert!(refused["error"]["details"]["at"].is_array(), "{refused}");
		}
		assert!(marks().await.is_empty());

		// What home heard in the note before it was let go.
		hear(&pool, A, "n1", "ggml-small-q5_1.bin", said("an idea let go", "en"), "2026-11-01T09:00:00.000Z").await;
		let heard = json!({
			"heard": { "text": "an idea let go", "language": "en", "model": "ggml-small-q5_1.bin", "heardAt": "2026-11-01T09:00:00.000Z" }
		});
		assert_eq!(get(&app, "/voice-notes/n1/heard", Some(&phone)).await, (StatusCode::OK, heard.clone()));

		let (status, body) = put(&app, SET_ASIDE, Some(&phone), "text/plain", b"").await;
		assert_eq!((status, body), (StatusCode::NO_CONTENT, Value::Null));
		let first = vec![(A.to_owned(), String::from("n1"), String::from("2026-11-01T09:15:00.000Z"))];
		assert_eq!(marks().await, first);
		let again = put(&app, "/voice-notes/n1/set-aside?at=2026-11-02T09:15:00Z", Some(&phone), "text/plain", b"").await;
		assert_eq!(again, (StatusCode::NO_CONTENT, Value::Null), "a repeat is idempotent");
		assert_eq!(marks().await, first, "the first mark's time stands");

		// Home keeps the note: its audio, and what it heard in it, as before.
		let audio: Vec<u8> = sqlx::query_scalar("SELECT body FROM voice_note_audio WHERE subject_id = ?1 AND note_id = 'n1'")
			.bind(A)
			.fetch_one(&pool)
			.await
			.unwrap();
		assert_eq!(audio, TAKE);
		assert_eq!(get(&app, "/voice-notes/n1/heard", Some(&phone)).await, (StatusCode::OK, heard), "its transcript is kept");
		assert_eq!(put(&app, NOTE, Some(&phone), "audio/webm", TAKE).await.1["change"], "unchanged");
	}

	/// RR6 (paulgsc/some-ui#2237, D4): a paired phone reads its own Rust
	/// Rounds note's audio back, byte for byte, with the note's SHA-256 as the
	/// `ETag` and a `304` for it; a refine's note names the one it refines. No
	/// session is a 401, a browser's a 403, and another subject's note, an
	/// unknown one and a note of another kind are each a 404.
	#[tokio::test]
	async fn a_paired_phone_reads_its_own_rust_round_notes_audio_back() {
		use axum::http::header::{ETAG, IF_NONE_MATCH};
		use base64::{engine::general_purpose::STANDARD, Engine};
		const ROUND_NOTE: &str = "/voice-notes/r1?of=rust-round&ref=indexing-01:a&at=2026-11-02T18:20:00Z";
		const REFINE_NOTE: &str = "/voice-notes/r2?of=rust-round&ref=indexing-01:refine-a:r1&at=2026-11-02T18:30:00Z";
		let state = state().await;
		let phone = session(&state, A, Carrier::Bearer).await;
		let browser = session(&state, A, Carrier::Cookie).await;
		let other = session(&state, B, Carrier::Bearer).await;
		let app = app(state);
		let (status, answer) = put(&app, ROUND_NOTE, Some(&phone), "audio/webm;codecs=opus", TAKE).await;
		assert_eq!(status, StatusCode::OK, "{answer}");
		assert_eq!(answer["note"]["of"], "rust-round");
		let (status, answer) = put(&app, REFINE_NOTE, Some(&phone), "audio/webm", b"again").await;
		assert_eq!((status, &answer["note"]["ref"]), (StatusCode::OK, &json!("indexing-01:refine-a:r1")), "{answer}");
		let (status, refused) = put(
			&app,
			"/voice-notes/r3?of=rust-round&ref=indexing-01&at=2026-11-02T18:30:00Z",
			Some(&phone),
			"audio/webm",
			TAKE,
		)
		.await;
		assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "a ref that names no card: {refused}");
		assert_eq!(put(&app, NOTE, Some(&phone), "audio/webm", TAKE).await.0, StatusCode::OK);

		let (status, body) = get(&app, "/voice-notes/r1/audio", Some(&phone)).await;
		assert_eq!(status, StatusCode::OK, "{body}");
		assert_eq!(
			body,
			json!({ "id": "r1", "contentType": "audio/webm", "sha256": TAKE_SHA256, "audio": STANDARD.encode(TAKE) })
		);
		let request = |headers: &[(HeaderName, &str)]| {
			let mut request = Request::builder().method(Method::GET).uri("/voice-notes/r1/audio");
			for (name, value) in headers {
				request = request.header(name, *value);
			}
			request.body(Body::empty()).unwrap()
		};
		let response = app.clone().oneshot(request(&[(phone.0.clone(), phone.1.as_str())])).await.unwrap();
		let etag = String::from("\"") + TAKE_SHA256 + "\"";
		assert_eq!(response.headers()[ETAG], etag.as_str());
		let response = app
			.clone()
			.oneshot(request(&[(phone.0.clone(), phone.1.as_str()), (IF_NONE_MATCH, etag.as_str())]))
			.await
			.unwrap();
		assert_eq!(response.status(), StatusCode::NOT_MODIFIED);
		assert_eq!(response.headers()[ETAG], etag.as_str());

		assert_eq!(get(&app, "/voice-notes/r1/audio", None).await.0, StatusCode::UNAUTHORIZED);
		assert_eq!(get(&app, "/voice-notes/r1/audio", Some(&browser)).await.0, StatusCode::FORBIDDEN, "a paired phone's only");
		let (status, hidden) = get(&app, "/voice-notes/r1/audio", Some(&other)).await;
		assert_eq!(status, StatusCode::NOT_FOUND, "another subject's note is not theirs to hear: {hidden}");
		assert_eq!(get(&app, "/voice-notes/r9/audio", Some(&phone)).await.0, StatusCode::NOT_FOUND);
		assert_eq!(
			get(&app, "/voice-notes/n1/audio", Some(&phone)).await.0,
			StatusCode::NOT_FOUND,
			"a reflection's audio stays home"
		);
		assert_eq!(get(&app, "/voice-notes/r2/audio", Some(&phone)).await.1["audio"], STANDARD.encode(b"again"));
	}

	/// A game bite's fields reach the repo from the query, percent-encoding
	/// undone, and come back as the answer's `bite`. Replays and refusals are
	/// `voice_repo`'s tests.
	#[tokio::test]
	async fn a_game_bite_reaches_home_with_its_metadata() {
		const BITE: &str = "/voice-notes/b1?of=game&ref=401772981&at=2026-10-26T20:41:09Z&scenario=in.third-long&how=offered&seq=12\
			&strip=KC%2017%20%E2%80%93%20BUF%2014%20%C2%B7%20Q3%204%3A12&context=Mahomes%209%2F14&watching=1\
			&started=2026-10-26T20:41:07.250Z&offset=-1350&mode=standard&catalog=1";
		let state = state().await;
		let phone = session(&state, A, Carrier::Bearer).await;
		let app = app(state);

		let (status, answer) = put(&app, BITE, Some(&phone), "audio/mp4", TAKE).await;
		assert_eq!(status, StatusCode::OK, "{answer}");
		assert_eq!(
			answer["note"]["bite"],
			json!({
				"scenario": "in.third-long", "how": "offered", "seq": 12, "strip": "KC 17 – BUF 14 · Q3 4:12",
				"context": "Mahomes 9/14", "watching": true, "started": "2026-10-26T20:41:07.250Z", "offset": -1350,
				"mode": "standard", "catalog": 1
			})
		);
	}
}

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
//! Neither the audio, nor its hash, nor a bite's strip or context line
//! reaches a log line: the span skips every argument, and a refusal names
//! fields with fixed words, never what was said.
//!
//! **The deletion hold.** Each handler takes a [`SubjectId`] first and keeps
//! it until it returns, so a write holds off `DELETE /auth/account` as every
//! other subject-scoped write does (docs/identity.md, invariant 9).
//!
//! **A Studio take's words** (paulgsc/some-ui#2322, ST24): [`get_hearings`]
//! reads what home heard in a take, and [`put_owner_words`] keeps the owner's
//! fix as one more transcript (`voice_repo::owner`), the one writer of
//! `voice_transcript` besides the transcriber. Neither the words nor the
//! text reaches a log line: both spans skip every argument, and a refusal
//! names fields with fixed words.

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
use voice_repo::owner::{Hearing, Hearings, Refused, NOT_HEARD};
use voice_repo::{Note, Said, SaidBite, VoiceError, REWRITTEN};

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

/// `PUT /voice-notes/:id/transcripts/owner/:n`
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WordsWritten {
	/// `added` or `unchanged` — `voice_repo::Change`.
	pub change: &'static str,
	/// The owner's save as home holds it, which the phone compares with what
	/// it sent before it counts the save as home's.
	pub hearing: Hearing,
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
/// says nothing more, as a reflection does; a redo is a new note.
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

/// `GET /voice-notes/:id/transcripts`: every hearing of a Studio take that
/// ended, the owner's saves included, newest first, and which one a reader
/// of its words takes (`voice_repo::owner::Hearings`).
///
/// # Errors
/// 401 without a session, 404 when the subject holds no Studio take under
/// that id (a note said for anything else included: its words are read by
/// no route), and 500 for a storage failure.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "voice_take_hearings", skip_all, fields(otel.kind = "server"))]
pub async fn get_hearings(subject: SubjectId, State(db): State<SqlitePool>, Path(id): Path<String>) -> Result<Json<Hearings>, FileHostError> {
	let mut conn = db.acquire().await?;
	voice_repo::owner::hearings(&mut conn, subject.as_str(), &id)
		.await?
		.map(Json)
		.ok_or(FileHostError::NotFound)
}

/// `PUT /voice-notes/:id/transcripts/owner/:n`, the body being the owner's
/// words for the take (`{ basedOn, words: [{ text, startMs, endMs,
/// language? }] }`): the owner's `n`th save, kept beside home's hearings.
///
/// `BEGIN IMMEDIATE`, so the reads that decide and the writes see one
/// snapshot.
///
/// # Errors
/// 401 without a session, 403 from an untrusted origin, 404 for no such
/// Studio take, 422 naming each refused field (`n`, `body`, `basedOn`,
/// `words`), 409 for other words under a held save, and 500 for a storage
/// failure.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "voice_take_owner_words_put", skip_all, fields(otel.kind = "server"))]
pub async fn put_owner_words(
	subject: SubjectId,
	State(db): State<SqlitePool>,
	Path((id, n)): Path<(String, String)>,
	body: Bytes,
) -> Result<Json<WordsWritten>, FileHostError> {
	let n = voice_repo::owner::save_number(&n);
	let words = voice_repo::owner::read_words(&body);
	let (n, words) = match (n, words) {
		(Some(n), Ok(words)) => (n, words),
		(n, words) => {
			let mut problems = words.err().unwrap_or_default();
			if n.is_none() {
				problems.insert(0, ("n", "not a save of the take: 1 to 9999"));
			}
			return Err(FileHostError::unprocessable_entity(problems));
		}
	};
	let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
	let (change, hearing) = voice_repo::owner::keep_save(&mut tx, subject.as_str(), &id, n, &words, &now())
		.await
		.map_err(|refused| match refused {
			Refused::NoTake => FileHostError::NotFound,
			Refused::NotHeard => FileHostError::unprocessable_entity([("basedOn", NOT_HEARD)]),
			Refused::Rewritten => FileHostError::Conflict(voice_repo::owner::REWRITTEN),
			Refused::Storage(err) => refusal(err),
		})?;
	tx.commit().await?;
	Ok(Json(WordsWritten { change: change.as_str(), hearing }))
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
		assert_eq!(module().descriptors().count(), 3);
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

	/// What home's transcriber heard in `subject`'s note `id`: `text`, each
	/// word half a second long.
	async fn heard(pool: &SqlitePool, subject: &str, id: &str, text: &str) {
		use voice_repo::transcript::{keep_attempt, Attempt, Outcome, Word};
		let words = (0_i64..)
			.zip(text.split_whitespace())
			.map(|(n, word)| Word {
				text: word.to_owned(),
				start_ms: 500 * n,
				end_ms: 500 * n + 400,
				p: Some(if word == "their" { 0.3 } else { 0.9 }),
				language: Some(String::from("en")),
			})
			.collect();
		let attempt = Attempt {
			model: "ggml-small-q5_1.bin",
			engine: "whisper.cpp 1.8.3",
			outcome: Outcome::Heard {
				language: String::from("en"),
				language_p: 0.97,
				text: text.to_owned(),
				words,
			},
			duration_ms: Some(1_200),
			took_ms: 800,
		};
		let mut tx = pool.begin().await.unwrap();
		keep_attempt(&mut tx, subject, id, &attempt, "2026-10-30T18:30:00.000Z").await.unwrap();
		tx.commit().await.unwrap();
	}

	/// The take review's round (paulgsc/some-ui#2322, ST24): the phone reads
	/// what home heard in a take, saves the owner's fix, a replay changes
	/// nothing, other words under that save are a 409, and the words read
	/// next are the owner's, home's kept beside them as heard.
	#[tokio::test]
	async fn a_takes_words_are_read_and_the_owners_fix_kept_beside_them() {
		const TAKE_PATH: &str = "/voice-notes/t1?of=studio&ref=build-journal-ep04:s3:2&at=2026-10-30T18:20:00Z";
		const WORDS: &str = "/voice-notes/t1/transcripts";
		const SAVE: &str = "/voice-notes/t1/transcripts/owner/1";
		let state = state().await;
		let pool = state.pool.clone();
		let phone = session(&state, A, Carrier::Bearer).await;
		let other = session(&state, B, Carrier::Bearer).await;
		let app = app(state);
		assert_eq!(put(&app, TAKE_PATH, Some(&phone), "audio/mp4", TAKE).await.0, StatusCode::OK);

		assert_eq!(get(&app, WORDS, None).await.0, StatusCode::UNAUTHORIZED);
		assert_eq!(
			get(&app, WORDS, Some(&phone)).await,
			(StatusCode::OK, json!({ "note": "t1", "chosen": null, "hearings": [] }))
		);
		heard(&pool, A, "t1", "over their").await;
		let (status, read) = get(&app, WORDS, Some(&phone)).await;
		assert_eq!(status, StatusCode::OK, "{read}");
		assert_eq!(
			read,
			json!({
				"note": "t1",
				"chosen": "ggml-small-q5_1.bin",
				"hearings": [{
					"model": "ggml-small-q5_1.bin", "by": "home", "outcome": "heard", "language": "en",
					"heardAt": "2026-10-30T18:30:00.000Z", "text": "over their",
					"words": [
						{ "text": "over", "startMs": 0, "endMs": 400, "p": 0.9, "language": "en" },
						{ "text": "their", "startMs": 500, "endMs": 900, "p": 0.3, "language": "en" }
					]
				}]
			})
		);
		assert_eq!(get(&app, WORDS, Some(&other)).await.0, StatusCode::NOT_FOUND, "A's take is not B's");

		let fixed = json!({
			"basedOn": "ggml-small-q5_1.bin",
			"words": [
				{ "text": "over", "startMs": 0, "endMs": 400, "language": "en" },
				{ "text": "there", "startMs": 500, "endMs": 900, "language": "en" }
			]
		})
		.to_string();
		let (status, saved) = put(&app, SAVE, Some(&phone), "application/json", fixed.as_bytes()).await;
		assert_eq!(status, StatusCode::OK, "{saved}");
		assert_eq!(saved["change"], "added");
		assert_eq!(saved["hearing"]["model"], "owner:1");
		assert_eq!(saved["hearing"]["by"], "owner");
		assert_eq!(saved["hearing"]["text"], "over there");
		let (status, again) = put(&app, SAVE, Some(&phone), "application/json", fixed.as_bytes()).await;
		assert_eq!((status, &again["change"]), (StatusCode::OK, &json!("unchanged")));
		assert_eq!(again["hearing"], saved["hearing"], "a replay answers the save as home holds it");

		let rewritten = fixed.replace("there", "where");
		let (status, answer) = put(&app, SAVE, Some(&phone), "application/json", rewritten.as_bytes()).await;
		assert_eq!(status, StatusCode::CONFLICT, "{answer}");
		assert_eq!(
			put(&app, "/voice-notes/t1/transcripts/owner/0", Some(&phone), "application/json", fixed.as_bytes()).await.0,
			StatusCode::UNPROCESSABLE_ENTITY
		);
		assert_eq!(
			put(&app, "/voice-notes/t9/transcripts/owner/1", Some(&phone), "application/json", fixed.as_bytes()).await.0,
			StatusCode::NOT_FOUND
		);
		assert_eq!(put(&app, SAVE, Some(&other), "application/json", fixed.as_bytes()).await.0, StatusCode::NOT_FOUND);

		let read = get(&app, WORDS, Some(&phone)).await.1;
		assert_eq!(read["chosen"], "owner:1", "the owner's words are the ones read");
		assert_eq!(read["hearings"][1]["text"], "over their", "home's hearing is kept as heard");
		assert_eq!(read["hearings"][0]["words"][1], json!({ "text": "there", "startMs": 500, "endMs": 900, "language": "en" }));
	}

	/// A save that breaks a rule is a 422 naming each field in fixed words,
	/// and writes nothing; a note said for anything but `studio` has no
	/// words to read or fix here.
	#[tokio::test]
	async fn a_refused_save_names_its_fields_and_only_a_take_has_words_here() {
		let state = state().await;
		let pool = state.pool.clone();
		let phone = session(&state, A, Carrier::Bearer).await;
		let app = app(state);
		assert_eq!(put(&app, NOTE, Some(&phone), "audio/webm", TAKE).await.0, StatusCode::OK);
		heard(&pool, A, "n1", "a reflection").await;
		assert_eq!(get(&app, "/voice-notes/n1/transcripts", Some(&phone)).await.0, StatusCode::NOT_FOUND);

		let named = |answer: (StatusCode, Value)| {
			assert_eq!(answer.0, StatusCode::UNPROCESSABLE_ENTITY, "{}", answer.1);
			let mut fields: Vec<String> = answer.1["error"]["details"].as_object().unwrap().keys().cloned().collect();
			fields.sort();
			fields
		};
		assert_eq!(
			named(put(&app, "/voice-notes/n1/transcripts/owner/x", Some(&phone), "application/json", b"{}").await),
			["body", "n"]
		);
		assert_eq!(
			named(
				put(
					&app,
					"/voice-notes/n1/transcripts/owner/1",
					Some(&phone),
					"application/json",
					br#"{"basedOn":"","words":[]}"#
				)
				.await
			),
			["basedOn", "words"]
		);
		let one = br#"{"basedOn":"ggml-small-q5_1.bin","words":[{"text":"hi","startMs":0,"endMs":400}]}"#;
		assert_eq!(
			put(&app, "/voice-notes/n1/transcripts/owner/1", Some(&phone), "application/json", one).await.0,
			StatusCode::NOT_FOUND,
			"a reflection is no take"
		);
		let owners: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM voice_transcript WHERE engine = 'owner'")
			.fetch_one(&pool)
			.await
			.unwrap();
		assert_eq!(owners, 0, "nothing refused was written");
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

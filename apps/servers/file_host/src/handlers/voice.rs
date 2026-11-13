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
//! [`post_station`] is the only caller of `voice_repo::conveyor::advance`,
//! the one writer of `studio_take_move`; it writes no note. A move's labels,
//! comment and issue reach no log line: the span skips every argument.
//!
//! **The one way a note is deleted but with the account** (docs/identity.md
//! invariant 23): [`delete_note`] is the only caller of
//! `voice_repo::delete::delete`, for a kind the owner switched on (none
//! yet), and the only writer of `voice_note_deletion`.
//!
//! [`get_deleted`] is the only reader of the whole list of receipts
//! (`voice_repo::delete::receipts`), for My data's list of what was deleted.
//!
//! **The one way audio leaves home** (docs/identity.md invariant 23):
//! [`get_audio`] is the only caller of `voice_repo::audio_head` and
//! `voice_repo::audio_body`, for the kinds `voice_repo::PLAYED_BACK` lists,
//! and its span skips every argument.
//!
//! [`get_takes`] is the only reader of the take library
//! (`voice_repo::takes`): what each Studio take is and the words home heard
//! in it, never its audio, and never a vector of the meaning index it
//! searches (docs/identity.md invariant 35).
//!
//! **The deletion hold.** The handler takes a [`SubjectId`] first and keeps
//! it until it returns, so a write holds off `DELETE /auth/account` as every
//! other subject-scoped write does (docs/identity.md, invariant 9).

use crate::handlers::db::activities::{etag_value, if_none_match_hits, not_modified};
use crate::meaning::MeaningContext;
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
use std::borrow::Cow;
use tracing::instrument;
use voice_repo::{
	conveyor::{Advanced, AtStation, Said as MoveSaid, Station, TakeStation},
	delete::{self as deletion, Deleted, Receipt, SWITCHED_ON},
	kept::{KindKept, NoteKept, WeekKept},
	meaning::Embed as _,
	set_aside::SetAside,
	takes::{Library, Search},
	transcript::{Hearing, Transcript},
	Note, Of, Said, SaidBite, VoiceError, PLAYED_BACK, REWRITTEN,
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
/// another note under a held id (`conflict`) or a note deleted for good
/// (`deleted_for_good`), 413 for a body past the server's limit, and 500 for
/// a storage failure.
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
	// A note deleted for good from My data stays deleted: a phone that still
	// holds its copy and sends it again is refused, not taken back.
	if voice_repo::delete::receipt(&mut tx, subject.as_str(), &note.id).await.map_err(refusal)?.is_some() {
		return Err(FileHostError::DeletedForGood);
	}
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
/// The caller's note `id` back from home, when it is of a kind in
/// `voice_repo::PLAYED_BACK` (a Rust Rounds note or a Studio take), for the
/// reasons that list gives.
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
/// (another subject's included) and for one of a kind not in
/// `voice_repo::PLAYED_BACK`, and 500 for a storage failure.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "voice_audio_get", skip_all, fields(otel.kind = "server"))]
pub async fn get_audio(paired: Paired, State(db): State<SqlitePool>, headers: HeaderMap, Path(id): Path<String>) -> Result<Response, FileHostError> {
	let mut conn = db.acquire().await?;
	let subject = paired.subject.as_str();
	let mut found = None;
	for of in PLAYED_BACK {
		if let Some(head) = voice_repo::audio_head(&mut conn, subject, &id, of).await.map_err(refusal)? {
			found = Some((of, head));
			break;
		}
	}
	let (of, head) = found.ok_or(FileHostError::NotFound)?;
	let etag = etag_value(&head.sha256)?;
	// Decided from the head, so a 304 reads no audio.
	if if_none_match_hits(&headers, &etag) {
		return Ok(not_modified(etag));
	}
	// The note is append-only, so a body missing here is a store that does
	// not read back, not a race.
	let body = voice_repo::audio_body(&mut conn, subject, &id, of)
		.await
		.map_err(refusal)?
		.ok_or(FileHostError::OperationError("a stored voice note does not read back".to_owned()))?;
	let mut response = Json(NoteAudio {
		id,
		content_type: head.content_type,
		sha256: head.sha256,
		audio: STANDARD.encode(&body),
	})
	.into_response();
	response.headers_mut().insert(ETAG, etag);
	Ok(response)
}

/// `GET /voice-notes/kept`
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Kept {
	/// What home keeps of each kind, in `voice_repo::Of`'s order; a kind it
	/// holds none of is not listed.
	pub kinds: Vec<KindKept>,
	/// What reached home week by week, oldest first.
	pub weeks: Vec<WeekKept>,
	/// With `of`, that kind's newest notes, at most
	/// `voice_repo::kept::NEWEST`; null without it.
	pub notes: Option<Vec<NoteKept>>,
}

/// `GET /voice-notes/kept[?of=<kind>[&note=<id>]]`
///
/// What home keeps of the caller's voice notes, for the phone's My data
/// page (paulgsc/some-ui#2476, MY-DATA MD2): how many of each kind, their
/// room, how many were set aside and since when, what reached home each
/// week, and, when `of` names a kind, that kind's newest notes, with the
/// one `note` names when it is older (`voice_repo::kept`). It answers descriptions only: no audio, no `ref`,
/// nothing said.
///
/// Read in one transaction, so its parts agree. The query is read as its pairs, so `of` or `note`
/// said twice is refused as an unknown kind is; other keys are ignored.
///
/// # Errors
/// 401 without a session, 403 for a browser's (only a paired phone reads
/// it), 422 for an `of` that names no kind or is said more than once, or a
/// `note` that is no note's id, is said more than once or comes without
/// `of`, and 500 for a storage failure.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "voice_kept_get", skip_all, fields(otel.kind = "server"))]
pub async fn get_kept(paired: Paired, State(db): State<SqlitePool>, Query(query): Query<Vec<(String, String)>>) -> Result<Json<Kept>, FileHostError> {
	let refused = || FileHostError::unprocessable_entity([("of", "a kind of voice note, said once")]);
	let said = |name: &str| query.iter().filter(|(key, _)| key == name).map(|(_, value)| value.as_str()).collect::<Vec<&str>>();
	let of = match said("of").as_slice() {
		[] => None,
		[name] => Some(Of::named(name).ok_or_else(refused)?),
		_ => return Err(refused()),
	};
	let unnamed = || FileHostError::unprocessable_entity([("note", "a voice note's id, said once, with of")]);
	let also = match said("note").as_slice() {
		[] => None,
		[id] if of.is_some() && voice_repo::is_note_id(id) => Some((*id).to_owned()),
		_ => return Err(unnamed()),
	};
	let subject = paired.subject.as_str();
	let mut tx = db.begin().await?;
	let kinds = voice_repo::kept::kinds(&mut tx, subject).await.map_err(refusal)?;
	let weeks = voice_repo::kept::weeks(&mut tx, subject).await.map_err(refusal)?;
	let notes = match of {
		Some(of) => Some(voice_repo::kept::newest(&mut tx, subject, of, SWITCHED_ON, also.as_deref()).await.map_err(refusal)?),
		None => None,
	};
	tx.commit().await?;
	Ok(Json(Kept { kinds, weeks, notes }))
}

/// `GET /voice-notes/takes[?q=<words>]`
///
/// The caller's take library (paulgsc/some-ui#2590, ST32; the owner's
/// decision 8 on paulgsc/some-ui#2248): every Studio take home keeps, newest
/// first, each with the words home heard in it, or with `q` the takes it
/// finds (`voice_repo::takes`): by meaning when the meaning index is on and
/// holds every heard take, the asked words embedded here, on the request's
/// path (`crate::meaning`); else the takes whose words hold every term of
/// it. `by` says which. It answers what each take is, never its audio, which
/// `GET /voice-notes/:id/audio` plays, and never a vector.
///
/// The query is read as its pairs, so `q` said twice is refused as a blank
/// one is; other keys are ignored.
///
/// # Errors
/// 401 without a session, 403 for a browser's (only a paired phone reads
/// it), 422 for a `q` with no term, longer than
/// `voice_repo::takes::MAX_SEARCH_CHARS` or said more than once, and 500 for
/// a storage failure.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "voice_takes_get", skip_all, fields(otel.kind = "server"))]
pub async fn get_takes(
	paired: Paired,
	State(db): State<SqlitePool>,
	State(meaning): State<Option<MeaningContext>>,
	Query(query): Query<Vec<(String, String)>>,
) -> Result<Json<Library>, FileHostError> {
	let refused = || FileHostError::unprocessable_entity([("q", "words to search for, at most 200 characters, said once")]);
	let asked: Vec<&str> = query.iter().filter(|(key, _)| key == "q").map(|(_, value)| value.as_str()).collect();
	let search = match asked.as_slice() {
		[] => None,
		[words] => {
			let search = Search::parse(words).ok_or_else(refused)?;
			Some(match meaning {
				Some(meaning) => search.with_meaning(meaning.model.model(), meaning.model.embed(words)),
				None => search,
			})
		}
		_ => return Err(refused()),
	};
	let mut conn = db.acquire().await?;
	let library = voice_repo::takes::library(&mut conn, paired.subject.as_str(), search.as_ref()).await.map_err(refusal)?;
	Ok(Json(library))
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

/// `GET /voice-notes/:id/station`: where the caller's Studio take `id` is.
///
/// On the conveyor, with its labels and every move that put it there, oldest
/// first (paulgsc/some-ui epic #2872, CONVEYOR1; `voice_repo::conveyor`). A take
/// no move has touched is `in-the-oven`, with no history.
///
/// # Errors
/// 401 without a session, 404 for a take the caller's subject does not hold
/// (another subject's, a note of another kind or an idea's answer
/// included), and 500 for a storage failure or a move that does not read
/// back.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "voice_station_get", skip_all, fields(otel.kind = "server"))]
pub async fn get_station(subject: SubjectId, State(db): State<SqlitePool>, Path(id): Path<String>) -> Result<Json<TakeStation>, FileHostError> {
	let mut conn = db.acquire().await?;
	let held = voice_repo::conveyor::station(&mut conn, subject.as_str(), &id).await.map_err(refusal)?;
	held.map(Json).ok_or(FileHostError::NotFound)
}

/// `GET /voice-notes/conveyor`'s query: which station.
#[derive(Debug, Deserialize)]
pub struct ConveyorQuery {
	station: Option<String>,
}

/// `GET /voice-notes/conveyor?station=`
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Conveyor {
	/// The station read.
	pub station: Station,
	/// Its takes, newest move first, at most `voice_repo::conveyor::AT_STATION`.
	pub takes: Vec<AtStation>,
	/// How many takes are there in all.
	pub total: i64,
}

/// `GET /voice-notes/conveyor?station=`: the caller's Studio takes at a
/// station.
///
/// For the conveyor page and the routine's pickup
/// (`voice_repo::conveyor::at_station`). Read in one transaction, so the
/// list and the total agree.
///
/// # Errors
/// 401 without a session, 422 naming `station` when it is missing or not a
/// station, and 500 for a storage failure or labels that do not read back.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "voice_conveyor_get", skip_all, fields(otel.kind = "server"))]
pub async fn get_conveyor(subject: SubjectId, State(db): State<SqlitePool>, Query(query): Query<ConveyorQuery>) -> Result<Json<Conveyor>, FileHostError> {
	let Some(station) = query.station.as_deref().and_then(Station::parse) else {
		return Err(FileHostError::unprocessable_entity([("station", "not a station")]));
	};
	let mut tx = db.begin().await?;
	let (takes, total) = voice_repo::conveyor::at_station(&mut tx, subject.as_str(), station).await.map_err(refusal)?;
	tx.commit().await?;
	Ok(Json(Conveyor { station, takes, total }))
}

/// `POST /voice-notes/:id/station`'s body: the move.
#[derive(Debug, Deserialize)]
pub struct MoveBody {
	from: String,
	to: String,
	at: String,
	labels: Option<Vec<String>>,
	comment: Option<String>,
	issue: Option<String>,
}

/// `POST /voice-notes/:id/station`
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TakeMoved {
	/// `moved`, or `unchanged` when the take's newest move is this one.
	pub change: &'static str,
	/// Where the take is now, and every move.
	pub take: TakeStation,
}

/// What a move from a station the take is not at answers with.
const STALE: &str = "the take is not at the station this move starts from";

/// `POST /voice-notes/:id/station`: moves the caller's Studio take `id`.
///
/// The body is `{ from, to, at, labels?, comment?, issue? }`; the take moves
/// along the conveyor (paulgsc/some-ui epic #2872, CONVEYOR1;
/// `voice_repo::conveyor::advance`), and the answer is where it is now.
///
/// A move names the station it starts from, and one from a station the take
/// is not at is a `409` whose `details.station` says where it is, so a phone
/// that missed a later move cannot skip it. A retry of the take's newest
/// move is a `200` with `change: "unchanged"` that writes nothing. Nothing
/// moves back to `in-the-oven`, a take reaches `picked-up` only from
/// `ready-for-pickup` and with its `issue`, and stays there.
///
/// `BEGIN IMMEDIATE`, so the read that finds where the take is and the write
/// see one snapshot.
///
/// # Errors
/// 401 without a session, 403 from an untrusted origin, 422 naming `body`
/// when it is not a move, or each refused field (`from`, `to`, `at`,
/// `labels`, `comment`, `issue`), 404 for a take the caller's subject does
/// not hold, 409 for a move from a station the take is not at, and 500 for
/// a storage failure.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "voice_station_post", skip_all, fields(otel.kind = "server"))]
pub async fn post_station(subject: SubjectId, State(db): State<SqlitePool>, Path(id): Path<String>, body: Bytes) -> Result<Json<TakeMoved>, FileHostError> {
	let Ok(body) = serde_json::from_slice::<MoveBody>(&body) else {
		return Err(FileHostError::unprocessable_entity([("body", "not a move")]));
	};
	let asked = voice_repo::conveyor::ask(&MoveSaid {
		from: &body.from,
		to: &body.to,
		at: &body.at,
		labels: body.labels.as_deref(),
		comment: body.comment.as_deref(),
		issue: body.issue.as_deref(),
	})
	.map_err(FileHostError::unprocessable_entity)?;
	let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
	let advanced = voice_repo::conveyor::advance(&mut tx, subject.as_str(), &id, &asked, &now()).await.map_err(refusal)?;
	tx.commit().await?;
	match advanced {
		Advanced::Moved(take) => Ok(Json(TakeMoved { change: "moved", take })),
		Advanced::Unchanged(take) => Ok(Json(TakeMoved { change: "unchanged", take })),
		Advanced::Stale(at) => Err(FileHostError::ConflictNaming {
			reason: STALE,
			details: std::iter::once((Cow::Borrowed("station"), vec![Cow::Borrowed(at.as_str())])).collect(),
		}),
		Advanced::NotHeld => Err(FileHostError::NotFound),
	}
}

/// `DELETE /voice-notes/:id`
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NoteDeleted {
	/// What home keeps of the note now: its receipt.
	pub receipt: Receipt,
}

/// What a delete of a note home keeps answers with.
const SWITCHED_OFF: &str = "this kind of voice note can't be deleted until the owner switches it on";
/// What a delete of a note kept for good answers with.
const KEPT_FOR_GOOD: &str = "this voice note is kept for good";

/// `DELETE /voice-notes/:id`: deletes the caller's note `id` for good.
///
/// Its audio and what home heard in it go with it, and the answer is its
/// receipt (paulgsc/some-ui#2477, MD3). Only from a paired phone's My data,
/// and only of a kind the owner switched on
/// (`voice_repo::delete::SWITCHED_ON`: Studio takes and idea answers): every
/// other note is kept
/// and nothing is written.
///
/// Idempotent: a repeat for a note already deleted answers with the same
/// receipt, so a phone whose answer was lost learns it is gone.
///
/// `BEGIN IMMEDIATE`, so the read, the deletes and the receipt see one
/// snapshot and land together.
///
/// # Errors
/// 401 without a session, 403 for a browser's session or an untrusted
/// origin, 404 for a note the caller's subject neither holds nor deleted
/// (another subject's included), 409 for a note home keeps (its kind
/// switched off, or kept for good), and 500 for a storage failure.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "voice_note_delete", skip_all, fields(otel.kind = "server"))]
pub async fn delete_note(paired: Paired, State(db): State<SqlitePool>, Path(id): Path<String>) -> Result<Json<NoteDeleted>, FileHostError> {
	let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
	let deleted = voice_repo::delete::delete(&mut tx, paired.subject.as_str(), &id, SWITCHED_ON, &now())
		.await
		.map_err(refusal)?;
	tx.commit().await?;
	match deleted {
		Deleted::Gone(receipt) => Ok(Json(NoteDeleted { receipt })),
		Deleted::Kept(deletion::Kept::SwitchedOff) => Err(FileHostError::Conflict(SWITCHED_OFF)),
		Deleted::Kept(deletion::Kept::ForGood) => Err(FileHostError::Conflict(KEPT_FOR_GOOD)),
		Deleted::NotHeld => Err(FileHostError::NotFound),
	}
}

/// `GET /voice-notes/deleted`
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NotesDeleted {
	/// The newest receipts, newest deleted first, at most
	/// `voice_repo::delete::RECEIPTS`.
	pub receipts: Vec<Receipt>,
	/// How many receipts home keeps in all.
	pub total: i64,
}

/// `GET /voice-notes/deleted`: the receipts of the caller's notes deleted for
/// good, for My data's list of what was deleted (paulgsc/server#667,
/// paulgsc/some-ui#2477 MD3).
///
/// Each is the receipt `DELETE /voice-notes/:id` answered with: the note's
/// id and kind, when it was recorded, reached home and was deleted, its
/// size, and what went and what stayed in fixed words. A receipt holds
/// nothing said, no `ref` and no audio, so neither does this. Read in one
/// transaction, so the list and the total agree.
///
/// # Errors
/// 401 without a session, 403 for a browser's (only a paired phone reads
/// it), and 500 for a storage failure or a receipt that does not read back.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "voice_deleted_get", skip_all, fields(otel.kind = "server"))]
pub async fn get_deleted(paired: Paired, State(db): State<SqlitePool>) -> Result<Json<NotesDeleted>, FileHostError> {
	let mut tx = db.begin().await?;
	let (receipts, total) = voice_repo::delete::receipts(&mut tx, paired.subject.as_str()).await.map_err(refusal)?;
	tx.commit().await?;
	Ok(Json(NotesDeleted { receipts, total }))
}

#[cfg(test)]
mod tests {
	//! Through the real router, with real phone sessions (bearer tokens, as a
	//! paired phone sends) and a browser's.

	use crate::auth::{
		cookie::{Carrier, SessionToken},
		now, AuthContext,
	};
	use crate::meaning::MeaningContext;
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

	fn module() -> Module<TestState> {
		crate::routes::voice::voice()
	}

	async fn state() -> TestState {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();
		TestState {
			auth: AuthContext::for_tests(pool.clone(), 100),
			pool,
			meaning: None,
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
		assert_eq!(module().descriptors().count(), 12);
		// A route named under `/voice-notes/` takes that name from the notes'
		// ids, so `voice_repo` must refuse it as one (`ROUTE_NAMES`).
		for route in module().descriptors() {
			let name = route.path.strip_prefix("/voice-notes/").unwrap();
			if !name.contains('/') && !name.starts_with(':') {
				assert!(!voice_repo::is_note_id(name), "{name} is a route and must not be a note's id");
			}
		}
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

	async fn post(app: &Router, path: &str, auth: Option<&(HeaderName, String)>, body: &Value) -> (StatusCode, Value) {
		let mut request = Request::builder().method(Method::POST).uri(path).header(CONTENT_TYPE, "application/json");
		if let Some((name, value)) = auth {
			request = request.header(name, value);
		}
		let response = app.clone().oneshot(request.body(Body::from(body.to_string())).unwrap()).await.unwrap();
		let status = response.status();
		let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
		(status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
	}

	/// CONVEYOR1 (paulgsc/some-ui epic #2872): a take lands in the oven, moves
	/// only from where it is, a retry changes nothing, and the station lists
	/// follow. Another subject's take, an unknown one and a bad move are
	/// refused and write nothing.
	#[tokio::test]
	async fn the_owner_moves_a_take_along_the_conveyor() {
		const TAKE_PATH: &str = "/voice-notes/t1?of=studio&ref=ep05:s1:1&at=2026-11-12T18:20:00Z";
		const STATION: &str = "/voice-notes/t1/station";
		let state = state().await;
		let pool = state.pool.clone();
		let phone = session(&state, A, Carrier::Bearer).await;
		let other = session(&state, B, Carrier::Bearer).await;
		let app = app(state);
		assert_eq!(put(&app, TAKE_PATH, Some(&phone), "audio/webm", TAKE).await.0, StatusCode::OK);
		let written = || async { sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM studio_take_move").fetch_one(&pool).await.unwrap() };

		assert_eq!(get(&app, STATION, None).await.0, StatusCode::UNAUTHORIZED);
		assert_eq!(
			get(&app, STATION, Some(&phone)).await,
			(StatusCode::OK, json!({ "id": "t1", "station": "in-the-oven", "labels": [], "history": [] }))
		);
		assert_eq!(
			get(&app, "/voice-notes/conveyor?station=in-the-oven", Some(&phone)).await,
			(
				StatusCode::OK,
				json!({ "station": "in-the-oven", "total": 1, "takes": [
					{ "id": "t1", "ref": "ep05:s1:1", "recordedAt": "2026-11-12T18:20:00.000Z", "movedAt": null, "labels": [], "issue": null }
				] })
			)
		);
		for bad in ["/voice-notes/conveyor", "/voice-notes/conveyor?station=oven"] {
			let (status, refused) = get(&app, bad, Some(&phone)).await;
			assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{bad}");
			assert!(refused["error"]["details"]["station"].is_array(), "{refused}");
		}

		let first = json!({ "from": "in-the-oven", "to": "cooking", "at": "2026-11-13T01:00:00-08:00", "labels": ["hook"], "comment": "opener?" });
		assert_eq!(post(&app, STATION, None, &first).await.0, StatusCode::UNAUTHORIZED);
		assert_eq!(get(&app, STATION, Some(&other)).await.0, StatusCode::NOT_FOUND, "A's take is not B's");
		assert_eq!(post(&app, STATION, Some(&other), &first).await.0, StatusCode::NOT_FOUND);
		assert_eq!(post(&app, "/voice-notes/t9/station", Some(&phone), &first).await.0, StatusCode::NOT_FOUND);
		let (status, refused) = post(&app, STATION, Some(&phone), &json!({ "from": "cooking" })).await;
		assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
		assert!(refused["error"]["details"]["body"].is_array(), "{refused}");
		let (status, refused) = post(&app, STATION, Some(&phone), &json!({ "from": "cooking", "to": "in-the-oven", "at": "soon" })).await;
		assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
		assert!(refused["error"]["details"]["to"].is_array() && refused["error"]["details"]["at"].is_array(), "{refused}");
		assert_eq!(written().await, 0, "nothing refused was written");

		let expected = json!({
			"id": "t1", "station": "cooking", "labels": ["hook"],
			"history": [{
				"seq": 1, "from": "in-the-oven", "to": "cooking", "at": "2026-11-13T09:00:00.000Z",
				"labels": ["hook"], "comment": "opener?", "issue": null, "storedAt": "x"
			}]
		});
		let normalised = |mut body: Value| {
			body["take"]["history"][0]["storedAt"] = json!("x");
			body
		};
		let (status, body) = post(&app, STATION, Some(&phone), &first).await;
		assert_eq!((status, normalised(body)), (StatusCode::OK, json!({ "change": "moved", "take": expected })));
		let (status, body) = post(&app, STATION, Some(&phone), &first).await;
		assert_eq!((status, normalised(body)), (StatusCode::OK, json!({ "change": "unchanged", "take": expected })), "a retry");
		assert_eq!(written().await, 1);

		let behind = json!({ "from": "in-the-oven", "to": "canned", "at": "2026-11-13T10:00:00Z" });
		let (status, refused) = post(&app, STATION, Some(&phone), &behind).await;
		assert_eq!(status, StatusCode::CONFLICT);
		assert_eq!(refused["error"]["details"]["station"], json!(["cooking"]), "{refused}");
		assert_eq!(written().await, 1);

		let ready = json!({ "from": "cooking", "to": "ready-for-pickup", "at": "2026-11-13T10:00:00Z" });
		assert_eq!(post(&app, STATION, Some(&phone), &ready).await.0, StatusCode::OK);
		let (status, listed) = get(&app, "/voice-notes/conveyor?station=ready-for-pickup", Some(&phone)).await;
		assert_eq!((status, listed["total"].clone()), (StatusCode::OK, json!(1)));
		assert_eq!(listed["takes"][0]["labels"], json!(["hook"]), "labels carry forward");
		assert_eq!(get(&app, "/voice-notes/conveyor?station=in-the-oven", Some(&phone)).await.1["total"], json!(0));
		assert_eq!(get(&app, "/voice-notes/conveyor?station=ready-for-pickup", Some(&other)).await.1["total"], json!(0));
		let library = get(&app, "/voice-notes/takes", Some(&phone)).await.1;
		assert_eq!(
			(library["takes"][0]["station"].clone(), library["takes"][0]["stationAt"].clone()),
			(json!("ready-for-pickup"), json!("2026-11-13T10:00:00.000Z")),
			"the take library says where each take is"
		);
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
	/// `ETag` and a `304` for it. No session is a 401, a browser's a 403, and
	/// another subject's note a 404. Which notes are read is `voice_repo`'s
	/// tests.
	#[tokio::test]
	async fn a_paired_phone_reads_its_own_rust_round_notes_audio_back() {
		use axum::http::header::{ETAG, IF_NONE_MATCH};
		use base64::{engine::general_purpose::STANDARD, Engine};
		const ROUND_NOTE: &str = "/voice-notes/r1?of=rust-round&ref=indexing-01:a&at=2026-11-02T18:20:00Z";
		let state = state().await;
		let phone = session(&state, A, Carrier::Bearer).await;
		let browser = session(&state, A, Carrier::Cookie).await;
		let other = session(&state, B, Carrier::Bearer).await;
		let app = app(state);
		let (status, answer) = put(&app, ROUND_NOTE, Some(&phone), "audio/webm;codecs=opus", TAKE).await;
		assert_eq!(status, StatusCode::OK, "{answer}");

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
	}

	/// ST32 (paulgsc/some-ui#2590; the owner's decision 8 on
	/// paulgsc/some-ui#2248): a Studio take is played from home too, to a
	/// paired phone, while every other kind but a Rust Rounds note stays home.
	/// Who may read it is the Rust Rounds test's.
	#[tokio::test]
	async fn a_paired_phone_plays_its_studio_take_from_home_and_no_other_kind() {
		use base64::{engine::general_purpose::STANDARD, Engine};
		let state = state().await;
		let phone = session(&state, A, Carrier::Bearer).await;
		let app = app(state);
		for path in ["/voice-notes/t1?of=studio&ref=library:t1&at=2026-11-04T18:20:00Z", NOTE] {
			let (status, answer) = put(&app, path, Some(&phone), "audio/webm;codecs=opus", TAKE).await;
			assert_eq!(status, StatusCode::OK, "{answer}");
		}

		let (status, body) = get(&app, "/voice-notes/t1/audio", Some(&phone)).await;
		assert_eq!(status, StatusCode::OK, "{body}");
		assert_eq!(
			body,
			json!({ "id": "t1", "contentType": "audio/webm", "sha256": TAKE_SHA256, "audio": STANDARD.encode(TAKE) })
		);
		assert_eq!(
			get(&app, "/voice-notes/n1/audio", Some(&phone)).await.0,
			StatusCode::NOT_FOUND,
			"a reflection's audio stays home"
		);
	}

	/// ST32 (paulgsc/some-ui#2590): the take library lists the caller's
	/// Studio takes with what home heard in each, searches their words, and
	/// is a paired phone's only. Which takes and words are chosen is
	/// `voice_repo::takes`'s tests.
	#[tokio::test]
	async fn a_paired_phone_reads_and_searches_its_take_library() {
		let state = state().await;
		let pool = state.pool.clone();
		let phone = session(&state, A, Carrier::Bearer).await;
		let browser = session(&state, A, Carrier::Cookie).await;
		let other = session(&state, B, Carrier::Bearer).await;
		let app = app(state);
		for path in [
			"/voice-notes/t1?of=studio&ref=ep04:s1:1&at=2026-11-04T18:20:00Z",
			"/voice-notes/t2?of=studio&ref=library:t2&at=2026-11-04T18:21:00Z",
			NOTE,
		] {
			let (status, answer) = put(&app, path, Some(&phone), "audio/webm;codecs=opus", TAKE).await;
			assert_eq!(status, StatusCode::OK, "{answer}");
		}
		sqlx::query(
			"INSERT INTO voice_transcript
			   (subject_id, note_id, model, engine, attempt, outcome, language, language_p, duration_ms, text, took_ms, heard_at)
			 VALUES (?1, 't1', 'small', 'whisper.cpp', 1, 'heard', 'en', 0.9, 1000, 'the brand intro', 5, '2026-11-04T18:30:00Z')",
		)
		.bind(A)
		.execute(&pool)
		.await
		.unwrap();

		let (status, all) = get(&app, "/voice-notes/takes", Some(&phone)).await;
		assert_eq!(status, StatusCode::OK, "{all}");
		assert_eq!(
			all,
			json!({
				"takes": [
					{ "id": "t2", "ref": "library:t2", "recordedAt": "2026-11-04T18:21:00.000Z", "storedAt": all["takes"][0]["storedAt"],
						"size": TAKE.len(), "setAsideAt": null, "heard": null, "station": "in-the-oven", "stationAt": null },
					{ "id": "t1", "ref": "ep04:s1:1", "recordedAt": "2026-11-04T18:20:00.000Z", "storedAt": all["takes"][1]["storedAt"],
						"size": TAKE.len(), "setAsideAt": null,
						"heard": { "text": "the brand intro", "language": "en", "model": "small", "heardAt": "2026-11-04T18:30:00Z" },
						"station": "in-the-oven", "stationAt": null }
				],
				"total": 2, "matched": null, "by": "newest"
			}),
			"the reflection is no take"
		);
		let (status, found) = get(&app, "/voice-notes/takes?q=Brand", Some(&phone)).await;
		assert_eq!(status, StatusCode::OK, "{found}");
		assert_eq!(
			(&found["takes"][0]["id"], &found["total"], &found["matched"], &found["by"]),
			(&json!("t1"), &json!(2), &json!(1), &json!("words"))
		);
		assert_eq!(found["takes"].as_array().map(Vec::len), Some(1));

		for refused in ["/voice-notes/takes?q=%20", "/voice-notes/takes?q=a&q=b"] {
			let (status, body) = get(&app, refused, Some(&phone)).await;
			assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{refused}: {body}");
		}
		let long = String::from("/voice-notes/takes?q=") + &"a".repeat(voice_repo::takes::MAX_SEARCH_CHARS + 1);
		assert_eq!(get(&app, &long, Some(&phone)).await.0, StatusCode::UNPROCESSABLE_ENTITY);
		assert_eq!(get(&app, "/voice-notes/takes", None).await.0, StatusCode::UNAUTHORIZED);
		assert_eq!(get(&app, "/voice-notes/takes", Some(&browser)).await.0, StatusCode::FORBIDDEN, "a paired phone's only");
		let (status, theirs) = get(&app, "/voice-notes/takes", Some(&other)).await;
		assert_eq!((status, &theirs["total"]), (StatusCode::OK, &json!(0)), "one subject's takes are never another's");
	}

	/// ST32 (docs/take-library.md): with the meaning index on, `q` is
	/// embedded on the request's path and the takes come back nearest its
	/// meaning first, `by` saying `meaning`, with no vector in the answer;
	/// while a heard take has no vector yet, the search is by words.
	#[tokio::test]
	async fn a_paired_phone_searches_its_takes_by_meaning() {
		let mut state = state().await;
		state.meaning = Some(MeaningContext {
			model: std::sync::Arc::new(crate::meaning::tests::fixture()),
		});
		let pool = state.pool.clone();
		let phone = session(&state, A, Carrier::Bearer).await;
		let app = app(state.clone());
		for (id, path, words) in [
			("t1", "/voice-notes/t1?of=studio&ref=ep04:s1:1&at=2026-11-04T18:20:00Z", "our new logo"),
			("t2", "/voice-notes/t2?of=studio&ref=ep04:s1:2&at=2026-11-04T18:21:00Z", "한국어 again"),
		] {
			let (status, answer) = put(&app, path, Some(&phone), "audio/webm;codecs=opus", TAKE).await;
			assert_eq!(status, StatusCode::OK, "{answer}");
			sqlx::query(
				"INSERT INTO voice_transcript
				   (subject_id, note_id, model, engine, attempt, outcome, language, language_p, duration_ms, text, took_ms, heard_at)
				 VALUES (?1, ?2, 'small', 'whisper.cpp', 1, 'heard', 'en', 0.9, 1000, ?3, 5, '2026-11-04T18:30:00Z')",
			)
			.bind(A)
			.bind(id)
			.bind(words)
			.execute(&pool)
			.await
			.unwrap();
		}

		let (status, found) = get(&app, "/voice-notes/takes?q=Brand", Some(&phone)).await;
		assert_eq!(status, StatusCode::OK, "{found}");
		assert_eq!(
			(&found["by"], &found["matched"]),
			(&json!("words"), &json!(0)),
			"not indexed yet: words, and none hold `brand`"
		);

		let mut conn = pool.acquire().await.unwrap();
		for id in ["t1", "t2"] {
			voice_repo::meaning::index(&mut conn, A, id, &*state.meaning.as_ref().unwrap().model).await.unwrap();
		}
		drop(conn);
		let (status, found) = get(&app, "/voice-notes/takes?q=Brand", Some(&phone)).await;
		assert_eq!(status, StatusCode::OK, "{found}");
		assert_eq!(
			(&found["by"], &found["matched"], &found["total"], &found["takes"][0]["id"]),
			(&json!("meaning"), &json!(1), &json!(2), &json!("t1")),
			"the logo take, though it never says `brand`"
		);
		let mut keys: Vec<&String> = found["takes"][0].as_object().unwrap().keys().collect();
		keys.sort();
		assert_eq!(
			keys,
			["heard", "id", "recordedAt", "ref", "setAsideAt", "size", "station", "stationAt", "storedAt"],
			"no vector"
		);
		let (_, listed) = get(&app, "/voice-notes/takes", Some(&phone)).await;
		assert_eq!((&listed["by"], &listed["total"]), (&json!("newest"), &json!(2)));
	}

	/// A delete from a paired phone deletes a Studio take, switched on since
	/// 2026-11-04, and answers its receipt, again on a repeat; a note kept
	/// for good is a 409 and stays whole. No session is a 401, a browser's a
	/// 403, and another subject's note a 404. What goes with each kind is
	/// `voice_repo::delete`'s tests.
	#[tokio::test]
	async fn a_paired_phone_deletes_a_take_and_home_keeps_every_other_kind() {
		const STUDIO_TAKE: &str = "/voice-notes/t1?of=studio&ref=build-journal-ep04:s3:2&at=2026-11-02T18:20:00Z";
		let state = state().await;
		let pool = state.pool.clone();
		let phone = session(&state, A, Carrier::Bearer).await;
		let browser = session(&state, A, Carrier::Cookie).await;
		let other = session(&state, B, Carrier::Bearer).await;
		let app = app(state);
		for path in [NOTE, STUDIO_TAKE] {
			let (status, answer) = put(&app, path, Some(&phone), "audio/webm;codecs=opus", TAKE).await;
			assert_eq!(status, StatusCode::OK, "{answer}");
		}
		let delete = |path: &'static str, auth: Option<(HeaderName, String)>| {
			let app = app.clone();
			async move {
				let mut request = Request::builder().method(Method::DELETE).uri(path);
				if let Some((name, value)) = auth {
					request = request.header(name, value);
				}
				let response = app.oneshot(request.body(Body::empty()).unwrap()).await.unwrap();
				let status = response.status();
				let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
				(status, serde_json::from_slice::<Value>(&bytes).unwrap_or(Value::Null))
			}
		};

		assert_eq!(delete("/voice-notes/t1", None).await.0, StatusCode::UNAUTHORIZED);
		assert_eq!(delete("/voice-notes/t1", Some(browser)).await.0, StatusCode::FORBIDDEN, "a paired phone's only");
		assert_eq!(
			delete("/voice-notes/t1", Some(other)).await.0,
			StatusCode::NOT_FOUND,
			"another subject's note is not theirs"
		);
		assert_eq!(
			delete("/voice-notes/n1", Some(phone.clone())).await.0,
			StatusCode::CONFLICT,
			"a reflection is kept for good"
		);
		let (status, body) = delete("/voice-notes/t1", Some(phone.clone())).await;
		assert_eq!(status, StatusCode::OK, "Studio takes are switched on: {body}");
		assert_eq!(body["receipt"]["id"], "t1");
		assert_eq!(body["receipt"]["went"], json!(["recording"]));
		assert_eq!(delete("/voice-notes/t1", Some(phone)).await, (StatusCode::OK, body), "a repeat answers with the receipt");
		let kept: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM voice_note_audio").fetch_one(&pool).await.unwrap();
		let receipts: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM voice_note_deletion").fetch_one(&pool).await.unwrap();
		assert_eq!((kept, receipts), (1, 1));
	}

	/// A note deleted for good is refused when the phone sends it again,
	/// and nothing is written: its receipt stands. The refusal has its own
	/// code, so the phone lets its copy go rather than reading it as another
	/// recording under the same name (the generic `conflict`).
	#[tokio::test]
	async fn a_note_deleted_for_good_is_not_kept_again() {
		let state = state().await;
		let pool = state.pool.clone();
		let phone = session(&state, A, Carrier::Bearer).await;
		let app = app(state);
		sqlx::query(
			"INSERT INTO voice_note_deletion (subject_id, note_id, of_kind, recorded_at, size, stored_at, deleted_at, went, stayed)
			 VALUES (?1, 'n1', 'reflection', 'then', 1, 'then', 'now', '[\"recording\"]', '[]')",
		)
		.bind(A)
		.execute(&pool)
		.await
		.unwrap();
		let (status, body) = put(&app, NOTE, Some(&phone), "audio/webm", TAKE).await;
		assert_eq!(status, StatusCode::CONFLICT, "{body}");
		assert_eq!(body["error"]["code"], "deleted_for_good", "{body}");
		let held: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM voice_note").fetch_one(&pool).await.unwrap();
		assert_eq!(held, 0);
	}

	/// The receipts read answers the caller's own receipts, as `DELETE`
	/// answered them, with the total; another subject's are not named. No
	/// session is a 401, a browser's a 403. Order and bound are
	/// `voice_repo::delete`'s tests.
	#[tokio::test]
	async fn a_paired_phone_reads_its_own_receipts() {
		let state = state().await;
		let pool = state.pool.clone();
		let phone = session(&state, A, Carrier::Bearer).await;
		let browser = session(&state, A, Carrier::Cookie).await;
		let other = session(&state, B, Carrier::Bearer).await;
		let app = app(state);
		assert_eq!(
			get(&app, "/voice-notes/deleted", Some(&phone)).await,
			(StatusCode::OK, json!({ "receipts": [], "total": 0 }))
		);
		for (subject, id) in [(A, "t1"), (B, "t2")] {
			sqlx::query(
				"INSERT INTO voice_note_deletion (subject_id, note_id, of_kind, recorded_at, size, stored_at, deleted_at, went, stayed)
				 VALUES (?1, ?2, 'studio', '2026-11-01T10:00:00.000Z', 120, '2026-11-01T10:05:00.000Z', '2026-11-03T09:00:00.000Z',
				         '[\"recording\",\"words\"]', '[\"drive copy\"]')",
			)
			.bind(subject)
			.bind(id)
			.execute(&pool)
			.await
			.unwrap();
		}
		let (status, body) = get(&app, "/voice-notes/deleted", Some(&phone)).await;
		assert_eq!(status, StatusCode::OK, "{body}");
		assert_eq!(
			body,
			json!({ "receipts": [{
				"id": "t1", "of": "studio", "recordedAt": "2026-11-01T10:00:00.000Z", "size": 120,
				"storedAt": "2026-11-01T10:05:00.000Z", "deletedAt": "2026-11-03T09:00:00.000Z",
				"went": ["recording", "words"], "stayed": ["drive copy"]
			}], "total": 1 })
		);
		assert_eq!(get(&app, "/voice-notes/deleted", None).await.0, StatusCode::UNAUTHORIZED);
		assert_eq!(get(&app, "/voice-notes/deleted", Some(&browser)).await.0, StatusCode::FORBIDDEN, "a paired phone's only");
		let (status, theirs) = get(&app, "/voice-notes/deleted", Some(&other)).await;
		assert_eq!((status, theirs["total"].clone()), (StatusCode::OK, json!(1)), "B's own, not A's");
	}

	/// The kept read answers the caller's own notes, by kind and week, and a
	/// kind's newest with `of`; descriptions only. Counting is
	/// `voice_repo::kept`'s tests.
	#[tokio::test]
	async fn a_paired_phone_reads_what_home_keeps_of_its_own_notes() {
		let state = state().await;
		let phone = session(&state, A, Carrier::Bearer).await;
		let browser = session(&state, A, Carrier::Cookie).await;
		let other = session(&state, B, Carrier::Bearer).await;
		let app = app(state);
		let (status, answer) = put(&app, NOTE, Some(&phone), "audio/webm;codecs=opus", TAKE).await;
		assert_eq!(status, StatusCode::OK, "{answer}");

		let (status, body) = get(&app, "/voice-notes/kept", Some(&phone)).await;
		assert_eq!(status, StatusCode::OK, "{body}");
		assert_eq!(
			body["kinds"],
			json!([{ "of": "reflection", "notes": 1, "bytes": TAKE.len(), "setAside": 0, "since": "2026-10-25T03:10:00.000Z" }])
		);
		assert_eq!(body["weeks"].as_array().map(Vec::len), Some(1));
		assert_eq!(body["notes"], Value::Null);

		let (status, body) = get(&app, "/voice-notes/kept?of=reflection", Some(&phone)).await;
		assert_eq!(status, StatusCode::OK, "{body}");
		let notes = body["notes"].as_array().cloned().unwrap_or_default();
		assert_eq!(notes.len(), 1, "{body}");
		assert!(notes[0]["storedAt"].is_string(), "{body}");
		let mut listed = notes[0].clone();
		listed.as_object_mut().unwrap().remove("storedAt");
		assert_eq!(
			listed,
			json!({ "id": "n1", "recordedAt": "2026-10-25T03:10:00.000Z", "size": TAKE.len(), "setAsideAt": null, "deletable": false })
		);

		let (status, body) = get(&app, "/voice-notes/kept", Some(&other)).await;
		assert_eq!(status, StatusCode::OK, "{body}");
		assert_eq!(body["kinds"], json!([]), "another subject's notes are not counted");
		assert_eq!(get(&app, "/voice-notes/kept?of=diary", Some(&phone)).await.0, StatusCode::UNPROCESSABLE_ENTITY);
		assert_eq!(get(&app, "/voice-notes/kept?of=lens&of=studio", Some(&phone)).await.0, StatusCode::UNPROCESSABLE_ENTITY);
		for bad in [
			"/voice-notes/kept?note=n1",
			"/voice-notes/kept?of=reflection&note=n1&note=n2",
			"/voice-notes/kept?of=reflection&note=a%20b",
		] {
			assert_eq!(get(&app, bad, Some(&phone)).await.0, StatusCode::UNPROCESSABLE_ENTITY, "{bad}");
		}
		let (status, body) = get(&app, "/voice-notes/kept?of=reflection&note=n1", Some(&phone)).await;
		assert_eq!(
			(status, body["notes"].as_array().map(Vec::len)),
			(StatusCode::OK, Some(1)),
			"a listed note is not listed twice"
		);
		assert_eq!(get(&app, "/voice-notes/kept", None).await.0, StatusCode::UNAUTHORIZED);
		assert_eq!(get(&app, "/voice-notes/kept", Some(&browser)).await.0, StatusCode::FORBIDDEN, "a paired phone's only");
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

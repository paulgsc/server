//! Handlers for the interview week (paulgsc/some-ui#2935, IW4, under epic
//! #2931): `routes::interview_weeks`' validate-and-answer half.
//!
//! **The week is the routine's.** The Monday routine puts it in the
//! mailbox's `brief` folder as `interview-week-<id>.json`, and the mailbox
//! pass keeps it in `mailbox_item` as it keeps every `brief` file; these
//! handlers only read it, as a week of Rust Rounds is read
//! (`handlers::rust_rounds`), and there is no `PUT` for one. A file under a
//! week's name that does not read as that week (`rust_round_repo::interview::read`)
//! is no week: left out of the list, a `404` by id.
//!
//! **What the owner sent is a draft until the deadline, final at it,** by
//! Rust Rounds' rules (RR22), which `rust_round_repo::draft` keeps once for
//! both kinds of week: each send before `due` replaces the last, a send after
//! it is refused, and the draft standing at `due` is written as final at the
//! first read after it (the record, a send refused for it, or the mailbox
//! pass's interview index, `mailbox::index::interview`).
//!
//! **A week's record, silence included**: each question answered or not and
//! each card read or not, with its take (a voice note, by id, never audio),
//! read from the week and the send that stands. Not answering is an answer,
//! so the record names every question and card no take answered whether or
//! not the phone ever sent anything.
//!
//! **Everything is the caller's.** Every handler takes a [`Paired`] phone and
//! reads and writes only under its subject: another subject's week is a
//! `404`, and its sends are not there (docs/identity.md invariant 28). The
//! extractor holds off account deletion while a handler runs (invariant 9).
//! Nothing of a body reaches a log line: the spans skip every argument
//! (invariant 6).

use crate::handlers::rust_rounds::{mailbox_failure, now, tagged, unreadable, WeekEntry, WeekList};
use crate::handlers::rust_week_record::SentAt;
use crate::subject::Paired;
use crate::FileHostError;
use axum::{
	body::Bytes,
	extract::{Path, State},
	http::{HeaderMap, StatusCode},
	response::{IntoResponse, Response},
	Json,
};
use rust_round_repo::draft::{self, DraftChange, Kind};
use rust_round_repo::interview::{self, Sent, Week};
use rust_round_repo::sent::SENT_TWICE;
use rust_round_repo::{instant_order, RoundError, LIST_LIMIT};
use serde::Serialize;
use serde_json::value::RawValue;
use sqlx::{Connection, SqliteConnection, SqlitePool};
use std::cmp::Ordering;
use tracing::instrument;

/// `GET /interview-weeks/:id`
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HeldInterviewWeek {
	pub id: String,
	pub sha256: String,
	pub received_at: String,
	/// The week, byte for byte as the routine put it.
	pub week: Box<RawValue>,
}

/// `PUT /interview-weeks/:id/sent`.
#[derive(Debug, Serialize)]
pub struct SentWritten {
	/// `added`, `replaced`, `unchanged` or `superseded`.
	pub change: &'static str,
	/// The week's draft as home keeps it after this send: this send, or for
	/// `superseded` the later one that stands.
	pub sent: Sent,
}

/// `GET /interview-weeks/:id/record`: the week's record, silence included.
/// The mailbox's interview index carries each final week in this shape.
#[derive(Debug, Serialize)]
pub struct InterviewRecord {
	pub week: String,
	/// When the week opened, as the week writes it.
	pub opens: String,
	/// The deadline, as the week writes it.
	pub due: String,
	/// Home's clock is past `due`: nothing sent after it counts.
	pub closed: bool,
	/// The week's send that stands: its draft before the deadline, its final
	/// after it, or null when home holds none.
	pub sent: Option<SentAt>,
	/// Each question, in the week's order.
	pub questions: Vec<QuestionRecord>,
	/// Each teleprompter card, in the week's order.
	pub cards: Vec<CardRecord>,
	/// The questions no take answered, in the week's order: every one when
	/// nothing was sent.
	pub unanswered: Vec<String>,
	/// The cards no take read, in the week's order.
	pub unread: Vec<String>,
}

/// One question of a week's record.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuestionRecord {
	pub question: String,
	pub answered: bool,
	/// The take's voice-note id, when answered.
	pub take: Option<String>,
	/// When the take was recorded, when answered.
	pub recorded_at: Option<String>,
}

/// One teleprompter card of a week's record.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CardRecord {
	pub card: String,
	pub read: bool,
	/// The take's voice-note id, when read.
	pub take: Option<String>,
	/// When the take was recorded, when read.
	pub recorded_at: Option<String>,
}

/// `body` as the week its name says, `id`, or `None`, logged by kind, never
/// by body: what the routine put is the routine's.
pub(crate) fn reads_as_named(id: &str, body: &[u8]) -> Option<Week> {
	match interview::read(body) {
		Ok(week) if week.id == id => Some(week),
		read => {
			let kind = if read.is_ok() { "misnamed" } else { "unreadable" };
			tracing::error!(kind, "a file held under an interview week's name is not that week; left out");
			None
		}
	}
}

/// The week `id` under `subject`, with its file, or a `404`.
async fn held_week(conn: &mut SqliteConnection, subject: &str, id: &str) -> Result<(Week, mailbox_repo::WeekFile), FileHostError> {
	if !rust_round_repo::week::is_week_id(id) {
		return Err(FileHostError::NotFound);
	}
	let held = mailbox_repo::rust_week(conn, subject, &interview::mailbox_name(id))
		.await
		.map_err(mailbox_failure)?
		.ok_or(FileHostError::NotFound)?;
	let week = reads_as_named(id, &held.body).ok_or(FileHostError::NotFound)?;
	Ok((week, held))
}

/// Whether home's clock, at `now`, is past `week`'s deadline. A week with no
/// deadline home can read is never closed.
pub(crate) fn is_closed(week: &Week, now: &str) -> bool {
	week.window.as_ref().is_some_and(|window| instant_order(now, &window.due) == Some(Ordering::Greater))
}

/// Make `week` final for `subject` when home's clock is past its deadline,
/// in the caller's transaction.
pub(crate) async fn finalize(conn: &mut SqliteConnection, subject: &str, week: &Week, now: &str) -> Result<(), sqlx::Error> {
	if let Some(window) = &week.window {
		draft::finalize(conn, Kind::Interview, subject, &week.id, &window.due, now).await?;
	}
	Ok(())
}

/// `GET /interview-weeks`: the weeks the routine put in the mailbox and home
/// read, under the caller's subject, newest first.
///
/// # Errors
/// 401 without a session, 403 for a browser's, 500 for a storage failure.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "interview_weeks", skip_all, fields(otel.kind = "server"))]
pub async fn list_weeks(paired: Paired, State(db): State<SqlitePool>) -> Result<Json<WeekList>, FileHostError> {
	let mut conn = db.acquire().await?;
	let held = mailbox_repo::weeks_named(&mut conn, paired.subject.as_str(), mailbox_repo::INTERVIEW_WEEK, -1)
		.await
		.map_err(mailbox_failure)?;
	let weeks = held
		.into_iter()
		.filter(|held| reads_as_named(&held.id, &held.body).is_some())
		.take(usize::try_from(LIST_LIMIT).unwrap_or(usize::MAX))
		.map(|held| WeekEntry {
			id: held.id,
			sha256: held.sha256,
			received_at: held.pulled_at,
		})
		.collect();
	Ok(Json(WeekList { weeks }))
}

/// `GET /interview-weeks/:id`: the week as home keeps it, under an `ETag` of
/// its hash; `If-None-Match` answers `304`.
///
/// # Errors
/// 401 without a session, 403 for a browser's, 404 for a week the caller's
/// subject does not hold (a file under the week's name that is not that week
/// included), 500 for a storage failure.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "interview_week", skip_all, fields(otel.kind = "server"))]
pub async fn get_week(paired: Paired, State(db): State<SqlitePool>, headers: HeaderMap, Path(id): Path<String>) -> Result<Response, FileHostError> {
	let mut conn = db.acquire().await?;
	let (week, held) = held_week(&mut conn, paired.subject.as_str(), &id).await?;
	let text = String::from_utf8(held.body).map_err(|_| unreadable())?;
	let held = HeldInterviewWeek {
		id: week.id,
		sha256: held.sha256,
		received_at: held.pulled_at,
		week: RawValue::from_string(text).map_err(|_| unreadable())?,
	};
	// Disallowed for tracing; this is the response body itself, hashed for
	// its tag.
	#[allow(clippy::disallowed_methods)]
	let body = serde_json::to_vec(&held)?;
	tagged(&headers, body)
}

/// `PUT /interview-weeks/:id/sent`, the body being the phone's send of the
/// week: the week's draft until its deadline, in a `BEGIN IMMEDIATE`
/// transaction. A send refused because the week is final makes it final
/// first.
///
/// # Errors
/// 401 without a session, 403 for a browser's or from an untrusted origin,
/// 404 for a week the caller's subject does not hold, 422 naming each
/// refused field (`at` after the deadline, and `week` once home's clock is
/// past it, among them), 409 for another body at the draft's moment, 500 for
/// a storage failure.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "interview_week_sent_put", skip_all, fields(otel.kind = "server"))]
pub async fn put_sent(paired: Paired, State(db): State<SqlitePool>, Path(id): Path<String>, body: Bytes) -> Result<Response, FileHostError> {
	let subject = paired.subject.as_str();
	let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
	// Read once the write lock is held, as Rust Rounds' send does: a
	// finalization that committed while this send waited did so after the
	// deadline, so this `now` is too, and the send is refused.
	let now = now();
	let (week, _) = held_week(&mut tx, subject, &id).await?;
	if draft::is_final(&mut tx, Kind::Interview, subject, &week.id).await? {
		return Err(FileHostError::unprocessable_entity(vec![(
			String::from("week"),
			"is final: its deadline has passed, and what stood then is the record",
		)]));
	}
	let sent = match interview::read_sent(&body, &week, &now) {
		Ok(sent) => sent,
		Err(problems) => {
			finalize(&mut tx, subject, &week, &now).await?;
			tx.commit().await?;
			return Err(FileHostError::unprocessable_entity(problems));
		}
	};
	let (change, sent) = draft::put_draft(&mut tx, subject, &sent, &now).await.map_err(|err| match err {
		RoundError::Rewritten => FileHostError::Conflict(SENT_TWICE),
		RoundError::Storage(err) => FileHostError::Sqlite(err),
	})?;
	tx.commit().await?;
	let status = if change == DraftChange::Added { StatusCode::CREATED } else { StatusCode::OK };
	Ok((status, Json(SentWritten { change: change.as_str(), sent })).into_response())
}

/// `GET /interview-weeks/:id/record`: the week and what stands of what was
/// sent, each question and card answered or not.
///
/// # Errors
/// 401 without a session, 403 for a browser's, 404 for a week the caller's
/// subject does not hold or whose deadline home cannot read, 500 for a
/// storage failure.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "interview_week_record", skip_all, fields(otel.kind = "server"))]
pub async fn get_record(paired: Paired, State(db): State<SqlitePool>, Path(id): Path<String>) -> Result<Json<InterviewRecord>, FileHostError> {
	let subject = paired.subject.as_str();
	let now = now();
	let mut conn = db.acquire().await?;
	let (week, _) = held_week(&mut conn, subject, &id).await?;
	if week.window.is_none() {
		tracing::error!(kind = "no_window", "an interview week home holds has no deadline it can read; no record");
		return Err(FileHostError::NotFound);
	}
	if is_closed(&week, &now) {
		let mut tx = conn.begin_with("BEGIN IMMEDIATE").await?;
		finalize(&mut tx, subject, &week, &now).await?;
		tx.commit().await?;
	}
	let record = week_record(&mut conn, subject, week, &now).await?;
	record.map(Json).ok_or(FileHostError::NotFound)
}

/// `week`'s record for `subject` with home's clock at `now`. It only reads:
/// a closed week's draft is made final by the caller first ([`finalize`]).
/// `None` for a week with no deadline home can read.
///
/// # Errors
/// Any `sqlx` failure, or a stored send that no longer reads.
pub(crate) async fn week_record(conn: &mut SqliteConnection, subject: &str, week: Week, now: &str) -> Result<Option<InterviewRecord>, sqlx::Error> {
	let closed = is_closed(&week, now);
	let Some(window) = week.window else {
		return Ok(None);
	};
	let sent = draft::standing::<Sent>(&mut *conn, subject, &week.id).await?;
	let (answers, reads) = sent.as_ref().map_or((&[][..], &[][..]), |held| (held.sent.answers.as_slice(), held.sent.reads.as_slice()));
	let mut unanswered = Vec::new();
	let questions = week
		.questions
		.into_iter()
		.map(|question| {
			let answer = answers.iter().find(|answer| answer.question == question);
			if answer.is_none() {
				unanswered.push(question.clone());
			}
			QuestionRecord {
				question,
				answered: answer.is_some(),
				take: answer.map(|answer| answer.take.clone()),
				recorded_at: answer.map(|answer| answer.recorded_at.clone()),
			}
		})
		.collect();
	let mut unread = Vec::new();
	let cards = week
		.cards
		.into_iter()
		.map(|card| {
			let read = reads.iter().find(|read| read.card == card);
			if read.is_none() {
				unread.push(card.clone());
			}
			CardRecord {
				card,
				read: read.is_some(),
				take: read.map(|read| read.take.clone()),
				recorded_at: read.map(|read| read.recorded_at.clone()),
			}
		})
		.collect();
	Ok(Some(InterviewRecord {
		week: week.id,
		opens: window.opens,
		due: window.due,
		closed,
		sent: sent.map(|held| SentAt {
			at: held.sent.at,
			received_at: held.received_at,
			draft: held.draft,
		}),
		questions,
		cards,
		unanswered,
		unread,
	}))
}

#[cfg(test)]
mod tests {
	use crate::auth::{
		cookie::{Carrier, SessionToken},
		now, AuthContext,
	};
	use crate::routes::table::Module;
	use axum::{
		body::Body,
		extract::FromRef,
		http::{
			header::{AUTHORIZATION, CONTENT_TYPE, COOKIE, ETAG, IF_NONE_MATCH},
			HeaderName, Method, Request, StatusCode,
		},
		Router,
	};
	use chrono::{Duration, SecondsFormat, Utc};
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

	fn module() -> Module<TestState> {
		crate::routes::interview_weeks::interview_weeks()
	}

	async fn state() -> TestState {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();
		TestState {
			auth: AuthContext::for_tests(pool.clone(), 100),
			pool,
		}
	}

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

	async fn call(
		app: &Router,
		method: Method,
		path: &str,
		auth: Option<&(HeaderName, String)>,
		extra: Option<(HeaderName, &str)>,
		body: &[u8],
	) -> (StatusCode, Option<String>, Value) {
		let mut request = Request::builder().method(method).uri(path).header(CONTENT_TYPE, "application/json");
		if let Some((name, value)) = auth {
			request = request.header(name, value);
		}
		if let Some((name, value)) = extra {
			request = request.header(name, value);
		}
		let response = app.clone().oneshot(request.body(Body::from(body.to_vec())).unwrap()).await.unwrap();
		let status = response.status();
		let etag = response.headers().get(ETAG).map(|value| value.to_str().unwrap().to_owned());
		let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
		(status, etag, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
	}

	/// An instant `days` from home's clock, as the phone writes one.
	fn days(days: i64) -> String {
		(Utc::now() + Duration::days(days)).to_rfc3339_opts(SecondsFormat::Millis, true)
	}

	/// A week the routine wrote, open over `opens`..`due`, kept in
	/// `subject`'s `brief` as the mailbox pass keeps one.
	async fn hold_week(pool: &SqlitePool, subject: &str, seq: i64, id: &str, (opens, due): (&str, &str)) {
		let body = json!({
			"schema": "some-ui.interview-week/1", "id": id, "opens": opens, "due": due,
			"company": { "name": "Fernbrook Labs" }, "role": "Platform engineer",
			"questions": [{ "id": "about-you", "prompt": "Tell me about yourself." }, { "id": "a-conflict", "prompt": "A time you disagreed." }],
			"cards": [{ "id": "home-first", "script": "I built home first." }],
		})
		.to_string();
		let sha256 = rust_round_repo::content_hash(body.as_bytes());
		let name = rust_round_repo::interview::mailbox_name(id);
		let item = mailbox_repo::ItemIn {
			folder: "brief",
			name: &name,
			seq,
			sha256: &sha256,
			body: body.as_bytes(),
		};
		mailbox_repo::keep_item(&mut pool.acquire().await.unwrap(), subject, item, "2026-11-16T08:00:00.000Z")
			.await
			.unwrap();
	}

	fn sent(week: &str, at: &str, recorded: &str) -> Vec<u8> {
		json!({
			"week": week, "at": at,
			"answers": [{ "question": "about-you", "take": "n1", "recordedAt": recorded }],
			"reads": [], "passed": ["a-conflict"], "passedReads": ["home-first"],
		})
		.to_string()
		.into_bytes()
	}

	/// Every route answers only a paired phone: no session is `401`, and a
	/// browser's cookie session `403`.
	#[tokio::test]
	async fn every_route_answers_a_paired_phone_only() {
		let state = state().await;
		let browser = session(&state, A, Carrier::Cookie).await;
		let app = module().into_table_router().with_state(state);
		assert_eq!(module().descriptors().count(), 4);
		for (method, path) in [
			(Method::GET, "/interview-weeks"),
			(Method::GET, "/interview-weeks/2026-w47"),
			(Method::PUT, "/interview-weeks/2026-w47/sent"),
			(Method::GET, "/interview-weeks/2026-w47/record"),
		] {
			assert_eq!(call(&app, method.clone(), path, None, None, b"{}").await.0, StatusCode::UNAUTHORIZED, "{method} {path}");
			assert_eq!(
				call(&app, method.clone(), path, Some(&browser), None, b"{}").await.0,
				StatusCode::FORBIDDEN,
				"{method} {path}"
			);
		}
	}

	/// The weeks home holds list newest first, a file under a week's name
	/// that is not that week left out; one reads back as the routine put it
	/// under an `ETag`; another subject's week and an id that is not a week
	/// are `404`s.
	#[tokio::test]
	async fn a_week_reads_back_as_the_routine_put_it_and_only_for_its_subject() {
		let state = state().await;
		let phone = session(&state, A, Carrier::Bearer).await;
		let theirs = session(&state, B, Carrier::Bearer).await;
		hold_week(&state.pool, A, 1, "2026-w46", (&days(-14), &days(-7))).await;
		hold_week(&state.pool, A, 2, "2026-w47", (&days(-1), &days(6))).await;
		hold_week(&state.pool, A, 3, "2026-w48", (&days(6), &days(13))).await;
		let misnamed = json!({ "schema": "some-ui.interview-week/1", "id": "2026-w50", "questions": [{ "id": "x" }] }).to_string();
		let sha256 = rust_round_repo::content_hash(misnamed.as_bytes());
		let item = mailbox_repo::ItemIn {
			folder: "brief",
			name: "interview-week-2026-w49.json",
			seq: 4,
			sha256: &sha256,
			body: misnamed.as_bytes(),
		};
		mailbox_repo::keep_item(&mut state.pool.acquire().await.unwrap(), A, item, "2026-11-16T08:00:00.000Z")
			.await
			.unwrap();
		let app = module().into_table_router().with_state(state);
		let (status, _, list) = call(&app, Method::GET, "/interview-weeks", Some(&phone), None, b"").await;
		assert_eq!(status, StatusCode::OK);
		let ids: Vec<&str> = list["weeks"].as_array().unwrap().iter().map(|week| week["id"].as_str().unwrap()).collect();
		assert_eq!(ids, ["2026-w48", "2026-w47", "2026-w46"]);
		let (status, etag, week) = call(&app, Method::GET, "/interview-weeks/2026-w47", Some(&phone), None, b"").await;
		assert_eq!(status, StatusCode::OK);
		assert_eq!((week["id"].as_str(), week["week"]["role"].as_str()), (Some("2026-w47"), Some("Platform engineer")));
		let etag = etag.unwrap();
		let (status, _, _) = call(&app, Method::GET, "/interview-weeks/2026-w47", Some(&phone), Some((IF_NONE_MATCH, &etag)), b"").await;
		assert_eq!(status, StatusCode::NOT_MODIFIED);
		for path in ["/interview-weeks/2026-w49", "/interview-weeks/week-47"] {
			assert_eq!(call(&app, Method::GET, path, Some(&phone), None, b"").await.0, StatusCode::NOT_FOUND, "{path}");
		}
		assert_eq!(
			call(&app, Method::GET, "/interview-weeks/2026-w47", Some(&theirs), None, b"").await.0,
			StatusCode::NOT_FOUND
		);
		let (_, _, list) = call(&app, Method::GET, "/interview-weeks", Some(&theirs), None, b"").await;
		assert_eq!(list["weeks"], json!([]));
		assert_eq!(
			call(&app, Method::GET, "/interview-weeks/2026-w47/record", Some(&theirs), None, b"").await.0,
			StatusCode::NOT_FOUND
		);
	}

	/// A send is the week's draft until its deadline, a later one replacing
	/// it, and the record names every question and card no take answered.
	#[tokio::test]
	async fn a_send_is_a_draft_and_the_record_names_each_silence() {
		let state = state().await;
		let phone = session(&state, A, Carrier::Bearer).await;
		hold_week(&state.pool, A, 1, "2026-w47", (&days(-2), &days(5))).await;
		let app = module().into_table_router().with_state(state);
		let (status, _, record) = call(&app, Method::GET, "/interview-weeks/2026-w47/record", Some(&phone), None, b"").await;
		assert_eq!(status, StatusCode::OK);
		assert_eq!(
			(&record["sent"], &record["unanswered"], &record["unread"]),
			(&json!(null), &json!(["about-you", "a-conflict"]), &json!(["home-first"]))
		);
		let first = sent("2026-w47", &days(-1), &days(-1));
		let (status, _, written) = call(&app, Method::PUT, "/interview-weeks/2026-w47/sent", Some(&phone), None, &first).await;
		assert_eq!((status, written["change"].as_str()), (StatusCode::CREATED, Some("added")));
		assert_eq!(
			call(&app, Method::PUT, "/interview-weeks/2026-w47/sent", Some(&phone), None, &first).await.2["change"],
			json!("unchanged")
		);
		let later = sent("2026-w47", &days(0), &days(-1));
		assert_eq!(
			call(&app, Method::PUT, "/interview-weeks/2026-w47/sent", Some(&phone), None, &later).await.2["change"],
			json!("replaced")
		);
		let (status, _, refused) = call(
			&app,
			Method::PUT,
			"/interview-weeks/2026-w47/sent",
			Some(&phone),
			None,
			&sent("2026-w47", &days(6), &days(-1)),
		)
		.await;
		assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{refused}");
		let (_, _, record) = call(&app, Method::GET, "/interview-weeks/2026-w47/record", Some(&phone), None, b"").await;
		assert_eq!(record["closed"], json!(false));
		assert_eq!(record["sent"]["draft"], json!(true));
		assert_eq!(
			record["questions"],
			json!([
				{ "question": "about-you", "answered": true, "take": "n1", "recordedAt": record["questions"][0]["recordedAt"] },
				{ "question": "a-conflict", "answered": false, "take": null, "recordedAt": null },
			])
		);
		assert_eq!((&record["unanswered"], &record["unread"]), (&json!(["a-conflict"]), &json!(["home-first"])));
		assert_eq!(
			call(&app, Method::PUT, "/interview-weeks/2026-w46/sent", Some(&phone), None, &first).await.0,
			StatusCode::NOT_FOUND
		);
	}

	/// Past the deadline, the first read makes the standing draft final, a
	/// send is refused, and the record says so.
	#[tokio::test]
	async fn the_draft_standing_at_the_deadline_is_final_at_the_first_read() {
		let state = state().await;
		let phone = session(&state, A, Carrier::Bearer).await;
		hold_week(&state.pool, A, 1, "2026-w46", (&days(-9), &days(-2))).await;
		let week = rust_round_repo::interview::read(
			&mailbox_repo::rust_week(&mut state.pool.acquire().await.unwrap(), A, "interview-week-2026-w46.json")
				.await
				.unwrap()
				.unwrap()
				.body,
		)
		.unwrap();
		// Sent while the week was open, as home's clock read then.
		let at = days(-3);
		let held = rust_round_repo::interview::read_sent(&sent("2026-w46", &at, &days(-4)), &week, &at).unwrap();
		rust_round_repo::draft::put_draft(&mut state.pool.acquire().await.unwrap(), A, &held, &at).await.unwrap();
		let pool = state.pool.clone();
		let app = module().into_table_router().with_state(state);
		let (status, _, record) = call(&app, Method::GET, "/interview-weeks/2026-w46/record", Some(&phone), None, b"").await;
		assert_eq!(status, StatusCode::OK);
		assert_eq!((&record["closed"], &record["sent"]["draft"]), (&json!(true), &json!(false)));
		assert!(
			rust_round_repo::draft::is_final(&mut pool.acquire().await.unwrap(), rust_round_repo::draft::Kind::Interview, A, "2026-w46")
				.await
				.unwrap()
		);
		let refused = call(&app, Method::PUT, "/interview-weeks/2026-w46/sent", Some(&phone), None, &sent("2026-w46", &at, &days(-4))).await;
		assert_eq!(refused.0, StatusCode::UNPROCESSABLE_ENTITY, "a final week takes no send");
		let finals: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM interview_week_final").fetch_one(&pool).await.unwrap();
		assert_eq!(finals, 1);
	}
}

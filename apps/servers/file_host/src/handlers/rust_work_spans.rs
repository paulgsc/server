//! Handlers for the owner's work spans on Rust Rounds (RR26).
//!
//! paulgsc/some-ui#3250: `routes::rust_rounds`' validate-and-answer half for
//! `/rust-work-spans`. What a start and a close must be, and the store's
//! rules (each written once, one span open at a time, append-only), are
//! `rust_round_repo::work_span`'s.
//!
//! **One span open at a time, visible while it is open.** A start while
//! another span is open is a `409` that carries the open span, and
//! `GET /rust-work-spans` answers the open span whatever week it is of, so a
//! second device can ask about it before starting another. Each write runs
//! in a `BEGIN IMMEDIATE` transaction, so two starts cannot both find nothing
//! open.
//!
//! **Everything is the caller's.** Every handler takes a [`Paired`] phone
//! and reads and writes only under its subject: another subject's span is
//! not there (docs/identity.md invariant 28). The extractor holds off
//! account deletion while a handler runs (invariant 9). Nothing of a body
//! reaches a log line: the spans skip every argument (invariant 6).

use crate::handlers::rust_rounds::now;
use crate::subject::Paired;
use crate::FileHostError;
use axum::{
	body::Bytes,
	extract::{Query, State},
	http::StatusCode,
	response::{IntoResponse, Response},
	Json,
};
use rust_round_repo::work_span::{self, Span, SpanError, SPAN_OPEN, SPAN_REWRITTEN};
use rust_round_repo::Change;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tracing::instrument;

/// `POST /rust-work-spans/start` and `POST /rust-work-spans/close`.
#[derive(Debug, Serialize)]
pub struct SpanWritten {
	/// `added` or `unchanged`.
	pub change: &'static str,
	/// The span as home keeps it after this write.
	pub span: Span,
}

/// `GET /rust-work-spans?week=`.
#[derive(Debug, Serialize)]
pub struct WeekSpans {
	pub week: String,
	/// The week's spans, in the order they started.
	pub spans: Vec<Span>,
	/// The span open now, whatever its week, or null.
	pub open: Option<Span>,
}

#[derive(Debug, Deserialize)]
pub struct WeekQuery {
	pub week: Option<String>,
}

/// A start refused while another span is open: the error envelope every
/// `409` carries, and the open span beside it.
#[derive(Debug, Serialize)]
struct OpenRefusal {
	error: RefusalBody,
	open: Span,
}

#[derive(Debug, Serialize)]
struct RefusalBody {
	code: &'static str,
	message: &'static str,
}

fn refused(err: SpanError) -> Result<Response, FileHostError> {
	match err {
		SpanError::Rewritten => Err(FileHostError::Conflict(SPAN_REWRITTEN)),
		SpanError::Open(open) => {
			tracing::warn!(kind = "span_open", "a start while another span is open");
			let body = OpenRefusal {
				error: RefusalBody {
					code: "conflict",
					message: SPAN_OPEN,
				},
				open: *open,
			};
			Ok((StatusCode::CONFLICT, Json(body)).into_response())
		}
		SpanError::Unknown => Err(FileHostError::NotFound),
		SpanError::Unfit(problems) => Err(FileHostError::unprocessable_entity(problems)),
		SpanError::Storage(err) => Err(FileHostError::Sqlite(err)),
	}
}

fn written(change: Change, span: Span) -> Response {
	let (status, change) = match change {
		Change::Added => (StatusCode::CREATED, "added"),
		Change::Unchanged => (StatusCode::OK, "unchanged"),
	};
	(status, Json(SpanWritten { change, span })).into_response()
}

/// `POST /rust-work-spans/start`, the body being the span's start:
/// `{ id, at, week, ticket, step, exercise, points, device }`.
///
/// # Errors
/// 401 without a session, 403 for a browser's or from an untrusted origin,
/// 422 naming each refused field, 409 for another start under the same id,
/// or, carrying `open`, while another span is open; 500 for a storage
/// failure.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "rust_work_span_start", skip_all, fields(otel.kind = "server"))]
pub async fn post_start(paired: Paired, State(db): State<SqlitePool>, body: Bytes) -> Result<Response, FileHostError> {
	let now = now();
	let start = work_span::read_start(&body, &now).map_err(FileHostError::unprocessable_entity)?;
	let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
	match work_span::start(&mut tx, paired.subject.as_str(), &start, &now).await {
		Ok((change, span)) => {
			tx.commit().await?;
			Ok(written(change, span))
		}
		Err(err) => refused(err),
	}
}

/// `POST /rust-work-spans/close`, the body being the span's close:
/// `{ id, at, stoppedBy, answer, countedSeconds }`.
///
/// # Errors
/// 401 without a session, 403 for a browser's or from an untrusted origin,
/// 404 for a span the caller's subject holds no start of, 422 naming each
/// refused field (among them a close before its start, or seconds that do
/// not fit the span and the answer), 409 for another close of the span, 500
/// for a storage failure.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "rust_work_span_close", skip_all, fields(otel.kind = "server"))]
pub async fn post_close(paired: Paired, State(db): State<SqlitePool>, body: Bytes) -> Result<Response, FileHostError> {
	let now = now();
	let close = work_span::read_close(&body, &now).map_err(FileHostError::unprocessable_entity)?;
	let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
	match work_span::close(&mut tx, paired.subject.as_str(), &close, &now).await {
		Ok((change, span)) => {
			tx.commit().await?;
			Ok(written(change, span))
		}
		Err(err) => refused(err),
	}
}

/// `GET /rust-work-spans?week=<id>`: the week's spans, each its start and
/// its close or null, and the span open now, whatever its week.
///
/// # Errors
/// 401 without a session, 403 for a browser's, 422 for a `week` missing or
/// not an ISO week, 500 for a storage failure.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "rust_work_span_list", skip_all, fields(otel.kind = "server"))]
pub async fn get_spans(paired: Paired, State(db): State<SqlitePool>, Query(query): Query<WeekQuery>) -> Result<Json<WeekSpans>, FileHostError> {
	let Some(week) = query.week.filter(|week| rust_round_repo::week::is_week_id(week)) else {
		return Err(FileHostError::unprocessable_entity([("week", "must be an ISO year and week, like 2026-w47")]));
	};
	let mut conn = db.acquire().await?;
	let spans = work_span::week_spans(&mut conn, paired.subject.as_str(), &week).await?;
	let open = work_span::open(&mut conn, paired.subject.as_str()).await?;
	Ok(Json(WeekSpans { week, spans, open }))
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
			header::{AUTHORIZATION, CONTENT_TYPE, COOKIE},
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
	const ONE: &str = "11111111-1111-4111-8111-111111111111";
	const TWO: &str = "22222222-2222-4222-8222-222222222222";

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
		crate::routes::rust_rounds::rust_rounds()
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

	async fn call(app: &Router, method: Method, path: &str, auth: Option<&(HeaderName, String)>, body: &Value) -> (StatusCode, Value) {
		let mut request = Request::builder().method(method).uri(path).header(CONTENT_TYPE, "application/json");
		if let Some((name, value)) = auth {
			request = request.header(name, value);
		}
		let body = if body.is_null() { Vec::new() } else { body.to_string().into_bytes() };
		let response = app.clone().oneshot(request.body(Body::from(body)).unwrap()).await.unwrap();
		let status = response.status();
		let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
		(status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
	}

	/// An instant `minutes` from home's clock, as the phone writes one.
	fn minutes(minutes: i64) -> String {
		(Utc::now() + Duration::minutes(minutes)).to_rfc3339_opts(SecondsFormat::Millis, true)
	}

	fn start(id: &str, at: &str, week: &str) -> Value {
		json!({
			"id": id, "at": at, "week": week, "ticket": "TB-101", "step": "read",
			"exercise": null, "points": [2], "device": "Pixel 8",
		})
	}

	fn close(id: &str, at: &str, answer: &str, counted: i64) -> Value {
		json!({ "id": id, "at": at, "stoppedBy": "you", "answer": answer, "countedSeconds": counted })
	}

	/// A span starts and closes; each again is the same answer, `200`; another
	/// body under the same span is a `409`; a second start while one is open
	/// is a `409` carrying the open span, which a second device reads.
	#[tokio::test]
	async fn a_span_starts_then_closes_and_retries_are_idempotent() {
		let state = state().await;
		let phone = session(&state, A, Carrier::Bearer).await;
		let other_device = session(&state, A, Carrier::Bearer).await;
		let theirs = session(&state, B, Carrier::Bearer).await;
		let app = module().into_table_router().with_state(state);
		let started_at = minutes(-30);

		let (status, written) = call(&app, Method::POST, "/rust-work-spans/start", Some(&phone), &start(ONE, &started_at, "2026-w47")).await;
		assert_eq!(status, StatusCode::CREATED, "{written}");
		assert_eq!(written["change"], "added");
		assert_eq!(
			(&written["span"]["id"], &written["span"]["points"], &written["span"]["exercise"], &written["span"]["closed"]),
			(&json!(ONE), &json!([2]), &Value::Null, &Value::Null)
		);
		let (status, again) = call(&app, Method::POST, "/rust-work-spans/start", Some(&phone), &start(ONE, &started_at, "2026-w47")).await;
		assert_eq!((status, &again["change"], &again["span"]), (StatusCode::OK, &json!("unchanged"), &written["span"]));
		let mut moved = start(ONE, &started_at, "2026-w47");
		moved["ticket"] = json!("TB-102");
		assert_eq!(call(&app, Method::POST, "/rust-work-spans/start", Some(&phone), &moved).await.0, StatusCode::CONFLICT);

		let (status, refusal) = call(&app, Method::POST, "/rust-work-spans/start", Some(&other_device), &start(TWO, &minutes(-1), "2026-w47")).await;
		assert_eq!(
			(status, &refusal["error"]["code"], &refusal["open"]),
			(StatusCode::CONFLICT, &json!("conflict"), &written["span"])
		);
		let (status, seen) = call(&app, Method::GET, "/rust-work-spans?week=2026-w48", Some(&other_device), &Value::Null).await;
		assert_eq!(
			(status, &seen["spans"], &seen["open"]),
			(StatusCode::OK, &json!([]), &written["span"]),
			"the open span, whatever its week"
		);
		let (_, stranger) = call(&app, Method::GET, "/rust-work-spans?week=2026-w47", Some(&theirs), &Value::Null).await;
		assert_eq!((&stranger["spans"], &stranger["open"]), (&json!([]), &Value::Null), "another subject's spans are not there");

		let ten_minutes = close(ONE, &minutes(-20), "part", 300);
		let (status, closed) = call(&app, Method::POST, "/rust-work-spans/close", Some(&phone), &ten_minutes).await;
		assert_eq!(status, StatusCode::CREATED, "{closed}");
		assert_eq!(
			(&closed["span"]["closed"]["answer"], &closed["span"]["closed"]["countedSeconds"]),
			(&json!("part"), &json!(300))
		);
		let (status, again) = call(&app, Method::POST, "/rust-work-spans/close", Some(&phone), &ten_minutes).await;
		assert_eq!((status, &again["change"], &again["span"]), (StatusCode::OK, &json!("unchanged"), &closed["span"]));
		assert_eq!(
			call(&app, Method::POST, "/rust-work-spans/close", Some(&phone), &close(ONE, &minutes(-20), "none", 0))
				.await
				.0,
			StatusCode::CONFLICT,
			"another close of a closed span"
		);
		assert_eq!(
			call(&app, Method::POST, "/rust-work-spans/close", Some(&theirs), &ten_minutes).await.0,
			StatusCode::NOT_FOUND,
			"another subject's span"
		);

		let (status, _) = call(&app, Method::POST, "/rust-work-spans/start", Some(&other_device), &start(TWO, &minutes(-1), "2026-w48")).await;
		assert_eq!(status, StatusCode::CREATED, "once the first is closed");
		let (status, week) = call(&app, Method::GET, "/rust-work-spans?week=2026-w47", Some(&phone), &Value::Null).await;
		assert_eq!(status, StatusCode::OK);
		assert_eq!((&week["week"], week["spans"].as_array().map(Vec::len)), (&json!("2026-w47"), Some(1)));
		assert_eq!(week["spans"][0], closed["span"], "each span folded: its start and its close");
		assert_eq!(week["open"]["id"], json!(TWO));
		for path in ["/rust-work-spans", "/rust-work-spans?week=w47"] {
			assert_eq!(
				call(&app, Method::GET, path, Some(&phone), &Value::Null).await.0,
				StatusCode::UNPROCESSABLE_ENTITY,
				"{path}"
			);
		}
	}

	/// A close is checked against its span: no more seconds than it held, all
	/// of them for `all`, none for `none`, and no earlier than its start; an
	/// unknown span is a `404`. A refused close leaves the span open.
	#[tokio::test]
	async fn a_close_is_checked_against_its_span() {
		let state = state().await;
		let phone = session(&state, A, Carrier::Bearer).await;
		let app = module().into_table_router().with_state(state);
		let started_at = minutes(-60);
		assert_eq!(
			call(&app, Method::POST, "/rust-work-spans/start", Some(&phone), &start(ONE, &started_at, "2026-w47"))
				.await
				.0,
			StatusCode::CREATED
		);
		let stopped_at = (chrono::DateTime::parse_from_rfc3339(&started_at).unwrap() + Duration::minutes(45)).to_rfc3339_opts(SecondsFormat::Millis, true);
		let before = (chrono::DateTime::parse_from_rfc3339(&started_at).unwrap() - Duration::seconds(1)).to_rfc3339_opts(SecondsFormat::Millis, true);
		for (body, field) in [
			(close(ONE, &stopped_at, "part", 2_701), "countedSeconds"),
			(close(ONE, &stopped_at, "all", 2_699), "countedSeconds"),
			(close(ONE, &stopped_at, "none", 60), "countedSeconds"),
			(close(ONE, &before, "none", 0), "at"),
			(close(ONE, &minutes(30), "none", 0), "at"),
		] {
			let (status, refusal) = call(&app, Method::POST, "/rust-work-spans/close", Some(&phone), &body).await;
			assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
			assert!(refusal["error"]["details"].get(field).is_some(), "{body}: {refusal}");
		}
		assert_eq!(
			call(&app, Method::POST, "/rust-work-spans/close", Some(&phone), &close(TWO, &stopped_at, "none", 0))
				.await
				.0,
			StatusCode::NOT_FOUND
		);
		let (_, week) = call(&app, Method::GET, "/rust-work-spans?week=2026-w47", Some(&phone), &Value::Null).await;
		assert_eq!(week["open"]["id"], json!(ONE), "still open");
		let (status, closed) = call(&app, Method::POST, "/rust-work-spans/close", Some(&phone), &close(ONE, &stopped_at, "all", 2_700)).await;
		assert_eq!((status, &closed["span"]["closed"]["countedSeconds"]), (StatusCode::CREATED, &json!(2_700)));
	}
}

//! Handlers for `routes::call_videos`: the weekly call's videos on the phone.
//!
//! STUDIO ST38, paulgsc/some-ui#2661, in the shape the phone read before
//! every feed had its videos (`handlers::feed_videos`, FEED4). What each does
//! is `feeds::videos`'s, for `Feed::Call`; these validate and answer. They
//! stay until the phone reads the feed routes (paulgsc/some-ui#2647).
//!
//! **A paired phone's only.** Every handler takes [`Paired`], so a request
//! without a session is `401` and a browser's session `403`: the owner
//! watches, publishes and lets go from the phone.
//!
//! **Nothing pasted reaches a log.** Every handler is
//! `#[instrument(skip_all)]`, and a link that names no `YouTube` video is
//! refused in fixed words, never echoed.

use crate::feeds::{
	videos::{self, EpisodeVideo, Folders, LetGo, LetGoError, LinkRecorded},
	Feed,
};
use crate::subject::Paired;
use crate::FileHostError;
use axum::{
	body::Body,
	extract::{Path, Request, State},
	http::{header::CONTENT_DISPOSITION, HeaderValue},
	response::{IntoResponse, Response},
	Json,
};
use call_week_repo::video::{link_of, Receipt, Step};
use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tower::ServiceExt;
use tower_http::services::ServeFile;
use tracing::instrument;

/// The 409 when a week's video is let go before its link is recorded.
pub const NO_LINK: &str = "record the week's YouTube link before its video goes";
/// The 409 when the rest of a week is let go before its video.
pub const VIDEO_FIRST: &str = "the rest of a week goes only after its video";
/// The 409 when a week with no video is let go.
pub const NOT_RENDERED: &str = "the week has no video yet";

pub(crate) fn now() -> String {
	// Fixed width, as `handlers::jobs`'s.
	Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// A week's link as the phone reads it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkView {
	/// `https://youtu.be/<id>`.
	pub url: String,
	pub recorded_at: String,
}

/// A step's receipt as the phone reads it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReceiptView {
	pub from: String,
	/// `video` or `rest`.
	pub step: &'static str,
	/// The week's link when it was let go.
	pub url: String,
	/// How many bytes went.
	pub size: i64,
	pub deleted_at: String,
}

impl From<Receipt> for ReceiptView {
	fn from(receipt: Receipt) -> Self {
		Self {
			step: receipt.step.as_str(),
			url: link_of(&receipt.youtube_id),
			from: receipt.from,
			size: receipt.size,
			deleted_at: receipt.deleted_at,
		}
	}
}

/// A week as the phone reads it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WeekView {
	pub from: String,
	pub to: String,
	/// Bytes of its video, while home has it.
	pub video_bytes: Option<u64>,
	/// Bytes of the rest of its folder, while home has it.
	pub rest_bytes: Option<u64>,
	pub link: Option<LinkView>,
	/// Each step let go, video first.
	pub receipts: Vec<ReceiptView>,
}

impl WeekView {
	fn of(week: EpisodeVideo, to: String) -> Self {
		Self {
			link: week.link.map(|link| LinkView {
				url: link_of(&link.youtube_id),
				recorded_at: link.recorded_at,
			}),
			receipts: week.receipts.into_iter().map(ReceiptView::from).collect(),
			from: week.key,
			to,
			video_bytes: week.video_bytes,
			rest_bytes: week.rest_bytes,
		}
	}
}

/// `GET /call/videos`
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Weeks {
	/// Each week home holds, newest first.
	pub weeks: Vec<WeekView>,
}

/// `GET /call/videos`: each week of the call home holds, newest first.
///
/// Each has the bytes of its video and of the rest of its folder while home
/// has them, its `YouTube` link once recorded, and a receipt for each step
/// let go.
///
/// # Errors
/// 401 without a session, 403 for a browser's, and 500 for a storage failure.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "call_videos_get", skip_all, fields(otel.kind = "server"))]
pub async fn get_weeks(_paired: Paired, State(db): State<SqlitePool>) -> Result<Json<Weeks>, FileHostError> {
	let weeks = videos::list(&db, Feed::Call, Feed::Call.renders()).await?;
	let days: std::collections::BTreeMap<String, String> = {
		let mut conn = db.acquire().await?;
		call_week_repo::held(&mut conn).await?.into_iter().map(|week| (week.from, week.to)).collect()
	};
	Ok(Json(Weeks {
		weeks: weeks
			.into_iter()
			.map(|week| {
				let to = days.get(&week.key).cloned().unwrap_or_default();
				WeekView::of(week, to)
			})
			.collect(),
	}))
}

/// `GET /call/videos/:from/video`: the week's `call.mp4`, streamed from the
/// drive (never held in memory), with ranges, as `call-<from>.mp4`.
///
/// # Errors
/// 401 without a session, 403 for a browser's, 404 for a week home doesn't
/// hold or hasn't rendered (or whose video was let go), and 500 for a
/// storage failure.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "call_videos_video", skip_all, fields(otel.kind = "server"))]
pub async fn get_video(_paired: Paired, State(db): State<SqlitePool>, Path(from): Path<String>, request: Request) -> Result<Response, FileHostError> {
	let Some(file) = videos::video_file(&db, Feed::Call, Feed::Call.renders(), &from).await? else {
		return Err(FileHostError::NotFound);
	};
	let served = match ServeFile::new(file).oneshot(request).await {
		Ok(served) => served,
		Err(never) => match never {},
	};
	let mut response = served.map(Body::new).into_response();
	let disposition = String::from("attachment; filename=\"call-") + &from + ".mp4\"";
	if let Ok(disposition) = HeaderValue::from_str(&disposition) {
		response.headers_mut().insert(CONTENT_DISPOSITION, disposition);
	}
	Ok(response)
}

/// `PUT /call/videos/:from/link`'s body.
#[derive(Debug, Deserialize)]
pub struct LinkBody {
	/// The link as pasted: `youtube.com/watch?v=…`, `youtu.be/…` or
	/// `youtube.com/shorts/…`.
	pub link: String,
}

/// `PUT /call/videos/:from/link`
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LinkKept {
	/// `https://youtu.be/<id>`: the week's link now.
	pub url: String,
}

/// `PUT /call/videos/:from/link`: records the week's `YouTube` link. Kept once
/// per video, append-only; the newest recorded is the week's.
///
/// # Errors
/// 401 without a session, 403 for a browser's, 404 for a week home doesn't
/// hold, 422 for a link that names no `YouTube` video, and 500 for a storage
/// failure.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "call_videos_link_put", skip_all, fields(otel.kind = "server"))]
pub async fn put_link(_paired: Paired, State(db): State<SqlitePool>, Path(from): Path<String>, Json(body): Json<LinkBody>) -> Result<Json<LinkKept>, FileHostError> {
	match videos::record_link(&db, Feed::Call, &from, &body.link, &now()).await? {
		LinkRecorded::Kept(id) => Ok(Json(LinkKept { url: link_of(&id) })),
		LinkRecorded::NotAVideo => Err(FileHostError::unprocessable_entity([("link", "names no YouTube video")])),
		LinkRecorded::NoEpisode => Err(FileHostError::NotFound),
	}
}

/// `DELETE /call/videos/:from/video` and `/rest`
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LetGoAnswer {
	pub receipt: ReceiptView,
}

/// `DELETE /call/videos/:from/video`: lets the week's `call.mp4` go.
///
/// From My data, once the week's `YouTube` link is recorded; the answer is
/// the receipt. A repeat answers with the same receipt and deletes nothing
/// more.
///
/// # Errors
/// 401 without a session, 403 for a browser's, 404 for a week home doesn't
/// hold, 409 for a week with no link or no video, and 500 for a storage
/// failure or a file the drive would not remove (then nothing was deleted).
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "call_videos_let_go_video", skip_all, fields(otel.kind = "server"))]
pub async fn delete_video(_paired: Paired, State(db): State<SqlitePool>, Path(from): Path<String>) -> Result<Json<LetGoAnswer>, FileHostError> {
	let_go(&db, &from, Step::Video).await
}

/// `DELETE /call/videos/:from/rest`: lets the rest of the week's folder go
/// (takes, stills, cut list), once its video has, and answers with the
/// receipt. A repeat answers with the same receipt.
///
/// # Errors
/// 401 without a session, 403 for a browser's, 404 for a week home doesn't
/// hold, 409 before the video has gone, and 500 for a storage failure or a
/// folder the drive would not remove (then nothing was deleted).
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "call_videos_let_go_rest", skip_all, fields(otel.kind = "server"))]
pub async fn delete_rest(_paired: Paired, State(db): State<SqlitePool>, Path(from): Path<String>) -> Result<Json<LetGoAnswer>, FileHostError> {
	let_go(&db, &from, Step::Rest).await
}

async fn let_go(db: &SqlitePool, from: &str, step: Step) -> Result<Json<LetGoAnswer>, FileHostError> {
	let receipt = let_go_answer(db, Feed::Call, from, step).await?;
	Ok(Json(LetGoAnswer { receipt: receipt.into() }))
}

/// One step of letting an episode of `feed` go, as the routes answer it.
///
/// # Errors
/// 404 for an episode home doesn't hold, 409 for one with no link, no video,
/// or the rest before the video, and 500 for a storage failure or a file the
/// drive would not remove.
pub(crate) async fn let_go_answer(db: &SqlitePool, feed: Feed, key: &str, step: Step) -> Result<Receipt, FileHostError> {
	let folders = Folders {
		renders: feed.renders(),
		input: feed.input(),
	};
	match videos::let_go(db, feed, folders, key, step, &now()).await {
		Ok(LetGo::Gone(receipt)) => Ok(receipt),
		Ok(LetGo::NoEpisode) => Err(FileHostError::NotFound),
		Ok(LetGo::NoLink) => Err(FileHostError::Conflict(NO_LINK)),
		Ok(LetGo::VideoFirst) => Err(FileHostError::Conflict(VIDEO_FIRST)),
		Ok(LetGo::NotRendered) => Err(FileHostError::Conflict(NOT_RENDERED)),
		Err(LetGoError::Storage(error)) => Err(FileHostError::Sqlite(error)),
		Err(LetGoError::Drive(error)) => Err(FileHostError::IoError(error)),
	}
}

#[cfg(test)]
mod tests {
	//! Through the real router, with a paired phone's session (a bearer
	//! token) and a browser's. The drive is home's real path, which a test
	//! has no week rendered in, so these hold the trust model and the
	//! refusals; `feeds::videos`'s tests hold what goes on the drive.

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
		crate::routes::call_videos::call_videos()
	}

	async fn state() -> TestState {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		crate::schema::MIGRATOR.run(&pool).await.unwrap();
		let mut conn = pool.acquire().await.unwrap();
		let week = call_week_repo::Week {
			from: "2026-10-30",
			to: "2026-11-05",
			body: b"{}",
			taken_at: "2026-11-06T08:00:00.000Z",
		};
		call_week_repo::keep(&mut conn, &week).await.unwrap();
		drop(conn);
		TestState {
			auth: AuthContext::for_tests(pool.clone(), 100),
			pool,
		}
	}

	async fn session(state: &TestState, carrier: Carrier) -> (HeaderName, String) {
		sqlx::query("INSERT OR IGNORE INTO account (subject_id, user_handle) VALUES ('subject-a', x'61')")
			.execute(&state.pool)
			.await
			.unwrap();
		let token = SessionToken::mint();
		let now = now();
		assert!(state.auth.repository().create_session(&token.hash(carrier), "subject-a", now + 3_600, now).await.unwrap());
		match carrier {
			Carrier::Bearer => (AUTHORIZATION, String::from("Bearer ") + &token.encoded()),
			Carrier::Cookie => (COOKIE, token.set_cookie(3_600).to_str().unwrap().split(';').next().unwrap().to_owned()),
		}
	}

	async fn send(app: &Router, method: Method, path: &str, auth: Option<&(HeaderName, String)>, body: Option<Value>) -> (StatusCode, Value) {
		let mut request = Request::builder().method(method).uri(path);
		if let Some((name, value)) = auth {
			request = request.header(name, value);
		}
		if body.is_some() {
			request = request.header(CONTENT_TYPE, "application/json");
		}
		let body = body.map_or_else(Body::empty, |json| Body::from(json.to_string()));
		let response = app.clone().oneshot(request.body(body).unwrap()).await.unwrap();
		let status = response.status();
		let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
		(status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
	}

	#[tokio::test]
	async fn only_a_paired_phone_reaches_the_call_videos() {
		let state = state().await;
		let (phone, browser) = (session(&state, Carrier::Bearer).await, session(&state, Carrier::Cookie).await);
		let app = module().into_table_router().with_state(state);
		assert_eq!(module().descriptors().count(), 5);

		assert_eq!(send(&app, Method::GET, "/call/videos", None, None).await.0, StatusCode::UNAUTHORIZED);
		assert_eq!(send(&app, Method::GET, "/call/videos", Some(&browser), None).await.0, StatusCode::FORBIDDEN);
		let (status, listed) = send(&app, Method::GET, "/call/videos", Some(&phone), None).await;
		assert_eq!(status, StatusCode::OK);
		assert_eq!(
			listed,
			json!({ "weeks": [{ "from": "2026-10-30", "to": "2026-11-05", "videoBytes": null, "restBytes": null, "link": null, "receipts": [] }] })
		);
		let video = "/call/videos/2026-10-30/video";
		assert_eq!(send(&app, Method::GET, video, Some(&browser), None).await.0, StatusCode::FORBIDDEN);
		assert_eq!(send(&app, Method::GET, video, Some(&phone), None).await.0, StatusCode::NOT_FOUND, "not rendered here");
		assert_eq!(send(&app, Method::DELETE, video, Some(&browser), None).await.0, StatusCode::FORBIDDEN);
	}

	#[tokio::test]
	async fn a_link_must_name_a_youtube_video_and_the_video_waits_for_one() {
		let state = state().await;
		let phone = session(&state, Carrier::Bearer).await;
		let app = module().into_table_router().with_state(state);
		let link = "/call/videos/2026-10-30/link";

		let (status, refused) = send(&app, Method::PUT, link, Some(&phone), Some(json!({ "link": "https://www.youtube.com/@me" }))).await;
		assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
		assert!(!refused.to_string().contains("@me"), "a pasted link is never echoed");
		let unheld = send(
			&app,
			Method::PUT,
			"/call/videos/2026-01-02/link",
			Some(&phone),
			Some(json!({ "link": "https://youtu.be/dQw4w9WgXcQ" })),
		);
		assert_eq!(unheld.await.0, StatusCode::NOT_FOUND);

		let no_link = send(&app, Method::DELETE, "/call/videos/2026-10-30/video", Some(&phone), None).await.0;
		assert_eq!(no_link, StatusCode::CONFLICT, "no link, nothing goes");
		assert_eq!(send(&app, Method::DELETE, "/call/videos/2026-10-30/rest", Some(&phone), None).await.0, StatusCode::CONFLICT);

		let (status, kept) = send(
			&app,
			Method::PUT,
			link,
			Some(&phone),
			Some(json!({ "link": "https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=3" })),
		)
		.await;
		assert_eq!((status, kept), (StatusCode::OK, json!({ "url": "https://youtu.be/dQw4w9WgXcQ" })));
		let (_, listed) = send(&app, Method::GET, "/call/videos", Some(&phone), None).await;
		assert_eq!(listed["weeks"][0]["link"]["url"], "https://youtu.be/dQw4w9WgXcQ");
		assert_eq!(
			send(&app, Method::DELETE, "/call/videos/2026-10-30/video", Some(&phone), None).await.0,
			StatusCode::CONFLICT,
			"no video rendered here, nothing goes"
		);
		assert_eq!(
			send(&app, Method::DELETE, "/call/videos/2026-01-02/video", Some(&phone), None).await.0,
			StatusCode::NOT_FOUND
		);
	}
}

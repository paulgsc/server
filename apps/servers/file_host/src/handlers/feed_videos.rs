//! Handlers for `routes::feed_videos`: each feed's videos on the phone.
//!
//! The call's and Playoff Delta's alike (FEED4, paulgsc/some-ui#2647). What each
//! does is `feeds::videos`'s; these validate and answer.
//!
//! **A paired phone's only.** Every handler takes [`Paired`], so a request
//! without a session is `401` and a browser's session `403`.
//!
//! **Nothing pasted reaches a log.** Every handler is
//! `#[instrument(skip_all)]`, and a link that names no `YouTube` video is
//! refused in fixed words, never echoed. A feed the routes don't know is
//! `404`, as an episode home doesn't hold is.

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

/// The 409 when an episode's video is let go before its link is recorded.
pub const NO_LINK: &str = "record the week's YouTube link before its video goes";
/// The 409 when the rest of an episode is let go before its video.
pub const VIDEO_FIRST: &str = "the rest of a week goes only after its video";
/// The 409 when an episode with no video is let go.
pub const NOT_RENDERED: &str = "the week has no video yet";

fn now() -> String {
	// Fixed width, as `handlers::jobs`'s.
	Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// An episode's link as the phone reads it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkView {
	/// `https://youtu.be/<id>`.
	pub url: String,
	pub recorded_at: String,
}

/// `PUT /feeds/:feed/videos/:key/link`'s body.
#[derive(Debug, Deserialize)]
pub struct LinkBody {
	/// The link as pasted: `youtube.com/watch?v=…`, `youtu.be/…` or
	/// `youtube.com/shorts/…`.
	pub link: String,
}

/// `PUT /feeds/:feed/videos/:key/link`
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LinkKept {
	/// `https://youtu.be/<id>`: the episode's link now.
	pub url: String,
}

/// One step of letting an episode of `feed` go, as the routes answer it.
///
/// # Errors
/// 404 for an episode home doesn't hold, 409 for one with no link, no video,
/// or the rest before the video, and 500 for a storage failure or a file the
/// drive would not remove.
async fn let_go_answer(db: &SqlitePool, feed: Feed, key: &str, step: Step) -> Result<Receipt, FileHostError> {
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

fn feed_of(name: &str) -> Result<Feed, FileHostError> {
	Feed::parse(name).ok_or(FileHostError::NotFound)
}

/// A step's receipt as the phone reads it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReceiptView {
	/// The episode's key.
	pub key: String,
	/// `video` or `rest`.
	pub step: &'static str,
	/// The episode's link when it was let go.
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
			key: receipt.from,
			size: receipt.size,
			deleted_at: receipt.deleted_at,
		}
	}
}

/// An episode as the phone reads it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EpisodeView {
	/// The call's week by its first day (`2026-10-30`), Playoff Delta's
	/// episode by `<season>-<team>-<NN>` (`2026-SF-08`).
	pub key: String,
	/// Bytes of its video, while home has it.
	pub video_bytes: Option<u64>,
	/// Bytes of the rest of its folder, while home has it.
	pub rest_bytes: Option<u64>,
	pub link: Option<LinkView>,
	/// Each step let go, video first.
	pub receipts: Vec<ReceiptView>,
}

impl From<EpisodeVideo> for EpisodeView {
	fn from(episode: EpisodeVideo) -> Self {
		Self {
			link: episode.link.map(|link| LinkView {
				url: link_of(&link.youtube_id),
				recorded_at: link.recorded_at,
			}),
			receipts: episode.receipts.into_iter().map(ReceiptView::from).collect(),
			key: episode.key,
			video_bytes: episode.video_bytes,
			rest_bytes: episode.rest_bytes,
		}
	}
}

/// `GET /feeds/:feed/videos`
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Episodes {
	/// Each episode home holds, newest first.
	pub episodes: Vec<EpisodeView>,
}

/// `GET /feeds/:feed/videos`: each episode of the feed home holds.
///
/// Newest first, with the bytes of its video and of the rest of its folder while
/// home has them, its `YouTube` link once recorded, and a receipt for each
/// step let go.
///
/// # Errors
/// 401 without a session, 403 for a browser's, 404 for a feed home doesn't
/// render, and 500 for a storage failure.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "feed_videos_get", skip_all, fields(otel.kind = "server"))]
pub async fn get_episodes(_paired: Paired, State(db): State<SqlitePool>, Path(feed): Path<String>) -> Result<Json<Episodes>, FileHostError> {
	let feed = feed_of(&feed)?;
	let episodes = videos::list(&db, feed, feed.renders()).await?;
	Ok(Json(Episodes {
		episodes: episodes.into_iter().map(EpisodeView::from).collect(),
	}))
}

/// `GET /feeds/:feed/videos/:key/video`: the episode's video, streamed from
/// the drive (never held in memory), with ranges, as `<feed>-<key>.mp4`.
///
/// # Errors
/// 401 without a session, 403 for a browser's, 404 for a feed or an episode
/// home doesn't hold or hasn't rendered (or whose video was let go), and 500
/// for a storage failure.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "feed_videos_video", skip_all, fields(otel.kind = "server"))]
pub async fn get_video(_paired: Paired, State(db): State<SqlitePool>, Path((feed, key)): Path<(String, String)>, request: Request) -> Result<Response, FileHostError> {
	let feed = feed_of(&feed)?;
	let Some(file) = videos::video_file(&db, feed, feed.renders(), &key).await? else {
		return Err(FileHostError::NotFound);
	};
	let served = match ServeFile::new(file).oneshot(request).await {
		Ok(served) => served,
		Err(never) => match never {},
	};
	let mut response = served.map(Body::new).into_response();
	// The key is one a feed's table holds: digits, capitals and dashes.
	let disposition = String::from("attachment; filename=\"") + feed.as_str() + "-" + &key + ".mp4\"";
	if let Ok(disposition) = HeaderValue::from_str(&disposition) {
		response.headers_mut().insert(CONTENT_DISPOSITION, disposition);
	}
	Ok(response)
}

/// `PUT /feeds/:feed/videos/:key/link`: records the episode's `YouTube`
/// link. Kept once per video, append-only; the newest recorded is the
/// episode's.
///
/// # Errors
/// 401 without a session, 403 for a browser's, 404 for a feed or an episode
/// home doesn't hold, 422 for a link that names no `YouTube` video, and 500
/// for a storage failure.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "feed_videos_link_put", skip_all, fields(otel.kind = "server"))]
pub async fn put_link(
	_paired: Paired,
	State(db): State<SqlitePool>,
	Path((feed, key)): Path<(String, String)>,
	Json(body): Json<LinkBody>,
) -> Result<Json<LinkKept>, FileHostError> {
	let feed = feed_of(&feed)?;
	match videos::record_link(&db, feed, &key, &body.link, &now()).await? {
		LinkRecorded::Kept(id) => Ok(Json(LinkKept { url: link_of(&id) })),
		LinkRecorded::NotAVideo => Err(FileHostError::unprocessable_entity([("link", "names no YouTube video")])),
		LinkRecorded::NoEpisode => Err(FileHostError::NotFound),
	}
}

/// `DELETE /feeds/:feed/videos/:key/video` and `/rest`
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LetGoAnswer {
	pub receipt: ReceiptView,
}

/// `DELETE /feeds/:feed/videos/:key/video`: lets the episode's video go.
///
/// From My data, once its `YouTube` link is recorded; the answer is the
/// receipt. A repeat answers with the same receipt and deletes nothing more.
///
/// # Errors
/// 401 without a session, 403 for a browser's, 404 for a feed or an episode
/// home doesn't hold, 409 for an episode with no link or no video, and 500
/// for a storage failure or a file the drive would not remove.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "feed_videos_let_go_video", skip_all, fields(otel.kind = "server"))]
pub async fn delete_video(_paired: Paired, State(db): State<SqlitePool>, Path((feed, key)): Path<(String, String)>) -> Result<Json<LetGoAnswer>, FileHostError> {
	let receipt = let_go_answer(&db, feed_of(&feed)?, &key, Step::Video).await?;
	Ok(Json(LetGoAnswer { receipt: receipt.into() }))
}

/// `DELETE /feeds/:feed/videos/:key/rest`: lets the rest of the episode's
/// folder go, once its video has, and answers with the receipt. A repeat
/// answers with the same receipt.
///
/// # Errors
/// 401 without a session, 403 for a browser's, 404 for a feed or an episode
/// home doesn't hold, 409 before the video has gone, and 500 for a storage
/// failure or a folder the drive would not remove.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "feed_videos_let_go_rest", skip_all, fields(otel.kind = "server"))]
pub async fn delete_rest(_paired: Paired, State(db): State<SqlitePool>, Path((feed, key)): Path<(String, String)>) -> Result<Json<LetGoAnswer>, FileHostError> {
	let receipt = let_go_answer(&db, feed_of(&feed)?, &key, Step::Rest).await?;
	Ok(Json(LetGoAnswer { receipt: receipt.into() }))
}

#[cfg(test)]
mod tests {
	//! Through the real router, with a paired phone's session and a browser's;
	//! `feeds::videos`' tests hold what goes on the drive.

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
		crate::routes::feed_videos::feed_videos()
	}

	async fn state() -> TestState {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		crate::schema::MIGRATOR.run(&pool).await.unwrap();
		let mut conn = pool.acquire().await.unwrap();
		let fill = pd_fill_repo::Fill {
			season: "2026",
			team: "SF",
			week: 9,
			version: 2,
			body: b"{}",
			season_body: b"{}",
			taken_at: "2026-11-10T20:00:00.000Z",
		};
		pd_fill_repo::keep(&mut conn, &fill).await.unwrap();
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
	async fn only_a_paired_phone_reaches_a_feeds_videos_and_an_unknown_feed_is_not_found() {
		let state = state().await;
		let (phone, browser) = (session(&state, Carrier::Bearer).await, session(&state, Carrier::Cookie).await);
		let app = module().into_table_router().with_state(state);
		assert_eq!(module().descriptors().count(), 5);

		let list = "/feeds/playoff-delta/videos";
		assert_eq!(send(&app, Method::GET, list, None, None).await.0, StatusCode::UNAUTHORIZED);
		assert_eq!(send(&app, Method::GET, list, Some(&browser), None).await.0, StatusCode::FORBIDDEN);
		let (status, listed) = send(&app, Method::GET, list, Some(&phone), None).await;
		assert_eq!(status, StatusCode::OK);
		assert_eq!(
			listed,
			json!({ "episodes": [{ "key": "2026-SF-09", "videoBytes": null, "restBytes": null, "link": null, "receipts": [] }] })
		);
		let (status, calls) = send(&app, Method::GET, "/feeds/call/videos", Some(&phone), None).await;
		assert_eq!((status, calls), (StatusCode::OK, json!({ "episodes": [] })), "each feed lists its own");
		assert_eq!(send(&app, Method::GET, "/feeds/drama/videos", Some(&phone), None).await.0, StatusCode::NOT_FOUND);

		let link = "/feeds/playoff-delta/videos/2026-SF-09/link";
		let (status, refused) = send(&app, Method::PUT, link, Some(&phone), Some(json!({ "link": "https://www.youtube.com/@me" }))).await;
		assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
		assert!(!refused.to_string().contains("@me"), "a pasted link is never echoed");
		let video = "/feeds/playoff-delta/videos/2026-SF-09/video";
		assert_eq!(send(&app, Method::DELETE, video, Some(&phone), None).await.0, StatusCode::CONFLICT, "no link, nothing goes");
		let (status, kept) = send(&app, Method::PUT, link, Some(&phone), Some(json!({ "link": "https://youtu.be/dQw4w9WgXcQ" }))).await;
		assert_eq!((status, kept), (StatusCode::OK, json!({ "url": "https://youtu.be/dQw4w9WgXcQ" })));
		assert_eq!(send(&app, Method::GET, video, Some(&phone), None).await.0, StatusCode::NOT_FOUND, "not rendered here");
		assert_eq!(send(&app, Method::DELETE, video, Some(&browser), None).await.0, StatusCode::FORBIDDEN);
		let unheld = send(
			&app,
			Method::PUT,
			"/feeds/playoff-delta/videos/2026-SF-10/link",
			Some(&phone),
			Some(json!({ "link": "https://youtu.be/dQw4w9WgXcQ" })),
		);
		assert_eq!(unheld.await.0, StatusCode::NOT_FOUND);
	}
}

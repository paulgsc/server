//! Handlers for `routes::clips`: the clip frames home keeps (`clips`), for
//! the paired phone.
//!
//! **A paired phone's only**, as every feed handler: it takes [`Paired`],
//! so a request without a session is `401` and a browser's session `403`.
//! A frame is served only once `clip_frame` holds it: a file on the drive
//! with no row (a pass that stopped between the two) is not home's yet.

use crate::subject::Paired;
use crate::FileHostError;
use axum::{
	body::Body,
	extract::{FromRef, Path, Request, State},
	http::{
		header::{CACHE_CONTROL, CONTENT_TYPE, ETAG},
		HeaderValue,
	},
	response::{IntoResponse, Response},
};
use sqlx::SqlitePool;
use std::path::PathBuf;
use tower::ServiceExt;
use tower_http::services::ServeFile;
use tracing::instrument;

use crate::handlers::db::activities::{etag_value, if_none_match_hits, not_modified};

/// Where the frames are: [`crate::clips::FRAMES_FOLDER`] in the server, a
/// temporary folder in tests.
#[derive(Debug, Clone)]
pub struct ClipFrames(pub PathBuf);

impl FromRef<crate::AppState> for ClipFrames {
	fn from_ref(_: &crate::AppState) -> Self {
		Self(PathBuf::from(crate::clips::FRAMES_FOLDER))
	}
}

/// `GET /clips/:id/:side/:file`: a frame home holds, `image/png`, its
/// SHA-256 as its `ETag` (a `304` for a phone that has it), cached for good:
/// a held frame never changes.
///
/// # Errors
/// 401 without a session, 403 for a browser's, 404 for a frame home doesn't
/// hold (or whose file is not on the drive), and 500 for a storage failure.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "clips_frame", skip_all, fields(otel.kind = "server"))]
pub async fn get_frame(
	_paired: Paired,
	State(db): State<SqlitePool>,
	State(ClipFrames(root)): State<ClipFrames>,
	Path((id, side, file)): Path<(String, String, String)>,
	request: Request,
) -> Result<Response, FileHostError> {
	let held = {
		let mut conn = db.acquire().await?;
		clip_frame_repo::one(&mut conn, &id, &side, &file).await?
	};
	// The row's names are the table's checked ones, so the path is three
	// plain segments under the root.
	let Some(held) = held else {
		return Err(FileHostError::NotFound);
	};
	let etag = etag_value(&held.sha256)?;
	if if_none_match_hits(request.headers(), &etag) {
		return Ok(not_modified(etag));
	}
	let path = root.join(&held.clip_id).join(&held.side).join(&held.file);
	if !tokio::fs::try_exists(&path).await.unwrap_or(false) {
		return Err(FileHostError::NotFound);
	}
	let served = match ServeFile::new(path).oneshot(request).await {
		Ok(served) => served,
		Err(never) => match never {},
	};
	let mut response = served.map(Body::new).into_response();
	let headers = response.headers_mut();
	headers.insert(CONTENT_TYPE, HeaderValue::from_static("image/png"));
	headers.insert(CACHE_CONTROL, HeaderValue::from_static("private, max-age=31536000, immutable"));
	headers.insert(ETAG, etag);
	Ok(response)
}

#[cfg(test)]
mod tests {
	//! Through the real router, with a paired phone's session and a browser's,
	//! as `handlers::renders`' tests are.

	use super::ClipFrames;
	use crate::auth::{
		cookie::{Carrier, SessionToken},
		now, AuthContext,
	};
	use axum::{
		body::Body,
		extract::FromRef,
		http::{
			header::{AUTHORIZATION, CACHE_CONTROL, CONTENT_TYPE, COOKIE, ETAG, IF_NONE_MATCH},
			HeaderName, Request, StatusCode,
		},
		Router,
	};
	use clip_frame_repo::{Frame, Made};
	use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};
	use std::path::PathBuf;
	use tower::ServiceExt;

	#[derive(Clone)]
	struct TestState {
		pool: SqlitePool,
		auth: AuthContext,
		frames: PathBuf,
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

	impl FromRef<TestState> for ClipFrames {
		fn from_ref(state: &TestState) -> Self {
			Self(state.frames.clone())
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

	async fn get(app: &Router, uri: &str, headers: &[(HeaderName, &str)]) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
		let mut request = Request::builder().uri(uri);
		for (name, value) in headers {
			request = request.header(name, *value);
		}
		let response = app.clone().oneshot(request.body(Body::empty()).unwrap()).await.unwrap();
		let (status, headers) = (response.status(), response.headers().clone());
		let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
		(status, headers, bytes.to_vec())
	}

	#[tokio::test]
	async fn a_frame_is_served_to_a_paired_phone_only() {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		crate::schema::MIGRATOR.run(&pool).await.unwrap();
		let dir = tempfile::tempdir().unwrap();
		let state = TestState {
			auth: AuthContext::for_tests(pool.clone(), 100),
			pool: pool.clone(),
			frames: dir.path().to_path_buf(),
		};
		let (phone, browser) = (session(&state, Carrier::Bearer).await, session(&state, Carrier::Cookie).await);
		let module = crate::routes::clips::clips::<TestState>();
		assert_eq!(module.descriptors().count(), 1);
		let app = module.into_table_router().with_state(state);

		// One frame held, on the drive with its row; one on the drive alone.
		let png = [&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a][..], b"frame"].concat();
		let folder = dir.path().join("ledger-wake/after");
		std::fs::create_dir_all(&folder).unwrap();
		std::fs::write(folder.join("01.png"), &png).unwrap();
		std::fs::write(folder.join("02.png"), &png).unwrap();
		let frame = Frame {
			clip_id: "ledger-wake",
			side: "after",
			file: "01.png",
			made: Made::Walk,
			bytes: &png,
			taken_at: "2026-11-11T09:00:00.000Z",
		};
		clip_frame_repo::keep(&mut pool.acquire().await.unwrap(), &frame).await.unwrap();
		let sha256 = clip_frame_repo::sha256_hex(&png);
		let phone = [(phone.0.clone(), phone.1.as_str())];

		let uri = "/clips/ledger-wake/after/01.png";
		assert_eq!(get(&app, uri, &[]).await.0, StatusCode::UNAUTHORIZED);
		assert_eq!(get(&app, uri, &[(browser.0.clone(), browser.1.as_str())]).await.0, StatusCode::FORBIDDEN);

		let (status, headers, body) = get(&app, "/clips/ledger-wake/after/01.png", &phone).await;
		assert_eq!(status, StatusCode::OK);
		assert_eq!(body, png);
		assert_eq!(headers[CONTENT_TYPE], "image/png");
		let etag = String::from("\"") + &sha256 + "\"";
		assert_eq!(headers[ETAG], etag.as_str());
		assert!(headers[CACHE_CONTROL].to_str().unwrap().contains("immutable"));

		let mut again = phone.to_vec();
		again.push((IF_NONE_MATCH, etag.as_str()));
		assert_eq!(get(&app, "/clips/ledger-wake/after/01.png", &again).await.0, StatusCode::NOT_MODIFIED);

		// Not held: on the drive with no row, unknown, or a name that isn't plain.
		for uri in [
			"/clips/ledger-wake/after/02.png",
			"/clips/ledger-wake/before/01.png",
			"/clips/other/after/01.png",
			"/clips/..%2F/after/01.png",
		] {
			assert_eq!(get(&app, uri, &phone).await.0, StatusCode::NOT_FOUND, "{uri}");
		}
	}
}

//! Handlers for `routes::cards_essays`: a cards essay's video on the phone
//! (CARDS7, #854; C338). What an essay is and where its video lives are
//! `cards_essays`'s; this validates and answers.
//!
//! **A paired phone's only.** Every handler takes [`Paired`], so a request
//! without a session is `401` and a browser's session `403`. A key that is
//! not an essay's, or an essay with no video yet, is `404`.

use crate::cards_essays::{pass::is_key, RENDERS_FOLDER, VIDEO};
use crate::subject::Paired;
use crate::FileHostError;
use axum::{
	body::Body,
	extract::{Path, Request},
	http::{header::CONTENT_DISPOSITION, HeaderValue},
	response::{IntoResponse, Response},
};
use std::path::PathBuf;
use tower::ServiceExt;
use tower_http::services::ServeFile;
use tracing::{instrument, warn};

/// The essay `key`'s video under `renders`, once the worker has made it: a
/// plain file, never a link, at `<renders>/<key>/cards.mp4`. A key that is
/// not an essay's is never joined to a path.
async fn video_file(renders: &std::path::Path, key: &str) -> Result<PathBuf, FileHostError> {
	if !is_key(key) {
		return Err(FileHostError::NotFound);
	}
	let file = renders.join(key).join(VIDEO);
	match tokio::fs::symlink_metadata(&file).await {
		Ok(meta) if meta.is_file() => Ok(file),
		Ok(_) => Err(FileHostError::NotFound),
		Err(err) if err.kind() == std::io::ErrorKind::NotFound => Err(FileHostError::NotFound),
		Err(err) => {
			// Its kind only: never a path.
			warn!(kind = ?err.kind(), "the drive refused a cards essay's video");
			Err(FileHostError::OperationError("the drive refused a cards essay's video".to_owned()))
		}
	}
}

/// `GET /cards-essays/:key/video`: the essay's video, streamed from the
/// drive (never held in memory), with ranges, as `<key>.mp4`.
///
/// # Errors
/// 401 without a session, 403 for a browser's, 404 for a key that is not an
/// essay's or an essay with no video yet, and 500 when the drive refuses.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "cards_essays_video", skip_all, fields(otel.kind = "server"))]
pub async fn get_video(_paired: Paired, Path(key): Path<String>, request: Request) -> Result<Response, FileHostError> {
	let file = video_file(std::path::Path::new(RENDERS_FOLDER), &key).await?;
	let served = match ServeFile::new(file).oneshot(request).await {
		Ok(served) => served,
		Err(never) => match never {},
	};
	let mut response = served.map(Body::new).into_response();
	// The key is checked plain: letters, digits and dashes.
	let disposition = String::from("attachment; filename=\"") + &key + ".mp4\"";
	if let Ok(disposition) = HeaderValue::from_str(&disposition) {
		response.headers_mut().insert(CONTENT_DISPOSITION, disposition);
	}
	Ok(response)
}

#[cfg(test)]
mod tests {
	use super::video_file;
	use crate::auth::{
		cookie::{Carrier, SessionToken},
		now, AuthContext,
	};
	use crate::routes::table::Module;
	use crate::FileHostError;
	use axum::{
		body::Body,
		extract::FromRef,
		http::{
			header::{AUTHORIZATION, COOKIE},
			HeaderName, Method, Request, StatusCode,
		},
		Router,
	};
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
		crate::routes::cards_essays::cards_essays()
	}

	async fn state() -> TestState {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		crate::schema::MIGRATOR.run(&pool).await.unwrap();
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

	async fn status(app: &Router, path: &str, auth: Option<&(HeaderName, String)>) -> StatusCode {
		let mut request = Request::builder().method(Method::GET).uri(path);
		if let Some((name, value)) = auth {
			request = request.header(name, value);
		}
		app.clone().oneshot(request.body(Body::empty()).unwrap()).await.unwrap().status()
	}

	#[tokio::test]
	async fn only_a_paired_phone_reaches_an_essays_video_and_a_key_that_is_not_one_is_not_found() {
		let state = state().await;
		let (phone, browser) = (session(&state, Carrier::Bearer).await, session(&state, Carrier::Cookie).await);
		let app = module().into_table_router().with_state(state);
		assert_eq!(module().descriptors().count(), 1);
		let path = "/cards-essays/nixos-upgrade-2026-11-14/video";
		assert_eq!(status(&app, path, None).await, StatusCode::UNAUTHORIZED);
		assert_eq!(status(&app, path, Some(&browser)).await, StatusCode::FORBIDDEN);
		for path in [path, "/cards-essays/NixOS-2026-11-14/video", "/cards-essays/..%2Fetc-2026-11-14/video"] {
			assert_eq!(status(&app, path, Some(&phone)).await, StatusCode::NOT_FOUND, "{path}: no video on this drive, or no key");
		}
	}

	#[tokio::test]
	async fn an_essay_s_video_is_its_plain_file_once_made() {
		let dir = tempfile::tempdir().unwrap();
		let renders = dir.path();
		let not_found = |result: Result<std::path::PathBuf, FileHostError>| matches!(result, Err(FileHostError::NotFound));
		assert!(not_found(video_file(renders, "nixos-upgrade-2026-11-14").await), "not made yet");
		std::fs::create_dir_all(renders.join("nixos-upgrade-2026-11-14")).unwrap();
		std::fs::write(renders.join("nixos-upgrade-2026-11-14").join("cards.mp4"), b"mp4").unwrap();
		assert_eq!(
			video_file(renders, "nixos-upgrade-2026-11-14").await.unwrap(),
			renders.join("nixos-upgrade-2026-11-14").join("cards.mp4")
		);
		std::fs::create_dir_all(renders.join("zfs-2026-11-02")).unwrap();
		std::os::unix::fs::symlink(renders.join("nixos-upgrade-2026-11-14").join("cards.mp4"), renders.join("zfs-2026-11-02").join("cards.mp4")).unwrap();
		assert!(not_found(video_file(renders, "zfs-2026-11-02").await), "a link is never followed");
		std::fs::create_dir_all(renders.join("..").join("x-2026-11-14")).ok();
		assert!(not_found(video_file(renders, "../x-2026-11-14").await), "not a key, never joined");
	}
}

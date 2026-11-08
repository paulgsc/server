//! Handlers for `routes::previews`: a template's preview on the phone
//! (SFEED12, paulgsc/some-ui#2757). What each does is `previews`'s; these
//! validate and answer.
//!
//! **A paired phone's only.** Every handler takes [`Paired`], so a request
//! without a session is `401` and a browser's session `403`. A template home
//! makes no preview of is `404`, as is a video that is not its template's
//! current, ready one.

use crate::previews::{self, Asked, Folders, Preview, State};
use crate::subject::Paired;
use crate::FileHostError;
use axum::{
	body::Body,
	extract::{Path, Request},
	http::{header::CONTENT_DISPOSITION, HeaderValue, StatusCode},
	response::{IntoResponse, Response},
	Json,
};
use serde::Serialize;
use std::time::SystemTime;
use tower::ServiceExt;
use tower_http::services::ServeFile;
use tracing::{instrument, warn};

fn template_of(name: &str) -> Result<&'static str, FileHostError> {
	previews::TEMPLATES.iter().copied().find(|known| *known == name).ok_or(FileHostError::NotFound)
}

fn drive(err: &std::io::Error) -> FileHostError {
	// Its kind only: never a path.
	warn!(kind = ?err.kind(), "the drive refused a preview's files");
	FileHostError::OperationError("the drive refused a preview's files".to_owned())
}

/// `POST /previews/:template`
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Asking {
	pub id: String,
	pub state: State,
}

/// `GET /previews/:template`
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Current {
	/// The template's newest preview, or `null` when it has none, or its
	/// newest is past its day.
	pub preview: Option<Preview>,
}

/// The answer to a request of `template` at `now`: `202` for a new preview,
/// `200` with the one still to be made.
async fn asked(folders: &Folders, template: &str, now: SystemTime) -> Result<(StatusCode, Json<Asking>), FileHostError> {
	Ok(match previews::request(folders, template, now).await.map_err(|err| drive(&err))? {
		Asked::New(preview) => (
			StatusCode::ACCEPTED,
			Json(Asking {
				id: preview.id,
				state: preview.state,
			}),
		),
		Asked::Pending(preview) => (
			StatusCode::OK,
			Json(Asking {
				id: preview.id,
				state: preview.state,
			}),
		),
	})
}

/// `POST /previews/:template`: asks home for a preview of the template, made
/// from its sample under the lane, unless one is still to be made.
///
/// # Errors
/// 401 without a session, 403 for a browser's, 404 for a template home makes
/// no preview of, and 500 when the drive refuses.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "previews_post", skip_all, fields(otel.kind = "server"))]
pub async fn post_preview(_paired: Paired, Path(template): Path<String>) -> Result<(StatusCode, Json<Asking>), FileHostError> {
	let template = template_of(&template)?;
	asked(&Folders::home(), template, SystemTime::now()).await
}

/// `GET /previews/:template`: the template's newest preview and where it is.
///
/// # Errors
/// 401 without a session, 403 for a browser's, 404 for a template home makes
/// no preview of, and 500 when the drive refuses.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "previews_get", skip_all, fields(otel.kind = "server"))]
pub async fn get_preview(_paired: Paired, Path(template): Path<String>) -> Result<Json<Current>, FileHostError> {
	let template = template_of(&template)?;
	let preview = previews::current(&Folders::home(), template, SystemTime::now()).await.map_err(|err| drive(&err))?;
	Ok(Json(Current { preview }))
}

/// The video of `template`'s preview `id`, if it is the template's current,
/// ready one.
async fn video_file(folders: &Folders, template: &str, id: &str, now: SystemTime) -> Result<std::path::PathBuf, FileHostError> {
	let current = previews::current(folders, template, now).await.map_err(|err| drive(&err))?;
	match current {
		Some(preview) if preview.id == id && preview.state == State::Ready => Ok(folders.renders.join(id).join(previews::VIDEO)),
		_ => Err(FileHostError::NotFound),
	}
}

/// `GET /previews/:template/:id/video`: the preview's video, streamed from
/// the drive (never held in memory), with ranges, as `<id>.mp4`.
///
/// # Errors
/// 401 without a session, 403 for a browser's, 404 for a template home makes
/// no preview of, or an id that is not its current, ready, unexpired
/// preview, and 500 when the drive refuses.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "previews_video", skip_all, fields(otel.kind = "server"))]
pub async fn get_video(_paired: Paired, Path((template, id)): Path<(String, String)>, request: Request) -> Result<Response, FileHostError> {
	let template = template_of(&template)?;
	let file = video_file(&Folders::home(), template, &id, SystemTime::now()).await?;
	let served = match ServeFile::new(file).oneshot(request).await {
		Ok(served) => served,
		Err(never) => match never {},
	};
	let mut response = served.map(Body::new).into_response();
	// The id is the template's current one: letters, digits and dashes.
	let disposition = String::from("attachment; filename=\"") + &id + ".mp4\"";
	if let Ok(disposition) = HeaderValue::from_str(&disposition) {
		response.headers_mut().insert(CONTENT_DISPOSITION, disposition);
	}
	Ok(response)
}

#[cfg(test)]
mod tests {
	use super::{asked, video_file};
	use crate::auth::{
		cookie::{Carrier, SessionToken},
		now, AuthContext,
	};
	use crate::previews::Folders;
	use crate::routes::table::Module;
	use axum::{
		body::Body,
		extract::FromRef,
		http::{
			header::{AUTHORIZATION, COOKIE},
			HeaderName, Method, Request, StatusCode,
		},
		response::IntoResponse as _,
		Router,
	};
	use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};
	use std::time::{Duration, UNIX_EPOCH};
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
		crate::routes::previews::previews()
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

	async fn status(app: &Router, method: Method, path: &str, auth: Option<&(HeaderName, String)>) -> StatusCode {
		let mut request = Request::builder().method(method).uri(path);
		if let Some((name, value)) = auth {
			request = request.header(name, value);
		}
		app.clone().oneshot(request.body(Body::empty()).unwrap()).await.unwrap().status()
	}

	#[tokio::test]
	async fn only_a_paired_phone_reaches_previews_and_an_unknown_template_is_not_found() {
		let state = state().await;
		let (phone, browser) = (session(&state, Carrier::Bearer).await, session(&state, Carrier::Cookie).await);
		let app = module().into_table_router().with_state(state);
		assert_eq!(module().descriptors().count(), 3);
		for (method, path) in [
			(Method::POST, "/previews/conference-call"),
			(Method::GET, "/previews/conference-call"),
			(Method::GET, "/previews/conference-call/conference-call-20261108T074500Z/video"),
		] {
			assert_eq!(status(&app, method.clone(), path, None).await, StatusCode::UNAUTHORIZED, "{path}");
			assert_eq!(status(&app, method.clone(), path, Some(&browser)).await, StatusCode::FORBIDDEN, "{path}");
		}
		for (method, path) in [
			(Method::POST, "/previews/podcast"),
			(Method::GET, "/previews/podcast"),
			(Method::GET, "/previews/podcast/podcast-20261108T074500Z/video"),
		] {
			assert_eq!(status(&app, method, path, Some(&phone)).await, StatusCode::NOT_FOUND, "{path}");
		}
	}

	fn folders(dir: &std::path::Path) -> Folders {
		let folders = Folders {
			requests: dir.join("requests"),
			renders: dir.join("renders"),
			status: dir.join("status"),
			work: dir.join("work"),
			lane: dir.join("lane"),
		};
		for folder in [&folders.requests, &folders.renders, &folders.status, &folders.work, &folders.lane] {
			std::fs::create_dir_all(folder).unwrap();
		}
		folders
	}

	#[tokio::test]
	async fn a_new_preview_is_accepted_and_one_still_to_be_made_answered_as_it_is() {
		let dir = tempfile::tempdir().unwrap();
		let folders = folders(dir.path());
		let when = UNIX_EPOCH + Duration::from_secs(1_794_123_900);
		let (code, body) = asked(&folders, "conference-call", when).await.unwrap();
		assert_eq!(code, StatusCode::ACCEPTED);
		assert_eq!(
			serde_json::to_value(&body.0).unwrap(),
			serde_json::json!({ "id": "conference-call-20261108T074500Z", "state": "waiting" })
		);
		let (code, again) = asked(&folders, "conference-call", when + Duration::from_secs(5)).await.unwrap();
		assert_eq!((code, again.0.id.as_str()), (StatusCode::OK, "conference-call-20261108T074500Z"), "nothing new is queued");
	}

	#[tokio::test]
	async fn only_the_current_ready_preview_has_a_video() {
		let dir = tempfile::tempdir().unwrap();
		let folders = folders(dir.path());
		let when = UNIX_EPOCH + Duration::from_secs(1_794_123_900);
		let id = "conference-call-20261108T074500Z";
		let (code, _) = asked(&folders, "conference-call", when).await.unwrap();
		assert_eq!(code, StatusCode::ACCEPTED);
		let not_found = |found: Result<std::path::PathBuf, crate::FileHostError>| found.map_err(|err| err.into_response().status()).err();
		assert_eq!(
			not_found(video_file(&folders, "conference-call", id, when).await),
			Some(StatusCode::NOT_FOUND),
			"not made yet"
		);
		std::fs::create_dir_all(folders.renders.join(id)).unwrap();
		std::fs::write(folders.renders.join(id).join("preview.mp4"), b"mp4").unwrap();
		std::fs::File::options()
			.write(true)
			.open(folders.renders.join(id).join("preview.mp4"))
			.unwrap()
			.set_modified(when)
			.unwrap();
		assert_eq!(
			video_file(&folders, "conference-call", id, when).await.unwrap(),
			folders.renders.join(id).join("preview.mp4")
		);
		assert_eq!(
			not_found(video_file(&folders, "conference-call", "conference-call-20261108T070000Z", when).await),
			Some(StatusCode::NOT_FOUND),
			"not the current one"
		);
		assert_eq!(
			not_found(video_file(&folders, "conference-call", id, when + crate::previews::LIFETIME).await),
			Some(StatusCode::NOT_FOUND),
			"past its day"
		);
	}
}

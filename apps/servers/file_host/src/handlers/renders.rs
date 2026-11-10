//! Handler for `routes::renders`: what home is rendering, on the phone's
//! Studio › Renders (SFEED13, paulgsc/some-ui#2761). What it reads is
//! `feeds::renders`'s.
//!
//! **A paired phone's only**, as every feed handler: it takes [`Paired`], so
//! a request without a session is `401` and a browser's session `403`.

use crate::feeds::renders::{self, Board, Folders};
use crate::subject::Paired;
use crate::FileHostError;
use axum::{extract::State, Json};
use sqlx::SqlitePool;
use tracing::instrument;

/// `GET /renders`: the job in progress (its state, why it waits, a rough
/// estimate), what waits after it, what couldn't finish, the last few
/// finished, and each feed's next expected episode.
///
/// # Errors
/// 401 without a session, 403 for a browser's, and 500 for a storage
/// failure.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "renders_get", skip_all, fields(otel.kind = "server"))]
pub async fn get_renders(paired: Paired, State(db): State<SqlitePool>) -> Result<Json<Board>, FileHostError> {
	let now = chrono::Utc::now().timestamp();
	Ok(Json(renders::read(&db, &Folders::server(), paired.subject.as_str(), now).await?))
}

#[cfg(test)]
mod tests {
	//! Through the real router, with a paired phone's session and a browser's,
	//! as `handlers::feed_videos`' tests are; `feeds::renders`' tests hold
	//! what is read.

	use crate::auth::{
		cookie::{Carrier, SessionToken},
		now, AuthContext,
	};
	use axum::{
		body::Body,
		extract::FromRef,
		http::{
			header::{AUTHORIZATION, COOKIE},
			HeaderName, Request, StatusCode,
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

	async fn get(app: &Router, auth: Option<&(HeaderName, String)>) -> (StatusCode, Value) {
		let mut request = Request::builder().uri("/renders");
		if let Some((name, value)) = auth {
			request = request.header(name, value);
		}
		let response = app.clone().oneshot(request.body(Body::empty()).unwrap()).await.unwrap();
		let status = response.status();
		let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
		(status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
	}

	#[tokio::test]
	async fn only_a_paired_phone_reads_what_home_is_rendering() {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		crate::schema::MIGRATOR.run(&pool).await.unwrap();
		let state = TestState {
			auth: AuthContext::for_tests(pool.clone(), 100),
			pool,
		};
		let (phone, browser) = (session(&state, Carrier::Bearer).await, session(&state, Carrier::Cookie).await);
		let module = crate::routes::renders::renders::<TestState>();
		assert_eq!(module.descriptors().count(), 1);
		let app = module.into_table_router().with_state(state);
		assert_eq!(get(&app, None).await.0, StatusCode::UNAUTHORIZED);
		assert_eq!(get(&app, Some(&browser)).await.0, StatusCode::FORBIDDEN);
		// Nothing held and no worker folders here: an empty board, in its shape.
		let (status, board) = get(&app, Some(&phone)).await;
		assert_eq!(status, StatusCode::OK);
		assert_eq!(board, json!({ "now": null, "waiting": [], "failed": [], "done": [], "coming": [] }));
	}
}

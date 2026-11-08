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
pub async fn get_renders(_paired: Paired, State(db): State<SqlitePool>) -> Result<Json<Board>, FileHostError> {
	let now = chrono::Utc::now().timestamp();
	Ok(Json(renders::read(&db, &Folders::server(), now).await?))
}

//! `/api/v1/presence` — the client saying what it is looking at, right now.
//!
//! ## What this is not
//!
//! Not a heartbeat that means "the app is open" — that was the WebSocket
//! design this replaces, and it was wrong for exactly that reason. Each
//! write names a `context_key`, and `nudge::presence` only ever asks "is
//! there a fresh lease on *this* context", never "does this subject have any
//! fresh lease at all". A client posts here on a visibility/route transition
//! and again on a sparse renewal while visible; it does not poll on a short
//! fixed interval, and the server does not care how it decided to post —
//! this route just records the claim.
//!
//! ## Trust model
//!
//! Same as `/push`: no authentication beyond the CORS origin allowlist, and
//! [`SubjectId`] is the seam auth lands on later. A lease is not itself a
//! secret — at most it tells another caller which session this subject was
//! looking at — but it is a write to state this deployment keeps, so it
//! carries the same trust caveats as everything else `SubjectId`-scoped.

use crate::{subject::SubjectId, AppState, FileHostError};
use axum::{extract::State, Json};
use chrono::Utc;
use presence_repo::PresenceLeaseRepository;
use serde::{Deserialize, Serialize};
use tracing::instrument;

/// What the client is looking at. `nudge::presence` matches this against
/// `StudyAction::session_id()`, so for every action but `GetStarted` this is
/// the session id the dashboard currently has open — see `docs/study-nudge.md`
/// for why `GetStarted` (no session yet) is never suppressible by a lease at
/// all, whatever context key is posted here.
#[derive(Debug, Deserialize)]
pub struct ObserveLeaseRequest {
	pub context_key: String,
}

#[derive(Debug, Serialize)]
pub struct ObserveLeaseResponse {
	pub context_key: String,
	pub observed_at: String,
}

/// `POST /presence/lease` — "I am looking at this, right now."
#[axum::debug_handler]
#[instrument(name = "presence_observe_lease", skip_all, fields(otel.kind = "server"))]
pub async fn observe_lease(State(state): State<AppState>, subject: SubjectId, Json(request): Json<ObserveLeaseRequest>) -> Result<Json<ObserveLeaseResponse>, FileHostError> {
	if request.context_key.trim().is_empty() {
		return Err(FileHostError::unprocessable_entity([("context_key", "must not be empty")]));
	}

	// The server's clock, not the client's: a lease's freshness is judged
	// against `now` at admission time, on the same machine that will read it,
	// so a skewed client clock must not be able to mint a lease that reads as
	// fresher (or staler) than the write actually was.
	let observed_at = Utc::now().to_rfc3339();

	PresenceLeaseRepository::new(state.core.shared_db)
		.observe(subject.as_str(), &request.context_key, &observed_at)
		.await
		.map_err(|err| FileHostError::OperationError(err.to_string()))?;

	Ok(Json(ObserveLeaseResponse {
		context_key: request.context_key,
		observed_at,
	}))
}

//! `/api/v1/signals` — where work actually originates.
//!
//! Everything the waker later does was decided here, at the moment something
//! happened. A signal folds into the subject's charge and the crossing instant
//! is re-solved on the spot; the waker only reads the answer.
//!
//! That is the inversion this milestone is about. A cron-driven design has no
//! endpoint like this, because nothing needs to tell it anything — it wakes up
//! and inspects the world. The cost of that convenience is that "why" can never
//! be part of the decision, only "when".

use crate::nudge::waker;
use crate::subject::SubjectId;
use crate::{AppState, FileHostError};
use axum::{extract::State, Json};
use serde::Serialize;
use study_domain::StudySignal;
use tracing::instrument;

#[derive(Debug, Serialize)]
pub struct SignalAccepted {
	pub kind: &'static str,
	/// When this subject could next become eligible for an intervention.
	///
	/// Returned because it is the single most useful thing for a caller to see
	/// while developing: it makes the arithmetic observable without waiting for
	/// a notification, and a signal that moves it in the wrong direction is
	/// immediately obvious.
	pub eligible_at: String,
}

/// `POST /signals`
///
/// The body is a `StudySignal`, tagged by `kind`. Adding a variant to the
/// domain adds an accepted shape here with no change to this handler — which is
/// the point of the domain being a separate crate.
#[axum::debug_handler]
#[instrument(name = "observe_signal", skip_all, fields(otel.kind = "server", signal = tracing::field::Empty))]
pub async fn observe(State(state): State<AppState>, subject: SubjectId, Json(signal): Json<StudySignal>) -> Result<Json<SignalAccepted>, FileHostError> {
	tracing::Span::current().record("signal", signal.kind());

	let eligible_at = waker::observe(&state, subject.as_str(), &signal)
		.await
		.map_err(|err| FileHostError::OperationError(err.to_string()))?;

	Ok(Json(SignalAccepted {
		kind: signal.kind(),
		eligible_at: eligible_at.to_rfc3339(),
	}))
}

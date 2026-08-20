//! `/api/v1/push` — consent, keys, and a manual trigger.
//!
//! ## The null case is silence
//!
//! There is no path through these routes that creates a subscription nobody
//! agreed to. `POST /push/subscriptions` requires a topic list, and an empty
//! one is honoured as "receives nothing" rather than quietly read as "receives
//! everything". A person who has never been asked is a person who is never
//! notified, which is the only defensible default for a channel that reaches a
//! locked screen.
//!
//! `GET /push/vapid-key` is the one that looks redundant and is not: hardcoding
//! the public key into the frontend bundle makes a key change a frontend
//! redeploy, and the `applicationServerKey` mismatch that results is invisible
//! until a send fails with `403`.
//!
//! ## Trust model, stated rather than assumed
//!
//! No authentication beyond the origin allowlist. Anyone who can reach the LAN
//! can register a subscription. `SubjectId` exists so that the day auth lands,
//! it lands at one boundary — see [`crate::subject`].

use crate::subject::SubjectId;
use crate::{AppState, FileHostError};
use axum::{extract::State, Json};
use chrono::Utc;
use push_kit::PushSubscription;
use push_repo::{Consent, PushSubscriptionRepository, Topic};
use serde::{Deserialize, Serialize};
use tracing::{info, instrument};

#[derive(Debug, Serialize)]
pub struct VapidKeyResponse {
	/// base64url, uncompressed P-256 point — what `applicationServerKey` wants.
	pub public_key: String,
	/// What this deployment can be subscribed to. Sent so the consent modal
	/// renders the options the sender will actually honour, rather than a list
	/// maintained separately in the frontend.
	pub topics: Vec<&'static str>,
}

/// The browser's `PushSubscription`, plus what they agreed to.
///
/// One request rather than two because a subscription without consent is not a
/// state this server should be able to hold, even briefly.
#[derive(Debug, Deserialize)]
pub struct SubscribeRequest {
	#[serde(flatten)]
	pub subscription: PushSubscription,
	pub topics: Vec<Topic>,
}

#[derive(Debug, Deserialize)]
pub struct UnsubscribeRequest {
	pub endpoint: String,
}

#[derive(Debug, Serialize)]
pub struct SubscribeResponse {
	pub endpoint: String,
	pub topics: Vec<&'static str>,
}

#[derive(Debug, Serialize)]
pub struct DeleteResponse {
	/// False when there was nothing to remove. The call still succeeds —
	/// unsubscribing twice is not an error.
	pub removed: bool,
}

/// `GET /push/vapid-key`
#[axum::debug_handler]
#[instrument(name = "push_vapid_key", skip_all, fields(otel.kind = "server"))]
pub async fn vapid_key(State(state): State<AppState>) -> Result<Json<VapidKeyResponse>, FileHostError> {
	let Some(identity) = state.nudge.as_ref().map(|nudge| &nudge.vapid) else {
		// 503 rather than 500: nothing is broken, the deployment simply has no
		// VAPID identity, and the browser should not subscribe.
		return Err(FileHostError::FeatureNotConfigured("push (VAPID_PUBLIC_KEY / VAPID_PRIVATE_KEY unset)"));
	};

	Ok(Json(VapidKeyResponse {
		public_key: identity.public_key().to_owned(),
		topics: Topic::ALL.iter().map(|topic| topic.as_str()).collect(),
	}))
}

/// `POST /push/subscriptions` — the moment consent is recorded.
#[axum::debug_handler]
#[instrument(name = "push_subscribe", skip_all, fields(otel.kind = "server"))]
pub async fn subscribe(State(state): State<AppState>, subject: SubjectId, Json(request): Json<SubscribeRequest>) -> Result<Json<SubscribeResponse>, FileHostError> {
	request
		.subscription
		.validate()
		.map_err(|reason| FileHostError::unprocessable_entity([("subscription", reason.to_string())]))?;

	let now = Utc::now().to_rfc3339();
	let consent = Consent {
		subject_id: subject.as_str().to_owned(),
		topics: request.topics,
		consented_at: now.clone(),
	};

	PushSubscriptionRepository::new(state.core.shared_db.clone())
		.upsert(&request.subscription, &consent, &now)
		.await
		.map_err(|err| FileHostError::OperationError(err.to_string()))?;

	// First contact: this is the event that says a subject exists at all —
	// see `waker::first_contact` for why this route, and not a signal, is the
	// honest place to seed the gate row. Propagated rather than swallowed:
	// hiding this failure would silently recreate the exact "never due" bug
	// (#278) this call exists to close, and the whole call is idempotent, so a
	// client retry after a `500` is harmless.
	crate::nudge::waker::first_contact(&state.core.shared_db, subject.as_str())
		.await
		.map_err(|err| FileHostError::OperationError(err.to_string()))?;

	info!(
		endpoint = %request.subscription.endpoint,
		subject = subject.as_str(),
		topics = ?consent.topics,
		"recorded consent and stored a push subscription"
	);

	Ok(Json(SubscribeResponse {
		endpoint: request.subscription.endpoint,
		topics: consent.topics.iter().map(|topic| topic.as_str()).collect(),
	}))
}

/// `DELETE /push/subscriptions` — withdrawing consent. Idempotent.
#[axum::debug_handler]
#[instrument(name = "push_unsubscribe", skip_all, fields(otel.kind = "server"))]
pub async fn unsubscribe(State(state): State<AppState>, Json(request): Json<UnsubscribeRequest>) -> Result<Json<DeleteResponse>, FileHostError> {
	let removed = PushSubscriptionRepository::new(state.core.shared_db)
		.delete_by_endpoint(&request.endpoint)
		.await
		.map_err(|err| FileHostError::OperationError(err.to_string()))?;

	Ok(Json(DeleteResponse { removed: removed > 0 }))
}

/// `POST /push/test` — send now, without waiting for engagement to decay.
///
/// The only honest end-to-end check is a human watching their desktop with the
/// browser closed, and a `201` proves nothing (see `push_kit::SendOutcome`). A
/// test that required a subject to actually drift would take days.
#[axum::debug_handler]
#[instrument(name = "push_test", skip_all, fields(otel.kind = "server"))]
pub async fn send_test(State(state): State<AppState>, subject: SubjectId) -> Result<Json<Vec<String>>, FileHostError> {
	let Some(nudge) = state.nudge.clone() else {
		return Err(FileHostError::FeatureNotConfigured("push"));
	};

	let subscriptions = PushSubscriptionRepository::new(state.core.shared_db.clone())
		.for_subject(subject.as_str())
		.await
		.map_err(|err| FileHostError::OperationError(err.to_string()))?;

	let payload = crate::nudge::payload::NudgePayload::for_session(
		&nudge.base_url,
		"test",
		"Test notification".to_owned(),
		"If you can read this, the whole chain works.".to_owned(),
	);
	let encoded = payload.to_bytes().map_err(|err| FileHostError::OperationError(err.to_string()))?;

	let mut outcomes = Vec::with_capacity(subscriptions.len());
	for stored in subscriptions {
		let outcome = nudge.sender.deliver(&stored.subscription, &encoded).await;
		outcomes.push(format!("{}: {}", stored.subscription.endpoint, outcome.label()));
	}

	Ok(Json(outcomes))
}

use crate::{AppState, FileHostError, WebSocketFsm};
use axum::{
	extract::{Json, State},
	http::StatusCode,
};
use serde::{Deserialize, Serialize};
use some_transport::NatsTransport;
use tracing::instrument;
use ws_events::events::{Event, UtterancePrompt};
use ws_events::UnifiedEvent;

/// `POST /api/v1/utter`: broadcast the utterance to its subject.
///
/// `200` once it is handed to NATS. Before ER1 (`crate::error`) a failed
/// broadcast was dropped and answered `200` too.
///
/// # Errors
///
/// [`FileHostError::BroadcastError`] (a `500`, code `broadcast_error`) when
/// the broadcast fails, NATS being down, say.
#[axum::debug_handler]
#[instrument(name = "utterance", skip_all)]
pub async fn utterance(State(state): State<AppState>, Json(payload): Json<UtterancePrompt>) -> Result<StatusCode, FileHostError> {
	broadcast(&state.realtime.ws, state.realtime.transport.clone(), payload).await
}

async fn broadcast(ws: &WebSocketFsm, transport: NatsTransport<UnifiedEvent>, payload: UtterancePrompt) -> Result<StatusCode, FileHostError> {
	ws.broadcast_event(transport, Event::from(payload)).await?;
	Ok(StatusCode::OK)
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ElementInfo {
	pub tag_name: String,
	#[serde(rename = "type")]
	pub element_type: Option<String>,
	pub id: Option<String>,
	pub name: Option<String>,
	pub class_name: Option<String>,
	pub placeholder: Option<String>,
	#[serde(rename = "formAction")]
	pub form_action: Option<String>,
	#[serde(rename = "formMethod")]
	pub form_method: Option<String>,
	#[serde(rename = "formId")]
	pub form_id: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UtteranceMetadata {
	pub url: String,
	pub domain: String,
	pub title: String,
	pub timestamp: String,
	pub element: ElementInfo,
}

#[cfg(test)]
mod tests {
	use super::broadcast;
	use crate::websocket::broadcast::disconnected_transport;
	use crate::WebSocketFsm;
	use axum::{body::to_bytes, http::StatusCode, response::IntoResponse};
	use serde_json::{json, Value};

	/// ER1: with NATS down the broadcast fails, and the utterance is answered
	/// as that failure rather than `200`.
	#[tokio::test]
	async fn a_failed_broadcast_is_answered_as_the_failure() {
		let payload = serde_json::from_value(json!({
			"text": "hello",
			"metadata": {
				"url": "https://example.test/", "domain": "example.test", "title": "t", "timestamp": "2026-10-26T00:00:00Z",
				"element": { "tagName": "textarea", "type": null, "id": null, "name": null, "className": null, "placeholder": null }
			}
		}))
		.unwrap();

		let response = broadcast(&WebSocketFsm::new(), disconnected_transport().await, payload).await.into_response();

		assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
		let body: Value = serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap()).unwrap();
		assert_eq!(body["error"]["code"], "broadcast_error");
		assert_eq!(body["error"]["message"], "internal server error");
	}
}

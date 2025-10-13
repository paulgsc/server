use crate::{AppState, Event, UtterancePrompt};
use axum::{
	extract::{Json, State},
	http::StatusCode,
};
use serde::{Deserialize, Serialize};
use tracing::instrument;

#[axum::debug_handler]
#[instrument(name = "utterance", skip(state))]
pub async fn utterance(State(state): State<AppState>, Json(payload): Json<UtterancePrompt>) -> StatusCode {
	let event = Event::from(payload);

	let _ = state.ws.broadcast_event(&event).await;

	StatusCode::OK
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UtteranceMetadata {
	pub url: String,
	pub domain: String,
	pub title: String,
	pub timestamp: String,
	pub element: ElementInfo,
}

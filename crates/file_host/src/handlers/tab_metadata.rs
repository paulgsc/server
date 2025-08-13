use crate::{AppState, Event, NowPlaying};
use axum::{
	extract::{Json, State},
	http::StatusCode,
};
use tracing::instrument;

#[axum::debug_handler]
#[instrument(name = "now_playing", skip(state))]
pub async fn now_playing(State(state): State<AppState>, Json(payload): Json<NowPlaying>) -> StatusCode {
	let event = Event::from(payload);

	let _ = state.ws.broadcast_event(&event).await;

	StatusCode::OK
}

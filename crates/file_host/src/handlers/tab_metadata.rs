use crate::{Event, NowPlaying, WebSocketFsm};
use axum::{
	extract::{Json, State},
	http::StatusCode,
};
use tracing::instrument;

#[axum::debug_handler]
#[instrument(name = "now_playing", skip(ws))]
pub async fn now_playing(State(ws): State<WebSocketFsm>, Json(payload): Json<NowPlaying>) -> StatusCode {
	let event = Event::from(payload);

	let _ = ws.broadcast_event(&event).await;

	StatusCode::OK
}

use axum::{
	extract::{ws::WebSocketUpgrade, State},
	response::IntoResponse,
};
use obs_websocket::ObsWebSocketWithBroadcast;
use std::sync::Arc;

#[axum::debug_handler]
pub async fn websocket_handler(ws: WebSocketUpgrade, State(client): State<Arc<ObsWebSocketWithBroadcast>>) -> impl IntoResponse {
	client.websocket_handler(ws)
}

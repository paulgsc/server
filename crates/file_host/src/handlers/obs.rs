use axum::{extract::ws::WebSocketUpgrade, response::IntoResponse, Json};
use obs_websocket::{client, ObsStatus};

#[axum::debug_handler]
pub async fn get_obs_status() -> Json<ObsStatus> {
	let status = client().get_status().await;
	Json(status)
}

#[axum::debug_handler]
pub async fn websocket_handler(ws: WebSocketUpgrade) -> impl IntoResponse {
	client().websocket_handler(ws)
}

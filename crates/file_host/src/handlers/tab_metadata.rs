use crate::AppState;
use axum::{
	extract::{Json, State},
	http::StatusCode,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::instrument;

#[derive(Serialize, Debug, Deserialize)]
pub struct NowPlaying {
	title: String,
	channel: String,
	video_id: String,
	current_time: u32,
	duration: u32,
	thumbnail: String,
}

#[axum::debug_handler]
#[instrument(name = "now_playing", skip(_state))]
pub async fn now_playing(State(_state): State<Arc<AppState>>, Json(payload): Json<NowPlaying>) -> StatusCode {
	log::info!("Now playing: {} by {}", payload.title, payload.channel);
	StatusCode::OK
}

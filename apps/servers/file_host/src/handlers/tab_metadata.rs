use crate::{AppState, Event, FileHostError, NowPlaying};
use axum::{
	extract::{Json, State},
	http::StatusCode,
};
use tracing::instrument;

#[axum::debug_handler]
#[instrument(name = "now_playing", skip(state))]
pub async fn now_playing(State(state): State<AppState>, Json(payload): Json<NowPlaying>) -> Result<StatusCode, FileHostError> {
	let event = Event::from(payload);

	// Cache the event
	// So that cliets that miss broadcast can fetch from cache.
	let key = "now_playing";
	let redis = state.dedup_cache.get_cache_store();
	let _ = redis.set(key, &event, Some(state.config.cache_ttl)).await?;

	let _ = state.ws.broadcast_event(&event).await;

	Ok(StatusCode::OK)
}

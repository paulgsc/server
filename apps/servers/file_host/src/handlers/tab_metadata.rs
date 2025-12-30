use crate::{AppState, FileHostError};
use axum::{
	extract::{Json, State},
	http::StatusCode,
};
use tracing::instrument;
use ws_events::events::{Event, NowPlaying};

#[axum::debug_handler]
#[instrument(name = "now_playing", skip(state))]
pub async fn now_playing(State(state): State<AppState>, Json(payload): Json<NowPlaying>) -> Result<StatusCode, FileHostError> {
	let event = Event::from(payload);

	// Cache the event
	// So that cliets that miss broadcast can fetch from cache.
	let key = "now_playing";
	let redis = state.realtime.dedup_cache.get_cache_store();
	let _ = redis.set(key, &event, Some(state.core.config.cache_ttl)).await?;
	let transport = state.realtime.transport.clone();

	let _ = state.realtime.ws.broadcast_event(transport, event).await?;

	Ok(StatusCode::OK)
}

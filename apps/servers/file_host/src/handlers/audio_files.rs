pub mod error;
mod service;
mod types;

use service::AudioService;
use types::{AudioConfig, AudioMetadata, AudioSearchResponse, CachedAudio, GetAudioRequest, SearchAudioRequest};

use crate::AppState;
use axum::{
	extract::{Path, Query, State},
	http::StatusCode,
	Json,
};
use std::collections::HashMap;

pub async fn get_audio(State(state): State<AppState>, Path(id): Path<String>, Query(params): Query<HashMap<String, String>>) -> Result<Json<CachedAudio>, StatusCode> {
	let force_refresh = params.get("force_refresh").and_then(|v| v.parse().ok()).unwrap_or(false);

	let req = GetAudioRequest {
		id,
		force_refresh: Some(force_refresh),
	};

	let audio_service = AudioService::new(state.gdrive_reader, state.dedup_cache, AudioConfig::default());

	match audio_service.get_audio(req).await {
		Ok((audio, _)) => Ok(Json(audio)),
		Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
	}
}

pub async fn search_audio(State(state): State<AppState>, Query(params): Query<SearchAudioRequest>) -> Result<Json<AudioSearchResponse>, StatusCode> {
	let audio_service = AudioService::new(state.gdrive_reader, state.dedup_cache, AudioConfig::default());

	match audio_service.search_audio(params).await {
		Ok(response) => Ok(Json(response)),
		Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
	}
}

pub async fn search_audio_post(State(state): State<AppState>, Json(req): Json<SearchAudioRequest>) -> Result<Json<AudioSearchResponse>, StatusCode> {
	let audio_service = AudioService::new(state.gdrive_reader, state.dedup_cache, AudioConfig::default());

	match audio_service.search_audio(req).await {
		Ok(response) => Ok(Json(response)),
		Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
	}
}

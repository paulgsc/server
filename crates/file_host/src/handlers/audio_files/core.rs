use crate::{AppState, CacheStore, FileHostError};
use axum::{
	body::Body,
	extract::{Path, Query, State},
	http::{
		header::{self, CACHE_CONTROL, CONTENT_TYPE},
		StatusCode,
	},
	response::Response,
	Json, Router,
};
use bytes::Bytes;
use sdk::{DriveError, ReadDrive};
use std::collections::HashMap;
use tracing::{error, info, warn};

// Audio-specific helper methods
impl CacheStore {
	pub async fn cache_audio(&self, id: &str, data: Bytes, content_type: String, ttl: Option<u64>) -> Result<(), FileHostError> {
		self.set_binary(&format!("audio:{}", id), &data, Some(content_type), ttl).await
	}

	pub async fn get_cached_audio(&self, id: &str) -> Result<Option<(Bytes, String)>, FileHostError> {
		match self.get_binary(&format!("audio:{}", id)).await? {
			Some((data, Some(content_type))) => Ok(Some((Bytes::from(data), content_type))),
			Some((data, None)) => Ok(Some((Bytes::from(data), "audio/mpeg".to_string()))), // Default
			None => Ok(None),
		}
	}
}

/// Get audio file by ID
pub async fn get_audio_by_id(State(state): State<AppState>, Path(id): Path<String>, Query(params): Query<HashMap<String, String>>) -> Result<Response, FileHostError> {
	info!("Fetching audio file: {}", id);

	// Check cache first
	if let Some((data, content_type)) = state.cache_store.get_cached_audio(&id).await? {
		info!("Serving cached audio: {}", id);
		return Ok(create_audio_response(data, &content_type, true));
	}

	// Validate the file ID format
	if id.is_empty() || id.len() > 100 {
		return Err(FileHostError::DriveError(sdk::DriveError::Other("Invalid file ID format".to_string())));
	}

	// Get file metadata first to validate it's an audio file
	let metadata = state.drive_client.get_file_metadata(&id).await.map_err(|e| {
		warn!("Failed to get metadata for file {}: {}", id, e);
		match e {
			DriveError::FileNotFound(_) => FileHostError::NotFound,
			_ => FileHostError::DriveError(e),
		}
	})?;

	// Validate it's an audio file
	if !state.config.supported_mime_types.contains(&metadata.mime_type) {
		return Err(FileHostError::InvalidMimeType(metadata.mime_type));
	}

	// Check file size
	if let Some(size) = metadata.size {
		if size > state.config.max_file_size {
			return Err(FileHostError::MaxRecordLimitExceeded);
		}
	}

	// Download the file
	let audio_data = state.drive_client.download_file(&id).await.map_err(|e| {
		error!("Failed to download audio file {}: {}", id, e);
		FileHostError::DriveError(e)
	})?;

	info!("Downloaded audio file: {} ({} bytes)", id, audio_data.len());

	// Cache the audio data
	if let Err(e) = state.cache_store.cache_audio(&id, audio_data.clone(), metadata.mime_type.clone(), None).await {
		warn!("Failed to cache audio {}: {}", id, e);
	}

	Ok(create_audio_response(audio_data, &metadata.mime_type, false))
}

/// Search for audio files
pub async fn search_audio(State(state): State<AppState>, Query(params): Query<AudioSearchParams>) -> Result<Json<AudioSearchResponse>, FileHostError> {
	let limit = params.limit.unwrap_or(20).min(100) as i32;
	let offset = params.offset.unwrap_or(0);

	info!("Searching audio files with query: {:?}", params.q);

	// Build search query for Google Drive
	let mut query_parts = Vec::new();

	// Filter by audio folder if specified
	if let Some(folder_id) = &state.config.audio_folder_id {
		query_parts.push(format!("'{}' in parents", folder_id));
	}

	// Add MIME type filters for audio files
	let mime_filter = state
		.config
		.supported_mime_types
		.iter()
		.map(|mime| format!("mimeType='{}'", mime))
		.collect::<Vec<_>>()
		.join(" or ");
	query_parts.push(format!("({})", mime_filter));

	// Add text search if provided
	if let Some(search_text) = &params.q {
		if !search_text.trim().is_empty() {
			query_parts.push(format!("name contains '{}'", search_text.trim()));
		}
	}

	// Add voice filter if provided
	if let Some(voice) = &params.voice {
		query_parts.push(format!("name contains '{}'", voice));
	}

	let full_query = query_parts.join(" and ");

	// Search files
	let files = state
		.drive_client
		.search_files(&full_query, limit + 10) // Get a few extra for pagination
		.await
		.map_err(|e| {
			error!("Failed to search audio files: {}", e);
			FileHostError::DriveError(e)
		})?;

	// Apply offset and limit
	let total_count = files.len();
	let start_idx = offset as usize;
	let end_idx = (start_idx + limit as usize).min(files.len());
	let paginated_files = if start_idx < files.len() { files[start_idx..end_idx].to_vec() } else { Vec::new() };

	// Convert to AudioMetadata
	let results = paginated_files
		.into_iter()
		.map(|file| AudioMetadata {
			id: file.id,
			name: file.name.clone(),
			mime_type: file.mime_type,
			size: file.size,
			created_time: file.created_time,
			modified_time: file.modified_time,
			web_view_link: file.web_view_link,
			voice_id: extract_voice_id_from_name(&file.name),
			text_preview: extract_text_preview_from_name(&file.name),
			duration_seconds: None, // Could be extracted from metadata in future
		})
		.collect();

	let has_more = total_count > end_idx;
	let next_offset = if has_more { Some(offset + limit as u32) } else { None };

	info!("Found {} audio files, returning {} results", total_count, results.len());

	Ok(Json(AudioSearchResponse {
		results,
		total_count,
		has_more,
		next_offset,
	}))
}

/// List audio files in a specific folder
pub async fn list_audio_files(State(state): State<AppState>, Query(params): Query<HashMap<String, String>>) -> Result<Json<Vec<AudioMetadata>>, FileHostError> {
	let folder_id = params.get("folder_id");
	let page_size = params.get("page_size").and_then(|s| s.parse::<i32>().ok()).unwrap_or(50).min(100);

	info!("Listing audio files in folder: {:?}", folder_id);

	let files = state.drive_client.list_files(folder_id.map(|s| s.as_str()), page_size).await.map_err(|e| {
		error!("Failed to list audio files: {}", e);
		FileHostError::DriveError(e)
	})?;

	// Filter only audio files
	let audio_files = files
		.into_iter()
		.filter(|file| state.config.supported_mime_types.contains(&file.mime_type))
		.map(|file| AudioMetadata {
			id: file.id,
			name: file.name.clone(),
			mime_type: file.mime_type,
			size: file.size,
			created_time: file.created_time,
			modified_time: file.modified_time,
			web_view_link: file.web_view_link,
			voice_id: extract_voice_id_from_name(&file.name),
			text_preview: extract_text_preview_from_name(&file.name),
			duration_seconds: None,
		})
		.collect();

	info!("Found {} audio files", audio_files.len());

	Ok(Json(audio_files))
}

/// Get audio file metadata
pub async fn get_audio_metadata(State(state): State<AppState>, Path(id): Path<String>) -> Result<Json<AudioMetadata>, FileHostError> {
	info!("Getting metadata for audio file: {}", id);

	let file = state.drive_client.get_file_metadata(&id).await.map_err(|e| {
		warn!("Failed to get metadata for file {}: {}", id, e);
		match e {
			DriveError::FileNotFound(_) => FileHostError::NotFound,
			_ => FileHostError::DriveError(e),
		}
	})?;

	// Validate it's an audio file
	if !state.config.supported_mime_types.contains(&file.mime_type) {
		return Err(FileHostError::InvalidMimeType(file.mime_type));
	}

	let metadata = AudioMetadata {
		id: file.id,
		name: file.name.clone(),
		mime_type: file.mime_type,
		size: file.size,
		created_time: file.created_time,
		modified_time: file.modified_time,
		web_view_link: file.web_view_link,
		voice_id: extract_voice_id_from_name(&file.name),
		text_preview: extract_text_preview_from_name(&file.name),
		duration_seconds: None,
	};

	Ok(Json(metadata))
}

fn create_audio_response(data: Bytes, content_type: &str, from_cache: bool) -> Response {
	let mut response = Response::builder()
		.status(StatusCode::OK)
		.header(CONTENT_TYPE, content_type)
		.header("Content-Length", data.len().to_string())
		.header("Accept-Ranges", "bytes")
		.header("Access-Control-Allow-Origin", "*")
		.header("Access-Control-Expose-Headers", "Content-Length,Content-Range");

	// Set cache headers
	if from_cache {
		response = response.header(CACHE_CONTROL, "public, max-age=1800"); // 30 minutes
		response = response.header("X-Cache", "HIT");
	} else {
		response = response.header(CACHE_CONTROL, "public, max-age=3600"); // 1 hour
		response = response.header("X-Cache", "MISS");
	}

	response
		.body(Body::from(data))
		.unwrap_or_else(|_| Response::builder().status(StatusCode::INTERNAL_SERVER_ERROR).body(Body::empty()).unwrap())
}

fn extract_voice_id_from_name(filename: &str) -> Option<String> {
	// Extract voice ID from filename pattern like "openai_voice_alloy_text_hash.mp3"
	let parts: Vec<&str> = filename.split('_').collect();
	if parts.len() >= 3 && parts[1] == "voice" {
		Some(parts[2].to_string())
	} else {
		None
	}
}
fn extract_text_preview_from_name(filename: &str) -> Option<String> {
	// Extract text preview from filename
	let parts: Vec<&str> = filename.splitn(2, "_text_").collect();
	if parts.len() == 2 {
		let text_part = parts[1];
		let cleaned_text = text_part.trim_end_matches(".mp3").replace('-', " ");
		Some(cleaned_text)
	} else {
		None
	}
}

// These are missing structs from the original code
// We need to define them to make the code compile
use serde::{Deserialize, Serialize};

#[derive(Serialize)]
pub struct AudioMetadata {
	pub id: String,
	pub name: String,
	pub mime_type: String,
	pub size: Option<i64>,
	pub created_time: Option<String>,
	pub modified_time: Option<String>,
	pub web_view_link: Option<String>,
	pub voice_id: Option<String>,
	pub text_preview: Option<String>,
	pub duration_seconds: Option<f64>,
}

#[derive(Deserialize)]
pub struct AudioSearchParams {
	pub q: Option<String>,
	pub voice: Option<String>,
	pub limit: Option<u32>,
	pub offset: Option<u32>,
}

#[derive(Serialize)]
pub struct AudioSearchResponse {
	pub results: Vec<AudioMetadata>,
	pub total_count: usize,
	pub has_more: bool,
	pub next_offset: Option<u32>,
}

use crate::{metrics::http::OPERATION_DURATION, timed_operation, AppState, FileHostError};
use axum::{
	extract::{Path, State},
	http::header,
	response::Response,
};
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use tracing::instrument;

use std::sync::OnceLock;

static ALLOWED_IMAGE_MIME_TYPES: OnceLock<Vec<&'static str>> = OnceLock::new();

fn allowed_image_mime_types() -> &'static Vec<&'static str> {
	ALLOWED_IMAGE_MIME_TYPES.get_or_init(|| vec!["image/jpeg", "image/png", "image/gif", "image/webp", "image/svg+xml"])
}

#[derive(Debug, Serialize, Deserialize)]
struct FileMetadata {
	mime_type: String,
	size: usize,
	id: String,
	name: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct GDriveResponse {
	data: Bytes,
	metadata: FileMetadata,
}

#[axum::debug_handler]
#[instrument(name = "serve_gdrive_image", skip(state), fields(image_id))]
pub async fn serve_gdrive_image(State(state): State<AppState>, Path(image_id): Path<String>) -> Result<Response<axum::body::Body>, FileHostError> {
	let cache_key = format!("get_drive_image{}", image_id);

	let cache_result = timed_operation!("serve_gdrive_image", "cached_check", true, { state.cache_store.get::<GDriveResponse>(&cache_key).await })?;

	if let Some(cached_data) = cache_result {
		let mime_type = cached_data.metadata.mime_type;

		if allowed_image_mime_types().contains(&mime_type.as_str()) {
			let response = Response::builder().header(header::CONTENT_TYPE, mime_type).body(axum::body::Body::from(cached_data.data))?;
			return Ok(response);
		} else {
			return Err(FileHostError::InvalidMimeType(mime_type.clone()));
		}
	}

	let drive_response = timed_operation!("serve_gdrive_image", "refetch", false, { refetch(state.clone(), &image_id).await })?;

	let mime_type = &drive_response.metadata.mime_type;

	if !allowed_image_mime_types().contains(&mime_type.as_str()) {
		return Err(FileHostError::InvalidMimeType(mime_type.clone()));
	}

	timed_operation!("serve_gdrive_image", "cache_set", false, { state.cache_store.set(&cache_key, &drive_response, None).await })?;

	let response = Response::builder()
		.header(header::CONTENT_TYPE, mime_type)
		.body(axum::body::Body::from(drive_response.data))?;

	Ok(response)
}

#[instrument(name = "refetch", skip(state), fields(image_id))]
async fn refetch(state: AppState, image_id: &str) -> Result<GDriveResponse, FileHostError> {
	let file = timed_operation!("refetch", "get_file_metadata", false, { state.gdrive_reader.get_file_metadata(image_id).await })?;

	let bytes = timed_operation!("refetch", "download_file", false, { state.gdrive_reader.download_file(image_id).await })?;

	let size = file.size.unwrap_or(0).try_into().unwrap_or(0);

	Ok(GDriveResponse {
		data: bytes,
		metadata: FileMetadata {
			size,
			id: file.id,
			name: file.name,
			mime_type: file.mime_type,
		},
	})
}

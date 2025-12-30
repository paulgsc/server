use crate::metrics::otel::{record_cache_hit, record_file_download, OperationTimer};
use crate::{AppState, DedupError, FileHostError};
use axum::{
	extract::{Path, State},
	http::header,
	response::Response,
};
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;
use tracing::instrument;

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
#[instrument(name = "serve_gdrive_image", skip(state), fields(image_id = %image_id, otel.kind = "server"))]
pub async fn serve_gdrive_image(State(state): State<AppState>, Path(image_id): Path<String>) -> Result<Response<axum::body::Body>, FileHostError> {
	let cache_key = format!("get_drive_image_binary_{}", image_id);

	let _timer = OperationTimer::new("serve_gdrive_image", "total");

	// Use binary cache for potentially better compression and performance
	let ((data, content_type), was_cached) = state
		.realtime
		.dedup_cache
		.get_or_fetch_binary(&cache_key, || async {
			let _fetch_timer = OperationTimer::new("serve_gdrive_image", "fetch_file");
			let drive_response = fetch_gdrive_file(state.clone(), &image_id).await?;
			let mime_type = drive_response.metadata.mime_type.clone();

			// Return as binary data with content type
			Ok((drive_response.data.to_vec(), Some(mime_type)))
		})
		.await?;

	record_cache_hit("serve_gdrive_image", was_cached);

	let mime_type = content_type.as_deref().unwrap_or("application/octet-stream");

	// Validate mime type
	if !allowed_image_mime_types().contains(&mime_type) {
		return Err(FileHostError::InvalidMimeType(mime_type.to_string()));
	}

	// Record file download metrics
	record_file_download(&image_id, mime_type, data.len(), "binary");

	// Build and return response
	let response = Response::builder()
		.header(header::CONTENT_TYPE, mime_type)
		.header(header::CONTENT_LENGTH, data.len().to_string())
		.body(axum::body::Body::from(data))?;

	Ok(response)
}

/// Fetch file from Google Drive with metadata
#[instrument(name = "fetch_gdrive_file", skip(state), fields(image_id, otel.kind = "internal"))]
async fn fetch_gdrive_file(state: AppState, image_id: &str) -> Result<GDriveResponse, DedupError> {
	// Fetch metadata
	let file = {
		let _timer = OperationTimer::new("fetch_gdrive_file", "get_file_metadata");
		state.external.gdrive_reader.get_file_metadata(image_id).await
	}?;

	// Download file content
	let bytes = {
		let _timer = OperationTimer::new("fetch_gdrive_file", "download_file");
		state.external.gdrive_reader.download_file(image_id).await
	}?;

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

/// Optimized version that caches metadata and file data separately
/// This can be useful if you frequently need just metadata without the full file
#[axum::debug_handler]
#[instrument(name = "serve_gdrive_image_optimized", skip(state), fields(image_id = %image_id, otel.kind = "server"))]
#[allow(dead_code)]
pub async fn serve_gdrive_image_optimized(State(state): State<AppState>, Path(image_id): Path<String>) -> Result<Response<axum::body::Body>, FileHostError> {
	let metadata_cache_key = format!("gdrive_metadata_{}", image_id);
	let file_cache_key = format!("gdrive_file_{}", image_id);

	let _timer = OperationTimer::new("serve_gdrive_image_optimized", "total");

	// First, get or fetch metadata (smaller, faster)
	let (metadata, metadata_was_cached) = state
		.realtime
		.dedup_cache
		.get_or_fetch(&metadata_cache_key, || async {
			let _meta_timer = OperationTimer::new("serve_gdrive_image_optimized", "get_file_metadata");
			let file = state.external.gdrive_reader.get_file_metadata(&image_id).await?;

			let size = file.size.unwrap_or(0).try_into().unwrap_or(0);
			let metadata = FileMetadata {
				size,
				id: file.id,
				name: file.name,
				mime_type: file.mime_type,
			};

			Ok(metadata)
		})
		.await?;

	record_cache_hit("serve_gdrive_image_optimized_metadata", metadata_was_cached);

	// Validate mime type early
	if !allowed_image_mime_types().contains(&metadata.mime_type.as_str()) {
		return Err(FileHostError::InvalidMimeType(metadata.mime_type.clone()));
	}

	// Then get or fetch the actual file data using binary cache
	let ((data, _), file_was_cached) = state
		.realtime
		.dedup_cache
		.get_or_fetch_binary(&file_cache_key, || async {
			let _download_timer = OperationTimer::new("serve_gdrive_image_optimized", "download_file");
			let bytes = state.external.gdrive_reader.download_file(&image_id).await?;

			Ok((bytes.to_vec(), Some(metadata.mime_type.clone())))
		})
		.await?;

	record_cache_hit("serve_gdrive_image_optimized_file", file_was_cached);

	// Record file download metrics
	record_file_download(&image_id, &metadata.mime_type, data.len(), "optimized_split");

	// Build and return response
	let response = Response::builder()
		.header(header::CONTENT_TYPE, &metadata.mime_type)
		.header(header::CONTENT_LENGTH, data.len().to_string())
		.body(axum::body::Body::from(data))?;

	Ok(response)
}

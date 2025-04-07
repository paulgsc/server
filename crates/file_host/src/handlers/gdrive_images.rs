use crate::{AppState, FileHostError};
use axum::{
	extract::{Path, State},
	http::header,
	response::Response,
};
use bytes::Bytes;
use sdk::ReadDrive;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

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
pub async fn serve_gdrive_image(State(state): State<Arc<AppState>>, Path(image_id): Path<String>) -> Result<Response<axum::body::Body>, FileHostError> {
	let cache_key = format!("get_drive_image{}", image_id);

	if let Some(cached_data) = state.cache_store.get_json::<GDriveResponse>(&cache_key).await? {
		log::info!("Cache hit for key: {}", &cache_key);
		let mime_type = cached_data.metadata.mime_type;
		let response = Response::builder().header(header::CONTENT_TYPE, mime_type).body(axum::body::Body::from(cached_data.data))?;

		return Ok(response);
	}

	let drive_response = refetch(&state, &image_id).await?;

	log::info!("Caching data for key: {}", &cache_key);
	state.cache_store.set_json(&cache_key, &drive_response).await?;

	let mime_type = drive_response.metadata.mime_type;
	let response = Response::builder()
		.header(header::CONTENT_TYPE, mime_type)
		.body(axum::body::Body::from(drive_response.data))?;

	Ok(response)
}

async fn refetch(state: &Arc<AppState>, image_id: &str) -> Result<GDriveResponse, FileHostError> {
	let secret_file = state.config.client_secret_file.clone();
	let use_email = state.config.email_service_url.clone().unwrap_or("".to_string());

	let reader = ReadDrive::new(use_email, secret_file)?;
	let file = reader.get_file_metadata(image_id).await?;
	let bytes = reader.download_file(image_id).await?;
	let size = file.size.unwrap().try_into().unwrap();

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

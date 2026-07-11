use crate::metrics::otel::{record_cache_hit, OperationTimer};
use crate::models::gdrive::{ListQuery, UpsertResponse};
use crate::{AppState, FileHostError};
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap};
use axum::Json;
use bytes::Bytes;
use sdk::{FileListPage, FileMetadata};
use some_cache::DedupCacheError;
use tracing::instrument;

const DEFAULT_PAGE_SIZE: i32 = 100;

#[axum::debug_handler]
#[instrument(name = "list_gdrive_root", skip(state, query), fields(otel.kind = "server"))]
pub async fn list_gdrive_root(State(state): State<AppState>, Query(query): Query<ListQuery>) -> Result<Json<FileListPage>, FileHostError> {
	list_gdrive_files(state, None, query).await
}

#[axum::debug_handler]
#[instrument(name = "list_gdrive_folder", skip(state, query), fields(folder_id = %folder_id, otel.kind = "server"))]
pub async fn list_gdrive_folder(State(state): State<AppState>, Path(folder_id): Path<String>, Query(query): Query<ListQuery>) -> Result<Json<FileListPage>, FileHostError> {
	list_gdrive_files(state, Some(folder_id), query).await
}

async fn list_gdrive_files(state: AppState, folder_id: Option<String>, query: ListQuery) -> Result<Json<FileListPage>, FileHostError> {
	let page_size = query.page_size.unwrap_or(DEFAULT_PAGE_SIZE);
	let cache_key = format!(
		"gdrive_list_{}_{}_{}",
		folder_id.as_deref().unwrap_or("root"),
		page_size,
		query.page_token.as_deref().unwrap_or("first")
	);

	let _timer = OperationTimer::new("list_gdrive_files", "total");

	let (page, was_cached) = state
		.realtime
		.dedup_cache
		.get_or_fetch(&cache_key, || async {
			state
				.external
				.gdrive_reader
				.list_files(folder_id.as_deref(), page_size, query.page_token.as_deref())
				.await
				.map_err(|e| DedupCacheError::OperationError(e.to_string()))
		})
		.await?;

	record_cache_hit("list_gdrive_files", was_cached);

	Ok(Json(page))
}

#[axum::debug_handler]
#[instrument(name = "read_gdrive_json", skip(state), fields(file_id = %file_id, otel.kind = "server"))]
pub async fn read_gdrive_json(State(state): State<AppState>, Path(file_id): Path<String>) -> Result<Json<serde_json::Value>, FileHostError> {
	let cache_key = format!("gdrive_json_{}", file_id);

	let _timer = OperationTimer::new("read_gdrive_json", "total");

	let (value, was_cached) = state
		.realtime
		.dedup_cache
		.get_or_fetch(&cache_key, || async {
			let bytes = state
				.external
				.gdrive_reader
				.download_file(&file_id)
				.await
				.map_err(|e| DedupCacheError::OperationError(e.to_string()))?;
			serde_json::from_slice::<serde_json::Value>(&bytes).map_err(|e| DedupCacheError::OperationError(e.to_string()))
		})
		.await?;

	record_cache_hit("read_gdrive_json", was_cached);

	Ok(Json(value))
}

/// Upserts `body` as `name` inside `folder_id`: updates the file in place if
/// one already exists by that name in that folder, otherwise creates it.
///
/// Cache-key strategy: a write is never itself cached (PUT isn't a cacheable
/// GET). Instead, on success it invalidates the specific read entries it
/// could have made stale — the file's own JSON cache entry and the default
/// (first page, default size) listing for its folder. Other paginated views
/// of the folder fall back to their normal TTL.
#[axum::debug_handler]
#[instrument(name = "upsert_gdrive_file", skip(state, headers, body), fields(folder_id = %folder_id, name = %name, otel.kind = "server"))]
pub async fn upsert_gdrive_file(
	State(state): State<AppState>,
	Path((folder_id, name)): Path<(String, String)>,
	headers: HeaderMap,
	body: Bytes,
) -> Result<Json<UpsertResponse>, FileHostError> {
	let _timer = OperationTimer::new("upsert_gdrive_file", "total");

	let mime_type = headers.get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or("application/json");

	let existing = {
		let _lookup_timer = OperationTimer::new("upsert_gdrive_file", "find_existing");
		state.external.gdrive_reader.find_file_by_name(&folder_id, &name).await.map_err(FileHostError::upstream)?
	};

	let (file, created): (FileMetadata, bool) = match existing {
		Some(existing_file) => {
			let _write_timer = OperationTimer::new("upsert_gdrive_file", "update");
			let updated = state
				.external
				.gdrive_writer
				.update_file_bytes(&existing_file.id, body.to_vec(), Some(mime_type))
				.await
				.map_err(FileHostError::upstream)?;
			(updated, false)
		}
		None => {
			let _write_timer = OperationTimer::new("upsert_gdrive_file", "create");
			let created_file = state
				.external
				.gdrive_writer
				.upload_bytes(&name, Some(&folder_id), body.to_vec(), Some(mime_type))
				.await
				.map_err(FileHostError::upstream)?;
			(created_file, true)
		}
	};

	let default_list_key = format!("gdrive_list_{}_{}_first", folder_id, DEFAULT_PAGE_SIZE);
	let _ = state.realtime.dedup_cache.delete(&format!("gdrive_json_{}", file.id)).await;
	let _ = state.realtime.dedup_cache.delete(&default_list_key).await;

	Ok(Json(UpsertResponse { file, created }))
}

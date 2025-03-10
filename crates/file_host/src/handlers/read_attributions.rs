use crate::{
	models::gsheet::{validate_range, RangeQuery},
	AppState, FileHostError,
};
use axum::extract::{Path, Query, State};
use axum::Json;
use sdk::ReadSheets;
use std::sync::Arc;

#[axum::debug_handler]
pub async fn get(
	State(state): State<Arc<AppState>>,
	Path(sheet_id): Path<String>,
	Query(range_query): Query<RangeQuery>,
) -> Result<Json<Vec<Vec<std::string::String>>>, FileHostError> {
	if let Some(cached_data) = state.cache_store.get_json(&sheet_id).await? {
		log::info!("Cache hit for key: {}", sheet_id);
		return Ok(Json(cached_data));
	}

	let range = range_query.range.ok_or_else(|| FileHostError::InvalidData)?;

	if !validate_range(&range) {
		return Err(FileHostError::SheetError(sdk::SheetError::InvalidRange(range.into())));
	}

	let secret_file = state.config.client_secret_file.clone();
	let use_email = state.config.email_service_url.clone().unwrap_or("".to_string());

	let reader = ReadSheets::new(use_email, secret_file)?;

	let data = reader.read_data(&sheet_id, &range).await?;

	if data.len() <= 100 {
		log::info!("Caching data for key: {}", &sheet_id);
		state.cache_store.set_json(&sheet_id, &data).await?;
	} else {
		log::info!("Data too large to cache (size: {})", data.len());
	}

	Ok(Json(data))
}

use crate::{
	models::gsheet::{validate_range, Attribution, FromGSheet, RangeQuery},
	AppState, FileHostError,
};
use axum::extract::{Path, Query, State};
use axum::Json;
use sdk::ReadSheets;
use std::sync::Arc;

#[axum::debug_handler]
pub async fn get(State(state): State<Arc<AppState>>, Path(id): Path<String>, Query(q): Query<RangeQuery>) -> Result<Json<Vec<Attribution>>, FileHostError> {
	if let Some(cached_data) = state.cache_store.get_json(&id).await? {
		log::info!("Cache hit for key: {}", &id);
		let attributions = Attribution::from_gsheet(&cached_data, true)?;
		return Ok(Json(attributions));
	}

	let q = extract_and_validate_range(q)?;
	let data = refetch(&state, &id, &q).await?;
	log::info!("data is: {:?}", &data);

	let attributions = Attribution::from_gsheet(&data, true)?;
	log::info!("attributions parsed: {:?}", attributions);

	if data.len() <= 100 {
		log::info!("Caching data for key: {}", &id);
		state.cache_store.set_json(&id, &data).await?;
	} else {
		log::info!("Data too large to cache (size: {})", data.len());
	}

	Ok(Json(attributions))
}

async fn refetch(state: &Arc<AppState>, sheet_id: &str, q: &str) -> Result<Vec<Vec<String>>, FileHostError> {
	let secret_file = state.config.client_secret_file.clone();
	let use_email = state.config.email_service_url.clone().unwrap_or("".to_string());

	let reader = ReadSheets::new(use_email, secret_file)?;

	let data = reader.read_data(&sheet_id, q).await?;

	Ok(data)
}

fn extract_and_validate_range(q: RangeQuery) -> Result<String, FileHostError> {
	let range = q.range.ok_or(FileHostError::InvalidData)?;
	if !validate_range(&range) {
		return Err(FileHostError::SheetError(sdk::SheetError::InvalidRange(range.into())));
	}
	Ok(range)
}

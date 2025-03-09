use crate::{AppState, FileHostError};
use axum::extract::Path;
use axum::extract::State;
use axum::Json;
use sdk::ReadSheets;
use std::sync::Arc;

#[axum::debug_handler]
pub async fn get(State(state): State<Arc<AppState>>, Path(sheet_id): Path<String>) -> Result<Json<Vec<Vec<std::string::String>>>, FileHostError> {
	if let Some(cached_data) = state.cache_store.get_json(&sheet_id).await? {
		log::info!("Cache hit for key: {}", sheet_id);
		return Ok(Json(cached_data));
	}

	let secret_file = state.config.client_secret_file.clone();
	let use_email = state.config.email_service_url.clone().unwrap_or("".to_string());

	let reader = ReadSheets::new(use_email, secret_file)?;

	println!("\nReading data from sheet...");
	let range = "default!A1:B4";
	let data = reader.read_data(&sheet_id, range).await?;

	if data.len() <= 100 {
		log::info!("Caching data for key: {}", &sheet_id);
		state.cache_store.set_json(&sheet_id, &data).await?;
	} else {
		log::info!("Data too large to cache (size: {})", data.len());
	}

	Ok(Json(data))
}

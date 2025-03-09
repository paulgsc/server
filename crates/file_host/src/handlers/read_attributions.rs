use crate::CacheStore;
use crate::FileHostError;
use axum::extract::Path;
use axum::extract::State;
use axum::Json;
use sdk::ReadSheets;
use std::sync::Arc;

#[axum::debug_handler]
pub async fn get(State(state): State<Arc<CacheStore>>, Path(sheet_id): Path<String>) -> Result<Json<Vec<Vec<std::string::String>>>, FileHostError> {
	if let Some(cached_data) = state.get_json(&sheet_id).await? {
		log::info!("Cache hit for key: {}", sheet_id);
		return Ok(Json(cached_data));
	}

	let user_email = "foo@gmai.com".to_string();
	let client_secret_file = "client_secret_file.json".to_string();

	// Now the ? operator will automatically convert SheetError to FileHostError
	let reader = ReadSheets::new(user_email.clone(), client_secret_file.clone())?;

	println!("\nReading data from sheet...");
	let range = "default!A1:B4";
	let data = reader.read_data(&sheet_id, range).await?;

	if data.len() <= 100 {
		log::info!("Caching data for key: {}", &sheet_id);
		state.set_json(&sheet_id, &data).await?;
	} else {
		log::info!("Data too large to cache (size: {})", data.len());
	}

	Ok(Json(data))
}

use crate::CacheStore;
use crate::FileHostError;
use axum::extract::Path;
use axum::extract::State;
use axum::Json;
use sdk::ReadSheets;
use std::sync::Arc;

#[axum::debug_handler]
pub async fn get(State(_state): State<Arc<CacheStore>>, Path(sheet_id): Path<String>) -> Result<Json<&'static str>, FileHostError> {
	println!("this is the path id: {sheet_id}");
	let user_email = "foo@gmai.com".to_string();
	let client_secret_file = "client_secret_file.json".to_string();

	// Now the ? operator will automatically convert SheetError to FileHostError
	let reader = ReadSheets::new(user_email.clone(), client_secret_file.clone()).unwrap();

	println!("\nReading data from sheet...");
	let range = "default!A1:C4";
	let _data = reader.read_data("spreadsheet_id", range).await.unwrap();
	Ok(Json("hello world"))
}

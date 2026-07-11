use sdk::FileMetadata;
use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Deserialize)]
pub struct ListQuery {
	pub page_token: Option<String>,
	pub page_size: Option<i32>,
}

#[derive(Debug, Serialize)]
pub struct UpsertResponse {
	#[serde(flatten)]
	pub file: FileMetadata,
	pub created: bool,
}

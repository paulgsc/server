use serde::{Deserialize, Serialize};

pub async fn get_all_tabs(State(state): State<AppState>) -> impl IntoResponse {
	let result = sqlx::query_as!(Tab, "SELECT * FROM chrome_tabs").fetch_all(&*state.db_pool).await;

	match result {
		Ok(tabs) => Json(BrowserTabsBody { browserTab }),
		Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to get tabs: {:?}", e)).into_response(),
	}
}

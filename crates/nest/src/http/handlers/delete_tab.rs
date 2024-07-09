use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};

use crate::http::schema::browser_tab::BrowserTabs;

struct AppState {
	db_pool: sqlx::SqlitePool,
}

pub async fn delete_tab(State(state): State<AppState>, Json(tab): Json<BrowserTabs>) -> impl IntoResponse {
	let result = sqlx::query!("DELETE FROM browser_tabs WHERE id = ?", tab.id).execute(&*state.db_pool).await;

	match result {
		Ok(_) => StatusCode::OK.into_response(),
		Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to delete tab: {:?}", e)).into_response(),
	}
}

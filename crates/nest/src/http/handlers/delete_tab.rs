pub async fn delete_tab(State(state): State<AppState>, Json(tab): Json<Tab>) -> impl IntoResponse {
	let result = sqlx::query!("DELETE FROM chrome_tabs WHERE id = ?", tab.id).execute(&*state.db_pool).await;

	match result {
		Ok(_) => StatusCode::OK.into_response(),
		Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to delete tab: {:?}", e)).into_response(),
	}
}

pub async fn update_tab(State(state): State<AppState>, Json(tab): Json<Tab>) -> impl IntoResponse {
	let result = sqlx::query!(
        "UPDATE chrome_tabs SET status = ?, index = ?, opener_tab_id = ?, title = ?, url = ?, pending_url = ?, pinned = ?, highlighted = ?, window_id = ?, active = ?, fav_icon_url = ?, incognito = ?, selected = ?, audible = ?, discarded = ?, auto_discardable = ?, muted_info = ?, width = ?, height = ?, session_id = ?, group_id = ?, last_accessed = ? WHERE id = ?",
        tab.status,
        tab.index,
        tab.opener_tab_id,
        tab.title,
        tab.url,
        tab.pending_url,
        tab.pinned,
        tab.highlighted,
        tab.window_id,
        tab.active,
        tab.fav_icon_url,
        tab.incognito,
        tab.selected,
        tab.audible,
        tab.discarded,
        tab.auto_discardable,
        tab.muted_info,
        tab.width,
        tab.height,
        tab.session_id,
        tab.group_id,
        tab.last_accessed,
        tab.id
    )
    .execute(&*state.db_pool)
    .await;

	match result {
		Ok(_) => StatusCode::OK.into_response(),
		Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to update tab: {:?}", e)).into_response(),
	}
}

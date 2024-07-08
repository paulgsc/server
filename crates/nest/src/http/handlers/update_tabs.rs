use anyhow::Ok;
use axum::{extract::State, Json};
use sqlx::SqlitePool;

use crate::http::schema::browser_tab::BrowserTabs;

pub async fn update_tab(State(pool): State<SqlitePool>, Json(tab): Json<BrowserTabs>) -> Result<Json<BrowserTabs>, String> {
	let result = sqlx::query!(
        "UPDATE browser_tabs SET status = ?, index = ?, opener_tab_id = ?, title = ?, url = ?, pending_url = ?, pinned = ?, highlighted = ?, window_id = ?, active = ?, fav_icon_url = ?, incognito = ?, selected = ?, audible = ?, discarded = ?, auto_discardable = ?, muted_info = ?, width = ?, height = ?, session_id = ?, group_id = ?, last_accessed = ? WHERE id = ?",
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
    .execute(&pool)
    .await?
    .map_err(|e| e.to_string())?;

	Ok(Json(tab))
}

use axum::{extract::State, Json};
use sqlx::SqlitePool;

use crate::http::schema::browser_tab::BrowserTabs;

pub async fn create_tab(State(pool): State<SqlitePool>, Json(tab): Json<BrowserTabs>) -> Result<Json<BrowserTabs>, String> {
	let result = sqlx::query!(
        "INSERT INTO browser_tabs (status, index, opener_tab_id, title, url, pending_url, pinned, highlighted, window_id, active, fav_icon_url, incognito, selected, audible, discarded, auto_discardable, muted_info, width, height, session_id, group_id, last_accessed) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
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
        tab.last_accessed
    )
    .execute(pool)
    .await?;

	Ok(Json(tab))
}

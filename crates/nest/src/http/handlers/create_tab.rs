use axum::{extract::State, Json};
use sqlx::SqlitePool;

use crate::http::schema::browser_tab::BrowserTabs;

pub async fn create_tab(State(pool): State<SqlitePool>, Json(tab): Json<BrowserTabs>) -> Result<Json<BrowserTabs>, String> {
	let result = sqlx::query!(
		r#"
        INSERT INTO browser_tabs (
            status, tab_index, opener_tab_id, title, url, pending_url, pinned,
            highlighted, window_id, active, favicon_url, incognito, selected,
            audible, discarded, auto_discardable, muted_info, width, height, last_accessed
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        "#,
		tab.status,
		tab.tab_index,
		tab.opener_tab_id,
		tab.title,
		tab.url,
		tab.pending_url,
		tab.pinned,
		tab.highlighted,
		tab.window_id,
		tab.active,
		tab.favicon_url,
		tab.incognito,
		tab.selected,
		tab.audible,
		tab.discarded,
		tab.auto_discardable,
		tab.muted_info,
		tab.width,
		tab.height,
		tab.last_accessed
	)
	.execute(&pool)
	.await?;

	Ok(Json(tab))
}

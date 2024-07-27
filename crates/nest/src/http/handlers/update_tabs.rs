use axum::{extract::State, Json};
use sqlx::SqlitePool;

use crate::http::schema::browser_tab::BrowserTabs;
use crate::http::error::Error;

pub async fn update_tab(State(pool): State<SqlitePool>, Json(tab): Json<BrowserTabs>) -> Result<Json<BrowserTabs>, Error> {
	sqlx::query!(
        r#" 
        UPDATE browser_tabs SET status = ?, tab_index = ?, opener_tab_id = ?, title = ?, url = ?, pending_url = ?, pinned = ?, highlighted = ?,
        window_id = ?, active = ?, favicon_url = ?, incognito = ?, selected = ?, audible = ?, discarded = ?, auto_discardable = ?, muted = ?,
        muted_reason = ?, muted_extension_id = ?, width = ?, height = ?, session_id = ?, group_id = ?, last_accessed = ?
        WHERE id = ?
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
        tab.muted,
        tab.muted_extension_id,
        tab.muted_reason,
        tab.width,
        tab.height,
        tab.session_id,
        tab.group_id,
        tab.last_accessed,
        tab.id
    )
    .execute(&pool)
    .await
    .map_err(|e| match e {
        sqlx::Error::Database(_) => Error::Sqlx(e),
        _ => Error::Anyhow(e.into()),
    })?;

	Ok(Json(tab))
}

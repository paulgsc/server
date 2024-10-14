use axum::{extract::State, Json};
use sqlx::SqlitePool;

use crate::schema::browser_tab::BrowserTabs;
use nest::http::error::{Error, ResultExt};

pub async fn create_tab(State(pool): State<SqlitePool>, Json(tab): Json<BrowserTabs>) -> Result<Json<BrowserTabs>, Error> {
	sqlx::query!(
		r#"
        INSERT INTO browser_tabs (
            status, tab_index, opener_tab_id, title, url, pending_url, pinned,
            highlighted, window_id, active, favicon_url, incognito, selected,
            audible, discarded, auto_discardable, muted, muted_reason, muted_extension_id, width, height, last_accessed, group_id, session_id
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
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
		tab.muted_reason,
		tab.muted_extension_id,
		tab.width,
		tab.height,
		tab.last_accessed,
		tab.group_id,
		tab.session_id
	)
	.execute(&pool)
	.await
	.on_constraint("window_id", |_| Error::unprocessable_entity(vec![("window_id", "window_id taken")]))?;

	Ok(Json(tab))
}

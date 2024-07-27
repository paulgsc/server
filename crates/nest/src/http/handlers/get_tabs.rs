use axum::{extract::State, Json};
use sqlx::SqlitePool;

use crate::http::schema::browser_tab::BrowserTabs;
use crate::http::error::Error;

pub async fn get_all_tabs(State(pool): State<SqlitePool>) -> Result<Json<Vec<BrowserTabs>>, Error> {
    let tabs = sqlx::query_as!(
        BrowserTabs,
        r#"
        SELECT
            id, status, tab_index, opener_tab_id, title, url, pending_url, pinned,
            highlighted, window_id, active, favicon_url, incognito, selected,
            audible, discarded, auto_discardable, muted, muted_reason, muted_extension_id,
            width, height, last_accessed, group_id, session_id
        FROM browser_tabs
        "#
        )
        .fetch_all(&pool)
        .await
        .map_err(|e| match e {
            sqlx::Error::Database(_) => Error::Sqlx(e),
            _ => Error::Anyhow(e.into()),
        })?;

    Ok(Json(tabs))

}

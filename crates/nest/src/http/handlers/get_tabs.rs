use anyhow::Ok;
use axum::{extract::State, Json};
use sqlx::SqlitePool;

use crate::http::schema::browser_tab::BrowserTabs;

pub async fn get_all_tabs(State(pool): State<SqlitePool>, Json(tab): Json<BrowserTabs>) -> Result<Json<BrowserTabs>, String> {
	let tabs = sqlx::query_as!(BrowserTabs, "SELECT * FROM browser_tabs ORDER BY id ASC")
		.fetch_all(&pool)
		.await
		.map_err(|e| e.to_string())?;

	Ok(Json(tabs))
}

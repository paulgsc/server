use axum::{extract::State, Json};
use sqlx::SqlitePool;

use crate::models::Tab;

pub async fn create_tab(State(pool): State<SqlitePool>, Json(tab): Json<Tab>) -> Result<Json<Tab>, String> {
	let result = sqlx::query!("INSERT INTO tabs (title, url) VALUES (?, ?)", tab.title, tab.url)
		.execute(&pool)
		.await
		.map_err(|e| e.to_string())?;

	tab.id = Some(result.last_insert_rowid() as i64);
	Ok(Json(tab))
}

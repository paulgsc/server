use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BrowserTabsBody {
	browserTab: BrowserTabs,
}

pub async fn get_tabs(State(pool): State<SqlitePool>) -> Result<Json<Vec<Tab>>, String> {
	let tabs = sqlx::query_as!(Tab, "SELECT id, title, url FROM tabs ORDER BY id DESC")
		.fetch_all(&pool)
		.await
		.map_err(|e| e.to_string())?;

	Ok(Json(BrowserTabsBody { browserTab }))
}

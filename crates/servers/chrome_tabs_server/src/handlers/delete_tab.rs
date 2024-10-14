use axum::{extract::State, Json};
use sqlx::SqlitePool;
use crate::http::error::Error;

#[derive(serde::Deserialize)]
pub struct DeleteTabRequest {
    pub id: i64,
}

pub async fn delete_tab(State(pool): State<SqlitePool>, Json(tab): Json<DeleteTabRequest>) ->  Result<Json<String>, Error> {
	let result = sqlx::query!("DELETE FROM browser_tabs WHERE id = ?", tab.id).execute(&pool).await
        .map_err(|e| {
            if let sqlx::Error::RowNotFound = e {
                Error::NotFound
            } else {
                Error::Sqlx(e)
            }
        })?;

    if result.rows_affected() == 0 {
        return Err(Error::NotFound);
    }

    Ok(Json("Tab deleted successfully".to_string()))
}


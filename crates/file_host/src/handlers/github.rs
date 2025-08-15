use crate::{metrics::http::OPERATION_DURATION, timed_operation, AppState, FileHostError};
use axum::{extract::State, Json};
use tracing::instrument;

use sdk::Repository;

#[axum::debug_handler]
#[instrument(name = "get_github_repos", skip(state))]
pub async fn get_github_repos(State(state): State<AppState>) -> Result<Json<Vec<Repository>>, FileHostError> {
	let cache_key = format!("get_github_repos");

	let cache_result = timed_operation!("get_github_repos", "cached_check", true, { state.cache_store.get::<Vec<Repository>>(&cache_key).await })?;

	if let Some(cached_data) = cache_result {
		return Ok(Json(cached_data));
	}

	let data = timed_operation!("get_github_repos", "fetch_data", false, { refetch(state.clone(),).await })?;

	timed_operation!("get_github_repos", "cache_set", false, {
		state.cache_store.set(&cache_key, &data, None).await?;
	});

	Ok(Json(data))
}

#[instrument(name = "refetch", skip(state))]
async fn refetch(state: AppState) -> Result<Vec<Repository>, FileHostError> {
	let data = timed_operation!("refetch", "get_repositories", false, { state.github_client.get_repositories().await })?;

	Ok(data)
}

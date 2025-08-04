use crate::{
	metrics::http::{CACHE_OPERATIONS, OPERATION_DURATION},
	record_cache_op, timed_operation, AppState, FileHostError,
};
use axum::{extract::State, Json};
use std::sync::Arc;
use tracing::instrument;

use sdk::{GitHubClient, Repository};

#[axum::debug_handler]
#[instrument(name = "get_github_repos", skip(state))]
pub async fn get_github_repos(State(state): State<Arc<AppState>>) -> Result<Json<Vec<Repository>>, FileHostError> {
	let cache_key = format!("get_github_repos");

	let cache_result = timed_operation!("get_github_repos", "cached_check", true, {
		state.cache_store.get_json::<Vec<Repository>>(&cache_key).await
	})?;

	if let Some(cached_data) = cache_result {
		record_cache_op!("get_github_repos", "get", "hit");

		return Ok(Json(cached_data));
	}

	record_cache_op!("get_github_repos", "get", "miss");

	let data = timed_operation!("get_github_repos", "fetch_data", false, { refetch(&state,).await })?;

	if data.len() <= 1000 {
		timed_operation!("get_github_repos", "cache_set", false, {
			async {
				match state.cache_store.set_json(&cache_key, &data).await {
					Ok(_) => {
						record_cache_op!("get_github_repos", "set", "success");
						tracing::info!("Caching data for key: {}", &cache_key);
					}
					Err(e) => {
						record_cache_op!("get_github_repos", "set", "error");
						tracing::warn!("Failed to cache data: {}", e);
					}
				}
			}
		})
		.await;
	} else {
		tracing::info!("Data too large to cache (size: {})", data.len());
	}

	Ok(Json(data))
}

#[instrument(name = "refetch", skip(state))]
async fn refetch(state: &Arc<AppState>) -> Result<Vec<Repository>, FileHostError> {
	let client = timed_operation!("refetch", "create_github_client", false, { GitHubClient::new(state.config.github_token.clone()) })?;

	let data = timed_operation!("refetch", "get_repositories", false, { client.get_repositories().await })?;

	Ok(data)
}

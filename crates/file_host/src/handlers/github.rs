use crate::{metrics::http::OPERATION_DURATION, timed_operation, AppState, FileHostError};
use axum::{extract::State, Json};
use sdk::Repository;
use tracing::instrument;

#[axum::debug_handler]
#[instrument(name = "get_github_repos", skip(state))]
pub async fn get_github_repos(State(state): State<AppState>) -> Result<Json<Vec<Repository>>, FileHostError> {
	let cache_key = "get_github_repos".to_string();

	let (repositories, _) = state
		.dedup_cache
		.get_or_fetch(&cache_key, || async {
			timed_operation!("get_github_repos", "fetch_data", false, { fetch_github_repositories(state.clone()).await })
		})
		.await?;

	Ok(Json(repositories))
}

/// Fetch repositories from GitHub API
#[instrument(name = "fetch_github_repositories", skip(state))]
async fn fetch_github_repositories(state: AppState) -> Result<Vec<Repository>, FileHostError> {
	let repositories = timed_operation!("fetch_github_repositories", "get_repositories", false, { state.github_client.get_repositories().await })?;

	Ok(repositories)
}

/// Alternative implementation with custom TTL for GitHub data
/// GitHub repository data changes less frequently, so we might want longer cache times
#[axum::debug_handler]
#[instrument(name = "get_github_repos_with_ttl", skip(state))]
pub async fn get_github_repos_with_ttl(State(state): State<AppState>) -> Result<Json<Vec<Repository>>, FileHostError> {
	let cache_key = "get_github_repos_ttl".to_string();
	// Cache GitHub repos for 1 hour (3600 seconds) since they don't change frequently
	let ttl = Some(3600);

	let (repositories, _) = state
		.dedup_cache
		.get_or_fetch_with_ttl(&cache_key, ttl, || async {
			timed_operation!("get_github_repos_with_ttl", "fetch_data", false, { fetch_github_repositories(state.clone()).await })
		})
		.await?;

	Ok(Json(repositories))
}

use crate::metrics::otel::{record_cache_hit, OperationTimer};
use crate::{AppState, DedupError, FileHostError};
use axum::{extract::State, Json};
use sdk::Repository;
use tracing::instrument;

#[axum::debug_handler]
#[instrument(name = "get_github_repos", skip(state), fields(otel.kind = "server"))]
pub async fn get_github_repos(State(state): State<AppState>) -> Result<Json<Vec<Repository>>, FileHostError> {
	let _timer = OperationTimer::new("get_github_repos", "total");

	let cache_key = "get_github_repos".to_string();

	let (repositories, was_cached) = state
		.realtime
		.dedup_cache
		.get_or_fetch(&cache_key, || async {
			let _fetch_timer = OperationTimer::new("get_github_repos", "fetch_data");
			fetch_github_repositories(state.clone()).await
		})
		.await?;

	record_cache_hit("get_github_repos", was_cached);

	Ok(Json(repositories))
}

/// Fetch repositories from GitHub API
#[instrument(name = "fetch_github_repositories", skip(state), fields(otel.kind = "internal"))]
async fn fetch_github_repositories(state: AppState) -> Result<Vec<Repository>, DedupError> {
	let _timer = OperationTimer::new("fetch_github_repositories", "get_repositories");

	let repositories = state.external.github_client.get_repositories().await?;

	Ok(repositories)
}

/// Alternative implementation with custom TTL for GitHub data
/// GitHub repository data changes less frequently, so we might want longer cache times
#[axum::debug_handler]
#[instrument(name = "get_github_repos_with_ttl", skip(state), fields(otel.kind = "server", cache_ttl_seconds = 3600))]
#[allow(dead_code)]
pub async fn get_github_repos_with_ttl(State(state): State<AppState>) -> Result<Json<Vec<Repository>>, FileHostError> {
	let _timer = OperationTimer::new("get_github_repos_with_ttl", "total");

	let cache_key = "get_github_repos_ttl".to_string();

	// Cache GitHub repos for 1 hour (3600 seconds) since they don't change frequently
	let ttl = Some(3600);

	let (repositories, was_cached) = state
		.realtime
		.dedup_cache
		.get_or_fetch_with_ttl(&cache_key, ttl, || async {
			let _fetch_timer = OperationTimer::new("get_github_repos_with_ttl", "fetch_data");
			fetch_github_repositories(state.clone()).await
		})
		.await?;

	record_cache_hit("get_github_repos_with_ttl", was_cached);

	Ok(Json(repositories))
}

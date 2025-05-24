use axum::{
	extract::State,
	http::StatusCode,
	response::{IntoResponse, Response},
	routing::get,
	Json, Router,
};
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tokio::sync::RwLock;

use crate::github::{client::GitHubClient, models::Repository};

// AppState for the Axum handlers
pub struct AppState {
	github_client: GitHubClient,
	org_name: String,
	// Cache for repositories data
	repos_cache: RwLock<Option<(Vec<Repository>, SystemTime)>>,
}

// Axum handler for fetching repositories
async fn get_repositories(State(state): State<Arc<AppState>>) -> Result<Json<Vec<Repository>>, StatusCode> {
	// Check if we have a valid cache
	{
		let cache = state.repos_cache.read().await;
		if let Some((repos, cache_time)) = &*cache {
			let elapsed = cache_time.elapsed().unwrap_or(Duration::from_secs(0));
			// Cache is valid for 5 minutes
			if elapsed.as_secs() < 300 {
				return Ok(Json(repos.clone()));
			}
		}
	}

	// Cache is invalid or doesn't exist, fetch new data
	let repositories = state.github_client.get_repositories(&state.org_name).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

	// Update cache
	{
		let mut cache = state.repos_cache.write().await;
		*cache = Some((repositories.clone(), SystemTime::now()));
	}

	Ok(Json(repositories))
}

// Error handler for API errors
pub async fn handle_error(error: StatusCode) -> Response {
	let body = Json(serde_json::json!({
	"error": error.to_string()
			}));

	(error, body).into_response()
}

// Function to configure the GitHub API routes
pub fn github_routes() -> Router<Arc<AppState>> {
	Router::new().route("/api/github/repositories", get(get_repositories))
}

// Function to create and initialize the AppState
pub fn create_github_state(github_token: String, org_name: String) -> Arc<AppState> {
	let github_client = GitHubClient::new(github_token);

	Arc::new(AppState {
		github_client,
		org_name,
		repos_cache: RwLock::new(None),
	})
}

// Function to register the GitHub API handlers with an existing Axum app
pub fn register_github_api(app: Router, github_token: String, org_name: String) -> Router {
	let state = create_github_state(github_token, org_name);
	app.merge(github_routes()).with_state(state)
}

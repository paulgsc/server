use crate::{AppState, FileHostError};

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

use sdk::{GitHubClient, Repository};

// AppState for the Axum handlers
pub struct AppState {
	github_client: GitHubClient,
	org_name: String,
	// Cache for repositories data
	repos_cache: RwLock<Option<(Vec<Repository>, SystemTime)>>,
}

#[axum::debug_handler]
#[instrument(name = "get_repositories", skip(state), fields(sheet_id = %id))]
async fn get_repositories(State(state): State<Arc<AppState>>) -> Result<Json<Vec<Repository>>, FileHostError> {
	let cache_key = format!("get_attributions_{}", id);

	let cache_result = timed_operation!("get_attributions", "cached_check", true, { state.cache_store.get_json(&cache_key).await })?;

	if let Some(cached_data) = cache_result {
		record_cache_op!("get_attributions", "get", "hit");

		let attributions = timed_operation!("get_attributions", "deserialzie_cache", true, { Attribution::from_gsheet(&cached_data, true) })?;

		return Ok(Json(attributions));
	}

	record_cache_op!("get_attributions", "get", "miss");

	let data = timed_operation!("get_attributions", "fetch_data", false, { refetch(&state, &id, Some(&q)).await })?;

	// Cache is invalid or doesn't exist, fetch new data
	let repositories = state.github_client.get_repositories(&state.org_name).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

	if data.len() <= 100 {
		timed_operation!("get_attributions", "cache_set", false, {
			async {
				match state.cache_store.set_json(&cache_key, &data).await {
					Ok(_) => {
						record_cache_op!("get_attributions", "set", "success");
						tracing::info!("Caching data for key: {}", &cache_key);
					}
					Err(e) => {
						record_cache_op!("get_attributions", "set", "error");
						tracing::warn!("Failed to cache data: {}", e);
					}
				}
			}
		})
		.await;
	} else {
		tracing::info!("Data too large to cache (size: {})", data.len());
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

async fn refetch(state: &Arc<AppState>, sheet_id: &str, q: Option<&str>) -> Result<Vec<Vec<String>>, FileHostError> {
	let secret_file = state.config.client_secret_file.clone();
	let use_email = state.config.email_service_url.clone().unwrap_or("".to_string());

	let reader = ReadSheets::new(use_email, secret_file)?;

	let data = match q {
		Some(query) => reader.read_data(sheet_id, query).await?,
		None => {
			let res = reader.retrieve_all_sheets_data(sheet_id).await?;
			let (_, v) = match res.into_iter().next() {
				Some(pair) => pair,
				None => return Err(FileHostError::UnexpectedSinglePair),
			};
			v
		}
	};

	Ok(data)
}

use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::de::DeserializeOwned;
use std::error::Error;
use std::fmt;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Repository {
	pub name: String,
	pub description: String,
	pub packages: Vec<Package>,
	pub expanded: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Package {
	pub id: String,
	pub name: String,
	pub description: String,
	#[serde(with = "chrono::serde::ts_milliseconds")]
	pub last_activity: DateTime<Utc>,
	pub status: PackageStatus,
	pub commit_count: u32,
	pub contributors: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageStatus {
	pub name: String,
	pub color: String,
	pub bg_color: String,
	pub border_color: String,
}

// GitHub API response types
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct GitHubRepository {
	pub name: String,
	pub description: Option<String>,
	pub updated_at: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct GitHubCommit {
	pub commit: GitHubCommitDetail,
	pub author: Option<GitHubAuthor>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct GitHubCommitDetail {
	pub author: GitHubCommitAuthor,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct GitHubCommitAuthor {
	pub date: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct GitHubAuthor {
	pub login: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct GitHubContributor {
	pub login: String,
	pub contributions: u32,
}

// Define package statuses similar to the frontend
pub fn get_status_map() -> HashMap<String, PackageStatus> {
	let mut statuses = HashMap::new();

	statuses.insert(
		"flourishing".to_string(),
		PackageStatus {
			name: "flourishing".to_string(),
			color: "text-emerald-500".to_string(),
			bg_color: "bg-emerald-100".to_string(),
			border_color: "border-emerald-200".to_string(),
		},
	);

	statuses.insert(
		"growing".to_string(),
		PackageStatus {
			name: "growing".to_string(),
			color: "text-green-500".to_string(),
			bg_color: "bg-green-100".to_string(),
			border_color: "border-green-200".to_string(),
		},
	);

	statuses.insert(
		"stale".to_string(),
		PackageStatus {
			name: "stale".to_string(),
			color: "text-amber-500".to_string(),
			bg_color: "bg-amber-100".to_string(),
			border_color: "border-amber-200".to_string(),
		},
	);

	statuses.insert(
		"neglected".to_string(),
		PackageStatus {
			name: "neglected".to_string(),
			color: "text-orange-500".to_string(),
			bg_color: "bg-orange-100".to_string(),
			border_color: "border-orange-200".to_string(),
		},
	);

	statuses.insert(
		"abandoned".to_string(),
		PackageStatus {
			name: "abandoned".to_string(),
			color: "text-red-500".to_string(),
			bg_color: "bg-red-100".to_string(),
			border_color: "border-red-200".to_string(),
		},
	);

	statuses
}

// Function to determine status based on last activity date
pub fn get_status(last_activity: DateTime<Utc>) -> PackageStatus {
	let now = Utc::now();
	let diff = now.signed_duration_since(last_activity);
	let diff_days = diff.num_days();

	let statuses = get_status_map();

	if diff_days < 3 {
		statuses.get("flourishing").unwrap().clone()
	} else if diff_days < 7 {
		statuses.get("growing").unwrap().clone()
	} else if diff_days < 14 {
		statuses.get("stale").unwrap().clone()
	} else if diff_days < 30 {
		statuses.get("neglected").unwrap().clone()
	} else {
		statuses.get("abandoned").unwrap().clone()
	}
}

// Custom error type for GitHub client
#[derive(Debug)]
pub enum GitHubError {
	RequestFailed(reqwest::Error),
	ApiError(u16, String),
	ParseError(String),
}

impl fmt::Display for GitHubError {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			GitHubError::RequestFailed(err) => write!(f, "Request failed: {}", err),
			GitHubError::ApiError(status, message) => write!(f, "API error {}: {}", status, message),
			GitHubError::ParseError(err) => write!(f, "Parse error: {}", err),
		}
	}
}

impl Error for GitHubError {}

impl From<reqwest::Error> for GitHubError {
	fn from(err: reqwest::Error) -> Self {
		GitHubError::RequestFailed(err)
	}
}

pub struct GitHubClient {
	client: Client,
	token: String,
}

impl GitHubClient {
	pub fn new(token: String) -> Self {
		let client = Client::builder().timeout(std::time::Duration::from_secs(10)).build().expect("Failed to create HTTP client");

		Self { client, token }
	}

	// Helper function to make GitHub API requests
	async fn request<T>(&self, url: &str) -> Result<T, GitHubError>
	where
		T: DeserializeOwned,
	{
		let response = self
			.client
			.get(url)
			.header("User-Agent", "rust-github-client")
			.header("Authorization", format!("token {}", self.token))
			.send()
			.await?;

		let status = response.status().as_u16();
		if !response.status().is_success() {
			let error_text = response.text().await.unwrap_or_else(|_| "Unknown error".to_string());
			return Err(GitHubError::ApiError(status, error_text));
		}

		response.json::<T>().await.map_err(|e| GitHubError::ParseError(e.to_string()))
	}

	// Fetch all repositories for an organization
	pub async fn get_repositories(&self, org_name: &str) -> Result<Vec<models::Repository>, GitHubError> {
		let repos_url = format!("https://api.github.com/orgs/{}/repos?per_page=100", org_name);
		let github_repos: Vec<models::GitHubRepository> = self.request(&repos_url).await?;

		let mut repositories = Vec::new();

		for repo in github_repos {
			// For each repository, fetch its packages (we'll use directories as mock packages)
			let contents_url = format!("https://api.github.com/repos/{}/{}/contents", org_name, repo.name);
			let contents: Vec<serde_json::Value> = match self.request(&contents_url).await {
				Ok(contents) => contents,
				Err(_) => continue, // Skip if we can't fetch contents
			};

			// Fetch commits to get activity data
			let commits_url = format!("https://api.github.com/repos/{}/{}/commits?per_page=100", org_name, repo.name);
			let commits: Vec<models::GitHubCommit> = match self.request(&commits_url).await {
				Ok(commits) => commits,
				Err(_) => vec![], // Use empty vector if we can't fetch commits
			};

			// Fetch contributors
			let contributors_url = format!("https://api.github.com/repos/{}/{}/contributors", org_name, repo.name);
			let contributors: Vec<models::GitHubContributor> = match self.request(&contributors_url).await {
				Ok(contributors) => contributors,
				Err(_) => vec![], // Use empty vector if we can't fetch contributors
			};

			let contributor_count = contributors.len() as u32;

			// Create packages for each directory in the repository
			let mut packages = Vec::new();

			for (i, content) in contents.iter().enumerate().take(10) {
				// Limit to 10 packages per repo
				if let Some(content_type) = content.get("type").and_then(|t| t.as_str()) {
					if content_type == "dir" {
						let pkg_name = content.get("name").and_then(|n| n.as_str()).unwrap_or("unnamed").to_string();

						// Parse the last activity date
						let last_activity = if !commits.is_empty() {
							let commit_date = &commits[0].commit.author.date;
							DateTime::parse_from_rfc3339(commit_date).map(|dt| dt.with_timezone(&Utc)).unwrap_or_else(|_| Utc::now())
						} else {
							Utc::now()
						};

						// Create package data
						packages.push(models::Package {
							id: format!("{}-pkg-{}", repo.name, i),
							name: pkg_name.clone(),
							description: format!("Package {} in {}", pkg_name, repo.name),
							last_activity,
							status: models::get_status(last_activity),
							commit_count: commits.len() as u32,
							contributors: contributor_count,
						});
					}
				}
			}

			// If we found no packages, create at least one mock package
			if packages.is_empty() {
				let last_activity = if !commits.is_empty() {
					let commit_date = &commits[0].commit.author.date;
					DateTime::parse_from_rfc3339(commit_date).map(|dt| dt.with_timezone(&Utc)).unwrap_or_else(|_| Utc::now())
				} else {
					Utc::now()
				};

				packages.push(models::Package {
					id: format!("{}-pkg-0", repo.name),
					name: format!("{}-main", repo.name),
					description: format!("Main package for {}", repo.name),
					last_activity,
					status: models::get_status(last_activity),
					commit_count: commits.len() as u32,
					contributors: contributor_count,
				});
			}

			// Add the repository to our list
			repositories.push(models::Repository {
				name: repo.name,
				description: repo.description.unwrap_or_else(|| "No description".to_string()),
				packages,
				expanded: true,
			});
		}

		Ok(repositories)
	}
}

use sdk::*;

#[tokio::main]
async fn main() {
	let token = "foo".to_string();

	let client = GitHubClient::new(token);

	match client.get_repositories().await {
		Ok(repos) => {
			println!("Fetched {} repositories:", repos.len());
			for repo in repos {
				println!("- {}", repo.name);
			}
		}
		Err(err) => {
			eprintln!("Error fetching repos: {:?}", err);
		}
	}
}

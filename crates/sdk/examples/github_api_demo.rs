use sdk::*;

#[tokio::main]
async fn main() {
<<<<<<< Updated upstream
	let token = "foo foo".to_string();
=======
	let token = "foo".to_string();
>>>>>>> Stashed changes

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

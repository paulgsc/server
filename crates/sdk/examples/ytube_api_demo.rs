use sdk::*;
use tokio;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
	rustls::crypto::ring::default_provider()
		.install_default()
		.map_err(|_| YtubeError::ServiceInit(format!("Failed to initialize crypto provider: ")))?;

	// Initialize clients with a user email
	let client_secret_file = "client_secret_file.json".to_string();

	// Use the `?` operator to propagate any errors from the `new` methods
	println!("Initializing reader and writer ...");
	let reader = ReadYouTube::new(client_secret_file.clone())?;
	let _writer = UpdateYouTube::new(client_secret_file)?;

	println!("attempting to get metadata...");
	let metadata = reader.get_video_metadata("av15VfIIo7I").await?;
	println!("{:#?}", metadata);  // Pretty prints the entire struct


	Ok(())
}

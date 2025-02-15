use sdk::*;
use tokio;

#[tokio::main]
async fn main() -> Result<(), GmailServiceError> {
	rustls::crypto::ring::default_provider()
		.install_default()
		.map_err(|_| GmailServiceError::ServiceInit(format!("Failed to initialize crypto provider: ")))?;

	let user_email = "streakfor@gmail.com";
	let client_secret_path = ".googleapis/oauth/client_secret_file.json".to_string();

	let gmail = ReadGmail::new(user_email, client_secret_path, Some("oauth"))?;

	let queries = vec![None, Some("in:inbox"), Some("after:2024/01/01")];

	for query in queries {
		println!("\nTesting query: {:?}", query);
		let message_ids = gmail.list_message_ids(query, 5).await?;
		for message_id in message_ids {
			let email_content = gmail.get_message_content(&message_id).await?;
			println!("{}", email_content);
		}
	}

	Ok(())
}

use sdk::*;
use tokio;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
	rustls::crypto::ring::default_provider()
		.install_default()
		.map_err(|_| GmailServiceError::ServiceInit(format!("Failed to initialize crypto provider: ")))?;

	// Replace these with your actual credentials
	let user_email = "aulgondu@gmail.com";
	let client_secret_path = "client_secret_file.json".to_string();

	// Example 1: List recent emails
	println!("=== Example 1: Listing Recent Emails ===");
	let gmail_reader = ReadGmail::new(user_email, client_secret_path.clone(), Some("service"))?;

	// Example 2: Search for specific emails
	println!("\n=== Example 2: Searching Emails ===");
	// Search for emails with "important" in subject
	let search_query = "subject:important";
	let search_results = gmail_reader.list_message_ids(Some(search_query), 5).await?;
	for result in search_results {
		println!("message_id: {}", result);
	}

	// Example 4: Send an email
	println!("\n=== Example 4: Sending Email ===");
	let gmail_sender = SendGmail::new(user_email, client_secret_path.clone(), Some("service"))?;

	let recipients = vec!["recipient@example.com".to_string()];
	let subject = "Test Email from Rust Gmail Client";
	let body = "This is a test email sent using the Rust Gmail client.\n\nBest regards,\nRust Gmail Client";

	let message_id = gmail_sender.send_email(recipients, subject, body).await?;
	println!("Email sent successfully! Message ID: {}", message_id);

	Ok(())
}

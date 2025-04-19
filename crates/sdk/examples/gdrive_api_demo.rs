use sdk::*;
use std::path::Path;
use tokio;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
	rustls::crypto::ring::default_provider()
		.install_default()
		.map_err(|_| SheetError::ServiceInit(format!("Failed to initialize crypto provider: ")))?;

	list_files_example().await?;
	//	search_files_example().await?;
	//	get_file_example().await?;
	// upload_file_example().await?;
	//	download_file_example().await?;

	Ok(())
}

pub async fn list_files_example() -> Result<(), DriveError> {
	let client_secret_path = "client_secret_file.json".to_string();
	let drive_client = ReadDrive::new("aulgondu@gmail.com".to_string(), client_secret_path)?;

	// List files in a specific folder (pass None to list files from the root)
	let folder_id = Some("1XBjkDVFE9xlYc7wWIWNAdKpUQvBN39ee");
	let page_size = 100; // Number of results to return

	let files = drive_client.list_files(folder_id, page_size).await?;

	println!("Found {} files:", files.len());
	for file in files {
		println!("ID: {}, Name: {}, Type: {}, Size: {:?}", file.id, file.name, file.mime_type, file.size);
	}

	Ok(())
}

pub async fn get_file_example() -> Result<(), DriveError> {
	let client_secret_path = "client_secret_file.json".to_string();
	let drive_client = ReadDrive::new("some-service@consulting-llc-6302f.iam.gserviceaccount.com".to_string(), client_secret_path)?;

	// List files in a specific folder (pass None to list files from the root)
	let file_id = Some("1_B-BTD0y3iYbyfChvntG6J7q0oumcxkE").unwrap();

	let file = drive_client.get_file_metadata(file_id).await?;

	println!("ID: {}, Name: {}, Type: {}, Size: {:?}", file.id, file.name, file.mime_type, file.size);

	Ok(())
}

pub async fn upload_file_example() -> Result<(), DriveError> {
	let client_secret_path = "client_secret_file.json".to_string();
	let user_email = "aulgondu@gmail.com".to_string();
	let drive_client = WriteToDrive::new(user_email.to_string(), client_secret_path)?;

	let file_path = Path::new("foo.txt");
	let parent_folder_id = Some("1XBjkDVFE9xlYc7wWIWNAdKpUQvBN39ee");
	let mime_type = Some("text/plain");

	let file = drive_client.upload_file(file_path, parent_folder_id, mime_type).await?;

	println!("uploaded metadata: {:?}", file);

	Ok(())
}

pub async fn search_files_example() -> Result<(), DriveError> {
	let client_secret_path = "client_secret_file.json".to_string();
	let drive_client = ReadDrive::new("some-service@consulting-llc-6302f.iam.gserviceaccount.com".to_string(), client_secret_path)?;

	// Search for PDF files containing "report" in the name
	let query = "name contains 'test' and mimeType = 'text/plain'";
	let page_size = 50;

	let files = drive_client.search_files(query, page_size).await?;

	println!("Search results: {} files found", files.len());
	for file in files {
		println!("Found: {} ({})", file.name, file.id);
		if let Some(link) = &file.web_view_link {
			println!("  View link: {}", link);
		}
	}

	Ok(())
}

pub async fn download_file_example() -> Result<(), DriveError> {
	let client_secret_path = "client_secret_file.json".to_string();
	let user_email = "aulgondu@gmail.com".to_string();
	let drive_client = ReadDrive::new(user_email.to_string(), client_secret_path)?;

	let file_id = "1_B-BTD0y3iYbyfChvntG6J7q0oumcxkE";

	let bytes = drive_client.download_file(file_id).await?;

	match std::str::from_utf8(&bytes) {
		Ok(text) => {
			println!("File content as text:");
			println!("{}", text);
		}
		Err(_) => {
			// If not valid UTF-8 (binary file), print bytes information
			println!("File is binary or not valid UTF-8 text");
			println!("File size: {} bytes", bytes.len());
			println!("First 20 bytes (hex): {:?}", &bytes[..bytes.len().min(20)]);
		}
	}

	Ok(())
}

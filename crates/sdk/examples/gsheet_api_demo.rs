use sdk::*;
use tokio;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
	rustls::crypto::ring::default_provider()
		.install_default()
		.map_err(|_| SheetError::ServiceInit(format!("Failed to initialize crypto provider: ")))?;

	// Initialize clients with a user email
	let user_email = "aulgondu@gmail.com".to_string();
	let client_secret_file = "client_secret_file.json".to_string();

	// Use the `?` operator to propagate any errors from the `new` methods
	let reader = ReadSheets::new(user_email.clone(), client_secret_file.clone())?;
	let writer = WriteToGoogleSheet::new(user_email, client_secret_file)?;

	// Create a new spreadsheet
	println!("Creating new spreadsheet...");
	let metadata = writer.create_new_spreadsheet("My Demo Sheet").await?;
	println!("Created spreadsheet: {}", metadata.url);
	println!("Spreadsheet ID: {}", metadata.spreadsheet_id);
	println!("Sheet names: {:?}", metadata.sheet_names);

	// Write some sample data
	let sample_data = vec![
		vec!["Name".to_string(), "Age".to_string(), "City".to_string()],
		vec!["Alice".to_string(), "25".to_string(), "New York".to_string()],
		vec!["Bob".to_string(), "30".to_string(), "San Francisco".to_string()],
		vec!["Charlie".to_string(), "35".to_string(), "Seattle".to_string()],
	];

	println!("\nWriting data to sheet...");
	writer.write_data_to_sheet("default", &metadata.spreadsheet_id, sample_data).await?;

	// Demonstrate reading data back
	println!("\nReading data from sheet...");
	let range = "default!A1:C4"; // Adjust range based on your data
	let data = reader.read_data(&metadata.spreadsheet_id, range).await?;

	println!("\nRetrieved data:");
	for row in data {
		println!("{:?}", row);
	}

	// Demonstrate range validation
	println!("\nValidating range...");
	let valid = reader.validate_range(&metadata.spreadsheet_id, range).await?;
	println!("Range '{}' is valid: {}", range, valid);

	// Demonstrate metadata retrieval
	println!("\nRetrieving spreadsheet metadata...");
	let spreadsheet = reader.retrieve_metadata(&metadata.spreadsheet_id).await?;
	println!("Spreadsheet title: {}", spreadsheet.properties.unwrap().title.unwrap_or_default());

	// Demonstrate datetime conversion utility
	let datetime = GoogleSheetsClient::convert_to_rfc_datetime(2024, 3, 15, 14, 30);
	println!("\nConverted datetime: {:?}", datetime);

	Ok(())
}

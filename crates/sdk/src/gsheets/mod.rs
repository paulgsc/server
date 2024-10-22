use chrono::{DateTime, Utc};
use google_sheets4::yup_oauth2;
use google_sheets4::yup_oauth2::{InstalledFlowAuthenticator, InstalledFlowReturnMethod};
use google_sheets4::Sheets;
use hyper::Client;
use hyper_rustls::HttpsConnector;
use once_cell::unsync::OnceCell;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

type SheetsClient = Sheets<HttpsConnector<hyper::client::HttpConnector>>;

#[derive(Debug, thiserror::Error)]
pub enum SheetError {
	#[error("Authentication error: {0}")]
	Auth(String),
	#[error("API error: {0}")]
	Api(String),
	#[error("Invalid range specified")]
	InvalidRange,
	#[error("Failed to initialize service: {0}")]
	InitializationError(String),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SpreadsheetMetadata {
	pub url: String,
	pub spreadsheet_id: String,
	pub sheet_names: Vec<String>,
}

pub struct GoogleSheetsClient {
	user_email: String,
	service: OnceCell<SheetsClient>,
}

impl GoogleSheetsClient {
	pub fn new(user_email: String) -> Self {
		Self {
			user_email,
			service: OnceCell::new(),
		}
	}

	async fn get_service(&self) -> Result<&SheetsClient, SheetError> {
		self
			.service
			.get_or_try_init(|| async {
				let secret = yup_oauth2::read_application_secret("client_secret_file.json")
					.await
					.map_err(|e| SheetError::Auth(e.to_string()))?;

				let cache_path = PathBuf::from("app").join("gsheets_pickle").join(format!("{}_token_sheets_v4.json", self.user_email));

				fs::create_dir_all(cache_path.parent().unwrap()).map_err(|e| SheetError::InitializationError(e.to_string()))?;

				let auth = InstalledFlowAuthenticator::builder(secret, InstalledFlowReturnMethod::HTTPRedirect)
					.persist_tokens_to_disk(cache_path)
					.build()
					.await
					.map_err(|e| SheetError::Auth(e.to_string()))?;

				let hub = Sheets::new(
					hyper::Client::builder().build(hyper_rustls::HttpsConnectorBuilder::new().with_native_roots().https_or_http().enable_http1().build()),
					auth,
				);

				Ok(hub)
			})
			.await
	}

	pub fn convert_to_rfc_datetime(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> DateTime<Utc> {
		chrono::DateTime::from_utc(
			chrono::NaiveDateTime::new(
				chrono::NaiveDate::from_ymd_opt(year, month, day).unwrap(),
				chrono::NaiveTime::from_hms_opt(hour, minute, 0).unwrap(),
			),
			Utc,
		)
	}
}

pub struct ReadSheets {
	client: GoogleSheetsClient,
}

impl ReadSheets {
	pub fn new(user_email: String) -> Self {
		Self {
			client: GoogleSheetsClient::new(user_email),
		}
	}

	pub async fn retrieve_metadata(&self, spreadsheet_id: &str) -> Result<google_sheets4::Spreadsheet, SheetError> {
		self
			.client
			.get_service()
			.await?
			.spreadsheets()
			.get(spreadsheet_id)
			.doit()
			.await
			.map(|(_, res)| res)
			.map_err(|e| SheetError::Api(e.to_string()))
	}

	pub async fn read_data(&self, spreadsheet_id: &str, range: &str) -> Result<Vec<Vec<String>>, SheetError> {
		let result = self
			.client
			.get_service()
			.await?
			.spreadsheets()
			.values_get(spreadsheet_id, range)
			.doit()
			.await
			.map_err(|e| SheetError::Api(e.to_string()))?;

		Ok(
			result
				.1
				.values
				.unwrap_or_default()
				.into_iter()
				.map(|row| row.into_iter().map(|cell| cell.to_string()).collect::<Vec<String>>())
				.collect(),
		)
	}

	pub async fn validate_range(&self, spreadsheet_id: &str, range: &str) -> Result<bool, SheetError> {
		self
			.client
			.get_service()
			.await?
			.spreadsheets()
			.values_get(spreadsheet_id, range)
			.doit()
			.await
			.map(|_| true)
			.map_err(|_| SheetError::InvalidRange)
	}
}

pub struct WriteToGoogleSheet {
	client: GoogleSheetsClient,
}

impl WriteToGoogleSheet {
	pub fn new(user_email: String) -> Self {
		Self {
			client: GoogleSheetsClient::new(user_email),
		}
	}

	pub async fn write_data_to_sheet(&self, worksheet_name: &str, spreadsheet_id: &str, data: Vec<Vec<String>>) -> Result<(), SheetError> {
		let request = google_sheets4::ValueRange {
			major_dimension: Some("ROWS".to_string()),
			range: Some(worksheet_name.to_string()),
			values: Some(data),
		};

		self
			.client
			.get_service()
			.await?
			.spreadsheets()
			.values_append(request, spreadsheet_id, worksheet_name)
			.value_input_option("RAW")
			.doit()
			.await
			.map(|_| ())
			.map_err(|e| SheetError::Api(e.to_string()))
	}

	pub async fn create_new_spreadsheet(&self, sheet_name: &str) -> Result<SpreadsheetMetadata, SheetError> {
		let spreadsheet = google_sheets4::Spreadsheet {
			properties: Some(google_sheets4::SpreadsheetProperties {
				title: Some(sheet_name.to_string()),
				locale: Some("en_US".to_string()),
				time_zone: Some("America/Los_Angeles".to_string()),
				..Default::default()
			}),
			sheets: Some(vec![google_sheets4::Sheet {
				properties: Some(google_sheets4::SheetProperties {
					title: Some("default".to_string()),
					..Default::default()
				}),
				..Default::default()
			}]),
			..Default::default()
		};

		let result = self
			.client
			.get_service()
			.await?
			.spreadsheets()
			.create(spreadsheet)
			.doit()
			.await
			.map_err(|e| SheetError::Api(e.to_string()))?;

		Ok(SpreadsheetMetadata {
			url: result.1.spreadsheet_url.unwrap_or_default(),
			spreadsheet_id: result.1.spreadsheet_id.unwrap_or_default(),
			sheet_names: result.1.sheets.unwrap_or_default().into_iter().filter_map(|sheet| sheet.properties?.title).collect(),
		})
	}
}

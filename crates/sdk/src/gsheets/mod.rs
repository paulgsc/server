use chrono::{DateTime, Utc};
use google_sheets4::yup_oauth2::{InstalledFlowAuthenticator, InstalledFlowReturnMethod};
use google_sheets4::{api::Spreadsheet, hyper, hyper_rustls, Sheets};
use hyper_rustls::HttpsConnector;
use hyper_util::client::legacy::connect::HttpConnector;
use once_cell::unsync::OnceCell;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

type HttpsConnectorType = HttpsConnector<HttpConnector>;
type SheetsClient = Sheets<HttpsConnectorType>;

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
	service: OnceCell<Arc<SheetsClient>>,
}

impl GoogleSheetsClient {
	pub fn new(user_email: String) -> Self {
		Self {
			user_email,
			service: OnceCell::new(),
		}
	}

	async fn initialize_service() -> Result<SheetsClient, SheetError> {
		let secret = google_sheets4::yup_oauth2::read_application_secret("client_secret_file.json")
			.await
			.map_err(|e| SheetError::Auth(e.to_string()))?;

		let cache_path = PathBuf::from("app").join("gsheets_pickle").join("token_sheets_v4.json");

		fs::create_dir_all(cache_path.parent().unwrap()).map_err(|e| SheetError::InitializationError(e.to_string()))?;

		let auth = InstalledFlowAuthenticator::builder(secret, InstalledFlowReturnMethod::HTTPRedirect)
			.persist_tokens_to_disk(cache_path)
			.build()
			.await
			.map_err(|e| SheetError::Auth(e.to_string()))?;

		let connector = hyper_rustls::HttpsConnectorBuilder::new()
			.with_native_roots()
			.unwrap()
			.https_or_http()
			.enable_http1()
			.build();

		let executor = hyper_util::rt::TokioExecutor::new();
		let client = hyper_util::client::legacy::Client::builder(executor).build(connector);

		Ok(Sheets::new(client, auth))
	}

	pub async fn get_service(&self) -> Result<&Arc<SheetsClient>, SheetError> {
		if self.service.get().is_none() {
			let service = Self::initialize_service().await?;
			self
				.service
				.set(Arc::new(service))
				.map_err(|_| SheetError::InitializationError("Failed to set service".to_string()))?;
		}
		Ok(self.service.get().unwrap())
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

	pub async fn retrieve_metadata(&self, spreadsheet_id: &str) -> Result<Spreadsheet, SheetError> {
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
				.map(|row| row.into_iter().map(|cell| cell.to_string()).collect())
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
		// Convert Vec<Vec<String>> to Vec<Vec<Value>>
		let values: Vec<Vec<Value>> = data.into_iter().map(|row| row.into_iter().map(|cell| Value::String(cell)).collect()).collect();

		let request = google_sheets4::api::ValueRange {
			major_dimension: Some("ROWS".to_string()),
			range: Some(worksheet_name.to_string()),
			values: Some(values),
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
		let spreadsheet = google_sheets4::api::Spreadsheet {
			properties: Some(google_sheets4::api::SpreadsheetProperties {
				title: Some(sheet_name.to_string()),
				locale: Some("en_US".to_string()),
				time_zone: Some("America/Los_Angeles".to_string()),
				..Default::default()
			}),
			sheets: Some(vec![google_sheets4::api::Sheet {
				properties: Some(google_sheets4::api::SheetProperties {
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

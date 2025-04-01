use crate::{GoogleServiceFilePath, SecretFilePathError};
use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc};
use google_sheets4::yup_oauth2::Error as OAuth2Error;
use google_sheets4::yup_oauth2::{InstalledFlowAuthenticator, InstalledFlowReturnMethod, ServiceAccountAuthenticator};
use google_sheets4::Error as GoogleSheetsError;
use google_sheets4::{api::Spreadsheet, hyper_rustls, Sheets};
use hyper::Error as HyperError;
use hyper_rustls::HttpsConnector;
use hyper_util::client::legacy::connect::HttpConnector;
use serde::{Deserialize, Serialize};
use serde_json::{to_value, Value};
use std::fs;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;

type HttpsConnectorType = HttpsConnector<HttpConnector>;
type SheetsClient = Sheets<HttpsConnectorType>;

const SCOPES: [&str; 1] = ["https://www.googleapis.com/auth/spreadsheets"];

#[derive(Debug, thiserror::Error)]
pub enum SheetError {
	#[error("OAuth2 error: {0}")]
	OAuth2(#[from] OAuth2Error),

	#[error("Google Sheets API error: {0}")]
	GoogleSheets(#[from] GoogleSheetsError),

	#[error("HTTP client error: {0}")]
	Hyper(#[from] HyperError),

	#[error("IO error: {0}")]
	Io(#[from] io::Error),

	#[error("Invalid range specified: {0}")]
	InvalidRange(Value),

	#[error("Missing credentials file: {0}")]
	MissingCredentials(String),

	#[error("Service initialization failed: {0}")]
	ServiceInit(String),

	#[error("Invalid sheet metadata: {0}")]
	InvalidMetadata(String),

	#[error("Invalid date")]
	InvalidDate { year: i32, month: u32, day: u32 },

	#[error("Invalid date time")]
	InvalidTime { hour: u32, minute: u32 },

	#[error("Secret file path error: {0}")]
	SecretFilePath(#[from] SecretFilePathError),

	#[error("Unexpected error: {0}")]
	TokenError(#[from] Box<dyn std::error::Error + Send + Sync>),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SpreadsheetMetadata {
	pub url: String,
	pub spreadsheet_id: String,
	pub sheet_names: Vec<String>,
}

pub struct GoogleSheetsClient {
	#[allow(dead_code)]
	user_email: String,
	service: Arc<Mutex<Option<Arc<SheetsClient>>>>,
	client_secret_path: GoogleServiceFilePath,
}

impl GoogleSheetsClient {
	pub fn new(user_email: String, client_secret_path: String) -> Result<Self, SheetError> {
		let validated_path = GoogleServiceFilePath::new(client_secret_path)?;

		Ok(Self {
			user_email,
			service: Arc::new(Mutex::new(None)),
			client_secret_path: validated_path,
		})
	}

	async fn initialize_service(&self) -> Result<SheetsClient, SheetError> {
		// rustls::crypto::ring::default_provider()
		// 	.install_default()
		// 	.map_err(|_| SheetError::ServiceInit(format!("Failed to initialize crypto provider: ")))?;

		let secret = google_sheets4::yup_oauth2::read_service_account_key(&self.client_secret_path.as_ref()).await?;

		let auth = ServiceAccountAuthenticator::builder(secret).build().await?;

		let connector = hyper_rustls::HttpsConnectorBuilder::new()
			.with_native_roots()
			.unwrap()
			.https_or_http()
			.enable_http1()
			.build();

		let executor = hyper_util::rt::TokioExecutor::new();
		let client = hyper_util::client::legacy::Client::builder(executor).build(connector);

		let service = Sheets::new(client, auth);
		let auth = &service.auth;
		auth.get_token(&SCOPES).await?;

		Ok(service)
	}

	#[allow(dead_code)]
	async fn initialize_oauth_service(&self) -> Result<SheetsClient, SheetError> {
		// rustls::crypto::ring::default_provider()
		// 	.install_default()
		// 	.map_err(|_| SheetError::ServiceInit(format!("Failed to initialize crypto provider: ")))?;

		let secret = google_sheets4::yup_oauth2::read_application_secret(&self.client_secret_path.as_ref()).await?;

		let cache_path = PathBuf::from("app").join("gsheets_pickle").join("token_sheets_v4.json");

		fs::create_dir_all(cache_path.parent().unwrap()).map_err(SheetError::Io)?;

		let auth = InstalledFlowAuthenticator::builder(secret, InstalledFlowReturnMethod::HTTPRedirect)
			.persist_tokens_to_disk(cache_path)
			.build()
			.await?;

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

	pub async fn get_service(&self) -> Result<Arc<SheetsClient>, SheetError> {
		let mut service_guard = self.service.lock().await;

		if service_guard.is_none() {
			let new_service = self.initialize_service().await?;
			*service_guard = Some(Arc::new(new_service));
		}

		Ok(Arc::clone(service_guard.as_ref().unwrap()))
	}

	pub fn convert_to_rfc_datetime(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> Result<DateTime<Utc>, SheetError> {
		let naive_date = NaiveDate::from_ymd_opt(year, month, day).ok_or(SheetError::InvalidDate { year, month, day })?;
		let naive_time = NaiveTime::from_hms_opt(hour, minute, 0).ok_or(SheetError::InvalidTime { hour, minute })?;
		let naive_datetime = NaiveDateTime::new(naive_date, naive_time);

		Ok(Utc.from_utc_datetime(&naive_datetime))
	}
}

pub struct ReadSheets {
	client: Arc<GoogleSheetsClient>,
}

impl ReadSheets {
	pub fn new(user_email: String, client_secret_path: String) -> Result<Self, SheetError> {
		Ok(Self {
			client: Arc::new(GoogleSheetsClient::new(user_email, client_secret_path)?),
		})
	}

	pub async fn retrieve_metadata(&self, spreadsheet_id: &str) -> Result<Spreadsheet, SheetError> {
		let result = self.client.get_service().await?.spreadsheets().get(spreadsheet_id).doit().await?;
		Ok(result.1)
	}

	pub async fn read_data(&self, spreadsheet_id: &str, range: &str) -> Result<Vec<Vec<String>>, SheetError> {
		let result = self.client.get_service().await?.spreadsheets().values_get(spreadsheet_id, range).doit().await?;

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
		match self.client.get_service().await?.spreadsheets().values_get(spreadsheet_id, range).doit().await {
			Ok(_) => Ok(true),
			Err(GoogleSheetsError::BadRequest(msg)) => Err(SheetError::InvalidRange(msg)),
			Err(e) => Err(SheetError::GoogleSheets(e)),
		}
	}
}

pub struct WriteToGoogleSheet {
	client: Arc<GoogleSheetsClient>,
}

impl WriteToGoogleSheet {
	pub fn new(user_email: String, client_secret_path: String) -> Result<Self, SheetError> {
		Ok(Self {
			client: Arc::new(GoogleSheetsClient::new(user_email, client_secret_path)?),
		})
	}

	pub async fn write_data_to_sheet<T: Serialize>(&self, worksheet_name: &str, spreadsheet_id: &str, data: Vec<T>) -> Result<(), SheetError> {
		let values: Vec<Vec<Value>> = data
			.into_iter()
			.map(|row| match to_value(row) {
				Ok(serde_json::Value::Object(obj)) => obj.values().map(|v| Self::json_to_sheets_value(v.clone())).collect(),
				Ok(serde_json::Value::Array(arr)) => arr.into_iter().map(Self::json_to_sheets_value).collect(),
				_ => vec![],
			})
			.collect();

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
			.add_scopes(&SCOPES)
			.doit()
			.await?;

		Ok(())
	}

	fn json_to_sheets_value(json: serde_json::Value) -> Value {
		match json {
			serde_json::Value::String(s) => Value::String(s),
			serde_json::Value::Number(n) => Value::String(n.to_string()),
			serde_json::Value::Bool(b) => Value::String(b.to_string()),
			serde_json::Value::Array(arr) => {
				let array_str = arr
					.iter()
					.map(|v| match v {
						serde_json::Value::String(s) => s.clone(),
						serde_json::Value::Number(n) => n.to_string(),
						serde_json::Value::Bool(b) => b.to_string(),
						serde_json::Value::Array(_) => "[array]".to_string(),
						serde_json::Value::Object(_) => "[object]".to_string(),
						serde_json::Value::Null => "null".to_string(),
					})
					.collect::<Vec<String>>()
					.join(",");
				Value::String(array_str)
			}
			serde_json::Value::Object(obj) => {
				// For nested objects, serialize to JSON string
				Value::String(serde_json::to_string(&obj).unwrap_or_default())
			}
			serde_json::Value::Null => Value::String("".to_string()),
		}
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

		let result = self.client.get_service().await?.spreadsheets().create(spreadsheet).doit().await?;

		Ok(SpreadsheetMetadata {
			url: result.1.spreadsheet_url.ok_or_else(|| SheetError::InvalidMetadata("Missing spreadsheet URL".to_string()))?,
			spreadsheet_id: result.1.spreadsheet_id.ok_or_else(|| SheetError::InvalidMetadata("Missing spreadsheet ID".to_string()))?,
			sheet_names: result
				.1
				.sheets
				.ok_or_else(|| SheetError::InvalidMetadata("Missing sheets".to_string()))?
				.into_iter()
				.filter_map(|sheet| sheet.properties?.title)
				.collect(),
		})
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use chrono::TimeZone;
	use google_sheets4::api::{Sheet, SheetProperties, Spreadsheet, SpreadsheetProperties, ValueRange};
	use mockall::predicate::*;
	use mockall::*;
	use std::sync::Arc;

	// Mock the Google Sheets API client
	mock! {
			SheetsClient {
					fn spreadsheets(&self) -> SpreadsheetsHub;
			}

			pub struct SpreadsheetsHub;
			impl Clone for SpreadsheetsHub {}

			trait SpreadsheetsHub {
					fn get(&self, spreadsheet_id: &str) -> SpreadsheetGet;
					fn values_get(&self, spreadsheet_id: &str, range: &str) -> ValuesGet;
					fn values_append(&self, request: ValueRange, spreadsheet_id: &str, range: &str) -> ValuesAppend;
					fn create(&self, spreadsheet: Spreadsheet) -> SpreadsheetCreate;
			}

			pub struct SpreadsheetGet;
			impl Clone for SpreadsheetGet {}

			trait SpreadsheetGet {
					fn doit(&self) -> Result<(hyper::Response<hyper::Body>, Spreadsheet), google_sheets4::Error>;
			}

			pub struct ValuesGet;
			impl Clone for ValuesGet {}

			trait ValuesGet {
					fn doit(&self) -> Result<(hyper::Response<hyper::Body>, ValueRange), google_sheets4::Error>;
			}

			pub struct ValuesAppend;
			impl Clone for ValuesAppend {}

			trait ValuesAppend {
					fn value_input_option(&self, value: &str) -> Self;
					fn doit(&self) -> Result<(hyper::Response<hyper::Body>, ValueRange), google_sheets4::Error>;
			}

			pub struct SpreadsheetCreate;
			impl Clone for SpreadsheetCreate {}

			trait SpreadsheetCreate {
					fn doit(&self) -> Result<(hyper::Response<hyper::Body>, Spreadsheet), google_sheets4::error::Error>;
			}
	}

	#[tokio::test]
	async fn test_read_data_success() {
		let mut mock_client = MockSheetsClient::new();
		let spreadsheet_id = "test_spreadsheet_id";
		let range = "Sheet1!A1:B2";

		// Set up mock expectations
		let mut mock_hub = MockSpreadsheetsHub::new();
		let mut mock_values_get = MockValuesGet::new();

		let test_data = ValueRange {
			values: Some(vec![vec!["Header1".into(), "Header2".into()], vec!["Value1".into(), "Value2".into()]]),
			..Default::default()
		};

		mock_values_get
			.expect_doit()
			.return_once(move || Ok((hyper::Response::new(hyper::Body::empty()), test_data)));

		mock_hub.expect_values_get().with(eq(spreadsheet_id), eq(range)).return_once(move |_, _| mock_values_get);

		mock_client.expect_spreadsheets().return_once(move || mock_hub);

		// Create client with mocked service
		let client = GoogleSheetsClient::new("test@example.com".to_string());
		client.service.set(Arc::new(mock_client)).unwrap();

		// Execute test
		let reader = ReadSheets::new("test@example.com".to_string());
		let result = reader.read_data(spreadsheet_id, range).await.unwrap();

		assert_eq!(result.len(), 2);
		assert_eq!(result[0], vec!["Header1", "Header2"]);
		assert_eq!(result[1], vec!["Value1", "Value2"]);
	}

	#[tokio::test]
	async fn test_write_data_success() {
		let mut mock_client = MockSheetsClient::new();
		let spreadsheet_id = "test_spreadsheet_id";
		let worksheet_name = "Sheet1";

		// Set up mock expectations
		let mut mock_hub = MockSpreadsheetsHub::new();
		let mut mock_values_append = MockValuesAppend::new();

		mock_values_append.expect_value_input_option().with(eq("RAW")).return_once(|_| MockValuesAppend::new());

		mock_values_append
			.expect_doit()
			.return_once(move || Ok((hyper::Response::new(hyper::Body::empty()), ValueRange::default())));

		mock_hub.expect_values_append().return_once(move |_, _, _| mock_values_append);

		mock_client.expect_spreadsheets().return_once(move || mock_hub);

		// Create client with mocked service
		let client = GoogleSheetsClient::new("test@example.com".to_string());
		client.service.set(Arc::new(mock_client)).unwrap();

		// Execute test
		let writer = WriteToGoogleSheet::new("test@example.com".to_string());
		let test_data = vec![vec!["Header1".to_string(), "Header2".to_string()], vec!["Value1".to_string(), "Value2".to_string()]];

		let result = writer.write_data_to_sheet(worksheet_name, spreadsheet_id, test_data).await;

		assert!(result.is_ok());
	}

	#[tokio::test]
	async fn test_create_spreadsheet_success() {
		let mut mock_client = MockSheetsClient::new();
		let sheet_name = "New Test Sheet";

		// Set up mock expectations
		let mut mock_hub = MockSpreadsheetsHub::new();
		let mut mock_create = MockSpreadsheetCreate::new();

		let response_spreadsheet = Spreadsheet {
			spreadsheet_id: Some("new_test_id".to_string()),
			spreadsheet_url: Some("https://sheets.google.com/test".to_string()),
			sheets: Some(vec![Sheet {
				properties: Some(SheetProperties {
					title: Some("default".to_string()),
					..Default::default()
				}),
				..Default::default()
			}]),
			..Default::default()
		};

		mock_create
			.expect_doit()
			.return_once(move || Ok((hyper::Response::new(hyper::Body::empty()), response_spreadsheet)));

		mock_hub.expect_create().return_once(move |_| mock_create);

		mock_client.expect_spreadsheets().return_once(move || mock_hub);

		// Create client with mocked service
		let client = GoogleSheetsClient::new("test@example.com".to_string());
		client.service.set(Arc::new(mock_client)).unwrap();

		// Execute test
		let writer = WriteToGoogleSheet::new("test@example.com".to_string());
		let result = writer.create_new_spreadsheet(sheet_name).await.unwrap();

		assert_eq!(result.spreadsheet_id, "new_test_id");
		assert_eq!(result.url, "https://sheets.google.com/test");
		assert_eq!(result.sheet_names, vec!["default"]);
	}

	#[test]
	fn test_convert_to_rfc_datetime() {
		let datetime = GoogleSheetsClient::convert_to_rfc_datetime(2024, 3, 15, 14, 30);
		let expected = Utc.with_ymd_and_hms(2024, 3, 15, 14, 30, 0).unwrap();
		assert_eq!(datetime, expected);
	}

	#[tokio::test]
	async fn test_validate_range_success() {
		let mut mock_client = MockSheetsClient::new();
		let spreadsheet_id = "test_spreadsheet_id";
		let range = "Sheet1!A1:B2";

		// Set up mock expectations
		let mut mock_hub = MockSpreadsheetsHub::new();
		let mut mock_values_get = MockValuesGet::new();

		mock_values_get
			.expect_doit()
			.return_once(move || Ok((hyper::Response::new(hyper::Body::empty()), ValueRange::default())));

		mock_hub.expect_values_get().with(eq(spreadsheet_id), eq(range)).return_once(move |_, _| mock_values_get);

		mock_client.expect_spreadsheets().return_once(move || mock_hub);

		// Create client with mocked service
		let client = GoogleSheetsClient::new("test@example.com".to_string());
		client.service.set(Arc::new(mock_client)).unwrap();

		// Execute test
		let reader = ReadSheets::new("test@example.com".to_string());
		let result = reader.validate_range(spreadsheet_id, range).await;

		assert!(result.is_ok());
		assert!(result.unwrap());
	}

	#[tokio::test]
	async fn test_validate_range_failure() {
		let mut mock_client = MockSheetsClient::new();
		let spreadsheet_id = "test_spreadsheet_id";
		let range = "Invalid!Range";

		// Set up mock expectations
		let mut mock_hub = MockSpreadsheetsHub::new();
		let mut mock_values_get = MockValuesGet::new();

		mock_values_get
			.expect_doit()
			.return_once(move || Err(google_sheets4::Error::BadRequest("Invalid range".to_string())));

		mock_hub.expect_values_get().with(eq(spreadsheet_id), eq(range)).return_once(move |_, _| mock_values_get);

		mock_client.expect_spreadsheets().return_once(move || mock_hub);

		// Create client with mocked service
		let client = GoogleSheetsClient::new("test@example.com".to_string());
		client.service.set(Arc::new(mock_client)).unwrap();

		// Execute test
		let reader = ReadSheets::new("test@example.com".to_string());
		let result = reader.validate_range(spreadsheet_id, range).await;

		assert!(matches!(result, Err(SheetError::InvalidRange)));
	}

	#[tokio::test]
	async fn test_retrieve_metadata_success() {
		let mut mock_client = MockSheetsClient::new();
		let spreadsheet_id = "test_spreadsheet_id";

		// Set up mock expectations
		let mut mock_hub = MockSpreadsheetsHub::new();
		let mut mock_get = MockSpreadsheetGet::new();

		let response_spreadsheet = Spreadsheet {
			properties: Some(SpreadsheetProperties {
				title: Some("Test Sheet".to_string()),
				..Default::default()
			}),
			..Default::default()
		};

		mock_get
			.expect_doit()
			.return_once(move || Ok((hyper::Response::new(hyper::Body::empty()), response_spreadsheet)));

		mock_hub.expect_get().with(eq(spreadsheet_id)).return_once(move |_| mock_get);

		mock_client.expect_spreadsheets().return_once(move || mock_hub);

		// Create client with mocked service
		let client = GoogleSheetsClient::new("test@example.com".to_string());
		client.service.set(Arc::new(mock_client)).unwrap();

		// Execute test
		let reader = ReadSheets::new("test@example.com".to_string());
		let result = reader.retrieve_metadata(spreadsheet_id).await.unwrap();

		assert_eq!(result.properties.unwrap().title.unwrap(), "Test Sheet");
	}
}

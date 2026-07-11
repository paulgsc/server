use crate::google_client::{self, ClientCache, GoogleClientError, HttpsConnectorType};
use crate::{util::column_number_to_letter, GoogleServiceFilePath, SecretFilePathError};
use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc};
use google_sheets4::api::{AddSheetRequest, BatchUpdateSpreadsheetRequest, Request as SheetsRequest, Sheet, SheetProperties, Spreadsheet, SpreadsheetProperties, ValueRange};
use google_sheets4::Error as GoogleSheetsError;
use google_sheets4::Sheets;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use serde_json::{to_value, Value};
use std::collections::HashMap;
use std::sync::Arc;

type SheetsClient = Sheets<HttpsConnectorType>;

const SCOPES: [&str; 1] = ["https://www.googleapis.com/auth/spreadsheets"];

#[derive(Debug, thiserror::Error)]
pub enum SheetError {
	#[error("Client error: {0}")]
	Client(#[from] GoogleClientError),

	#[error("Google Sheets API error: {0}")]
	GoogleSheets(#[from] GoogleSheetsError),

	#[error("Invalid range specified: {0}")]
	InvalidRange(Value),

	#[error("Missing credentials file: {0}")]
	MissingCredentials(String),

	#[error("No Sheets found in spreadsheet")]
	NoSheetsFound,

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
}

pub enum SheetOperation {
	CreateTab,
	Rewrite,
	Append,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SpreadsheetMetadata {
	pub url: String,
	pub spreadsheet_id: String,
	pub sheet_names: Vec<String>,
}

/// The mockable service boundary: everything `ReadSheets`/`WriteToGoogleSheet`
/// need from a Sheets hub, expressed in plain data so a test double can
/// implement it without standing up a real HTTP stack.
#[async_trait]
pub trait SheetsService: Send + Sync {
	async fn get_spreadsheet(&self, spreadsheet_id: &str) -> Result<Spreadsheet, SheetError>;
	async fn get_values(&self, spreadsheet_id: &str, range: &str) -> Result<Vec<Vec<Value>>, SheetError>;
	async fn update_values(&self, spreadsheet_id: &str, range: &str, values: Vec<Vec<Value>>) -> Result<(), SheetError>;
	async fn append_values(&self, spreadsheet_id: &str, range: &str, values: Vec<Vec<Value>>) -> Result<(), SheetError>;
	async fn add_sheet(&self, spreadsheet_id: &str, title: &str) -> Result<(), SheetError>;
	async fn create_spreadsheet(&self, title: &str) -> Result<Spreadsheet, SheetError>;
}

/// Real implementation backed by a live `Sheets` hub.
struct RealSheetsService {
	hub: SheetsClient,
}

#[async_trait]
impl SheetsService for RealSheetsService {
	async fn get_spreadsheet(&self, spreadsheet_id: &str) -> Result<Spreadsheet, SheetError> {
		let result = self.hub.spreadsheets().get(spreadsheet_id).doit().await?;
		Ok(result.1)
	}

	async fn get_values(&self, spreadsheet_id: &str, range: &str) -> Result<Vec<Vec<Value>>, SheetError> {
		match self.hub.spreadsheets().values_get(spreadsheet_id, range).doit().await {
			Ok(result) => Ok(result.1.values.unwrap_or_default()),
			Err(GoogleSheetsError::BadRequest(msg)) => Err(SheetError::InvalidRange(msg)),
			Err(e) => Err(SheetError::GoogleSheets(e)),
		}
	}

	async fn update_values(&self, spreadsheet_id: &str, range: &str, values: Vec<Vec<Value>>) -> Result<(), SheetError> {
		let request = ValueRange {
			major_dimension: Some("ROWS".to_string()),
			range: Some(range.to_string()),
			values: Some(values),
		};

		self
			.hub
			.spreadsheets()
			.values_update(request, spreadsheet_id, range)
			.value_input_option("RAW")
			.add_scopes(&SCOPES)
			.doit()
			.await?;

		Ok(())
	}

	async fn append_values(&self, spreadsheet_id: &str, range: &str, values: Vec<Vec<Value>>) -> Result<(), SheetError> {
		let request = ValueRange {
			major_dimension: Some("ROWS".to_string()),
			range: Some(range.to_string()),
			values: Some(values),
		};

		self
			.hub
			.spreadsheets()
			.values_append(request, spreadsheet_id, range)
			.value_input_option("RAW")
			.add_scopes(&SCOPES)
			.doit()
			.await?;

		Ok(())
	}

	async fn add_sheet(&self, spreadsheet_id: &str, title: &str) -> Result<(), SheetError> {
		let add_sheet_request = BatchUpdateSpreadsheetRequest {
			requests: Some(vec![SheetsRequest {
				add_sheet: Some(AddSheetRequest {
					properties: Some(SheetProperties {
						title: Some(title.to_string()),
						..Default::default()
					}),
				}),
				..Default::default()
			}]),
			..Default::default()
		};

		self.hub.spreadsheets().batch_update(add_sheet_request, spreadsheet_id).add_scopes(&SCOPES).doit().await?;
		Ok(())
	}

	async fn create_spreadsheet(&self, title: &str) -> Result<Spreadsheet, SheetError> {
		let spreadsheet = Spreadsheet {
			properties: Some(SpreadsheetProperties {
				title: Some(title.to_string()),
				locale: Some("en_US".to_string()),
				time_zone: Some("America/Los_Angeles".to_string()),
				..Default::default()
			}),
			sheets: Some(vec![Sheet {
				properties: Some(SheetProperties {
					title: Some("default".to_string()),
					..Default::default()
				}),
				..Default::default()
			}]),
			..Default::default()
		};

		let result = self.hub.spreadsheets().create(spreadsheet).doit().await?;
		Ok(result.1)
	}
}

static SHEETS_CLIENT_CACHE: Lazy<ClientCache<dyn SheetsService>> = Lazy::new(ClientCache::new);

pub struct GoogleSheetsClient {
	user_email: String,
	client_secret_path: GoogleServiceFilePath,
}

impl GoogleSheetsClient {
	pub fn new(user_email: String, client_secret_path: String) -> Result<Self, SheetError> {
		let validated_path = GoogleServiceFilePath::new(client_secret_path)?;

		Ok(Self {
			user_email,
			client_secret_path: validated_path,
		})
	}

	pub async fn get_service(&self) -> Result<Arc<dyn SheetsService>, SheetError> {
		let secret_path = self.client_secret_path.clone();

		SHEETS_CLIENT_CACHE
			.get_or_try_init("sheets", &self.user_email, self.client_secret_path.as_str(), move || async move {
				let auth = google_client::build_service_account_authenticator(&secret_path).await?;
				let client = google_client::build_http_client()?;
				let hub = Sheets::new(client, auth);
				Ok::<Arc<dyn SheetsService>, GoogleClientError>(Arc::new(RealSheetsService { hub }))
			})
			.await
			.map_err(SheetError::from)
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
		self.client.get_service().await?.get_spreadsheet(spreadsheet_id).await
	}

	pub async fn retrieve_all_sheets_data(&self, spreadsheet_id: &str) -> Result<HashMap<String, Vec<Vec<String>>>, SheetError> {
		let spreadsheet = self.retrieve_metadata(spreadsheet_id).await?;

		let mut all_data = HashMap::new();

		let sheets = match spreadsheet.sheets {
			Some(sheets) => sheets,
			None => return Err(SheetError::NoSheetsFound),
		};

		for sheet in sheets {
			let sheet_properties = match sheet.properties {
				Some(props) => props,
				None => continue,
			};

			let sheet_title = match sheet_properties.title {
				Some(title) => title,
				None => continue,
			};

			let _ = match sheet_properties.sheet_id {
				Some(id) => id,
				None => continue,
			};

			let grid_props = match sheet_properties.grid_properties {
				Some(grid) => grid,
				None => continue,
			};

			let row_count = grid_props.row_count.unwrap_or(1000);
			let col_count = grid_props.column_count.unwrap_or(26); // A-Z

			let last_column = column_number_to_letter(col_count as u32);
			let range = format!("{}!A1:{}{}", sheet_title, last_column, row_count);

			let sheet_data = self.read_data(spreadsheet_id, &range).await?;

			all_data.insert(sheet_title, sheet_data);
		}

		Ok(all_data)
	}

	pub async fn read_data(&self, spreadsheet_id: &str, range: &str) -> Result<Vec<Vec<String>>, SheetError> {
		let service = self.client.get_service().await?;
		let values = service.get_values(spreadsheet_id, range).await?;

		Ok(values.into_iter().map(|row| row.into_iter().map(|cell| cell.to_string()).collect()).collect())
	}

	pub async fn validate_range(&self, spreadsheet_id: &str, range: &str) -> Result<bool, SheetError> {
		let service = self.client.get_service().await?;
		match service.get_values(spreadsheet_id, range).await {
			Ok(_) => Ok(true),
			Err(e) => Err(e),
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

	pub async fn write_data_to_sheet<T: Serialize>(&self, worksheet_name: &str, spreadsheet_id: &str, data: Vec<T>, operation: SheetOperation) -> Result<(), SheetError> {
		let values: Vec<Vec<Value>> = data
			.into_iter()
			.map(|row| match to_value(row) {
				Ok(serde_json::Value::Object(obj)) => obj.values().map(|v| Self::json_to_sheets_value(v.clone())).collect(),
				Ok(serde_json::Value::Array(arr)) => arr.into_iter().map(Self::json_to_sheets_value).collect(),
				_ => vec![],
			})
			.collect();

		let service = self.client.get_service().await?;

		match operation {
			SheetOperation::CreateTab => {
				service.add_sheet(spreadsheet_id, worksheet_name).await?;
				service.update_values(spreadsheet_id, worksheet_name, values).await?;
			}
			SheetOperation::Rewrite => {
				service.update_values(spreadsheet_id, worksheet_name, values).await?;
			}
			SheetOperation::Append => {
				service.append_values(spreadsheet_id, worksheet_name, values).await?;
			}
		}

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
		let service = self.client.get_service().await?;
		let spreadsheet = service.create_spreadsheet(sheet_name).await?;

		Ok(SpreadsheetMetadata {
			url: spreadsheet
				.spreadsheet_url
				.ok_or_else(|| SheetError::InvalidMetadata("Missing spreadsheet URL".to_string()))?,
			spreadsheet_id: spreadsheet
				.spreadsheet_id
				.ok_or_else(|| SheetError::InvalidMetadata("Missing spreadsheet ID".to_string()))?,
			sheet_names: spreadsheet
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
	use std::sync::Mutex as StdMutex;

	#[derive(Default)]
	struct MockSheets {
		spreadsheet: StdMutex<Option<Spreadsheet>>,
		values: StdMutex<HashMap<String, Vec<Vec<Value>>>>,
		updated: StdMutex<Vec<(String, Vec<Vec<Value>>)>>,
		appended: StdMutex<Vec<(String, Vec<Vec<Value>>)>>,
		added_sheets: StdMutex<Vec<String>>,
		invalid_ranges: StdMutex<Vec<String>>,
	}

	#[async_trait]
	impl SheetsService for MockSheets {
		async fn get_spreadsheet(&self, _spreadsheet_id: &str) -> Result<Spreadsheet, SheetError> {
			self.spreadsheet.lock().unwrap().clone().ok_or(SheetError::NoSheetsFound)
		}

		async fn get_values(&self, _spreadsheet_id: &str, range: &str) -> Result<Vec<Vec<Value>>, SheetError> {
			if self.invalid_ranges.lock().unwrap().contains(&range.to_string()) {
				return Err(SheetError::InvalidRange(Value::String(range.to_string())));
			}
			Ok(self.values.lock().unwrap().get(range).cloned().unwrap_or_default())
		}

		async fn update_values(&self, _spreadsheet_id: &str, range: &str, values: Vec<Vec<Value>>) -> Result<(), SheetError> {
			self.updated.lock().unwrap().push((range.to_string(), values));
			Ok(())
		}

		async fn append_values(&self, _spreadsheet_id: &str, range: &str, values: Vec<Vec<Value>>) -> Result<(), SheetError> {
			self.appended.lock().unwrap().push((range.to_string(), values));
			Ok(())
		}

		async fn add_sheet(&self, _spreadsheet_id: &str, title: &str) -> Result<(), SheetError> {
			self.added_sheets.lock().unwrap().push(title.to_string());
			Ok(())
		}

		async fn create_spreadsheet(&self, title: &str) -> Result<Spreadsheet, SheetError> {
			Ok(Spreadsheet {
				spreadsheet_id: Some("sheet-id".to_string()),
				spreadsheet_url: Some(format!("https://sheets.example/{title}")),
				sheets: Some(vec![Sheet {
					properties: Some(SheetProperties {
						title: Some(title.to_string()),
						..Default::default()
					}),
					..Default::default()
				}]),
				properties: Some(SpreadsheetProperties {
					title: Some(title.to_string()),
					..Default::default()
				}),
				..Default::default()
			})
		}
	}

	#[tokio::test]
	async fn get_values_maps_bad_request_to_invalid_range() {
		let mock = MockSheets::default();
		mock.invalid_ranges.lock().unwrap().push("Sheet1!A1:Z1".to_string());

		let err = mock.get_values("id", "Sheet1!A1:Z1").await.unwrap_err();
		assert!(matches!(err, SheetError::InvalidRange(_)));
	}

	#[tokio::test]
	async fn get_values_returns_stored_rows() {
		let mock = MockSheets::default();
		mock.values.lock().unwrap().insert("Sheet1!A1:B2".to_string(), vec![vec![Value::String("a".to_string())]]);

		let rows = mock.get_values("id", "Sheet1!A1:B2").await.unwrap();
		assert_eq!(rows, vec![vec![Value::String("a".to_string())]]);
	}

	#[tokio::test]
	async fn create_spreadsheet_reports_sheet_names() {
		let mock = MockSheets::default();
		let spreadsheet = mock.create_spreadsheet("My Sheet").await.unwrap();

		assert_eq!(spreadsheet.spreadsheet_id.as_deref(), Some("sheet-id"));
		let names: Vec<String> = spreadsheet.sheets.unwrap().into_iter().filter_map(|s| s.properties?.title).collect();
		assert_eq!(names, vec!["My Sheet".to_string()]);
	}

	#[tokio::test]
	async fn append_values_records_the_call() {
		let mock = MockSheets::default();
		mock.append_values("id", "Sheet1", vec![vec![Value::String("x".to_string())]]).await.unwrap();

		let appended = mock.appended.lock().unwrap();
		assert_eq!(appended.len(), 1);
		assert_eq!(appended[0].0, "Sheet1");
	}

	#[tokio::test]
	async fn json_to_sheets_value_stringifies_non_strings() {
		assert_eq!(WriteToGoogleSheet::json_to_sheets_value(Value::Bool(true)), Value::String("true".to_string()));
		assert_eq!(WriteToGoogleSheet::json_to_sheets_value(Value::Null), Value::String(String::new()));
	}

	#[test]
	fn convert_to_rfc_datetime_rejects_invalid_dates() {
		let err = GoogleSheetsClient::convert_to_rfc_datetime(2024, 2, 30, 10, 0).unwrap_err();
		assert!(matches!(err, SheetError::InvalidDate { .. }));
	}
}

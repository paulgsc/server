use crate::{GoogleServiceFilePath, SecretFilePathError};
use chrono::{DateTime, Utc};
use google_drive3::api::Scope;
use google_drive3::hyper_rustls;
use google_drive3::yup_oauth2::Error as OAuth2Error;
use google_drive3::yup_oauth2::ServiceAccountAuthenticator;
use google_drive3::Error as GoogleDriveError;
use google_drive3::{hyper, DriveHub};
use http_body_util::BodyExt;
use hyper::{body::Bytes, Error as HyperError};
use hyper_rustls::HttpsConnector;
use hyper_util::client::legacy::connect::HttpConnector;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::Mutex;

type HttpsConnectorType = HttpsConnector<HttpConnector>;
type DriveClient = DriveHub<HttpsConnectorType>;

#[derive(Debug, thiserror::Error)]
pub enum DriveError {
	#[error("OAuth2 error: {0}")]
	OAuth2(#[from] OAuth2Error),

	#[error("Google Drive API error: {0}")]
	GoogleDrive(#[from] GoogleDriveError),

	#[error("HTTP client error: {0}")]
	Hyper(#[from] HyperError),

	#[error("IO error: {0}")]
	Io(#[from] io::Error),

	#[error("Invalid file ID: {0}")]
	InvalidFileId(String),

	#[error("Missing credentials file: {0}")]
	MissingCredentials(String),

	#[error("Service initialization failed: {0}")]
	ServiceInit(String),

	#[error("File not found: {0}")]
	FileNotFound(String),

	#[error("OwnersNotFound not found")]
	OwnersNotFound,

	#[error("OwnerEmailNotFound not found:")]
	OwnerEmailNotFound,

	#[error("File too large: {0}")]
	FileTooLarge(String),

	#[error("Invalid file metadata: {0}")]
	InvalidMetadata(String),

	#[error("Invalid mime type: {0}")]
	InvalidMimeType(String),

	#[error("Secret file path error: {0}")]
	SecretFilePath(#[from] SecretFilePathError),

	#[error("Unexpected error: {0}")]
	TokenError(#[from] Box<dyn std::error::Error + Send + Sync>),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FileMetadata {
	pub id: String,
	pub name: String,
	pub mime_type: String,
	pub size: Option<i64>,
	pub created_time: Option<DateTime<Utc>>,
	pub modified_time: Option<DateTime<Utc>>,
	pub web_view_link: Option<String>,
	pub parents: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FileListPage {
	pub files: Vec<FileMetadata>,
	pub next_page_token: Option<String>,
}

pub struct GoogleDriveClient {
	#[allow(dead_code)]
	user_email: String,
	service: Arc<Mutex<Option<Arc<DriveClient>>>>,
	client_secret_path: GoogleServiceFilePath,
	scope: Scope,
}

impl GoogleDriveClient {
	pub fn new(user_email: String, client_secret_path: String, scope: Scope) -> Result<Self, DriveError> {
		let validated_path = GoogleServiceFilePath::new(client_secret_path)?;

		Ok(Self {
			user_email,
			service: Arc::new(Mutex::new(None)),
			client_secret_path: validated_path,
			scope,
		})
	}

	async fn initialize_service(&self) -> Result<DriveClient, DriveError> {
		let secret = google_drive3::yup_oauth2::read_service_account_key(&self.client_secret_path.as_ref()).await?;

		let auth = ServiceAccountAuthenticator::builder(secret).build().await?;

		let connector = hyper_rustls::HttpsConnectorBuilder::new().with_native_roots()?.https_or_http().enable_http1().build();

		let executor = hyper_util::rt::TokioExecutor::new();
		let client = hyper_util::client::legacy::Client::builder(executor).build(connector);

		auth.token(&[self.scope.as_ref()]).await?;

		Ok(DriveHub::new(client, auth))
	}

	pub async fn get_service(&self) -> Result<Arc<DriveClient>, DriveError> {
		let mut service_guard = self.service.lock().await;

		if service_guard.is_none() {
			let new_service = self.initialize_service().await?;
			*service_guard = Some(Arc::new(new_service));
		}

		Ok(Arc::clone(service_guard.as_ref().unwrap()))
	}
}

pub struct ReadDrive {
	client: Arc<GoogleDriveClient>,
}

impl ReadDrive {
	pub fn new(user_email: String, client_secret_path: String) -> Result<Self, DriveError> {
		Ok(Self {
			client: Arc::new(GoogleDriveClient::new(user_email, client_secret_path, Scope::Readonly)?),
		})
	}

	pub async fn list_files(&self, folder_id: Option<&str>, page_size: i32, page_token: Option<&str>) -> Result<FileListPage, DriveError> {
		let mut query = String::new();

		if let Some(folder) = folder_id {
			query.push_str(&format!("'{}' in parents", folder));
		}

		let service = self.client.get_service().await?;
		let mut call = service
			.files()
			.list()
			.q(&query)
			.page_size(page_size)
			.supports_team_drives(true)
			.supports_all_drives(true)
			.include_items_from_all_drives(true)
			.corpora("allDrives")
			.param("fields", "nextPageToken, files(id, name, mimeType, size, createdTime, modifiedTime, webViewLink, parents)")
			.add_scope(Scope::Readonly.as_ref());

		if let Some(token) = page_token {
			call = call.page_token(token);
		}

		let result = call.doit().await?;

		let files = result.1.files.unwrap_or_default();
		let next_page_token = result.1.next_page_token;

		let metadata = files
			.into_iter()
			.map(|file| FileMetadata {
				id: file.id.unwrap_or_default(),
				name: file.name.unwrap_or_default(),
				mime_type: file.mime_type.unwrap_or_default(),
				size: file.size,
				created_time: file.created_time,
				modified_time: file.modified_time,
				web_view_link: file.web_view_link,
				parents: file.parents.unwrap_or_default(),
			})
			.collect();

		Ok(FileListPage { files: metadata, next_page_token })
	}

	pub async fn get_file_metadata(&self, file_id: &str) -> Result<FileMetadata, DriveError> {
		let result = self
			.client
			.get_service()
			.await?
			.files()
			.get(file_id)
			.supports_team_drives(true)
			.supports_all_drives(true)
			.include_permissions_for_view("published")
			.add_scope(Scope::Readonly.as_ref())
			.doit()
			.await?;

		let file = result.1;

		Ok(FileMetadata {
			id: file.id.ok_or_else(|| DriveError::InvalidMetadata("Missing file ID".to_string()))?,
			name: file.name.ok_or_else(|| DriveError::InvalidMetadata("Missing file name".to_string()))?,
			mime_type: file.mime_type.ok_or_else(|| DriveError::InvalidMetadata("Missing MIME type".to_string()))?,
			size: file.size,
			created_time: file.created_time,
			modified_time: file.modified_time,
			web_view_link: file.web_view_link,
			parents: file.parents.unwrap_or_default(),
		})
	}

	pub async fn download_file(&self, file_id: &str) -> Result<Bytes, DriveError> {
		const MAX_IN_MEMORY_SIZE: i64 = 10 * 1024 * 1024;

		// First, get the file metadata to determine if it's a Google Doc or regular file
		let metadata = self.get_file_metadata(file_id).await?;
		let service = self.client.get_service().await?;
		let content: Bytes;

		if let Some(size) = metadata.size {
			if size > MAX_IN_MEMORY_SIZE {
				return Err(DriveError::FileTooLarge(format!(
					"File size {} bytes exceeds the maximum allowed size of {} bytes",
					size, MAX_IN_MEMORY_SIZE
				)));
			}
		}

		// Handle file download based on MIME type
		if metadata.mime_type.starts_with("application/vnd.google-apps") {
			// Google Docs, Sheets, etc. need to be exported
			let export_mime_type = match metadata.mime_type.as_str() {
				"application/vnd.google-apps.document" => "application/pdf",
				"application/vnd.google-apps.spreadsheet" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
				"application/vnd.google-apps.presentation" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
				_ => "application/pdf", // Default to PDF for other Google types
			};

			let result = service.files().export(file_id, export_mime_type).add_scope(Scope::Readonly.as_ref()).doit().await?;
			content = result.into_body().collect().await?.to_bytes();
		} else {
			// Regular files can be downloaded directly
			let (response, _file_metadata) = service.files().get(file_id).param("alt", "media").add_scopes(&[Scope::Readonly.as_ref()]).doit().await?;
			content = response.into_body().collect().await?.to_bytes();
		}

		Ok(content)
	}

	pub async fn search_files(&self, query: &str, page_size: i32) -> Result<Vec<FileMetadata>, DriveError> {
		let result = self
			.client
			.get_service()
			.await?
			.files()
			.list()
			.q(query)
			.page_size(page_size)
			.spaces("drive")
			.param("fields", "files(id, name, mimeType, size, createdTime, modifiedTime, webViewLink, parents)")
			.add_scope(Scope::Readonly.as_ref())
			.doit()
			.await?;

		let files = result.1.files.unwrap_or_default();

		let metadata = files
			.into_iter()
			.map(|file| FileMetadata {
				id: file.id.unwrap_or_default(),
				name: file.name.unwrap_or_default(),
				mime_type: file.mime_type.unwrap_or_default(),
				size: file.size,
				created_time: file.created_time,
				modified_time: file.modified_time,
				web_view_link: file.web_view_link,
				parents: file.parents.unwrap_or_default(),
			})
			.collect();

		Ok(metadata)
	}

	pub async fn get_file_owner(&self, file_id: &str) -> Result<String, DriveError> {
		let service = self.client.get_service().await?;

		let file = service
			.files()
			.get(file_id)
			.supports_all_drives(true)
			.add_scope(Scope::Readonly.as_ref())
			.param("fields", "owners")
			.doit()
			.await?
			.1;

		match file.owners {
			Some(owners) if !owners.is_empty() => match &owners[0].email_address {
				Some(email) => Ok(email.clone()),
				None => Err(DriveError::OwnerEmailNotFound),
			},
			_ => Err(DriveError::OwnersNotFound),
		}
	}
}

pub struct WriteToDrive {
	client: Arc<GoogleDriveClient>,
}

impl WriteToDrive {
	pub fn new(user_email: String, client_secret_path: String) -> Result<Self, DriveError> {
		Ok(Self {
			client: Arc::new(GoogleDriveClient::new(user_email, client_secret_path, Scope::File)?),
		})
	}

	pub async fn upload_file(&self, file_path: &Path, parent_folder_id: Option<&str>, mime_type: Option<&str>) -> Result<FileMetadata, DriveError> {
		if !file_path.exists() {
			return Err(DriveError::FileNotFound(file_path.display().to_string()));
		}

		let file_name = file_path
			.file_name()
			.ok_or_else(|| DriveError::InvalidMetadata("Invalid file path".to_string()))?
			.to_string_lossy()
			.to_string();

		let content_type = mime_type.unwrap_or("application/octet-stream");
		let mime: mime::Mime = content_type.parse().map_err(|_| DriveError::InvalidMimeType(content_type.to_string()))?;
		let file_content = fs::File::open(file_path)?;

		let mut file = google_drive3::api::File::default();
		file.name = Some(file_name);
		file.mime_type = Some(content_type.to_string());

		if let Some(parent_id) = parent_folder_id {
			file.parents = Some(vec![parent_id.to_string()]);
		}

		let result = self
			.client
			.get_service()
			.await?
			.files()
			.create(file)
			.use_content_as_indexable_text(true)
			.supports_team_drives(true)
			.supports_all_drives(true)
			.keep_revision_forever(false)
			.include_permissions_for_view("published")
			.ignore_default_visibility(true)
			.add_scope(Scope::File.as_ref())
			.upload(file_content, mime)
			.await?;

		let uploaded_file = result.1;

		Ok(FileMetadata {
			id: uploaded_file.id.ok_or_else(|| DriveError::InvalidMetadata("Missing file ID".to_string()))?,
			name: uploaded_file.name.ok_or_else(|| DriveError::InvalidMetadata("Missing file name".to_string()))?,
			mime_type: uploaded_file.mime_type.ok_or_else(|| DriveError::InvalidMetadata("Missing MIME type".to_string()))?,
			size: uploaded_file.size,
			created_time: uploaded_file.created_time,
			modified_time: uploaded_file.modified_time,
			web_view_link: uploaded_file.web_view_link,
			parents: uploaded_file.parents.unwrap_or_default(),
		})
	}

	pub async fn update_file(&self, file_id: &str, new_file_path: &Path, mime_type: Option<&str>) -> Result<FileMetadata, DriveError> {
		if !new_file_path.exists() {
			return Err(DriveError::FileNotFound(new_file_path.display().to_string()));
		}

		let content_type = mime_type.unwrap_or("application/octet-stream");
		let mime: mime::Mime = content_type.parse().map_err(|_| DriveError::InvalidMimeType(content_type.to_string()))?;
		let file_content = fs::File::open(new_file_path)?;

		let file = google_drive3::api::File::default();

		let result = self
			.client
			.get_service()
			.await?
			.files()
			.update(file, file_id)
			.add_scope(Scope::File.as_ref())
			.upload(file_content, mime)
			.await?;

		let updated_file = result.1;

		Ok(FileMetadata {
			id: updated_file.id.ok_or_else(|| DriveError::InvalidMetadata("Missing file ID".to_string()))?,
			name: updated_file.name.ok_or_else(|| DriveError::InvalidMetadata("Missing file name".to_string()))?,
			mime_type: updated_file.mime_type.ok_or_else(|| DriveError::InvalidMetadata("Missing MIME type".to_string()))?,
			size: updated_file.size,
			created_time: updated_file.created_time,
			modified_time: updated_file.modified_time,
			web_view_link: updated_file.web_view_link,
			parents: updated_file.parents.unwrap_or_default(),
		})
	}

	pub async fn delete_file(&self, file_id: &str) -> Result<(), DriveError> {
		self.client.get_service().await?.files().delete(file_id).add_scope(Scope::File.as_ref()).doit().await?;

		Ok(())
	}

	pub async fn delete_file_with_service_account(&self, file_id: &str) -> Result<(), DriveError> {
		let read_client = ReadDrive::new(self.client.user_email.clone(), self.client.client_secret_path.clone().as_str().to_string())?;

		let current_owner = read_client.get_file_owner(file_id).await?;

		if current_owner != self.client.user_email.clone() {
			self.transfer_ownership(file_id).await?;

			// Optional: Add a small delay to ensure the permission change propagates
			tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
		}

		self.delete_file(file_id).await?;

		Ok(())
	}

	pub async fn transfer_ownership(&self, file_id: &str) -> Result<(), DriveError> {
		let service = self.client.get_service().await?;
		let owner_permission = google_drive3::api::Permission {
			role: Some("owner".to_string()),
			type_: Some("user".to_string()),
			email_address: Some(self.client.user_email.clone()),
			..Default::default()
		};

		service
			.permissions()
			.create(owner_permission, file_id)
			.transfer_ownership(true)
			.supports_all_drives(true)
			.add_scope(Scope::File.as_ref())
			.doit()
			.await?;

		Ok(())
	}
}

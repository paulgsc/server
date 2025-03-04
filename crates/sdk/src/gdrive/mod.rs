use crate::{GoogleServiceFilePath, SecretFilePathError};
use chrono::{DateTime, Utc};
use google_drive3::hyper_rustls;
use google_drive3::yup_oauth2::Error as OAuth2Error;
use google_drive3::yup_oauth2::ServiceAccountAuthenticator;
use google_drive3::Error as GoogleDriveError;
use google_drive3::{hyper, DriveHub};
use hyper::Error as HyperError;
use hyper_rustls::HttpsConnector;
use hyper_util::client::legacy::connect::HttpConnector;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
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

	#[error("Invalid file metadata: {0}")]
	InvalidMetadata(String),

	#[error("Secret file path error: {0}")]
	SecretFilePath(#[from] SecretFilePathError),
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

pub struct GoogleDriveClient {
	#[allow(dead_code)]
	user_email: String,
	service: Arc<Mutex<Option<Arc<DriveClient>>>>,
	client_secret_path: GoogleServiceFilePath,
}

impl GoogleDriveClient {
	pub fn new(user_email: String, client_secret_path: String) -> Result<Self, DriveError> {
		let validated_path = GoogleServiceFilePath::new(client_secret_path)?;

		Ok(Self {
			user_email,
			service: Arc::new(Mutex::new(None)),
			client_secret_path: validated_path,
		})
	}

	async fn initialize_service(&self) -> Result<DriveClient, DriveError> {
		let secret = google_drive3::yup_oauth2::read_service_account_key(&self.client_secret_path.as_ref()).await?;

		let auth = ServiceAccountAuthenticator::builder(secret).build().await?;

		let connector = hyper_rustls::HttpsConnectorBuilder::new()
			.with_native_roots()
			.unwrap()
			.https_or_http()
			.enable_http1()
			.build();

		let executor = hyper_util::rt::TokioExecutor::new();
		let client = hyper_util::client::legacy::Client::builder(executor).build(connector);

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
			client: Arc::new(GoogleDriveClient::new(user_email, client_secret_path)?),
		})
	}

	pub async fn list_files(&self, folder_id: Option<&str>, page_size: i32) -> Result<Vec<FileMetadata>, DriveError> {
		let mut query = String::new();

		if let Some(folder) = folder_id {
			query.push_str(&format!("'{}' in parents", folder));
		}

		let result = self
			.client
			.get_service()
			.await?
			.files()
			.list()
			.q(&query)
			.page_size(page_size)
			.spaces("drive")
			.fields("files(id, name, mimeType, size, createdTime, modifiedTime, webViewLink, parents)")
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
				created_time: file.created_time.and_then(|t| t.parse::<DateTime<Utc>>().ok()),
				modified_time: file.modified_time.and_then(|t| t.parse::<DateTime<Utc>>().ok()),
				web_view_link: file.web_view_link,
				parents: file.parents.unwrap_or_default(),
			})
			.collect();

		Ok(metadata)
	}

	pub async fn get_file_metadata(&self, file_id: &str) -> Result<FileMetadata, DriveError> {
		let result = self
			.client
			.get_service()
			.await?
			.files()
			.get(file_id)
			.fields("id, name, mimeType, size, createdTime, modifiedTime, webViewLink, parents")
			.doit()
			.await?;

		let file = result.1;

		Ok(FileMetadata {
			id: file.id.ok_or_else(|| DriveError::InvalidMetadata("Missing file ID".to_string()))?,
			name: file.name.ok_or_else(|| DriveError::InvalidMetadata("Missing file name".to_string()))?,
			mime_type: file.mime_type.ok_or_else(|| DriveError::InvalidMetadata("Missing MIME type".to_string()))?,
			size: file.size,
			created_time: file.created_time.and_then(|t| t.parse::<DateTime<Utc>>().ok()),
			modified_time: file.modified_time.and_then(|t| t.parse::<DateTime<Utc>>().ok()),
			web_view_link: file.web_view_link,
			parents: file.parents.unwrap_or_default(),
		})
	}

	pub async fn download_file(&self, file_id: &str, destination: &Path) -> Result<PathBuf, DriveError> {
		// First, get the file metadata to determine if it's a Google Doc or regular file
		let metadata = self.get_file_metadata(file_id).await?;
		let service = self.client.get_service().await?;

		// Create parent directories if they don't exist
		if let Some(parent) = destination.parent() {
			fs::create_dir_all(parent)?;
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

			let result = service.files().export(file_id, export_mime_type).doit().await?;
			let content = hyper::body::to_bytes(result.0.into_body()).await?;
			fs::write(destination, content)?;
		} else {
			// Regular files can be downloaded directly
			let result = service.files().get(file_id).param("alt", "media").doit().await?;
			let content = hyper::body::to_bytes(result.0.into_body()).await?;
			fs::write(destination, content)?;
		}

		Ok(destination.to_path_buf())
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
			.fields("files(id, name, mimeType, size, createdTime, modifiedTime, webViewLink, parents)")
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
				created_time: file.created_time.and_then(|t| t.parse::<DateTime<Utc>>().ok()),
				modified_time: file.modified_time.and_then(|t| t.parse::<DateTime<Utc>>().ok()),
				web_view_link: file.web_view_link,
				parents: file.parents.unwrap_or_default(),
			})
			.collect();

		Ok(metadata)
	}
}

pub struct WriteToDrive {
	client: Arc<GoogleDriveClient>,
}

impl WriteToDrive {
	pub fn new(user_email: String, client_secret_path: String) -> Result<Self, DriveError> {
		Ok(Self {
			client: Arc::new(GoogleDriveClient::new(user_email, client_secret_path)?),
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
		let file_content = fs::read(file_path)?;

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
			.upload(hyper::Body::from(file_content), content_type.parse().unwrap())
			.await?;

		let uploaded_file = result.1;

		Ok(FileMetadata {
			id: uploaded_file.id.ok_or_else(|| DriveError::InvalidMetadata("Missing file ID".to_string()))?,
			name: uploaded_file.name.ok_or_else(|| DriveError::InvalidMetadata("Missing file name".to_string()))?,
			mime_type: uploaded_file.mime_type.ok_or_else(|| DriveError::InvalidMetadata("Missing MIME type".to_string()))?,
			size: uploaded_file.size,
			created_time: uploaded_file.created_time.and_then(|t| t.parse::<DateTime<Utc>>().ok()),
			modified_time: uploaded_file.modified_time.and_then(|t| t.parse::<DateTime<Utc>>().ok()),
			web_view_link: uploaded_file.web_view_link,
			parents: uploaded_file.parents.unwrap_or_default(),
		})
	}

	pub async fn create_folder(&self, folder_name: &str, parent_folder_id: Option<&str>) -> Result<FileMetadata, DriveError> {
		let mut folder = google_drive3::api::File::default();
		folder.name = Some(folder_name.to_string());
		folder.mime_type = Some("application/vnd.google-apps.folder".to_string());

		if let Some(parent_id) = parent_folder_id {
			folder.parents = Some(vec![parent_id.to_string()]);
		}

		let result = self.client.get_service().await?.files().create(folder).doit().await?;

		let created_folder = result.1;

		Ok(FileMetadata {
			id: created_folder.id.ok_or_else(|| DriveError::InvalidMetadata("Missing folder ID".to_string()))?,
			name: created_folder.name.ok_or_else(|| DriveError::InvalidMetadata("Missing folder name".to_string()))?,
			mime_type: created_folder.mime_type.ok_or_else(|| DriveError::InvalidMetadata("Missing MIME type".to_string()))?,
			size: created_folder.size,
			created_time: created_folder.created_time.and_then(|t| t.parse::<DateTime<Utc>>().ok()),
			modified_time: created_folder.modified_time.and_then(|t| t.parse::<DateTime<Utc>>().ok()),
			web_view_link: created_folder.web_view_link,
			parents: created_folder.parents.unwrap_or_default(),
		})
	}

	pub async fn update_file(&self, file_id: &str, new_file_path: &Path, mime_type: Option<&str>) -> Result<FileMetadata, DriveError> {
		if !new_file_path.exists() {
			return Err(DriveError::FileNotFound(new_file_path.display().to_string()));
		}

		let content_type = mime_type.unwrap_or("application/octet-stream");
		let file_content = fs::read(new_file_path)?;

		let file = google_drive3::api::File::default();

		let result = self
			.client
			.get_service()
			.await?
			.files()
			.update(file, file_id)
			.upload(hyper::Body::from(file_content), content_type.parse().unwrap())
			.await?;

		let updated_file = result.1;

		Ok(FileMetadata {
			id: updated_file.id.ok_or_else(|| DriveError::InvalidMetadata("Missing file ID".to_string()))?,
			name: updated_file.name.ok_or_else(|| DriveError::InvalidMetadata("Missing file name".to_string()))?,
			mime_type: updated_file.mime_type.ok_or_else(|| DriveError::InvalidMetadata("Missing MIME type".to_string()))?,
			size: updated_file.size,
			created_time: updated_file.created_time.and_then(|t| t.parse::<DateTime<Utc>>().ok()),
			modified_time: updated_file.modified_time.and_then(|t| t.parse::<DateTime<Utc>>().ok()),
			web_view_link: updated_file.web_view_link,
			parents: updated_file.parents.unwrap_or_default(),
		})
	}

	pub async fn delete_file(&self, file_id: &str) -> Result<(), DriveError> {
		self.client.get_service().await?.files().delete(file_id).doit().await?;

		Ok(())
	}

	pub async fn move_file(&self, file_id: &str, new_parent_id: &str, remove_parents: Option<&str>) -> Result<FileMetadata, DriveError> {
		let result = self
			.client
			.get_service()
			.await?
			.files()
			.update(google_drive3::api::File::default(), file_id)
			.add_parents(new_parent_id)
			.remove_parents(remove_parents.unwrap_or(""))
			.doit()
			.await?;

		let moved_file = result.1;

		Ok(FileMetadata {
			id: moved_file.id.ok_or_else(|| DriveError::InvalidMetadata("Missing file ID".to_string()))?,
			name: moved_file.name.ok_or_else(|| DriveError::InvalidMetadata("Missing file name".to_string()))?,
			mime_type: moved_file.mime_type.ok_or_else(|| DriveError::InvalidMetadata("Missing MIME type".to_string()))?,
			size: moved_file.size,
			created_time: moved_file.created_time.and_then(|t| t.parse::<DateTime<Utc>>().ok()),
			modified_time: moved_file.modified_time.and_then(|t| t.parse::<DateTime<Utc>>().ok()),
			web_view_link: moved_file.web_view_link,
			parents: moved_file.parents.unwrap_or_default(),
		})
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use google_drive3::api::{File, FileList};
	use mockall::predicate::*;
	use mockall::*;
	use std::sync::Arc;

	// Mock the Google Drive API client
	mock! {
	DriveClient {
						fn files(&self) -> FilesHub;
							}

			pub struct FilesHub;
					impl Clone for FilesHub {}

							trait FilesHub {
												fn list(&self) -> FilesList;
															fn get(&self, file_id: &str) -> FilesGet;
																		fn create(&self, file: File) -> FilesCreate;
																					fn update(&self, file: File, file_id: &str) -> FilesUpdate;
																								fn delete(&self, file_id: &str) -> FilesDelete;
																											fn export(&self, file_id: &str, mime_type: &str) -> FilesExport;
																												}

									pub struct FilesList;
											impl Clone for FilesList {}

													trait FilesList {
																		fn q(&self, query: &str) -> Self;
																					fn page_size(&self, page_size: i32) -> Self;
																								fn spaces(&self, spaces: &str) -> Self;
																											fn fields(&self, fields: &str) -> Self;
																														fn doit(&self) -> Result<(hyper::Response<hyper::Body>, FileList), google_drive3::Error>;
																															}

															pub struct FilesGet;
																	impl Clone for FilesGet {}

																			trait FilesGet {
																								fn fields(&self, fields: &str) -> Self;
																											fn param(&self, param_name: &str, param_value: &str) -> Self;
																														fn doit(&self) -> Result<(hyper::Response<hyper::Body>, File), google_drive3::Error>;
																															}

																					pub struct FilesCreate;
																							impl Clone for FilesCreate {}

																									trait FilesCreate {
																														fn upload<T: Into<hyper::Body>>(&self, content: T, mime_type: mime::Mime) ->
																																				Result<(hyper::Response<hyper::Body>, File), google_drive3::Error>;
																																	fn doit(&self) -> Result<(hyper::Response<hyper::Body>, File), google_drive3::Error>;
																																		}

																											pub struct FilesUpdate;
																													impl Clone for FilesUpdate {}

																															trait FilesUpdate {
																																				fn upload<T: Into<hyper::Body>>(&self, content: T, mime_type: mime::Mime) ->
																																										Result<(hyper::Response<hyper::Body>, File), google_drive3::Error>;
																																							fn add_parents(&self, parents: &str) -> Self;
																																										fn remove_parents(&self, parents: &str) -> Self;
																																													fn doit(&self) -> Result<(hyper::Response<hyper::Body>, File), google_drive3::Error>;
																																														}

																																	pub struct FilesDelete;
																																			impl Clone for FilesDelete {}

																																					trait FilesDelete {
																																										fn doit(&self) -> Result<(hyper::Response<hyper::Body>, ()), google_drive3::Error>;
																																											}

																																							pub struct FilesExport;
																																									impl Clone for FilesExport {}

																																											trait FilesExport {
																																																fn doit(&self) -> Result<(hyper::Response<hyper::Body>, ()), google_drive3::Error>;
																																																	}
																																											}

	#[tokio::test]
	async fn test_list_files() {
		let mut mock_client = MockDriveClient::new();
		let folder_id = "test_folder_id";
		let page_size = 10;

		// Set up mock expectations
		let mut mock_hub = MockFilesHub::new();
		let mut mock_list = MockFilesList::new();

		let file_list = FileList {
			files: Some(vec![
				File {
					id: Some("file1".to_string()),
					name: Some("Test File 1".to_string()),
					mime_type: Some("text/plain".to_string()),
					size: Some(1024),
					parents: Some(vec!["test_folder_id".to_string()]),
					..Default::default()
				},
				File {
					id: Some("file2".to_string()),
					name: Some("Test File 2".to_string()),
					mime_type: Some("application/pdf".to_string()),
					size: Some(2048),
					parents: Some(vec!["test_folder_id".to_string()]),
					..Default::default()
				},
			]),
			..Default::default()
		};

		mock_list.expect_q().with(eq(format!("'{}' in parents", folder_id))).return_once(|_| MockFilesList::new());
		mock_list.expect_page_size().with(eq(page_size)).return_once(|_| MockFilesList::new());
		mock_list.expect_spaces().with(eq("drive")).return_once(|_| MockFilesList::new());
		mock_list
			.expect_fields()
			.with(eq("files(id, name, mimeType, size, createdTime, modifiedTime, webViewLink, parents)"))
			.return_once(|_| MockFilesList::new());
		mock_list.expect_doit().return_once(move || Ok((hyper::Response::new(hyper::Body::empty()), file_list)));

		mock_hub.expect_list().return_once(move || mock_list);
		mock_client.expect_files().return_once(move || mock_hub);

		// Create client with mocked service
		let client = GoogleDriveClient::new("test@example.com".to_string(), "/path/to/credentials.json".to_string()).unwrap();
		client.service.lock().await.replace(Arc::new(mock_client));

		// Execute test
		let reader = ReadDrive { client: Arc::new(client) };
		let files = reader.list_files(Some(folder_id), page_size).await.unwrap();

		assert_eq!(files.len(), 2);
		assert_eq!(files[0].id, "file1");
		assert_eq!(files[0].name, "Test File 1");
		assert_eq!(files[0].mime_type, "text/plain");
		assert_eq!(files[0].size.unwrap(), 1024);
		assert_eq!(files[1].id, "file2");
	}

	#[tokio::test]
	async fn test_create_folder() {
		let mut mock_client = MockDriveClient::new();
		let folder_name = "New Folder";
		let parent_id = "parent_folder_id";

		// Set up mock expectations
		let mut mock_hub = MockFilesHub::new();
		let mut mock_create = MockFilesCreate::new();

		let created_folder = File {
			id: Some("new_folder_id".to_string()),
			name: Some(folder_name.to_string()),
			mime_type: Some("application/vnd.google-apps.folder".to_string()),
			parents: Some(vec![parent_id.to_string()]),
			..Default::default()
		};

		mock_create
			.expect_doit()
			.return_once(move || Ok((hyper::Response::new(hyper::Body::empty()), created_folder)));
		mock_hub.expect_create().return_once(move |_| mock_create);
		mock_client.expect_files().return_once(move || mock_hub);

		// Create client with mocked service
		let client = GoogleDriveClient::new("test@example.com".to_string(), "/path/to/credentials.json".to_string()).unwrap();
		client.service.lock().await.replace(Arc::new(mock_client));

		// Execute test
		let writer = WriteToDrive { client: Arc::new(client) };
		let result = writer.create_folder(folder_name, Some(parent_id)).await.unwrap();

		assert_eq!(result.id, "new_folder_id");
		assert_eq!(result.name, folder_name);
		assert_eq!(result.mime_type, "application/vnd.google-apps.folder");
		assert_eq!(result.parents[0], parent_id);
	}

	#[tokio::test]
	async fn test_delete_file() {
		let mut mock_client = MockDriveClient::new();
		let file_id = "file_to_delete";

		// Set up mock expectations
		let mut mock_hub = MockFilesHub::new();
		let mut mock_delete = MockFilesDelete::new();

		mock_delete.expect_doit().return_once(move || Ok((hyper::Response::new(hyper::Body::empty()), ())));
		mock_hub.expect_delete().with(eq(file_id)).return_once(move |_| mock_delete);
		mock_client.expect_files().return_once(move || mock_hub);

		// Create client with mocked service
		let client = GoogleDriveClient::new("test@example.com".to_string(), "/path/to/credentials.json".to_string()).unwrap();
		client.service.lock().await.replace(Arc::new(mock_client));

		// Execute test
		let writer = WriteToDrive { client: Arc::new(client) };
		let result = writer.delete_file(file_id).await;

		assert!(result.is_ok());
	}
}

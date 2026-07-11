use crate::google_client::{self, ClientCache, GoogleClientError, HttpsConnectorType};
use crate::{GoogleServiceFilePath, SecretFilePathError};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use google_drive3::api::Scope;
use google_drive3::api::{File as DriveFile, Permission};
use google_drive3::DriveHub;
use google_drive3::Error as GoogleDriveError;
use http_body_util::BodyExt;
use hyper::{body::Bytes, Error as HyperError};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::Path;
use std::sync::Arc;

type DriveClient = DriveHub<HttpsConnectorType>;

#[derive(Debug, thiserror::Error)]
pub enum DriveError {
	#[error("Client error: {0}")]
	Client(#[from] GoogleClientError),

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

impl TryFrom<DriveFile> for FileMetadata {
	type Error = DriveError;

	fn try_from(file: DriveFile) -> Result<Self, Self::Error> {
		Ok(Self {
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
}

/// Lossy conversion used for listing/search results, where a file missing
/// optional metadata should still show up in the page rather than aborting
/// the whole call.
fn file_metadata_lossy(file: DriveFile) -> FileMetadata {
	FileMetadata {
		id: file.id.unwrap_or_default(),
		name: file.name.unwrap_or_default(),
		mime_type: file.mime_type.unwrap_or_default(),
		size: file.size,
		created_time: file.created_time,
		modified_time: file.modified_time,
		web_view_link: file.web_view_link,
		parents: file.parents.unwrap_or_default(),
	}
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FileListPage {
	pub files: Vec<FileMetadata>,
	pub next_page_token: Option<String>,
}

/// The mockable service boundary: everything `ReadDrive`/`WriteToDrive` need
/// from a Drive hub, expressed in plain data (no `hyper`/hub builder types),
/// so a test double can implement it without standing up a real HTTP stack.
#[async_trait]
pub trait DriveService: Send + Sync {
	async fn list_files(&self, folder_id: Option<&str>, page_size: i32, page_token: Option<&str>) -> Result<(Vec<DriveFile>, Option<String>), DriveError>;
	async fn get_file(&self, file_id: &str) -> Result<DriveFile, DriveError>;
	async fn download_file(&self, file_id: &str) -> Result<Bytes, DriveError>;
	async fn export_file(&self, file_id: &str, export_mime_type: &str) -> Result<Bytes, DriveError>;
	async fn search_files(&self, query: &str, page_size: i32) -> Result<Vec<DriveFile>, DriveError>;
	async fn get_file_owner_email(&self, file_id: &str) -> Result<String, DriveError>;
	async fn create_file(&self, metadata: DriveFile, content: Vec<u8>, mime: mime::Mime) -> Result<DriveFile, DriveError>;
	async fn update_file_content(&self, file_id: &str, content: Vec<u8>, mime: mime::Mime) -> Result<DriveFile, DriveError>;
	async fn delete_file(&self, file_id: &str) -> Result<(), DriveError>;
	async fn grant_owner_permission(&self, file_id: &str, email: &str) -> Result<(), DriveError>;
}

/// Real implementation backed by a live `DriveHub`.
struct RealDriveService {
	hub: DriveClient,
}

#[async_trait]
impl DriveService for RealDriveService {
	async fn list_files(&self, folder_id: Option<&str>, page_size: i32, page_token: Option<&str>) -> Result<(Vec<DriveFile>, Option<String>), DriveError> {
		let mut query = String::new();
		if let Some(folder) = folder_id {
			query.push_str(&format!("'{}' in parents", folder));
		}

		let mut call = self
			.hub
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
		Ok((result.1.files.unwrap_or_default(), result.1.next_page_token))
	}

	async fn get_file(&self, file_id: &str) -> Result<DriveFile, DriveError> {
		let result = self
			.hub
			.files()
			.get(file_id)
			.supports_team_drives(true)
			.supports_all_drives(true)
			.include_permissions_for_view("published")
			.add_scope(Scope::Readonly.as_ref())
			.doit()
			.await?;

		Ok(result.1)
	}

	async fn download_file(&self, file_id: &str) -> Result<Bytes, DriveError> {
		let (response, _) = self.hub.files().get(file_id).param("alt", "media").add_scopes(&[Scope::Readonly.as_ref()]).doit().await?;
		Ok(response.into_body().collect().await?.to_bytes())
	}

	async fn export_file(&self, file_id: &str, export_mime_type: &str) -> Result<Bytes, DriveError> {
		let result = self.hub.files().export(file_id, export_mime_type).add_scope(Scope::Readonly.as_ref()).doit().await?;
		Ok(result.into_body().collect().await?.to_bytes())
	}

	async fn search_files(&self, query: &str, page_size: i32) -> Result<Vec<DriveFile>, DriveError> {
		let result = self
			.hub
			.files()
			.list()
			.q(query)
			.page_size(page_size)
			.spaces("drive")
			.param("fields", "files(id, name, mimeType, size, createdTime, modifiedTime, webViewLink, parents)")
			.add_scope(Scope::Readonly.as_ref())
			.doit()
			.await?;

		Ok(result.1.files.unwrap_or_default())
	}

	async fn get_file_owner_email(&self, file_id: &str) -> Result<String, DriveError> {
		let file = self
			.hub
			.files()
			.get(file_id)
			.supports_all_drives(true)
			.add_scope(Scope::Readonly.as_ref())
			.param("fields", "owners")
			.doit()
			.await?
			.1;

		match file.owners {
			Some(owners) if !owners.is_empty() => owners[0].email_address.clone().ok_or(DriveError::OwnerEmailNotFound),
			_ => Err(DriveError::OwnersNotFound),
		}
	}

	async fn create_file(&self, metadata: DriveFile, content: Vec<u8>, mime: mime::Mime) -> Result<DriveFile, DriveError> {
		let cursor = io::Cursor::new(content);
		let result = self
			.hub
			.files()
			.create(metadata)
			.use_content_as_indexable_text(true)
			.supports_team_drives(true)
			.supports_all_drives(true)
			.keep_revision_forever(false)
			.include_permissions_for_view("published")
			.ignore_default_visibility(true)
			.add_scope(Scope::File.as_ref())
			.upload(cursor, mime)
			.await?;

		Ok(result.1)
	}

	async fn update_file_content(&self, file_id: &str, content: Vec<u8>, mime: mime::Mime) -> Result<DriveFile, DriveError> {
		let cursor = io::Cursor::new(content);
		let result = self
			.hub
			.files()
			.update(DriveFile::default(), file_id)
			.add_scope(Scope::File.as_ref())
			.upload(cursor, mime)
			.await?;

		Ok(result.1)
	}

	async fn delete_file(&self, file_id: &str) -> Result<(), DriveError> {
		self.hub.files().delete(file_id).add_scope(Scope::File.as_ref()).doit().await?;
		Ok(())
	}

	async fn grant_owner_permission(&self, file_id: &str, email: &str) -> Result<(), DriveError> {
		let owner_permission = Permission {
			role: Some("owner".to_string()),
			type_: Some("user".to_string()),
			email_address: Some(email.to_string()),
			..Default::default()
		};

		self
			.hub
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

static DRIVE_CLIENT_CACHE: Lazy<ClientCache<dyn DriveService>> = Lazy::new(ClientCache::new);

pub struct GoogleDriveClient {
	user_email: String,
	client_secret_path: GoogleServiceFilePath,
}

impl GoogleDriveClient {
	pub fn new(user_email: String, client_secret_path: String) -> Result<Self, DriveError> {
		let validated_path = GoogleServiceFilePath::new(client_secret_path)?;

		Ok(Self {
			user_email,
			client_secret_path: validated_path,
		})
	}

	pub async fn get_service(&self) -> Result<Arc<dyn DriveService>, DriveError> {
		let secret_path = self.client_secret_path.clone();

		DRIVE_CLIENT_CACHE
			.get_or_try_init("drive", &self.user_email, self.client_secret_path.as_str(), move || async move {
				let auth = google_client::build_service_account_authenticator(&secret_path).await?;
				let client = google_client::build_http_client()?;
				let hub = DriveHub::new(client, auth);
				Ok::<Arc<dyn DriveService>, GoogleClientError>(Arc::new(RealDriveService { hub }))
			})
			.await
			.map_err(DriveError::from)
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

	pub async fn list_files(&self, folder_id: Option<&str>, page_size: i32, page_token: Option<&str>) -> Result<FileListPage, DriveError> {
		let service = self.client.get_service().await?;
		let (files, next_page_token) = service.list_files(folder_id, page_size, page_token).await?;

		Ok(FileListPage {
			files: files.into_iter().map(file_metadata_lossy).collect(),
			next_page_token,
		})
	}

	pub async fn get_file_metadata(&self, file_id: &str) -> Result<FileMetadata, DriveError> {
		let service = self.client.get_service().await?;
		service.get_file(file_id).await?.try_into()
	}

	pub async fn download_file(&self, file_id: &str) -> Result<Bytes, DriveError> {
		const MAX_IN_MEMORY_SIZE: i64 = 10 * 1024 * 1024;

		let metadata = self.get_file_metadata(file_id).await?;
		let service = self.client.get_service().await?;

		if let Some(size) = metadata.size {
			if size > MAX_IN_MEMORY_SIZE {
				return Err(DriveError::FileTooLarge(format!(
					"File size {} bytes exceeds the maximum allowed size of {} bytes",
					size, MAX_IN_MEMORY_SIZE
				)));
			}
		}

		if metadata.mime_type.starts_with("application/vnd.google-apps") {
			let export_mime_type = match metadata.mime_type.as_str() {
				"application/vnd.google-apps.document" => "application/pdf",
				"application/vnd.google-apps.spreadsheet" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
				"application/vnd.google-apps.presentation" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
				_ => "application/pdf",
			};

			service.export_file(file_id, export_mime_type).await
		} else {
			service.download_file(file_id).await
		}
	}

	pub async fn search_files(&self, query: &str, page_size: i32) -> Result<Vec<FileMetadata>, DriveError> {
		let service = self.client.get_service().await?;
		let files = service.search_files(query, page_size).await?;
		Ok(files.into_iter().map(file_metadata_lossy).collect())
	}

	pub async fn get_file_owner(&self, file_id: &str) -> Result<String, DriveError> {
		let service = self.client.get_service().await?;
		service.get_file_owner_email(file_id).await
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
		let mime: mime::Mime = content_type.parse().map_err(|_| DriveError::InvalidMimeType(content_type.to_string()))?;
		let content = fs::read(file_path)?;

		let metadata = DriveFile {
			name: Some(file_name),
			mime_type: Some(content_type.to_string()),
			parents: parent_folder_id.map(|id| vec![id.to_string()]),
			..Default::default()
		};

		let service = self.client.get_service().await?;
		service.create_file(metadata, content, mime).await?.try_into()
	}

	pub async fn update_file(&self, file_id: &str, new_file_path: &Path, mime_type: Option<&str>) -> Result<FileMetadata, DriveError> {
		if !new_file_path.exists() {
			return Err(DriveError::FileNotFound(new_file_path.display().to_string()));
		}

		let content_type = mime_type.unwrap_or("application/octet-stream");
		let mime: mime::Mime = content_type.parse().map_err(|_| DriveError::InvalidMimeType(content_type.to_string()))?;
		let content = fs::read(new_file_path)?;

		let service = self.client.get_service().await?;
		service.update_file_content(file_id, content, mime).await?.try_into()
	}

	pub async fn delete_file(&self, file_id: &str) -> Result<(), DriveError> {
		let service = self.client.get_service().await?;
		service.delete_file(file_id).await
	}

	pub async fn delete_file_with_service_account(&self, file_id: &str) -> Result<(), DriveError> {
		let service = self.client.get_service().await?;
		let current_owner = service.get_file_owner_email(file_id).await?;

		if current_owner != self.client.user_email {
			self.transfer_ownership(file_id).await?;

			// Optional: Add a small delay to ensure the permission change propagates
			tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
		}

		self.delete_file(file_id).await
	}

	pub async fn transfer_ownership(&self, file_id: &str) -> Result<(), DriveError> {
		let service = self.client.get_service().await?;
		service.grant_owner_permission(file_id, &self.client.user_email).await
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::sync::Mutex as StdMutex;

	#[derive(Default)]
	struct MockDrive {
		files: StdMutex<Vec<DriveFile>>,
		deleted: StdMutex<Vec<String>>,
		owner_email: StdMutex<Option<String>>,
		granted_owner: StdMutex<Option<(String, String)>>,
	}

	impl MockDrive {
		fn with_files(files: Vec<DriveFile>) -> Self {
			Self {
				files: StdMutex::new(files),
				..Default::default()
			}
		}
	}

	#[async_trait]
	impl DriveService for MockDrive {
		async fn list_files(&self, _folder_id: Option<&str>, _page_size: i32, _page_token: Option<&str>) -> Result<(Vec<DriveFile>, Option<String>), DriveError> {
			Ok((self.files.lock().unwrap().clone(), None))
		}

		async fn get_file(&self, file_id: &str) -> Result<DriveFile, DriveError> {
			self
				.files
				.lock()
				.unwrap()
				.iter()
				.find(|f| f.id.as_deref() == Some(file_id))
				.cloned()
				.ok_or_else(|| DriveError::FileNotFound(file_id.to_string()))
		}

		async fn download_file(&self, _file_id: &str) -> Result<Bytes, DriveError> {
			Ok(Bytes::from_static(b"raw-bytes"))
		}

		async fn export_file(&self, _file_id: &str, _export_mime_type: &str) -> Result<Bytes, DriveError> {
			Ok(Bytes::from_static(b"exported-bytes"))
		}

		async fn search_files(&self, _query: &str, _page_size: i32) -> Result<Vec<DriveFile>, DriveError> {
			Ok(self.files.lock().unwrap().clone())
		}

		async fn get_file_owner_email(&self, _file_id: &str) -> Result<String, DriveError> {
			self.owner_email.lock().unwrap().clone().ok_or(DriveError::OwnersNotFound)
		}

		async fn create_file(&self, metadata: DriveFile, _content: Vec<u8>, _mime: mime::Mime) -> Result<DriveFile, DriveError> {
			let mut file = metadata;
			file.id = Some("new-file-id".to_string());
			self.files.lock().unwrap().push(file.clone());
			Ok(file)
		}

		async fn update_file_content(&self, file_id: &str, _content: Vec<u8>, _mime: mime::Mime) -> Result<DriveFile, DriveError> {
			Ok(DriveFile {
				id: Some(file_id.to_string()),
				name: Some("updated.txt".to_string()),
				mime_type: Some("text/plain".to_string()),
				..Default::default()
			})
		}

		async fn delete_file(&self, file_id: &str) -> Result<(), DriveError> {
			self.deleted.lock().unwrap().push(file_id.to_string());
			Ok(())
		}

		async fn grant_owner_permission(&self, file_id: &str, email: &str) -> Result<(), DriveError> {
			*self.granted_owner.lock().unwrap() = Some((file_id.to_string(), email.to_string()));
			Ok(())
		}
	}

	fn sample_file(id: &str, name: &str) -> DriveFile {
		DriveFile {
			id: Some(id.to_string()),
			name: Some(name.to_string()),
			mime_type: Some("text/plain".to_string()),
			size: Some(42),
			..Default::default()
		}
	}

	#[tokio::test]
	async fn file_metadata_conversion_requires_id_name_and_mime() {
		let file = sample_file("f1", "hello.txt");
		let metadata: FileMetadata = file.try_into().unwrap();
		assert_eq!(metadata.id, "f1");
		assert_eq!(metadata.name, "hello.txt");
		assert_eq!(metadata.size, Some(42));
	}

	#[tokio::test]
	async fn file_metadata_conversion_fails_without_id() {
		let file = DriveFile {
			name: Some("hello.txt".to_string()),
			mime_type: Some("text/plain".to_string()),
			..Default::default()
		};
		let result: Result<FileMetadata, DriveError> = file.try_into();
		assert!(matches!(result, Err(DriveError::InvalidMetadata(_))));
	}

	#[tokio::test]
	async fn list_files_lossily_converts_pages() {
		let mock: Arc<dyn DriveService> = Arc::new(MockDrive::with_files(vec![sample_file("a", "a.txt"), sample_file("b", "b.txt")]));
		let (files, next) = mock.list_files(None, 10, None).await.unwrap();
		assert_eq!(files.len(), 2);
		assert!(next.is_none());
	}

	#[tokio::test]
	async fn get_file_owner_email_reports_missing_owner() {
		let mock = MockDrive::default();
		let err = mock.get_file_owner_email("missing").await.unwrap_err();
		assert!(matches!(err, DriveError::OwnersNotFound));
	}

	#[tokio::test]
	async fn create_file_assigns_an_id() {
		let mock = MockDrive::default();
		let metadata = DriveFile {
			name: Some("new.txt".to_string()),
			..Default::default()
		};
		let created = mock.create_file(metadata, b"hi".to_vec(), mime::TEXT_PLAIN).await.unwrap();
		assert_eq!(created.id.as_deref(), Some("new-file-id"));
	}

	#[tokio::test]
	async fn delete_file_records_the_id() {
		let mock = MockDrive::default();
		mock.delete_file("f1").await.unwrap();
		assert_eq!(mock.deleted.lock().unwrap().as_slice(), ["f1".to_string()]);
	}

	#[tokio::test]
	async fn grant_owner_permission_records_transfer() {
		let mock = MockDrive::default();
		mock.grant_owner_permission("f1", "owner@example.com").await.unwrap();
		assert_eq!(mock.granted_owner.lock().unwrap().clone(), Some(("f1".to_string(), "owner@example.com".to_string())));
	}
}

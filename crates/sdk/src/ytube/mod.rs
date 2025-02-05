use crate::{GoogleServiceFilePath, SecretFilePathError};
use google_youtube3::hyper_rustls;
use google_youtube3::yup_oauth2::Error as OAuth2Error;
use google_youtube3::yup_oauth2::{InstalledFlowAuthenticator, InstalledFlowReturnMethod, ServiceAccountAuthenticator};
use google_youtube3::YouTube;
use hyper::Error as HyperError;
use hyper_rustls::HttpsConnector;
use hyper_util::client::legacy::connect::HttpConnector;
use once_cell::unsync::OnceCell;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;

type HttpsConnectorType = HttpsConnector<HttpConnector>;
type YouTubeClient = YouTube<HttpsConnectorType>;

#[derive(Debug, thiserror::Error)]
pub enum YtubeError {
	#[error("OAuth2 error: {0}")]
	OAuth2(#[from] OAuth2Error),

	#[error("YouTube API error: {0}")]
	YouTube(#[from] google_youtube3::Error),

	#[error("HTTP client error: {0}")]
	Hyper(#[from] HyperError),

	#[error("IO error: {0}")]
	Io(#[from] io::Error),

	#[error("Missing credentials file: {0}")]
	MissingCredentials(String),

	#[error("Service initialization failed: {0}")]
	ServiceInit(String),

	#[error("Invalid video metadata: {0}")]
	InvalidMetadata(String),

	#[error("Video not found: {0}")]
	VideoNotFound(String),

	#[error("Secret file path error: {0}")]
	SecretFilePath(#[from] SecretFilePathError),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct VideoMetadata {
	pub video_id: String,
	pub title: String,
	pub description: String,
	pub tags: Vec<String>,
	pub url: String,
}

pub struct YouTubeApiClient {
	service: OnceCell<Arc<YouTubeClient>>,
	client_secret_path: GoogleServiceFilePath,
}

impl YouTubeApiClient {
	pub fn new(client_secret_path: String) -> Result<Self, YtubeError> {
		let validated_path = GoogleServiceFilePath::new(client_secret_path)?;

		Ok(Self {
			service: OnceCell::new(),
			client_secret_path: validated_path,
		})
	}

	async fn initialize_service(&self) -> Result<YouTubeClient, YtubeError> {
		let secret = google_youtube3::yup_oauth2::read_service_account_key(&self.client_secret_path.as_ref()).await?;

		let auth = ServiceAccountAuthenticator::builder(secret).build().await?;

		let connector = hyper_rustls::HttpsConnectorBuilder::new()
			.with_native_roots()
			.unwrap()
			.https_or_http()
			.enable_http1()
			.build();

		let executor = hyper_util::rt::TokioExecutor::new();
		let client = hyper_util::client::legacy::Client::builder(executor).build(connector);

		Ok(YouTube::new(client, auth))
	}

	#[allow(dead_code)]
	async fn initialize_oauth_service(&self) -> Result<YouTubeClient, YtubeError> {
		let secret = google_youtube3::yup_oauth2::read_application_secret(&self.client_secret_path.as_ref()).await?;

		let cache_path = PathBuf::from("app").join("youtube_pickle").join("token_youtube_v3.json");

		fs::create_dir_all(cache_path.parent().unwrap()).map_err(YtubeError::Io)?;

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

		Ok(YouTube::new(client, auth))
	}

	pub async fn get_service(&self) -> Result<&Arc<YouTubeClient>, YtubeError> {
		if self.service.get().is_none() {
			let service = self.initialize_service().await?;
			self
				.service
				.set(Arc::new(service))
				.map_err(|_| YtubeError::ServiceInit("Failed to set service".to_string()))?;
		}
		Ok(self.service.get().unwrap())
	}
}

pub struct ReadYouTube {
	client: YouTubeApiClient,
}

impl ReadYouTube {
	pub fn new(client_secret_path: String) -> Result<Self, YtubeError> {
		Ok(Self {
			client: YouTubeApiClient::new(client_secret_path)?,
		})
	}

	pub async fn get_video_metadata(&self, video_id: &str) -> Result<VideoMetadata, YtubeError> {
		let response = self.client.get_service().await?.videos().list(&vec!["snippet".to_string()]).add_id(video_id).doit().await?;

		let video = response
			.1
			.items
			.and_then(|mut items| items.pop())
			.ok_or_else(|| YtubeError::VideoNotFound(video_id.to_string()))?;

		let snippet = video.snippet.ok_or_else(|| YtubeError::InvalidMetadata("Missing snippet".to_string()))?;

		Ok(VideoMetadata {
			video_id: video_id.to_string(),
			title: snippet.title.unwrap_or_default(),
			description: snippet.description.unwrap_or_default(),
			tags: snippet.tags.unwrap_or_default(),
			url: format!("https://www.youtube.com/watch?v={}", video_id),
		})
	}
}

pub struct UpdateYouTube {
	client: YouTubeApiClient,
}

impl UpdateYouTube {
	pub fn new(client_secret_path: String) -> Result<Self, YtubeError> {
		Ok(Self {
			client: YouTubeApiClient::new(client_secret_path)?,
		})
	}

	pub async fn update_video_description(&self, video_id: &str, new_description: &str) -> Result<(), YtubeError> {
		let mut request = google_youtube3::api::Video::default();
		request.id = Some(video_id.to_string());

		let mut snippet = google_youtube3::api::VideoSnippet::default();
		snippet.description = Some(new_description.to_string());
		request.snippet = Some(snippet);

		self.client.get_service().await?.videos().update(request).add_part("snippet").doit().await?;

		Ok(())
	}

	pub async fn update_video_metadata(&self, video_id: &str, title: Option<String>, description: Option<String>, tags: Option<Vec<String>>) -> Result<(), YtubeError> {
		// First get existing metadata to preserve unmodified fields
		let existing = self
			.client
			.get_service()
			.await?
			.videos()
			.list(&vec!["snippet".to_string()])
			.add_id(video_id)
			.doit()
			.await?
			.1
			.items
			.and_then(|mut items| items.pop())
			.ok_or_else(|| YtubeError::VideoNotFound(video_id.to_string()))?;

		let mut snippet = existing.snippet.ok_or_else(|| YtubeError::InvalidMetadata("Missing snippet".to_string()))?;

		// Update only provided fields
		if let Some(new_title) = title {
			snippet.title = Some(new_title);
		}
		if let Some(new_description) = description {
			snippet.description = Some(new_description);
		}
		if let Some(new_tags) = tags {
			snippet.tags = Some(new_tags);
		}

		let mut request = google_youtube3::api::Video::default();
		request.id = Some(video_id.to_string());
		request.snippet = Some(snippet);

		self.client.get_service().await?.videos().update(request).add_part("snippet").doit().await?;

		Ok(())
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use google_youtube3::api::{Video, VideoListResponse, VideoSnippet};
	use mockall::predicate::*;
	use mockall::*;

	// Mock the YouTube API client
	mock! {
			YouTubeClient {
					fn videos(&self) -> VideosHub;
			}

			pub struct VideosHub;
			impl Clone for VideosHub {}

			trait VideosHub {
					fn list(&self, parts: &Vec<String>) -> VideoList;
					fn update(&self, request: Video) -> VideoUpdate;
			}

			pub struct VideoList;
			impl Clone for VideoList {}

			trait VideoList {
					fn add_id(&self, video_id: &str) -> Self;
					fn doit(&self) -> Result<(hyper::Response<hyper::Body>, VideoListResponse), google_youtube3::Error>;
			}

			pub struct VideoUpdate;
			impl Clone for VideoUpdate {}

			trait VideoUpdate {
					fn add_part(&self, part: &str) -> Self;
					fn doit(&self) -> Result<(hyper::Response<hyper::Body>, Video), google_youtube3::Error>;
			}
	}

	#[tokio::test]
	async fn test_get_video_metadata() {
		let mut mock_client = MockYouTubeClient::new();
		let video_id = "test_video_id";

		// Set up mock expectations
		let mut mock_hub = MockVideosHub::new();
		let mut mock_list = MockVideoList::new();

		let response = VideoListResponse {
			items: Some(vec![Video {
				id: Some(video_id.to_string()),
				snippet: Some(VideoSnippet {
					title: Some("Test Video".to_string()),
					description: Some("Test Description".to_string()),
					tags: Some(vec!["test".to_string()]),
					..Default::default()
				}),
				..Default::default()
			}]),
			..Default::default()
		};

		mock_list.expect_add_id().with(eq(video_id)).return_once(|_| MockVideoList::new());

		mock_list.expect_doit().return_once(move || Ok((hyper::Response::new(hyper::Body::empty()), response)));

		mock_hub.expect_list().with(eq(vec!["snippet".to_string()])).return_once(|_| mock_list);

		mock_client.expect_videos().return_once(move || mock_hub);

		// Create client with mocked service
		let client = YouTubeApiClient::new("test@example.com".to_string(), "test_path".to_string()).unwrap();
		client.service.set(Arc::new(mock_client)).unwrap();

		// Execute test
		let reader = ReadYouTube::new("test@example.com".to_string(), "test_path".to_string()).unwrap();
		let result = reader.get_video_metadata(video_id).await.unwrap();

		assert_eq!(result.video_id, video_id);
		assert_eq!(result.title, "Test Video");
		assert_eq!(result.description, "Test Description");
		assert_eq!(result.tags, vec!["test"]);
	}

	// Add more tests for update functionality...
}

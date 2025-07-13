pub mod toa;

use crate::{GoogleServiceFilePath, SecretFilePathError};
use google_youtube3::hyper_rustls;
use google_youtube3::yup_oauth2::Error as OAuth2Error;
use google_youtube3::yup_oauth2::{InstalledFlowAuthenticator, InstalledFlowReturnMethod, ServiceAccountAuthenticator};
use google_youtube3::YouTube;
use google_youtubeanalytics2::YouTubeAnalytics;
use hyper::Error as HyperError;
use hyper_rustls::HttpsConnector;
use hyper_util::client::legacy::connect::HttpConnector;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::OnceCell;

type HttpsConnectorType = HttpsConnector<HttpConnector>;
type YouTubeClient = YouTube<HttpsConnectorType>;
type YoutubeAnalyticsClient = YouTubeAnalytics<HttpsConnectorType, yup_oauth2::Authenticator>;

#[derive(Debug, thiserror::Error)]
pub enum YtubeError {
	#[error("OAuth2 error: {0}")]
	OAuth2(#[from] OAuth2Error),

	#[error("YouTube API error: {0}")]
	YoutubeData(#[from] google_youtube3::Error),

	#[error("Youtube Analytics API error: {0}")]
	YouTubeAnalytics(#[from] google_youtubeanalytics2::Error),

	#[error("HTTP client error: {0}")]
	Hyper(#[from] HyperError),

	#[error("IO error: {0}")]
	Io(#[from] io::Error),

	#[error("Missing credentials file: {0}")]
	MissingCredentials(String),

	#[error("Service initialization failed: {0}")]
	ServiceInit(String),

	#[error("Data parsing error: {0}")]
	DataParsingError(String),

	#[error("Invalid video metadata: {0}")]
	InvalidMetadata(String),

	#[error("Video not found: {0}")]
	VideoNotFound(String),

	#[error("Channel not found: {0}")]
	ChannelNotFound(String),

	#[error("Analytics data not available: {0}")]
	AnalyticsUnavailable(String),

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

#[derive(Debug, Serialize, Deserialize)]
pub struct VideoAnalytics {
	pub video_id: String,
	pub views: u64,
	pub impressions: Option<u64>,
	pub ctr: Option<f64>,
	pub avd: Option<u64>,
	pub watch_t: Option<u64>,
	pub likes: Option<u64>,
	pub dislikes: Option<u64>,
	pub comments: Option<u64>,
	pub shares: Option<u64>,
	pub sub_gained: Option<i64>,
	pub est_rev: Option<f64>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ChannelAnalytics {
	pub channel_id: String,
	pub sub_count: u64,
	pub video_count: u64,
	pub view_count: u64,
	pub est_min_watched: Option<u64>,
	pub avg_view_duration: Option<u64>,
	pub create_date: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum VideoPrivacyStatus {
	Private,
	Public,
	Unlisted,
}

impl From<String> for VideoPrivacyStatus {
	fn from(s: String) -> Self {
		match s.as_str() {
			"private" => Self::Private,
			"public" => Self::Public,
			"unlisted" => Self::Unlisted,
			_ => Self::Private,
		}
	}
}

impl From<VideoPrivacyStatus> for String {
	fn from(status: VideoPrivacyStatus) -> Self {
		match status {
			VideoPrivacyStatus::Private => "private".to_string(),
			VideoPrivacyStatus::Public => "public".to_string(),
			VideoPrivacyStatus::Unlisted => "unlisted".to_string(),
		}
	}
}

#[derive(Debug, Serialize, Deserialize)]
pub struct VideoStatus {
	pub video_id: String,
	pub privacy_status: VideoPrivacyStatus,
	pub upload_status: Option<String>,
	pub failure_reason: Option<String>,
	pub rejection_reason: Option<String>,
	pub license: Option<String>,
	pub embeddable: Option<bool>,
	pub public_stats_viewable: Option<bool>,
	pub made_for_kids: Option<bool>,
	pub self_declared_made_for_kids: Option<bool>,
}

pub struct YouTubeApiClient {
	data_service: OnceCell<Arc<YouTubeClient>>,
	analytics_service: OnceCell<Arc<YoutubeAnalyticsClient>>,
	client_secret_path: GoogleServiceFilePath,
}

impl YouTubeApiClient {
	pub fn new(client_secret_path: String) -> Result<Self, YtubeError> {
		let validated_path = GoogleServiceFilePath::new(client_secret_path)?;

		Ok(Self {
			data_service: OnceCell::new(),
			analytics_service: OnceCell::new(),
			client_secret_path: validated_path,
		})
	}

	async fn create_http_client(&self) -> hyper_util::client::legacy::Client<HttpsConnectorType, hyper::body::Body> {
		let connector = hyper_rustls::HttpsConnectorBuilder::new()
			.with_native_roots()
			.unwrap()
			.https_or_http()
			.enable_http1()
			.build();

		let executor = hyper_util::rt::TokioExecutor::new();
		hyper_util::client::legacy::Client::builder(executor).build(connector)
	}

	async fn initialize_data_service(&self) -> Result<YouTubeClient, YtubeError> {
		let secret = google_youtube3::yup_oauth2::read_service_account_key(&self.client_secret_path.as_ref()).await?;
		let auth = ServiceAccountAuthenticator::builder(secret).build().await?;
		let client = self.create_http_client().await;

		Ok(YouTube::new(client, auth))
	}

	async fn initialize_analytics_service(&self) -> Result<YoutubeAnalyticsClient, YtubeError> {
		let secret = google_youtubeanalytics2::yup_oauth2::read_service_account_key(&self.client_secret_path.as_ref()).await?;
		let auth = ServiceAccountAuthenticator::builder(secret)
			.add_scope("https://www.googleapis.com/auth/yt-analytics.readonly")
			.build()
			.await?;
		let client = self.create_http_client().await;

		Ok(YouTubeAnalytics::new(client, auth))
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

	pub async fn get_data_service(&self) -> Result<&Arc<YouTubeClient>, YtubeError> {
		self
			.data_service
			.get_or_try_init(|| async {
				let service = self.initialize_data_service().await?;
				Ok(Arc::new(service))
			})
			.await
	}

	pub async fn get_analytics_service(&self) -> Result<&Arc<YoutubeAnalyticsClient>, YtubeError> {
		self
			.analytics_service
			.get_or_try_init(|| async {
				let service = self.initialize_analytics_service().await?;
				Ok(Arc::new(service))
			})
			.await
			.map_err(|_| YtubeError::ServiceInit("Failed to initialize Youtube Analytics API service".to_string()))
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
		let response = self
			.client
			.get_data_service()
			.await?
			.videos()
			.list(&vec!["snippet".to_string()])
			.add_id(video_id)
			.doit()
			.await?;

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

	pub async fn get_video_analytics(&self, video_id: &str, start_date: &&str, end_date: &str) -> Result<VideoAnalytics, YtubeError> {
		let data_response = self
			.client
			.get_data_service()
			.await?
			.videos()
			.list(&vec!["statistics".to_string()])
			.add_id(video_id)
			.doit()
			.await?;

		let video = data_response
			.1
			.items
			.and_then(|mut items| items.pop())
			.ok_or_else(|| YtubeError::VideoNotFound(video_id.to_string()))?;

		let statistics = video.statistics.ok_or_else(|| YtubeError::InvalidMetadata("Missing statistics".to_string()))?;

		let analytics_response = self
			.client
			.get_analytics_service()
			.await?
			.reports()
			.query(
				start_date,
				end_date,
				"yt:channel==MINE",
				"views,comments,likes,shares,estimatedMinutesWatched,averageViewDuration,subscribersGained,estimatedRevenue,impressions,ctr",
			)
			.add_filters(&format!("video=={}", video_id))
			.doit()
			.await
			.map_err(YtubeError::YouTubeAnalytics)?;

		let mut impressions: Option<u64> = None;
		let mut ctr: Option<f64> = None;
		let mut average_view_duration_analytics: Option<u64> = None;
		let mut watch_time_minutes: Option<u64> = None;
		let mut subscribers_gained: Option<i64> = None;
		let mut estimated_revenue: Option<f64> = None;

		if let Some(rows) = analytics_response.rows {
			if let Some(row) = rows.get(0) {
				if let Some(value) = row.get(0) {}
				if let Some(value) = row.get(1) {}
				if let Some(value) = row.get(2) {}
				if let Some(value) = row.get(3) {}
				if let Some(value) = row.get(4) {
					watch_time_minutes = value.as_str().and_then(|s| s.parse().ok());
				}
				if let Some(value) = row.get(5) {
					average_view_duration_analytics = value.as_str().and_then(|s| s.parse().ok())
				}
				if let Some(value) = row.get(6) {
					subscribers_gained = value.as_str().and_then(|s| s.parse().ok())
				}
				if let Some(value) = row.get(7) {
					estimated_revenue = value.as_str().and_then(|s| s.parse().ok())
				}
				if let Some(value) = row.get(8) {
					impressions = value.as_str().and_then(|s| s.parse().ok())
				}
				if let Some(value) = row.get(9) {
					ctr = value.as_str().and_then(|s| s.parse().ok())
				}
			}
		}

		Ok(VideoAnalytics {
			video_id: video_id.to_string(),
			views: statistics.view_count.unwrap_or_default().parse().unwrap_or(0),
			impressions,
			ctr,
			average_view_duration: average_view_duration_analytics,
			watch_time_minutes,
			likes: statistics.like_count.map(|c| c.parse().unwrap_or(0)),
			dislikes: None,
			comments: statistics.comment_count.map(|c| c.parse().unwrap_or(0)),
			shares: None,
			subscribers_gained,
			estimated_revenue,
		})
	}

	pub async fn get_channel_analytics(&self, channel_id: &str, start_date: &str, end_date: &str) -> Result<ChannelAnalytics, YtubeError> {
		let data_response = self
			.client
			.get_data_service()
			.await?
			.channels()
			.list(&vec!["statistics".to_string(), "snippet".to_string()])
			.add_id(channel_id)
			.doit()
			.await?;

		let channel = data_response
			.1
			.items
			.and_then(|mut items| items.pop())
			.ok_or_else(|| YtubeError::ChannelNotFound(channel_id.to_string()))?;

		let statistics = channel.statistics.ok_or_else(|| YtubeError::InvalidMetadata("Missing channel statistics".to_string()))?;
		let snippet = channel.snippet;

		let analytics_response = self
			.client
			.get_analytics_service()
			.await?
			.reports()
			.query(
				start_date,
				end_date,
				&format!("channel=={}", channel_id),
				"estimatedMinutesWatched,averageViewDuration,estimatedRevenue,views",
			)
			.doit()
			.await
			.map_err(YtubeError::YouTubeAnalytics)?;

		let mut estimated_minutes_watched: Option<u64> = None;
		let mut average_view_duration_analytics: Option<u64> = None;
		let mut estimated_revenue: Option<f64> = None;
		let mut views_analytics: Option<u64> = None;

		if let Some(rows) = analytics_response.rows {
			if let Some(row) = rows.get(0) {
				if let Some(value) = row.get(0) {
					estimated_minutes_watched = value.as_str().and_then(|s| s.parse().ok());
				}
				if let Some(value) = row.get(1) {
					average_view_duration = value.as_str().and_then(|s| s.parse().ok());
				}
				if let Some(value) = row.get(2) {
					estimated_revenue = value.as_str().and_then(|s| s.parse().ok());
				}
				if let Some(value) = row.get(3) {
					views_analytics = value.as_str().and_then(|s| s.parse().ok());
				}
			}
		}

		Ok(ChannelAnalytics {
			channel_id: channel_id.to_string(),
			sub_count: statistics.subscriber_count.unwrap_or_default().parse().unwrap_or(0),
			video_count: statistics.video_count.unwrap_or_default().parse().unwrap_or(0),
			view_count: statistics.view_count.unwrap_or_default().parse().unwrap_or(0),
			estimated_minutes_watched,
			average_view_duration: average_view_duration_analytics,
			create_date: snippet.and_then(|s| s.published_at),
			estimated_revenue,
			views_analytics,
		})
	}

	pub async fn get_video_status(&self, video_id: &str) -> Result<VideoStatus, YtubeError> {
		let response = self
			.client
			.get_data_service()
			.await?
			.videos()
			.list(&vec!["status".to_string()])
			.add_id(video_id)
			.doit()
			.await?;

		let video = response
			.1
			.items
			.and_then(|mut items| items.pop())
			.ok_or_else(|| YtubeError::VideoNotFound(video_id.to_string()))?;

		let status = video.status.ok_or_else(|| YtubeError::InvalidMetadata("Missing video status".to_string()))?;

		Ok(VideoStatus {
			video_id: video_id.to_string(),
			privacy_status: status.privacy_status.unwrap_or_default().into(),
			upload_status: status.upload_status,
			failure_reason: status.failure_reason,
			rejection_reason: status.rejection_reason,
			license: status.license,
			embeddable: status.embeddable,
			public_stats_viewable: status.public_stats_viewable,
			made_for_kids: status.made_for_kids,
			self_declared_made_for_kids: status.self_declared_made_for_kids,
		})
	}

	pub async fn get_multiple_videos_analytics(&self, video_ids: &[String], start_date: &str, end_date: &str) -> Result<Vec<VideoAnalytics>, YtubeError> {
		let mut analytics_results = Vec::new();

		let mut data_api_videos = HashMap::new();
		for chunk in video_ids.chunks(50) {
			let mut request = self.client.get_data_service().await?.videos().list(&vec!["statistics".to_string()]);
			for video_id in chunk {
				request = request.add_id(video_id);
			}
			let response = request.doit().await?;
			if let Some(items) = response.1.items {
				for video in items {
					if let (Some(id), Some(statistics)) = (video.id, video.statistics) {
						data_api_videos.insert(id, statistics);
					}
				}
			}
		}

		for video_id in video_ids {
			let mut video_analytics = VideoAnalytics {
				video_id: video_id.clone(),
				views: 0,
				impressions: None,
				ctr: None,
				average_view_duration: None,
				watch_time_minutes: None,
				likes: None,
				dislikes: None,
				comments: None,
				shares: None,
				subscribers_gained: None,
				estimated_revenue: None,
			};

			if let Some(statistics) = data_api_videos.get(video_id) {
				video_analytics.views = statistics.view_count.unwrap_or_default().parse().unwrap_or(0);
				video_analytics.likes = statistics.like_count.map(|c| c.parse().unwrap_or(0));
				video_analytics.comments = statistics.comment_count.map(|c| c.parse().unwrap_or(0));
			}

			let analytics_response = self
				.client
				.get_analytics_service()
				.await?
				.reports()
				.query(
					start_date,
					end_date,
					"yt:channel==MINE",
					"views,comments,likes,shares,estimatedMinutesWatched,averageViewDuration,subscribersGained,estimatedRevenue,impressions,ctr",
				)
				.add_filters(&format!("video=={}", video_id))
				.doit()
				.await
				.map_err(YtubeError::YouTubeAnalytics)?;

			if let Some(rows) = analytics_response.rows {
				if let Some(row) = rows.get(0) {
					if let Some(value) = row.get(4) {
						video_analytics.watch_time_minutes = value.as_str().and_then(|s| s.parse().ok());
					}
					if let Some(value) = row.get(5) {
						video_analytics.average_view_duration = value.as_str().and_then(|s| s.parse().ok());
					}
					if let Some(value) = row.get(6) {
						video_analytics.subscribers_gained = value.as_str().and_then(|s| s.parse().ok());
					}
					if let Some(value) = row.get(7) {
						video_analytics.estimated_revenue = value.as_str().and_then(|s| s.parse().ok());
					}
					if let Some(value) = row.get(8) {
						video_analytics.impressions = value.as_str().and_then(|s| s.parse().ok());
					}
					if let Some(value) = row.get(9) {
						video_analytics.click_through_rate = value.as_str().and_then(|s| s.parse().ok());
					}
				}
			}
			analytics_results.push(video_analytics);
		}

		Ok(analytics_results)
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

		self.client.get_data_service().await?.videos().update(request).add_part("snippet").doit().await?;

		Ok(())
	}

	pub async fn update_video_metadata(&self, video_id: &str, title: Option<String>, description: Option<String>, tags: Option<Vec<String>>) -> Result<(), YtubeError> {
		// First get existing metadata to preserve unmodified fields
		let existing = self
			.client
			.get_data_service()
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

		self.client.get_data_service().await?.videos().update(request).add_part("snippet").doit().await?;

		Ok(())
	}

	pub async fn update_video_privacy_status(&self, video_id: &str, privacy_status: VideoPrivacyStatus) -> Result<(), YtubeError> {
		let existing = self
			.client
			.get_data_service()
			.await?
			.videos()
			.list(&vec!["status".to_string()])
			.add_id(video_id)
			.doit()
			.await?
			.1
			.items
			.and_then(|mut items| items.pop())
			.ok_or_else(|| YtubeError::VideoNotFound(video_id.to_string()))?;

		let mut status = existing.status.unwrap_or_default();
		status.privacy_status = Some(privacy_status.into());

		let mut request = google_youtube3::api::Video::default();
		request.id = Some(video_id.to_string());
		request.status = Some(status);

		self.client.get_data_service().await?.videos().update(request).add_part("status").doit().await?;

		Ok(())
	}

	pub async fn update_video_status(
		&self,
		video_id: &str,
		privacy_status: Option<VideoPrivacyStatus>,
		embeddable: Option<bool>,
		public_stats_viewable: Option<bool>,
		made_for_kids: Option<bool>,
	) -> Result<(), YtubeError> {
		let existing = self
			.client
			.get_data_service()
			.await?
			.videos()
			.list(&vec!["status".to_string()])
			.add_id(video_id)
			.doit()
			.await?
			.1
			.items
			.and_then(|mut items| items.pop())
			.ok_or_else(|| YtubeError::VideoNotFound(video_id.to_string()))?;

		let mut status = existing.status.unwrap_or_default();

		if let Some(new_privacy) = privacy_status {
			status.privacy_status = Some(new_privacy.into());
		}
		if let Some(new_embeddable) = embeddable {
			status.embeddable = Some(new_embeddable);
		}
		if let Some(new_public_stats) = public_stats_viewable {
			status.public_stats_viewable = Some(new_public_stats);
		}
		if let Some(new_made_for_kids) = made_for_kids {
			status.made_for_kids = Some(new_made_for_kids);
		}

		let mut request = google_youtube3::api::Video::default();
		request.id = Some(video_id.to_string());
		request.status = Some(status);

		self.client.get_data_service().await?.videos().update(request).add_part("status").doit().await?;

		Ok(())
	}

	pub async fn batch_update_privacy_status(&self, video_ids: &[String], privacy_status: VideoPrivacyStatus) -> Result<Vec<Result<(), YtubeError>>, YtubeError> {
		let mut results = Vec::new();

		for video_id in video_ids {
			let result = self.update_video_privacy_status(video_id, privacy_status.clone()).await;
			results.push(result);
		}

		Ok(results)
	}

	pub async fn schedule_video_publicatio(&self, video_id: &str, publish_at: &str) -> Result<(), YtubeError> {
		let existing = self
			.client
			.get_data_service()
			.await?
			.videos()
			.list(&vec!["status".to_string()])
			.add_id(video_id)
			.doit()
			.await?
			.1
			.items
			.and_then(|mut itmes| items.pop())
			.ok_or_else(|| YtubeError::VideoNotFound(video_id.to_string()))?;

		let mut status = existing.status.unwrap_or_default();
		status.privacy_status = Some("private".to_string());
		status.publish_at = Some(publish_at.to_string());

		let mut request = google_youtube3::apiI::Video::default();
		request.id = Some(video_id.to_string());
		request.status = Some(status);

		self.client.get_data_service().await?.videos().update(request).add_part("status").doit().await?;

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

use crate::handlers::audio_files::{
	types::*,
	validation::{contract::*, core::*},
};
use crate::{CacheStore, ReadDrive};
use std::sync::Arc;
use thiserror::Error;
use tokio::time::{Duration, Instant};

#[derive(Debug, Error)]
pub enum AudioServiceError {
	#[error("Invalid audio file ID: {id}")]
	InvalidFileId {
		id: String,
		#[source]
		source: Option<anyhow::Error>,
	},

	#[error("Unsupported audio type: {mime_type}")]
	UnsupportedAudioType { mime_type: String },

	#[error("Failed to validate audio metadata")]
	ValidationFailed {
		#[from]
		source: ValidationError,
	},

	#[error("Search query is too complex or malformed")]
	InvalidSearchQuery {
		query: Option<String>,
		#[source]
		source: Option<anyhow::Error>,
	},

	#[error("Audio download failed")]
	DownloadFailed {
		id: String,
		#[source]
		source: anyhow::Error,
	},

	#[error("Metadata retrieval failed")]
	MetadataFetchFailed {
		id: String,
		#[source]
		source: anyhow::Error,
	},

	#[error("Search timed out")]
	SearchTimeout,

	#[error("Search failed due to backend issue")]
	SearchFailed {
		#[source]
		source: anyhow::Error,
	},

	#[error("Quota exceeded on file storage backend")]
	QuotaExceeded,

	#[error("Internal logic error: {message}")]
	Internal {
		message: String,
		#[source]
		source: Option<anyhow::Error>,
	},
}

#[async_trait::async_trait]
pub trait AudioService: Send + Sync + Clone + 'static {
	type Error: std::error::Error + Send + Sync + 'static;

	async fn get_audio(&mut self, req: GetAudioRequest) -> Result<(CachedAudio, bool), Self::Error>;
	async fn search_audio(&mut self, req: SearchAudioRequest) -> Result<AudioSearchResponse, Self::Error>;
}

// ========== UNIFIED REQUEST FOR TOWER SERVICE ==========

#[derive(Debug, Clone, Validate)]
pub enum AudioServiceRequest {
	#[validate]
	Get(GetAudioRequest),
	#[validate]
	Search(SearchAudioRequest),
}

#[derive(Debug)]
pub enum AudioServiceResponse {
	Get((CachedAudio, bool)),
	Search(AudioSearchResponse),
}

#[derive(Clone)]
pub struct CoreAudioService {
	drive_client: Arc<ReadDrive>,
	cache: Arc<CacheStore>,
	validator: AudioValidtor,
}

impl CoreAudioService {
	pub fn new() -> Result<Self, FileHostError> {
		let validator = AudioValidtor::basic_validator(ValidationConstraints::default());

		Ok(Self { validator })
	}

	async fn fetch_and_validate_audio(&self, id: &str) -> Result<CachedAudio, AudioServiceError> {
		let span = tracing::Span::current();
		span.record("audio_id", &id);

		let start_time = Instant::now();

		// Get metadata
		let metadata = self.drive_client.get_file_metadata(id).await.map_err(|e| AudioServiceError::MetadataFetchFailed {
			id: id.to_string(),
			source: e.into(),
		})?;

		// Validate before download
		self.validator.validate_complete(&metadata).map_err(|e| AudioServiceError::ValidationFailed { source: e })?;

		// Download
		let audio_data = self.drive_client.download_file(id).await.map_err(|e| AudioServiceError::DownloadFailed {
			id: id.to_string(),
			source: e.into(),
		})?;

		let download_duration = start_time.elapsed();
		tracing::info!("Downloaded audio {} in {:?} ({} bytes)", id, download_duration, audio_data.len());

		let etag = self.generate_etag(&audio_data, &metadata);

		let last_modified = metadata
			.modified_time
			.as_deref()
			.and_then(|t| t.parse::<std::time::SystemTime>().ok())
			.unwrap_or_else(std::time::SystemTime::now);

		Ok(CachedAudio {
			audio_data,
			content_type: metadata.mime_type,
			etag,
			last_modified,
			size: audio_data.len() as u64,
		})
	}

	fn generate_etag(&self, data: &Bytes, metadata: &sdk::FileMetadata) -> String {
		use std::collections::hash_map::DefaultHasher;
		use std::hash::{Hash, Hasher};

		let mut hasher = DefaultHasher::new();
		data.len().hash(&mut hasher);
		metadata.modified_time.hash(&mut hasher);
		metadata.mime_type.hash(&mut hasher);

		format!("\"{}\"", hasher.finish())
	}
}

#[async_trait::async_trait]
impl AudioService for CoreAudioService {
	type Error = FileHostError;

	async fn get_audio(&mut self, req: GetAudioRequest) -> Result<(CachedAudio, bool), Self::Error> {
		// Validate request first
		req.validate().map_err(|e| AudioServiceError::InvalidFileId {
			id: req.id.clone(),
			source: Some(e.into()),
		})?;

		if req.force_refresh {
			let audio = self.fetch_and_validate_audio(&req.id).await?;
			return Ok((audio, false));
		}

		self
			.cache
			.get_or_fetch(&req.id, || async {
				self.fetch_and_validate_audio(&req.id).await.map_err(Into::into) // AudioServiceError → FileHostError
			})
			.await
	}

	async fn search_audio(&mut self, req: SearchAudioRequest) -> Result<AudioSearchResponse, Self::Error> {
		// Validate request
		req.validate().map_err(|e| AudioServiceError::InvalidSearchQuery {
			query: req.query.clone(),
			source: Some(e.into()),
		})?;

		let limit = req.limit.unwrap_or(20).min(100) as i32;
		let offset = req.offset.unwrap_or(0);

		let sanitized_query = req.query.as_ref().map(|q| sanitize_search_query(q)).filter(|q| !q.is_empty());
		let mut query_parts = Vec::new();

		if let Some(folder_id) = &self.validator.config().audio_folder_id {
			if !folder_id.contains("..") && !folder_id.contains('/') {
				query_parts.push(format!("'{}' in parents", folder_id));
			}
		}

		let mime_filter = self
			.validator
			.supported_types()
			.iter()
			.map(|mime| format!("mimeType='{}'", mime.replace('\'', "")))
			.collect::<Vec<_>>()
			.join(" or ");
		query_parts.push(format!("({})", mime_filter));

		if let Some(q) = &sanitized_query {
			query_parts.push(format!("name contains '{}'", q));
		}

		if let Some(voice) = &req.voice {
			let sv = sanitize_search_query(voice);
			if !sv.is_empty() {
				query_parts.push(format!("name contains '{}'", sv));
			}
		}

		let full_query = query_parts.join(" and ");

		let files = tokio::time::timeout(Duration::from_secs(10), self.drive_client.search_files(&full_query, limit + 10)).await?;

		let total = files.len();
		let start = (offset as usize).min(total);
		let end = (start + limit as usize).min(total);
		let paginated = &files[start..end];

		let results = paginated
			.iter()
			.filter(|f| self.validator.is_audio_type(&f.mime_type))
			.map(|f| AudioMetadata {
				id: f.id.clone(),
				name: f.name.clone(),
				mime_type: f.mime_type.clone(),
				size: f.size,
				created_time: f.created_time.clone(),
				modified_time: f.modified_time.clone(),
				web_view_link: f.web_view_link.clone(),
				voice_id: extract_voice_id_from_name(&f.name),
				text_preview: extract_text_preview_from_name(&f.name),
				duration_seconds: None,
			})
			.collect();

		let has_more = total > end;
		let next_offset = if has_more { Some(offset + limit as u32) } else { None };

		Ok(AudioSearchResponse {
			results,
			total_count: total,
			has_more,
			next_offset,
		})
	}
}

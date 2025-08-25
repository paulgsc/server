use super::{AudioConfig, AudioMetadata, AudioSearchResponse, CachedAudio, GetAudioRequest, SearchAudioRequest};
use crate::{AudioServiceError, DedupCache, DedupError, FileHostError, ReadDrive};
use bytes::Bytes;
use garde::Validate;
use sdk::FileMetadata;
use std::sync::Arc;
use tracing::{error, info, instrument, warn};

pub struct AudioService {
	drive_client: Arc<ReadDrive>,
	cache: Arc<DedupCache>,
	supported_mime_types: Vec<String>,
	audio_folder_id: Option<String>,
	max_file_size: u64,
	cache_ttl: Option<u64>,
}

impl AudioService {
	pub fn new(drive_client: Arc<ReadDrive>, cache: Arc<DedupCache>, config: AudioConfig) -> Self {
		Self {
			drive_client,
			cache,
			supported_mime_types: config.supported_mime_types,
			audio_folder_id: config.audio_folder_id,
			max_file_size: config.max_file_size as u64,
			cache_ttl: config.cache_ttl.as_secs().into(),
		}
	}

	#[instrument(skip(self), fields(audio_id = %req.id, force_refresh = req.force_refresh.unwrap_or(false)))]
	pub async fn get_audio(&self, req: GetAudioRequest) -> Result<(CachedAudio, bool), FileHostError> {
		if let Err(e) = req.validate() {
			error!("Audio request validation failed for id {}: {}", req.id, e);
			return Err(
				AudioServiceError::ValidationFailed {
					message: format!("Invalid request: {}", e),
				}
				.into(),
			);
		}

		if let Err(e) = self.validate_file_id(&req.id) {
			error!("Invalid file ID provided: {}", req.id);
			return Err(e.into());
		}

		let cache_key = format!("audio:{}", req.id);

		// Handle force refresh by deleting from cache first
		if req.force_refresh.unwrap_or(false) {
			if let Err(e) = self.cache.delete(&cache_key).await {
				warn!("Failed to delete cache entry for force refresh: {}", e);
			}
		}

		// Use DedupCache's get_or_fetch_with_ttl to handle everything automatically
		let (audio, was_cached) = match self
			.cache
			.get_or_fetch_with_ttl(&cache_key, self.cache_ttl, || async { self.fetch_audio(&req.id).await })
			.await
		{
			Ok(result) => result,
			Err(e) => {
				error!("Failed to get or fetch audio for id {}: {}", req.id, e);
				return Err(e.into());
			}
		};

		if was_cached {
			info!("Cache hit for audio: {}", req.id);
		} else {
			info!("Cache miss for audio: {}", req.id);
		}

		// Return inverted boolean - DedupCache returns true if cached, we want true if fetched
		Ok((audio, !was_cached))
	}

	#[instrument(skip(self), fields(query = ?req.query, voice = ?req.voice, limit = req.limit, offset = req.offset))]
	pub async fn search_audio(&self, req: SearchAudioRequest) -> Result<AudioSearchResponse, AudioServiceError> {
		req.validate().map_err(|e| AudioServiceError::ValidationFailed {
			message: format!("Invalid search request: {}", e),
		})?;

		let limit = req.limit.unwrap_or(20).min(100);
		let offset = req.offset.unwrap_or(0) as usize;

		let query = self.build_search_query(&req)?;

		let files = self
			.drive_client
			.search_files(&query, (limit + 10) as i32)
			.await
			.map_err(|_| AudioServiceError::SearchFailed)?;

		let total = files.len();
		let start = offset.min(total);
		let end = (start + limit as usize).min(total);

		let results: Result<Vec<AudioMetadata>, AudioServiceError> = files[start..end]
			.iter()
			.filter(|f| self.is_supported_audio_type(&f.mime_type))
			.map(|f| {
				Ok(AudioMetadata {
					id: f.id.clone(),
					name: f.name.clone(),
					mime_type: f.mime_type.clone(),
					size: f.size.map(|val| val.try_into()).transpose()?,
					created_time: f.created_time.clone(),
					modified_time: f.modified_time.clone(),
					web_view_link: f.web_view_link.clone(),
					voice_id: extract_voice_id(&f.name),
					text_preview: extract_text_preview(&f.name),
					duration_seconds: None,
				})
			})
			.collect();

		let results: Vec<AudioMetadata> = results?;

		let has_more = total > end;
		let next_offset = if has_more { Some((offset + limit as usize) as u32) } else { None };

		info!("Audio search completed: {} results, has_more: {}", results.len(), has_more);

		Ok(AudioSearchResponse {
			results,
			total_count: total,
			has_more,
			next_offset,
		})
	}

	#[instrument(skip(self), fields(audio_id = %id))]
	async fn fetch_audio(&self, id: &str) -> Result<CachedAudio, DedupError> {
		let metadata = match self.drive_client.get_file_metadata(id).await {
			Ok(metadata) => metadata,
			Err(e) => {
				error!("Failed to fetch metadata for audio file {}: {}", id, e);
				return Err(AudioServiceError::MetadataFetchFailed { id: id.to_string() }.into());
			}
		};

		if let Err(e) = self.validate_audio_metadata(&metadata) {
			error!("Audio metadata validation failed for file {}: {}", id, e);
			return Err(e.into());
		}

		let data = match self.drive_client.download_file(id).await {
			Ok(data) => data,
			Err(e) => {
				error!("Failed to download audio file {}: {}", id, e);
				return Err(AudioServiceError::DownloadFailed { id: id.to_string() }.into());
			}
		};

		let etag = self.generate_etag(&data, &metadata);
		let last_modified = metadata.modified_time;

		info!("Successfully fetched audio: {} ({} bytes)", id, data.len());

		Ok(CachedAudio {
			audio_data: data.clone(),
			content_type: metadata.mime_type,
			etag,
			last_modified,
			size: data.len() as u64,
		})
	}

	fn validate_file_id(&self, id: &str) -> Result<(), AudioServiceError> {
		if id.is_empty() || id.len() > 100 {
			return Err(AudioServiceError::InvalidFileId { id: id.to_string() });
		}

		if id.contains("..") || id.contains('/') || id.contains('\\') {
			return Err(AudioServiceError::InvalidFileId { id: id.to_string() });
		}

		Ok(())
	}

	fn validate_audio_metadata(&self, metadata: &FileMetadata) -> Result<(), AudioServiceError> {
		if !self.is_supported_audio_type(&metadata.mime_type) {
			return Err(AudioServiceError::UnsupportedAudioType {
				mime_type: metadata.mime_type.clone(),
			});
		}

		if let Some(size) = metadata.size {
			let size = size.try_into()?;
			if size > self.max_file_size {
				return Err(AudioServiceError::FileTooLarge { size });
			}
		}

		Ok(())
	}

	fn is_supported_audio_type(&self, mime_type: &str) -> bool {
		self.supported_mime_types.contains(&mime_type.to_lowercase())
	}

	fn build_search_query(&self, req: &SearchAudioRequest) -> Result<String, AudioServiceError> {
		let mut query_parts = Vec::new();

		// Folder constraint
		if let Some(folder_id) = &self.audio_folder_id {
			if folder_id.contains("..") || folder_id.contains('/') {
				return Err(AudioServiceError::InvalidSearchQuery {
					query: Some("Invalid folder ID in config".to_string()),
				});
			}
			query_parts.push(format!("'{}' in parents", folder_id));
		}

		// MIME type filter
		let mime_filter = self.supported_mime_types.iter().map(|mime| format!("mimeType='{}'", mime)).collect::<Vec<_>>().join(" or ");
		query_parts.push(format!("({})", mime_filter));

		// Search terms
		if let Some(q) = &req.query {
			let clean = sanitize_search_term(q);
			if !clean.is_empty() {
				query_parts.push(format!("name contains '{}'", clean));
			}
		}

		if let Some(voice) = &req.voice {
			let clean = sanitize_search_term(voice);
			if !clean.is_empty() {
				query_parts.push(format!("name contains '{}'", clean));
			}
		}

		Ok(query_parts.join(" and "))
	}

	fn generate_etag(&self, data: &Bytes, metadata: &FileMetadata) -> String {
		use std::collections::hash_map::DefaultHasher;
		use std::hash::{Hash, Hasher};

		let mut hasher = DefaultHasher::new();
		data.len().hash(&mut hasher);
		metadata.modified_time.hash(&mut hasher);
		format!("\"{}\"", hasher.finish())
	}
}

// Helper functions
fn sanitize_search_term(s: &str) -> String {
	s.trim()
		.chars()
		.filter(|c| c.is_alphanumeric() || c.is_whitespace() || *c == '-' || *c == '_')
		.collect::<String>()
		.replace('\'', "\\'")
}

fn extract_voice_id(name: &str) -> Option<String> {
	name.split('_').nth(1).map(|s| s.to_string())
}

fn extract_text_preview(name: &str) -> Option<String> {
	let stem = name.split('.').next().unwrap_or("");
	let parts: Vec<&str> = stem.split('_').collect();
	if parts.len() > 2 {
		Some(parts[2..].join(" "))
	} else {
		None
	}
}

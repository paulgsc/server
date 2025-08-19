use bytes::Bytes;
use garde::Validate;
use serde::{Deserialize, Serialize};
use std::time::{Duration, SystemTime};

#[derive(Debug, Clone, Deserialize, Validate)]
pub struct GetAudioRequest {
	#[garde(length(min = 1, max = 100))]
	pub id: String,

	#[garde(skip)]
	#[serde(default)]
	pub force_refresh: bool,

	#[garde(skip)]
	pub range_request: Option<RangeRequest>,
}

#[derive(Debug, Clone, Deserialize, Validate)]
pub struct SearchAudioRequest {
	#[garde(length(max = 200,))]
	pub query: Option<String>,

	#[garde(length(max = 50,))]
	pub voice: Option<String>,

	#[garde(range(min = 1, max = 100,))]
	pub limit: Option<u32>,

	#[garde(range(min = 0,))]
	pub offset: Option<u32>,
}

#[derive(Clone)]
pub struct CachedAudio {
	pub data: Bytes,
	pub content_type: String,
	pub etag: String,
	pub last_modified: SystemTime,
	pub size: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RangeRequest {
	pub start: Option<u64>,
	pub end: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AudioSearchParams {
	pub q: Option<String>,
	pub voice: Option<String>,
	pub limit: Option<u32>,
	pub offset: Option<u32>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AudioMetadata {
	pub id: String,
	pub name: String,
	pub mime_type: String,
	pub size: Option<i64>,
	pub web_view_link: Option<String>,
	pub voice_id: Option<String>,
	pub text_preview: Option<String>,
	pub duration_seconds: Option<f64>,
}

#[derive(Debug, Serialize)]
pub struct AudioSearchResponse {
	pub results: Vec<AudioMetadata>,
	pub total_count: usize,
	pub has_more: bool,
	pub next_offset: Option<u32>,
}

#[derive(Debug, Serialize)]
pub struct ErrorResponse {
	pub error: String,
	pub code: String,
	pub details: Option<serde_json::Value>,
}

// ============================================================================
// Application State
// ============================================================================

#[derive(Debug, Clone)]
pub struct AudioConfig {
	pub max_file_size: i64,
	pub cache_ttl: Duration,
	pub supported_mime_types: Vec<String>,
	pub audio_folder_id: Option<String>,
	pub enable_cache: bool,
	pub max_cache_entries: usize,
}

impl Default for AudioConfig {
	fn default() -> Self {
		Self {
			max_file_size: 50 * 1024 * 1024,         // 50MB
			cache_ttl: Duration::from_secs(30 * 60), // 30 minutes
			supported_mime_types: vec![
				"audio/mpeg".to_string(),
				"audio/wav".to_string(),
				"audio/ogg".to_string(),
				"audio/mp4".to_string(),
				"audio/aac".to_string(),
				"audio/webm".to_string(),
			],
			audio_folder_id: None,
			enable_cache: true,
			max_cache_entries: 1000,
		}
	}
}

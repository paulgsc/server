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
	pub created_time: Option<chrono::DateTime<chrono::Utc>>,
	pub modified_time: Option<chrono::DateTime<chrono::Utc>>,
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

#[derive(Debug, Clone)]
pub struct CacheEntry {
	pub data: Bytes,
	pub content_type: String,
	pub created_at: SystemTime,
	pub ttl: Duration,
}

impl CacheEntry {
	pub fn is_expired(&self) -> bool {
		SystemTime::now().duration_since(self.created_at).unwrap_or(Duration::from_secs(0)) > self.ttl
	}
}

// ============================================================================
// Application State
// ============================================================================

#[derive(Clone)]
pub struct AppState {
	pub drive_client: Arc<ReadDrive>,
	pub cache: Arc<RwLock<HashMap<String, CacheEntry>>>,
	pub config: AudioConfig,
}

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

// Audio-specific helper methods
impl CacheStore {
	pub async fn cache_audio(&self, id: &str, data: Bytes, content_type: String, ttl: Option<u64>) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
		self.set_binary(&format!("audio:{}", id), &data, Some(content_type), ttl).await
	}

	pub async fn get_cached_audio(&self, id: &str) -> Result<Option<(Bytes, String)>, Box<dyn std::error::Error + Send + Sync>> {
		match self.get_binary(&format!("audio:{}", id)).await? {
			Some((data, Some(content_type))) => Ok(Some((Bytes::from(data), content_type))),
			Some((data, None)) => Ok(Some((Bytes::from(data), "audio/mpeg".to_string()))), // Default
			None => Ok(None),
		}
	}
}

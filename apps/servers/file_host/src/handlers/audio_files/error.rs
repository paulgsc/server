use thiserror::Error;

#[allow(dead_code)]
#[derive(Debug, Error)]
pub enum AudioServiceError {
	#[error("integer conversion error: {0}")]
	IntConversion(#[from] std::num::TryFromIntError),

	#[error("Invalid audio file ID: {id}")]
	InvalidFileId { id: String },

	#[error("Unsupported audio type: {mime_type}")]
	UnsupportedAudioType { mime_type: String },

	#[error("Validation failed: {message}")]
	ValidationFailed { message: String },

	#[error("Invalid search query: {query:?}")]
	InvalidSearchQuery { query: Option<String> },

	#[error("Audio download failed for ID: {id}")]
	DownloadFailed { id: String },

	#[error("Metadata retrieval failed for ID: {id}")]
	MetadataFetchFailed { id: String },

	#[error("Search failed")]
	SearchFailed,

	#[error("File too large: {size} bytes")]
	FileTooLarge { size: u64 },
}

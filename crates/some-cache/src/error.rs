use prometheus::Error as PrometheusError;
use thiserror::Error;

/// Low-level Redis/compression/serde errors for `CacheStore`.
#[derive(Debug, Error)]
pub enum CacheError {
	#[error("Failed to register Prometheus metric")]
	MetricRegistrationError(#[from] PrometheusError),

	#[error("Redis connection failed")]
	RedisConnectionError(#[from] redis::RedisError),

	#[error("Compression failed")]
	Compression(std::io::Error),

	#[error("Decompression failed")]
	Decompression(std::io::Error),

	/// Leading flag byte is not 0x00 or 0x01 — payload is corrupt or
	/// written by an incompatible version of this code.
	#[error("Leading flag byte is not 0x00 or 0x01: {0}")]
	InvalidEncoding(u8),

	#[error("Serialization/deserialization failed: {0}")]
	SerializationError(#[from] serde_json::Error),

	#[error("System time error")]
	SystemTimeError(#[from] std::time::SystemTimeError),

	#[error("Integer conversion error")]
	TryFromIntError(#[from] std::num::TryFromIntError),

	#[error("Cache operation failed: {0}")]
	OperationError(String),

	#[error(transparent)]
	Generic(#[from] anyhow::Error),
}

/// Dedup-layer errors surfaced by `DedupCache`.
///
/// This is the **boundary type** exported to downstream services.
#[derive(Debug, Error)]
pub enum DedupCacheError {
	/// Key not found in persistent store
	#[error("not found")]
	NotFound,

	/// Serialization/deserialization failure
	#[error("serialization error")]
	SerializationError(#[from] serde_json::Error),

	/// Underlying cache/store failure
	#[error("cache store error")]
	StoreError(#[from] CacheError),

	/// Logical misuse (e.g. invalid state transitions)
	#[error("operation error: {0}")]
	OperationError(String),

	/// Type mismatch (generic vs binary etc.)
	#[error("type mismatch: {0}")]
	TypeMismatch(String),
}

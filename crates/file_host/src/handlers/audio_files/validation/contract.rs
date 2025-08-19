use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::marker::PhantomData;

/// Compile-time known audio file states
#[derive(Debug, Clone, PartialEq)]
pub enum AudioFileState {
	/// Raw bytes received, no validation
	Unvalidated,
	/// Identity established (hash, sanitization complete)
	IdentityVerified,
	/// Structure verified (parseable audio, codec known)  
	StructurallyValid,
	/// Resource bounds verified (size, duration, metadata limits)
	ResourceBounded,
	/// Security validated (codec whitelisted, no polyglots)
	SecurityCleared,
	/// Ready for storage/processing
	Validated,
	/// Validation failed - terminal state
	Rejected,
}

/// Transition evidence required for each state change
#[derive(Debug, Clone)]
pub struct TransitionEvidence {
	/// Identity proof
	pub file_hash: Option<String>,
	pub sanitized_id: Option<String>,

	/// Structural proof
	pub codec_info: Option<CodecInfo>,
	pub is_parseable: Option<bool>,

	/// Resource proof
	pub size_bytes: Option<u64>,
	pub duration_seconds: Option<f64>,
	pub metadata_size_bytes: Option<u64>,

	/// Security proof
	pub codec_whitelisted: Option<bool>,
	pub polyglot_free: Option<bool>,

	/// Audit trail
	pub violations: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodecInfo {
	pub codec_name: String,
	pub sample_rate: u32,
	pub channels: u32,
	pub bitrate: u32,
}

/// Type-safe audio file at specific validation state
#[derive(Debug)]
pub struct AudioFile<S> {
	pub id: String,
	pub source: AudioSource,
	pub evidence: TransitionEvidence,
	_state: PhantomData<S>,
}

/// Audio sources - abstracted from raw data handling
#[derive(Debug, Clone)]
pub enum AudioSource {
	HttpPost { url: String, headers: Vec<(String, String)> },
	HttpFetch { url: String },
	LocalFile { path: String },
	ByteStream { metadata: SourceMetadata },
}

#[derive(Debug, Clone)]
pub struct SourceMetadata {
	pub content_length: Option<u64>,
	pub content_type: Option<String>,
	pub last_modified: Option<String>,
}

/// State type markers
pub struct Unvalidated;
pub struct IdentityVerified;
pub struct StructurallyValid;
pub struct ResourceBounded;
pub struct SecurityCleared;
pub struct Validated;
pub struct Rejected;

/// Validation configuration - compile-time constraints
#[derive(Debug, Clone)]
pub struct ValidationConstraints {
	pub max_file_size: u64,
	pub max_duration_seconds: f64,
	pub max_metadata_size: u64,
	pub allowed_codecs: HashSet<String>,
	pub require_unique_hash: bool,
}

impl Default for ValidationConstraints {
	fn default() -> Self {
		let mut allowed_codecs = HashSet::new();
		allowed_codecs.extend(["mp3", "wav", "flac", "ogg", "aac"].iter().map(|s| s.to_string()));

		Self {
			max_file_size: 50 * 1024 * 1024,
			max_duration_seconds: 600.0,
			max_metadata_size: 1024 * 1024,
			allowed_codecs,
			require_unique_hash: true,
		}
	}
}

/// FSM-based audio validator - enforces valid state transitions
pub struct AudioValidator {
	constraints: ValidationConstraints,
	identity_service: Box<dyn IdentityService>,
	structure_service: Box<dyn StructureService>,
	security_service: Box<dyn SecurityService>,
}

/// External services abstracted away from core validation logic
pub trait IdentityService {
	fn generate_hash(&self, source: &AudioSource) -> Result<String, ValidationError>;
	fn sanitize_id(&self, id: &str) -> Result<String, ValidationError>;
}

pub trait StructureService {
	fn analyze_structure(&self, source: &AudioSource) -> Result<CodecInfo, ValidationError>;
	fn verify_parseable(&self, source: &AudioSource) -> Result<bool, ValidationError>;
}

pub trait SecurityService {
	fn check_polyglot(&self, source: &AudioSource) -> Result<bool, ValidationError>;
	fn verify_codec_safety(&self, codec: &str, whitelist: &HashSet<String>) -> Result<bool, ValidationError>;
}

#[derive(Debug, thiserror::Error)]
pub enum ValidationError {
	#[error("Identity validation failed: {0}")]
	IdentityFailed(String),
	#[error("Structural validation failed: {0}")]
	StructuralFailed(String),
	#[error("Resource bounds exceeded: {0}")]
	ResourceBoundsExceeded(String),
	#[error("Security validation failed: {0}")]
	SecurityFailed(String),
	#[error("Invalid state transition: {current:?} -> {target:?}")]
	InvalidTransition { current: AudioFileState, target: AudioFileState },
	#[error("External service error: {0}")]
	ServiceError(String),
}

impl AudioValidator {
	pub fn new(
		constraints: ValidationConstraints,
		identity_service: Box<dyn IdentityService>,
		structure_service: Box<dyn StructureService>,
		security_service: Box<dyn SecurityService>,
	) -> Self {
		Self {
			constraints,
			identity_service,
			structure_service,
			security_service,
		}
	}

	/// State transition: Unvalidated -> IdentityVerified
	pub fn verify_identity(&self, file: AudioFile<Unvalidated>) -> Result<AudioFile<IdentityVerified>, (AudioFile<Rejected>, ValidationError)> {
		let mut evidence = file.evidence;

		// Generate cryptographic hash
		match self.identity_service.generate_hash(&file.source) {
			Ok(hash) => evidence.file_hash = Some(hash),
			Err(e) => {
				evidence.violations.push(format!("Hash generation failed: {}", e));
				return Err((
					AudioFile {
						id: file.id,
						source: file.source,
						evidence,
						_state: PhantomData::<Rejected>,
					},
					ValidationError::IdentityFailed(e.to_string()),
				));
			}
		}

		// Sanitize identifier
		match self.identity_service.sanitize_id(&file.id) {
			Ok(sanitized) => evidence.sanitized_id = Some(sanitized),
			Err(e) => {
				evidence.violations.push(format!("ID sanitization failed: {}", e));
				return Err((
					AudioFile {
						id: file.id,
						source: file.source,
						evidence,
						_state: PhantomData::<Rejected>,
					},
					ValidationError::IdentityFailed(e.to_string()),
				));
			}
		}

		Ok(AudioFile {
			id: file.id,
			source: file.source,
			evidence,
			_state: PhantomData::<IdentityVerified>,
		})
	}

	/// State transition: IdentityVerified -> StructurallyValid  
	pub fn verify_structure(&self, file: AudioFile<IdentityVerified>) -> Result<AudioFile<StructurallyValid>, (AudioFile<Rejected>, ValidationError)> {
		let mut evidence = file.evidence;

		// Verify file is parseable audio
		match self.structure_service.verify_parseable(&file.source) {
			Ok(is_parseable) => {
				evidence.is_parseable = Some(is_parseable);
				if !is_parseable {
					evidence.violations.push("File is not parseable audio".to_string());
					return Err((
						AudioFile {
							id: file.id,
							source: file.source,
							evidence,
							_state: PhantomData::<Rejected>,
						},
						ValidationError::StructuralFailed("Not parseable audio".to_string()),
					));
				}
			}
			Err(e) => {
				evidence.violations.push(format!("Parseability check failed: {}", e));
				return Err((
					AudioFile {
						id: file.id,
						source: file.source,
						evidence,
						_state: PhantomData::<Rejected>,
					},
					ValidationError::StructuralFailed(e.to_string()),
				));
			}
		}

		// Analyze codec structure
		match self.structure_service.analyze_structure(&file.source) {
			Ok(codec_info) => evidence.codec_info = Some(codec_info),
			Err(e) => {
				evidence.violations.push(format!("Codec analysis failed: {}", e));
				return Err((
					AudioFile {
						id: file.id,
						source: file.source,
						evidence,
						_state: PhantomData::<Rejected>,
					},
					ValidationError::StructuralFailed(e.to_string()),
				));
			}
		}

		Ok(AudioFile {
			id: file.id,
			source: file.source,
			evidence,
			_state: PhantomData::<StructurallyValid>,
		})
	}

	/// State transition: StructurallyValid -> ResourceBounded
	pub fn verify_resource_bounds(&self, file: AudioFile<StructurallyValid>) -> Result<AudioFile<ResourceBounded>, (AudioFile<Rejected>, ValidationError)> {
		let mut evidence = file.evidence;
		let mut violations = Vec::new();

		// Check size bounds
		if let Some(size) = self.get_source_size(&file.source) {
			evidence.size_bytes = Some(size);
			if size > self.constraints.max_file_size {
				violations.push(format!("File too large: {} > {} bytes", size, self.constraints.max_file_size));
			}
		}

		// Check duration bounds
		if let Some(codec_info) = &evidence.codec_info {
			// Duration would be extracted from codec analysis
			let estimated_duration = self.estimate_duration(&file.source, codec_info);
			evidence.duration_seconds = Some(estimated_duration);

			if estimated_duration > self.constraints.max_duration_seconds {
				violations.push(format!("Audio too long: {:.2}s > {:.2}s", estimated_duration, self.constraints.max_duration_seconds));
			}
		}

		// Check metadata size bounds
		let metadata_size = self.estimate_metadata_size(&file.source);
		evidence.metadata_size_bytes = Some(metadata_size);
		if metadata_size > self.constraints.max_metadata_size {
			violations.push(format!("Metadata too large: {} > {} bytes", metadata_size, self.constraints.max_metadata_size));
		}

		if !violations.is_empty() {
			evidence.violations.extend(violations.clone());
			return Err((
				AudioFile {
					id: file.id,
					source: file.source,
					evidence,
					_state: PhantomData::<Rejected>,
				},
				ValidationError::ResourceBoundsExceeded(violations.join("; ")),
			));
		}

		Ok(AudioFile {
			id: file.id,
			source: file.source,
			evidence,
			_state: PhantomData::<ResourceBounded>,
		})
	}

	/// State transition: ResourceBounded -> SecurityCleared
	pub fn verify_security(&self, file: AudioFile<ResourceBounded>) -> Result<AudioFile<SecurityCleared>, (AudioFile<Rejected>, ValidationError)> {
		let mut evidence = file.evidence;
		let mut violations = Vec::new();

		// Check codec whitelist
		if let Some(codec_info) = &evidence.codec_info {
			match self.security_service.verify_codec_safety(&codec_info.codec_name, &self.constraints.allowed_codecs) {
				Ok(is_safe) => {
					evidence.codec_whitelisted = Some(is_safe);
					if !is_safe {
						violations.push(format!("Codec '{}' not in whitelist", codec_info.codec_name));
					}
				}
				Err(e) => violations.push(format!("Codec safety check failed: {}", e)),
			}
		}

		// Check for polyglot attacks
		match self.security_service.check_polyglot(&file.source) {
			Ok(is_polyglot_free) => {
				evidence.polyglot_free = Some(is_polyglot_free);
				if !is_polyglot_free {
					violations.push("Polyglot file detected".to_string());
				}
			}
			Err(e) => violations.push(format!("Polyglot check failed: {}", e)),
		}

		if !violations.is_empty() {
			evidence.violations.extend(violations.clone());
			return Err((
				AudioFile {
					id: file.id,
					source: file.source,
					evidence,
					_state: PhantomData::<Rejected>,
				},
				ValidationError::SecurityFailed(violations.join("; ")),
			));
		}

		Ok(AudioFile {
			id: file.id,
			source: file.source,
			evidence,
			_state: PhantomData::<SecurityCleared>,
		})
	}

	/// State transition: SecurityCleared -> Validated (final)
	pub fn finalize_validation(&self, file: AudioFile<SecurityCleared>) -> AudioFile<Validated> {
		AudioFile {
			id: file.id,
			source: file.source,
			evidence: file.evidence,
			_state: PhantomData::<Validated>,
		}
	}

	/// Complete validation pipeline - all transitions in sequence
	pub fn validate_complete(&self, file: AudioFile<Unvalidated>) -> Result<AudioFile<Validated>, (AudioFile<Rejected>, ValidationError)> {
		let identity_verified = self.verify_identity(file)?;
		let structurally_valid = self.verify_structure(identity_verified)?;
		let resource_bounded = self.verify_resource_bounds(structurally_valid)?;
		let security_cleared = self.verify_security(resource_bounded)?;

		Ok(self.finalize_validation(security_cleared))
	}

	// Helper methods - these would delegate to external services
	fn get_source_size(&self, source: &AudioSource) -> Option<u64> {
		match source {
			AudioSource::ByteStream { metadata } => metadata.content_length,
			AudioSource::HttpPost { .. } => None,  // Would fetch from headers
			AudioSource::HttpFetch { .. } => None, // Would fetch from HEAD request
			AudioSource::LocalFile { path } => std::fs::metadata(path).ok().map(|m| m.len()),
		}
	}

	fn estimate_duration(&self, _source: &AudioSource, _codec_info: &CodecInfo) -> f64 {
		// This would delegate to structure service for actual calculation
		0.0 // Placeholder
	}

	fn estimate_metadata_size(&self, _source: &AudioSource) -> u64 {
		// This would delegate to structure service for actual calculation
		0 // Placeholder
	}
}

/// Factory for creating unvalidated audio files from various sources
impl AudioFile<Unvalidated> {
	pub fn from_http_post(id: String, url: String, headers: Vec<(String, String)>) -> Self {
		Self {
			id,
			source: AudioSource::HttpPost { url, headers },
			evidence: TransitionEvidence::default(),
			_state: PhantomData,
		}
	}

	pub fn from_http_fetch(id: String, url: String) -> Self {
		Self {
			id,
			source: AudioSource::HttpFetch { url },
			evidence: TransitionEvidence::default(),
			_state: PhantomData,
		}
	}

	pub fn from_local_file(id: String, path: String) -> Self {
		Self {
			id,
			source: AudioSource::LocalFile { path },
			evidence: TransitionEvidence::default(),
			_state: PhantomData,
		}
	}

	pub fn from_byte_stream(id: String, metadata: SourceMetadata) -> Self {
		Self {
			id,
			source: AudioSource::ByteStream { metadata },
			evidence: TransitionEvidence::default(),
			_state: PhantomData,
		}
	}
}

impl Default for TransitionEvidence {
	fn default() -> Self {
		Self {
			file_hash: None,
			sanitized_id: None,
			codec_info: None,
			is_parseable: None,
			size_bytes: None,
			duration_seconds: None,
			metadata_size_bytes: None,
			codec_whitelisted: None,
			polyglot_free: None,
			violations: Vec::new(),
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	// Mock implementations for testing
	struct MockIdentityService;
	struct MockStructureService;
	struct MockSecurityService;

	impl IdentityService for MockIdentityService {
		fn generate_hash(&self, _source: &AudioSource) -> Result<String, ValidationError> {
			Ok("mock_hash_123".to_string())
		}

		fn sanitize_id(&self, id: &str) -> Result<String, ValidationError> {
			if id.contains("..") {
				Err(ValidationError::IdentityFailed("Invalid ID".to_string()))
			} else {
				Ok(id.to_string())
			}
		}
	}

	impl StructureService for MockStructureService {
		fn analyze_structure(&self, _source: &AudioSource) -> Result<CodecInfo, ValidationError> {
			Ok(CodecInfo {
				codec_name: "mp3".to_string(),
				sample_rate: 44100,
				channels: 2,
				bitrate: 128000,
			})
		}

		fn verify_parseable(&self, _source: &AudioSource) -> Result<bool, ValidationError> {
			Ok(true)
		}
	}

	impl SecurityService for MockSecurityService {
		fn check_polyglot(&self, _source: &AudioSource) -> Result<bool, ValidationError> {
			Ok(true) // polyglot-free
		}

		fn verify_codec_safety(&self, codec: &str, whitelist: &HashSet<String>) -> Result<bool, ValidationError> {
			Ok(whitelist.contains(codec))
		}
	}

	#[test]
	fn test_complete_validation_pipeline() {
		let validator = AudioValidator::new(
			ValidationConstraints::default(),
			Box::new(MockIdentityService),
			Box::new(MockStructureService),
			Box::new(MockSecurityService),
		);

		let unvalidated_file = AudioFile::from_http_post("test_audio".to_string(), "https://example.com/audio.mp3".to_string(), vec![]);

		let result = validator.validate_complete(unvalidated_file);
		assert!(result.is_ok());

		let validated_file = result.unwrap();
		assert!(validated_file.evidence.file_hash.is_some());
		assert!(validated_file.evidence.codec_info.is_some());
	}

	#[test]
	fn test_invalid_transition_caught() {
		let validator = AudioValidator::new(
			ValidationConstraints::default(),
			Box::new(MockIdentityService),
			Box::new(MockStructureService),
			Box::new(MockSecurityService),
		);

		let bad_file = AudioFile::from_http_post("../../../etc/passwd".to_string(), "https://example.com/bad.mp3".to_string(), vec![]);

		let result = validator.verify_identity(bad_file);
		assert!(result.is_err());

		let (rejected_file, error) = result.unwrap_err();
		assert!(matches!(error, ValidationError::IdentityFailed(_)));
		assert!(!rejected_file.evidence.violations.is_empty());
	}
}

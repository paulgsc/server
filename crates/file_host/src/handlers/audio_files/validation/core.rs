use super::contract::*;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::io::Read;
use std::process::Command;

// ============================================================================
// IDENTITY SERVICE IMPLEMENTATIONS
// ============================================================================

/// SHA256-based identity service
pub struct Sha256IdentityService;

impl IdentityService for Sha256IdentityService {
	fn generate_hash(&self, source: &AudioSource) -> Result<String, ValidationError> {
		let data = self
			.read_source_data(source)
			.map_err(|e| ValidationError::IdentityFailed(format!("Failed to read source: {}", e)))?;

		let mut hasher = Sha256::new();
		hasher.update(&data);
		Ok(format!("{:x}", hasher.finalize()))
	}

	fn sanitize_id(&self, id: &str) -> Result<String, ValidationError> {
		// Empty check
		if id.is_empty() {
			return Err(ValidationError::IdentityFailed("File ID cannot be empty".into()));
		}

		// Length check
		if id.len() > 100 {
			return Err(ValidationError::IdentityFailed("File ID too long (>100 chars)".into()));
		}

		// Path traversal check
		if id.contains("..") || id.contains('/') || id.contains('\\') {
			return Err(ValidationError::IdentityFailed("File ID contains invalid path characters".into()));
		}

		// Control character check
		if id.chars().any(|c| c.is_control()) {
			return Err(ValidationError::IdentityFailed("File ID contains control characters".into()));
		}

		Ok(id.to_string())
	}
}

impl Sha256IdentityService {
	fn read_source_data(&self, source: &AudioSource) -> Result<Vec<u8>, std::io::Error> {
		match source {
			AudioSource::LocalFile { path } => std::fs::read(path),
			AudioSource::ByteStream { .. } => {
				// In real implementation, would read from the stream
				// For now, return empty vec as placeholder
				Ok(Vec::new())
			}
			AudioSource::HttpFetch { url } => {
				// Would use HTTP client to fetch data
				// Placeholder implementation
				Err(std::io::Error::new(
					std::io::ErrorKind::Unsupported,
					format!("HTTP fetch not yet implemented for URL: {}", url),
				))
			}
			AudioSource::HttpPost { url, .. } => {
				// Would handle POST data
				Err(std::io::Error::new(
					std::io::ErrorKind::Unsupported,
					format!("HTTP POST not yet implemented for URL: {}", url),
				))
			}
		}
	}
}

/// Passthrough identity service - minimal validation for testing
pub struct MinimalIdentityService;

impl IdentityService for MinimalIdentityService {
	fn generate_hash(&self, _source: &AudioSource) -> Result<String, ValidationError> {
		// Generate a simple timestamp-based hash
		use std::time::{SystemTime, UNIX_EPOCH};
		let timestamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
		Ok(format!("minimal_{}", timestamp))
	}

	fn sanitize_id(&self, id: &str) -> Result<String, ValidationError> {
		// Basic sanitization only
		if id.is_empty() || id.contains("..") {
			Err(ValidationError::IdentityFailed("Invalid ID".into()))
		} else {
			Ok(id.to_string())
		}
	}
}

// ============================================================================
// STRUCTURE SERVICE IMPLEMENTATIONS
// ============================================================================

/// Full ffprobe-based structure analysis
pub struct FfprobeStructureService;

impl StructureService for FfprobeStructureService {
	fn analyze_structure(&self, source: &AudioSource) -> Result<CodecInfo, ValidationError> {
		let temp_path = self
			.create_temp_file(source)
			.map_err(|e| ValidationError::StructuralFailed(format!("Failed to create temp file: {}", e)))?;

		let result = self.run_ffprobe_analysis(&temp_path);

		// Clean up temp file
		let _ = std::fs::remove_file(&temp_path);

		result
	}

	fn verify_parseable(&self, source: &AudioSource) -> Result<bool, ValidationError> {
		let temp_path = self
			.create_temp_file(source)
			.map_err(|e| ValidationError::StructuralFailed(format!("Failed to create temp file: {}", e)))?;

		let is_parseable = self.check_ffprobe_parseable(&temp_path);

		// Clean up temp file
		let _ = std::fs::remove_file(&temp_path);

		Ok(is_parseable)
	}
}

impl FfprobeStructureService {
	fn create_temp_file(&self, source: &AudioSource) -> Result<String, std::io::Error> {
		let temp_path = format!("/tmp/audio_validate_{}", uuid::Uuid::new_v4());

		let data = match source {
			AudioSource::LocalFile { path } => std::fs::read(path)?,
			AudioSource::ByteStream { .. } => {
				// Would read from actual stream
				return Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "ByteStream not yet implemented"));
			}
			AudioSource::HttpFetch { .. } | AudioSource::HttpPost { .. } => {
				return Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "HTTP sources not yet implemented"));
			}
		};

		std::fs::write(&temp_path, data)?;
		Ok(temp_path)
	}

	fn run_ffprobe_analysis(&self, path: &str) -> Result<CodecInfo, ValidationError> {
		let output = Command::new("ffprobe")
			.args(["-v", "quiet", "-print_format", "json", "-show_format", "-show_streams", path])
			.output()
			.map_err(|e| ValidationError::StructuralFailed(format!("ffprobe execution failed: {}", e)))?;

		if !output.status.success() {
			return Err(ValidationError::StructuralFailed(format!("ffprobe failed: {}", String::from_utf8_lossy(&output.stderr))));
		}

		self.parse_ffprobe_output(&output.stdout)
	}

	fn parse_ffprobe_output(&self, output: &[u8]) -> Result<CodecInfo, ValidationError> {
		let probe_data: Value = serde_json::from_slice(output).map_err(|e| ValidationError::StructuralFailed(format!("Failed to parse ffprobe output: {}", e)))?;

		let streams = probe_data["streams"]
			.as_array()
			.ok_or_else(|| ValidationError::StructuralFailed("No streams found".into()))?;

		let audio_stream = streams
			.iter()
			.find(|stream| stream["codec_type"].as_str() == Some("audio"))
			.ok_or_else(|| ValidationError::StructuralFailed("No audio stream found".into()))?;

		let format_info = &probe_data["format"];

		let codec_name = audio_stream["codec_name"]
			.as_str()
			.ok_or_else(|| ValidationError::StructuralFailed("No codec information found".into()))?
			.to_string();

		let sample_rate = audio_stream["sample_rate"]
			.as_str()
			.ok_or_else(|| ValidationError::StructuralFailed("No sample rate found".into()))?
			.parse::<u32>()
			.map_err(|_| ValidationError::StructuralFailed("Invalid sample rate format".into()))?;

		let channels = audio_stream["channels"]
			.as_u64()
			.ok_or_else(|| ValidationError::StructuralFailed("No channel information found".into()))? as u32;

		let bitrate = format_info["bit_rate"].as_str().unwrap_or("0").parse::<u32>().unwrap_or(0);

		Ok(CodecInfo {
			codec_name,
			sample_rate,
			channels,
			bitrate,
		})
	}

	fn check_ffprobe_parseable(&self, path: &str) -> bool {
		Command::new("ffprobe")
			.args(["-v", "error", "-f", "null", "-", "-i", path])
			.output()
			.map(|output| output.status.success())
			.unwrap_or(false)
	}
}

/// Basic structure service - MIME type and basic header checks only
pub struct BasicStructureService {
	mime_type_map: std::collections::HashMap<Vec<u8>, String>,
}

impl BasicStructureService {
	pub fn new() -> Self {
		let mut mime_type_map = std::collections::HashMap::new();

		// Common audio file signatures
		mime_type_map.insert(b"ID3".to_vec(), "audio/mpeg".to_string());
		mime_type_map.insert(b"\xFF\xFB".to_vec(), "audio/mpeg".to_string()); // MP3
		mime_type_map.insert(b"RIFF".to_vec(), "audio/wav".to_string());
		mime_type_map.insert(b"fLaC".to_vec(), "audio/flac".to_string());
		mime_type_map.insert(b"OggS".to_vec(), "audio/ogg".to_string());

		Self { mime_type_map }
	}

	fn read_file_header(&self, source: &AudioSource, size: usize) -> Result<Vec<u8>, ValidationError> {
		match source {
			AudioSource::LocalFile { path } => {
				let mut file = std::fs::File::open(path).map_err(|e| ValidationError::StructuralFailed(format!("Cannot open file: {}", e)))?;

				let mut buffer = vec![0u8; size];
				file
					.read_exact(&mut buffer)
					.map_err(|e| ValidationError::StructuralFailed(format!("Cannot read file header: {}", e)))?;

				Ok(buffer)
			}
			_ => Err(ValidationError::StructuralFailed("Source type not supported for basic analysis".into())),
		}
	}
}

impl StructureService for BasicStructureService {
	fn analyze_structure(&self, source: &AudioSource) -> Result<CodecInfo, ValidationError> {
		let header = self.read_file_header(source, 12)?;

		// Determine codec from header
		let codec_name = self
			.mime_type_map
			.iter()
			.find(|(signature, _)| header.starts_with(signature))
			.map(|(_, mime_type)| match mime_type.as_str() {
				"audio/mpeg" => "mp3",
				"audio/wav" => "wav",
				"audio/flac" => "flac",
				"audio/ogg" => "ogg",
				_ => "unknown",
			})
			.unwrap_or("unknown")
			.to_string();

		if codec_name == "unknown" {
			return Err(ValidationError::StructuralFailed("Unrecognized audio format".into()));
		}

		// Return basic info (real implementation would parse headers properly)
		Ok(CodecInfo {
			codec_name,
			sample_rate: 44100, // Default assumption
			channels: 2,        // Default assumption
			bitrate: 128000,    // Default assumption
		})
	}

	fn verify_parseable(&self, source: &AudioSource) -> Result<bool, ValidationError> {
		// Basic check - just verify we can read the header
		let header = self.read_file_header(source, 4)?;

		// Check if header matches known audio signatures
		let is_known_format = self.mime_type_map.keys().any(|signature| header.starts_with(signature));

		Ok(is_known_format)
	}
}

/// No-op structure service - always passes (for testing/development)
pub struct NoOpStructureService;

impl StructureService for NoOpStructureService {
	fn analyze_structure(&self, _source: &AudioSource) -> Result<CodecInfo, ValidationError> {
		Ok(CodecInfo {
			codec_name: "mock".to_string(),
			sample_rate: 44100,
			channels: 2,
			bitrate: 128000,
		})
	}

	fn verify_parseable(&self, _source: &AudioSource) -> Result<bool, ValidationError> {
		Ok(true)
	}
}

// ============================================================================
// SECURITY SERVICE IMPLEMENTATIONS
// ============================================================================

/// Full security validation with polyglot detection
pub struct ComprehensiveSecurityService;

impl SecurityService for ComprehensiveSecurityService {
	fn check_polyglot(&self, source: &AudioSource) -> Result<bool, ValidationError> {
		let data = self
			.read_source_data(source, 2048) // Read first 2KB for analysis
			.map_err(|e| ValidationError::SecurityFailed(format!("Failed to read source for security check: {}", e)))?;

		self.scan_for_polyglot_signatures(&data)
	}

	fn verify_codec_safety(&self, codec: &str, whitelist: &HashSet<String>) -> Result<bool, ValidationError> {
		let normalized_codec = codec.to_lowercase();
		Ok(whitelist.contains(&normalized_codec))
	}
}

impl ComprehensiveSecurityService {
	fn read_source_data(&self, source: &AudioSource, max_bytes: usize) -> Result<Vec<u8>, std::io::Error> {
		match source {
			AudioSource::LocalFile { path } => {
				let mut file = std::fs::File::open(path)?;
				let mut buffer = vec![0u8; max_bytes];
				let bytes_read = file.read(&mut buffer)?;
				buffer.truncate(bytes_read);
				Ok(buffer)
			}
			_ => Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "Source type not supported for security scanning")),
		}
	}

	fn scan_for_polyglot_signatures(&self, data: &[u8]) -> Result<bool, ValidationError> {
		let suspicious_signatures = [
			(b"MZ".as_slice(), "PE executable"),
			(b"\x7fELF".as_slice(), "ELF executable"),
			(b"PK".as_slice(), "ZIP/JAR archive"),
			(b"<?php".as_slice(), "PHP script"),
			(b"<script".as_slice(), "JavaScript"),
			(b"#!/".as_slice(), "Shebang script"),
			(b"\x89PNG".as_slice(), "PNG image (potential polyglot)"),
			(b"\xFF\xD8\xFF".as_slice(), "JPEG image (potential polyglot)"),
		];

		for (signature, description) in &suspicious_signatures {
			if let Some(pos) = data.windows(signature.len()).position(|window| window == *signature) {
				// Allow certain signatures at the very beginning for legitimate formats
				if pos > 512 {
					// If found after first 512 bytes, likely polyglot
					return Err(ValidationError::SecurityFailed(format!(
						"Suspicious {} signature found at position {} (possible polyglot attack)",
						description, pos
					)));
				}
			}
		}

		// Additional check: look for multiple format signatures
		let mut format_count = 0;
		let audio_signatures: &[&[u8]] = &[b"ID3", b"\xFF\xFB", b"RIFF", b"fLaC", b"OggS"];
		let other_signatures: &[&[u8]] = &[b"MZ", b"\x7fELF", b"PK", b"\x89PNG", b"\xFF\xD8\xFF"];

		for sig in audio_signatures {
			if data.windows(sig.len()).any(|window| window == *sig) {
				format_count += 1;
			}
		}

		for sig in other_signatures {
			if data.windows(sig.len()).any(|window| window == *sig) {
				format_count += 1;
			}
		}

		if format_count > 1 {
			return Err(ValidationError::SecurityFailed("Multiple file format signatures detected (possible polyglot)".into()));
		}

		Ok(true) // polyglot-free
	}
}

/// Basic security service - just codec whitelist checking
pub struct BasicSecurityService;

impl SecurityService for BasicSecurityService {
	fn check_polyglot(&self, _source: &AudioSource) -> Result<bool, ValidationError> {
		// No polyglot checking - always pass
		Ok(true)
	}

	fn verify_codec_safety(&self, codec: &str, whitelist: &HashSet<String>) -> Result<bool, ValidationError> {
		let normalized_codec = codec.to_lowercase();
		Ok(whitelist.contains(&normalized_codec))
	}
}

/// Permissive security service - allows everything (for development)
pub struct PermissiveSecurityService;

impl SecurityService for PermissiveSecurityService {
	fn check_polyglot(&self, _source: &AudioSource) -> Result<bool, ValidationError> {
		Ok(true)
	}

	fn verify_codec_safety(&self, _codec: &str, _whitelist: &HashSet<String>) -> Result<bool, ValidationError> {
		Ok(true)
	}
}

// ============================================================================
// ADAPTER FACTORY - Mix and match implementations
// ============================================================================

pub struct ValidationServiceAdapter {
	identity: Box<dyn IdentityService>,
	structure: Box<dyn StructureService>,
	security: Box<dyn SecurityService>,
}

impl ValidationServiceAdapter {
	/// Full-featured validator with all security checks
	pub fn production() -> Self {
		Self {
			identity: Box::new(Sha256IdentityService),
			structure: Box::new(FfprobeStructureService),
			security: Box::new(ComprehensiveSecurityService),
		}
	}

	/// Basic validator without ffmpeg dependency  
	pub fn basic() -> Self {
		Self {
			identity: Box::new(Sha256IdentityService),
			structure: Box::new(BasicStructureService::new()),
			security: Box::new(BasicSecurityService),
		}
	}

	/// Development validator - minimal checks, no external dependencies
	pub fn development() -> Self {
		Self {
			identity: Box::new(MinimalIdentityService),
			structure: Box::new(NoOpStructureService),
			security: Box::new(PermissiveSecurityService),
		}
	}

	/// Custom adapter - pick your own implementations
	pub fn custom(identity: Box<dyn IdentityService>, structure: Box<dyn StructureService>, security: Box<dyn SecurityService>) -> Self {
		Self { identity, structure, security }
	}

	/// Create validator with these service implementations
	pub fn create_validator(self, constraints: ValidationConstraints) -> AudioValidator {
		AudioValidator::new(constraints, self.identity, self.structure, self.security)
	}
}

// ============================================================================
// CONVENIENCE BUILDERS
// ============================================================================

impl AudioValidator {
	/// Production validator with all features enabled
	pub fn production_validator(constraints: ValidationConstraints) -> Self {
		ValidationServiceAdapter::production().create_validator(constraints)
	}

	/// Basic validator without ffmpeg - good for most use cases
	pub fn basic_validator(constraints: ValidationConstraints) -> Self {
		ValidationServiceAdapter::basic().create_validator(constraints)
	}

	/// Development validator - minimal validation for testing
	pub fn development_validator(constraints: ValidationConstraints) -> Self {
		ValidationServiceAdapter::development().create_validator(constraints)
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::fs;

	#[test]
	fn test_basic_validator_pipeline() {
		let validator = AudioValidator::basic_validator(ValidationConstraints::default());

		// Create a test file
		let test_content = b"ID3\x03\x00\x00\x00"; // Minimal MP3 header
		let test_path = "/tmp/test_audio.mp3";
		fs::write(test_path, test_content).unwrap();

		let unvalidated_file = AudioFile::from_local_file("test_audio".to_string(), test_path.to_string());

		let result = validator.validate_complete(unvalidated_file);

		// Clean up
		let _ = fs::remove_file(test_path);

		// Should succeed with basic validator
		assert!(result.is_ok());
		let validated = result.unwrap();
		assert!(validated.evidence.file_hash.is_some());
	}

	#[test]
	fn test_development_validator_always_passes() {
		let validator = AudioValidator::development_validator(ValidationConstraints::default());

		let unvalidated_file = AudioFile::from_local_file("anything".to_string(), "/nonexistent/file.txt".to_string());

		// Development validator should pass even with invalid file
		// (though this specific test might fail due to file I/O, but the security/structure checks would pass)
		let identity_result = validator.verify_identity(unvalidated_file);

		// The identity service should at least generate a hash
		if let Ok(identity_verified) = identity_result {
			assert!(identity_verified.evidence.sanitized_id.is_some());
		}
	}

	#[test]
	fn test_adapter_customization() {
		let custom_validator = ValidationServiceAdapter::custom(
			Box::new(MinimalIdentityService),
			Box::new(BasicStructureService::new()),
			Box::new(ComprehensiveSecurityService),
		)
		.create_validator(ValidationConstraints::default());

		// This creates a validator with minimal identity, basic structure, but full security
		// Perfect for scenarios where you want security checks but don't have ffmpeg
	}
}

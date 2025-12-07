use super::schedule::SceneMetadata;
use serde::{Deserialize, Serialize};

/// Time in milliseconds
pub type TimeMs = i64;

/// Scene identifier
pub type SceneId = String;

/// Scene configuration with name and duration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SceneConfig {
	pub scene_name: String,
	pub duration: TimeMs,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub metadata: Option<SceneMetadata>,
}

impl SceneConfig {
	pub fn new(scene_name: impl Into<String>, duration: TimeMs) -> Self {
		Self {
			scene_name: scene_name.into(),
			duration,
			metadata: None,
		}
	}

	pub fn with_metadata(mut self, metadata: SceneMetadata) -> Self {
		self.metadata = Some(metadata);
		self
	}

	pub fn id(&self) -> SceneId {
		format!("scene_{}", self.scene_name)
	}
}

/// Progress through the orchestration (0.0 to 1.0)
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct Progress(f64);

impl Progress {
	pub fn new(current: TimeMs, total: TimeMs) -> Self {
		if total == 0 {
			return Self(0.0);
		}
		Self((current as f64 / total as f64).clamp(0.0, 1.0))
	}

	pub fn value(&self) -> f64 {
		self.0
	}

	pub fn percentage(&self) -> f64 {
		self.0 * 100.0
	}
}

impl Default for Progress {
	fn default() -> Self {
		Self(0.0)
	}
}

impl From<f64> for Progress {
	fn from(value: f64) -> Self {
		Self(value.clamp(0.0, 1.0))
	}
}

/// Timecode in HH:MM:SS.mmm format
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Timecode(String);

impl Timecode {
	/// Parse timecode string to milliseconds
	pub fn parse(timecode: &str) -> Result<TimeMs, String> {
		let parts: Vec<&str> = timecode.split(':').collect();
		if parts.len() != 3 {
			return Err(format!("Invalid timecode format: {}", timecode));
		}

		let hours = parts[0].parse::<i64>().map_err(|_| format!("Invalid hours: {}", parts[0]))?;
		let minutes = parts[1].parse::<i64>().map_err(|_| format!("Invalid minutes: {}", parts[1]))?;

		let seconds_parts: Vec<&str> = parts[2].split('.').collect();
		let seconds = seconds_parts[0].parse::<i64>().map_err(|_| format!("Invalid seconds: {}", seconds_parts[0]))?;

		let milliseconds = if seconds_parts.len() > 1 {
			let ms_str = format!("{:0<3}", seconds_parts[1]);
			ms_str[..3].parse::<i64>().map_err(|_| format!("Invalid milliseconds: {}", seconds_parts[1]))?
		} else {
			0
		};

		Ok(hours * 3600 * 1000 + minutes * 60 * 1000 + seconds * 1000 + milliseconds)
	}

	/// Convert milliseconds to timecode string
	pub fn from_ms(ms: TimeMs) -> Self {
		let hours = ms / (3600 * 1000);
		let minutes = (ms % (3600 * 1000)) / (60 * 1000);
		let seconds = (ms % (60 * 1000)) / 1000;
		let milliseconds = ms % 1000;

		Self(format!("{:02}:{:02}:{:02}.{:03}", hours, minutes, seconds, milliseconds))
	}

	pub fn as_str(&self) -> &str {
		&self.0
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_timecode_parse() {
		assert_eq!(Timecode::parse("00:00:00.000").unwrap(), 0);
		assert_eq!(Timecode::parse("00:00:01.000").unwrap(), 1000);
		assert_eq!(Timecode::parse("00:01:00.000").unwrap(), 60000);
		assert_eq!(Timecode::parse("01:00:00.000").unwrap(), 3600000);
		assert_eq!(Timecode::parse("01:23:45.678").unwrap(), 5025678);
	}

	#[test]
	fn test_timecode_format() {
		assert_eq!(Timecode::from_ms(0).as_str(), "00:00:00.000");
		assert_eq!(Timecode::from_ms(1000).as_str(), "00:00:01.000");
		assert_eq!(Timecode::from_ms(60000).as_str(), "00:01:00.000");
		assert_eq!(Timecode::from_ms(3600000).as_str(), "01:00:00.000");
		assert_eq!(Timecode::from_ms(5025678).as_str(), "01:23:45.678");
	}

	#[test]
	fn test_progress() {
		assert_eq!(Progress::new(0, 100).value(), 0.0);
		assert_eq!(Progress::new(50, 100).value(), 0.5);
		assert_eq!(Progress::new(100, 100).value(), 1.0);
		assert_eq!(Progress::new(0, 0).value(), 0.0);
	}
}

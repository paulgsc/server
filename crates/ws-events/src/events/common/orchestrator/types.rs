use serde::{Deserialize, Serialize};

/// Time in milliseconds
pub type TimeMs = i64;

/// Unique identifier for a lifetime (scene, overlay, etc.)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct LifetimeId(pub u32);

/// Scene identifier
pub type SceneId = String;

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

impl From<f64> for Progress {
	fn from(value: f64) -> Self {
		Self(value.clamp(0.0, 1.0))
	}
}

impl Default for Progress {
	fn default() -> Self {
		Self(0.0)
	}
}

/// Timecode in HH:MM:SS.mmm format
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Timecode(String);

impl Timecode {
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

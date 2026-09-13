use num_traits::ToPrimitive;
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
	#[must_use]
	pub fn new(current: TimeMs, total: TimeMs) -> Self {
		if total == 0 {
			return Self(0.0);
		}
		let current = current.to_f64().unwrap_or_else(|| if current.is_negative() { f64::MIN } else { f64::MAX });
		let total = total.to_f64().unwrap_or_else(|| if total.is_negative() { f64::MIN } else { f64::MAX });
		Self((current / total).clamp(0.0, 1.0))
	}

	#[must_use]
	pub const fn value(&self) -> f64 {
		self.0
	}

	#[must_use]
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
		Self(owned_format!("{hours:02}:{minutes:02}:{seconds:02}.{milliseconds:03}"))
	}

	pub fn as_str(&self) -> &str {
		&self.0
	}
}

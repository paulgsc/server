#![cfg(feature = "events")]

use super::types::{SceneConfig, TimeMs};
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrchestratorConfig {
	/// List of scenes to orchestrate
	pub scenes: Vec<SceneConfig>,

	/// Tick interval for the orchestrator engine
	#[serde(default = "default_tick_interval")]
	pub tick_interval_ms: u64,

	/// Whether to loop scenes after completion
	#[serde(default)]
	pub loop_scenes: bool,

	/// Grace period before considering stream as stopped (ms)
	#[serde(default = "default_stream_grace_period")]
	pub stream_grace_period_ms: TimeMs,
}

fn default_tick_interval() -> u64 {
	100 // 100ms tick rate
}

fn default_stream_grace_period() -> TimeMs {
	5000 // 5 seconds
}

impl OrchestratorConfig {
	pub fn new(scenes: Vec<SceneConfig>) -> Self {
		Self {
			scenes,
			tick_interval_ms: default_tick_interval(),
			loop_scenes: false,
			stream_grace_period_ms: default_stream_grace_period(),
		}
	}

	pub fn with_tick_interval(mut self, interval_ms: u64) -> Self {
		self.tick_interval_ms = interval_ms;
		self
	}

	pub fn with_looping(mut self, enabled: bool) -> Self {
		self.loop_scenes = enabled;
		self
	}

	pub fn with_grace_period(mut self, grace_ms: TimeMs) -> Self {
		self.stream_grace_period_ms = grace_ms;
		self
	}

	pub fn tick_interval(&self) -> Duration {
		Duration::from_millis(self.tick_interval_ms)
	}

	pub fn total_duration(&self) -> TimeMs {
		self.scenes.iter().map(|s| s.duration).sum()
	}

	pub fn validate(&self) -> Result<(), String> {
		if self.scenes.is_empty() {
			return Ok(());
		}

		for (idx, scene) in self.scenes.iter().enumerate() {
			if scene.scene_name.is_empty() {
				return Err(format!("Scene {} has empty name", idx));
			}
			if scene.duration == 0 {
				return Err(format!("Scene '{}' has zero duration", scene.scene_name));
			}
		}

		// Check for duplicate scene names
		let mut names = std::collections::HashSet::new();
		for scene in &self.scenes {
			if !names.insert(&scene.scene_name) {
				return Err(format!("Duplicate scene name: {}", scene.scene_name));
			}
		}

		Ok(())
	}
}

impl Default for OrchestratorConfig {
	fn default() -> Self {
		Self {
			scenes: Vec::new(),
			tick_interval_ms: default_tick_interval(),
			loop_scenes: false,
			stream_grace_period_ms: default_stream_grace_period(),
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_config_validation() {
		let config = OrchestratorConfig::new(vec![]);
		assert!(config.validate().is_err());

		let config = OrchestratorConfig::new(vec![SceneConfig::new("intro", 5000), SceneConfig::new("main", 10000)]);
		assert!(config.validate().is_ok());

		let config = OrchestratorConfig::new(vec![SceneConfig::new("intro", 5000), SceneConfig::new("intro", 10000)]);
		assert!(config.validate().is_err());
	}

	#[test]
	fn test_total_duration() {
		let config = OrchestratorConfig::new(vec![SceneConfig::new("intro", 5000), SceneConfig::new("main", 10000), SceneConfig::new("outro", 3000)]);
		assert_eq!(config.total_duration(), 18000);
	}
}

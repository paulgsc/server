use super::{LifetimeEvent, LifetimeId, LifetimeKind, OrchestratorEvent, SceneId, ScenePayload, TimeMs, TimedEvent, Timeline};
use super::{OrchestratorConfigData, SceneConfigData, UILayoutIntentData};
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Scene configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SceneConfig {
	pub scene_name: String,
	pub duration: TimeMs,
	pub start_time: Option<TimeMs>,
	pub ui: Vec<UILayoutIntentData>,
}

impl SceneConfig {
	pub fn new(scene_name: impl Into<String>, duration: TimeMs) -> Self {
		Self {
			scene_name: scene_name.into(),
			duration,
			start_time: None,
			ui: Vec::new(),
		}
	}

	pub fn starting_at(mut self, start: TimeMs) -> Self {
		self.start_time = Some(start);
		self
	}

	pub fn with_ui(mut self, ui: Vec<UILayoutIntentData>) -> Self {
		self.ui = ui;
		self
	}

	pub fn id(&self) -> SceneId {
		format!("scene_{}", self.scene_name)
	}
}

/// Convert from entity type to service type
impl From<SceneConfigData> for SceneConfig {
	fn from(data: SceneConfigData) -> Self {
		Self {
			scene_name: data.scene_name,
			duration: data.duration,
			start_time: data.start_time,
			ui: data.ui,
		}
	}
}

/// Convert from service type to entity type
impl From<SceneConfig> for SceneConfigData {
	fn from(config: SceneConfig) -> Self {
		Self {
			scene_name: config.scene_name,
			duration: config.duration,
			start_time: config.start_time,
			ui: config.ui,
		}
	}
}

/// Orchestrator configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrchestratorConfig {
	pub scenes: Vec<SceneConfig>,
	pub tick_interval_ms: u64,
	pub loop_scenes: bool,
}

impl OrchestratorConfig {
	pub fn new(scenes: Vec<SceneConfig>) -> Self {
		Self {
			scenes,
			tick_interval_ms: 100,
			loop_scenes: false,
		}
	}

	pub fn with_tick_interval(mut self, ms: u64) -> Self {
		self.tick_interval_ms = ms;
		self
	}

	pub fn with_loop(mut self, enable: bool) -> Self {
		self.loop_scenes = enable;
		self
	}

	pub fn tick_interval(&self) -> Duration {
		Duration::from_millis(self.tick_interval_ms)
	}

	pub fn validate(&self) -> Result<(), String> {
		if self.scenes.is_empty() {
			return Err("No scenes configured".to_string());
		}
		for scene in &self.scenes {
			if scene.duration <= 0 {
				return Err(format!("Scene '{}' has invalid duration", scene.scene_name));
			}
		}
		Ok(())
	}

	/// Compile scenes into a timeline of events
	/// Scenes become paired LifetimeStart/LifetimeEnd events
	pub fn compile_timeline(&self) -> Timeline {
		let mut events: Vec<TimedEvent<OrchestratorEvent>> = Vec::new();
		let mut next_lifetime_id = 0u32;

		for scene in &self.scenes {
			let lifetime_id = LifetimeId(next_lifetime_id);
			next_lifetime_id += 1;

			// Use explicit start time if set, otherwise cumulative
			let start_time = scene.start_time.unwrap_or_else(|| events.iter().map(|e| e.at).max().unwrap_or(0));

			// Start event
			events.push(TimedEvent {
				at: start_time,
				event: OrchestratorEvent::Lifetime(LifetimeEvent::Start {
					id: lifetime_id,
					kind: LifetimeKind::Scene(ScenePayload {
						scene_id: scene.id(),
						scene_name: scene.scene_name.clone(),
						ui: scene.ui.clone(),
						duration: scene.duration.clone(),
					}),
				}),
			});

			// End event
			events.push(TimedEvent {
				at: start_time + scene.duration,
				event: OrchestratorEvent::Lifetime(LifetimeEvent::End { id: lifetime_id }),
			});
		}

		// Sort events just in case
		events.sort_by_key(|e| e.at);
		Timeline::new(events)
	}
}

/// Convert from entity type to service type
impl From<OrchestratorConfigData> for OrchestratorConfig {
	fn from(data: OrchestratorConfigData) -> Self {
		Self {
			scenes: data.scenes.into_iter().map(SceneConfig::from).collect(),
			tick_interval_ms: data.tick_interval_ms,
			loop_scenes: data.loop_scenes,
		}
	}
}

/// Convert from service type to entity type
impl From<OrchestratorConfig> for OrchestratorConfigData {
	fn from(config: OrchestratorConfig) -> Self {
		Self {
			scenes: config.scenes.into_iter().map(SceneConfigData::from).collect(),
			tick_interval_ms: config.tick_interval_ms,
			loop_scenes: config.loop_scenes,
		}
	}
}

impl Default for OrchestratorConfig {
	fn default() -> Self {
		Self {
			scenes: Vec::new(),
			tick_interval_ms: 100,
			loop_scenes: false,
		}
	}
}

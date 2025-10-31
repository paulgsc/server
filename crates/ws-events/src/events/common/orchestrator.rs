use serde::{Deserialize, Serialize};

mod config;
mod schedule;
mod state;
mod types;

pub use config::OrchestratorConfig;
pub use schedule::{SceneSchedule, ScheduledElement};
pub use state::StreamStatus;
pub use types::{Progress, SceneConfig, SceneId, TimeMs, Timecode};

/// Commands that can be sent to the tick engine
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TickCommand {
	Start,
	Stop,
	Pause,
	Resume,
	Reset,
	ForceScene(String),
	SkipCurrentScene,
	UpdateStreamStatus { is_streaming: bool, stream_time: TimeMs, timecode: String },
	Reconfigure(OrchestratorConfig),
}

/// The current state of the orchestrator
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrchestratorState {
	/// Whether the orchestrator is running
	pub is_running: bool,

	/// Whether the orchestrator is paused
	pub is_paused: bool,

	/// Current active scene name
	pub current_active_scene: Option<String>,

	/// Current scene index
	pub current_scene_index: i32,

	/// Progress through the entire orchestration (0.0 to 1.0)
	pub progress: Progress,

	/// Current time in the orchestration (milliseconds)
	pub current_time: TimeMs,

	/// Time remaining in orchestration (milliseconds)
	pub time_remaining: TimeMs,

	/// List of currently active element IDs
	pub active_elements: Vec<SceneId>,

	/// All scheduled elements
	pub scheduled_elements: Vec<ScheduledElement>,

	/// Scene configuration
	pub scenes: Vec<SceneConfig>,

	/// Total duration of all scenes
	pub total_duration: TimeMs,

	/// Stream status
	pub stream_status: StreamStatus,
}

impl OrchestratorState {
	pub fn new() -> Self {
		Self {
			is_running: false,
			is_paused: false,
			current_active_scene: None,
			current_scene_index: -1,
			progress: Progress::default(),
			current_time: 0,
			time_remaining: 0,
			active_elements: Vec::new(),
			scheduled_elements: Vec::new(),
			scenes: Vec::new(),
			total_duration: 0,
			stream_status: StreamStatus::default(),
		}
	}

	pub fn from_schedule(schedule: &SceneSchedule, scenes: Vec<SceneConfig>) -> Self {
		let total_duration = schedule.total_duration();
		Self {
			is_running: false,
			is_paused: false,
			current_active_scene: None,
			current_scene_index: -1,
			progress: Progress::default(),
			current_time: 0,
			time_remaining: total_duration,
			active_elements: Vec::new(),
			scheduled_elements: schedule.elements().to_vec(),
			scenes,
			total_duration,
			stream_status: StreamStatus::default(),
		}
	}

	/// Update state based on current time
	pub fn update_from_time(&mut self, current_time: TimeMs, schedule: &SceneSchedule) {
		self.current_time = current_time;
		self.progress = Progress::new(current_time, self.total_duration);
		self.time_remaining = self.total_duration.saturating_sub(current_time);

		// Update active elements
		let active_elements = schedule.get_active_elements(current_time);
		self.active_elements = active_elements.iter().map(|e| e.id.clone()).collect();

		// Update current scene
		if let Some(current_scene) = schedule.get_current_scene(current_time) {
			self.current_active_scene = Some(current_scene.scene_name.clone());
			self.current_scene_index = schedule.get_scene_index(&current_scene.scene_name).map(|i| i as i32).unwrap_or(-1);
		} else {
			self.current_active_scene = None;
			self.current_scene_index = -1;
		}

		// Update scheduled elements' active status
		for element in &mut self.scheduled_elements {
			element.is_active = element.is_active_at(current_time);
		}
	}

	pub fn start(&mut self) {
		self.is_running = true;
		self.is_paused = false;
	}

	pub fn stop(&mut self) {
		self.is_running = false;
		self.is_paused = false;
	}

	pub fn pause(&mut self) {
		if self.is_running {
			self.is_paused = true;
		}
	}

	pub fn resume(&mut self) {
		if self.is_running {
			self.is_paused = false;
		}
	}

	pub fn reset(&mut self) {
		self.is_running = false;
		self.is_paused = false;
		self.current_time = 0;
		self.current_active_scene = None;
		self.current_scene_index = -1;
		self.progress = Progress::default();
		self.time_remaining = self.total_duration;
		self.active_elements.clear();

		// Reset all elements to inactive
		for element in &mut self.scheduled_elements {
			element.is_active = false;
		}
	}

	pub fn update_stream_status(&mut self, is_streaming: bool, stream_time: TimeMs, timecode: String) {
		self.stream_status.update(is_streaming, stream_time, timecode);
	}

	pub fn is_complete(&self) -> bool {
		self.current_time >= self.total_duration
	}

	pub fn current_timecode(&self) -> Timecode {
		Timecode::from_ms(self.current_time)
	}
}

impl Default for OrchestratorState {
	fn default() -> Self {
		Self::new()
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::stream_orch::types::SceneConfig;

	#[test]
	fn test_state_lifecycle() {
		let mut state = OrchestratorState::new();

		assert!(!state.is_running);
		assert!(!state.is_paused);

		state.start();
		assert!(state.is_running);
		assert!(!state.is_paused);

		state.pause();
		assert!(state.is_running);
		assert!(state.is_paused);

		state.resume();
		assert!(state.is_running);
		assert!(!state.is_paused);

		state.stop();
		assert!(!state.is_running);
		assert!(!state.is_paused);
	}

	#[test]
	fn test_state_reset() {
		let scenes = vec![SceneConfig::new("test", 5000)];
		let schedule = SceneSchedule::from_scenes(&scenes);
		let mut state = OrchestratorState::from_schedule(&schedule, scenes);

		state.start();
		state.update_from_time(2500, &schedule);

		assert_eq!(state.current_time, 2500);
		assert!(state.current_active_scene.is_some());

		state.reset();

		assert_eq!(state.current_time, 0);
		assert!(state.current_active_scene.is_none());
		assert!(!state.is_running);
	}
}

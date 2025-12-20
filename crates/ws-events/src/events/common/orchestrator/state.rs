use super::{LifetimeId, LifetimeKind, Progress, TimeMs, Timecode};
use serde::{Deserialize, Serialize};

/// An active lifetime in the current state
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveLifetime {
	pub id: LifetimeId,
	pub kind: LifetimeKind,
	pub started_at: TimeMs,
}

impl ActiveLifetime {
	/// Get scene name if this is a scene lifetime
	pub fn scene_name(&self) -> Option<&str> {
		match &self.kind {
			LifetimeKind::Scene(scene) => Some(&scene.scene_name),
		}
	}
}

/// Stream status from external source (OBS)
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StreamStatus {
	pub is_streaming: bool,
	pub stream_time: TimeMs,
	pub timecode: String,
}

/// The orchestrator's observable state (immutable snapshot)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrchestratorState {
	pub is_running: bool,
	pub is_paused: bool,
	pub current_time: TimeMs,
	pub total_duration: TimeMs,
	pub progress: Progress,
	pub time_remaining: TimeMs,

	/// Currently active lifetimes (scenes, overlays, etc.)
	pub active_lifetimes: Vec<ActiveLifetime>,

	/// Current active scene (first active scene lifetime if any)
	pub current_active_scene: Option<String>,

	/// Stream status
	pub stream_status: StreamStatus,
}

impl OrchestratorState {
	pub fn new(total_duration: TimeMs) -> Self {
		Self {
			is_running: false,
			is_paused: false,
			current_time: 0,
			total_duration,
			progress: Progress::default(),
			time_remaining: total_duration,
			active_lifetimes: Vec::new(),
			current_active_scene: None,
			stream_status: StreamStatus::default(),
		}
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
		Self {
			is_running: false,
			is_paused: false,
			current_time: 0,
			total_duration: 0,
			progress: Progress::default(),
			time_remaining: 0,
			active_lifetimes: Vec::new(),
			current_active_scene: None,
			stream_status: StreamStatus::default(),
		}
	}
}

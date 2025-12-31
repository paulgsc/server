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

/// The orchestrator's lifecycle mode (FSM state)
#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum OrchestratorMode {
	/// No configuration loaded
	Unconfigured,
	/// Configured but not running
	Idle,
	/// Actively playing timeline
	Running,
	/// Paused during playback
	Paused,
	/// Completed naturally (terminal)
	Finished,
	/// Stopped by user command (terminal)
	Stopped,
	/// Unrecoverable error occurred (terminal)
	Error,
}

impl OrchestratorMode {
	/// Returns true if this mode is terminal (requires cleanup)
	pub fn is_terminal(&self) -> bool {
		matches!(self, OrchestratorMode::Finished | OrchestratorMode::Stopped | OrchestratorMode::Error)
	}

	/// Returns true if this mode allows playback operations
	pub fn is_active(&self) -> bool {
		matches!(self, OrchestratorMode::Idle | OrchestratorMode::Running | OrchestratorMode::Paused)
	}
}

/// The orchestrator's observable state (immutable snapshot)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrchestratorState {
	/// Current lifecycle mode
	pub mode: OrchestratorMode,
	/// Current playback time
	pub current_time: TimeMs,
	/// Total timeline duration
	pub total_duration: TimeMs,
	/// Playback progress (0.0 to 1.0)
	pub progress: Progress,
	/// Time remaining in timeline
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
			mode: OrchestratorMode::Unconfigured,
			current_time: 0,
			total_duration,
			progress: Progress::default(),
			time_remaining: total_duration,
			active_lifetimes: Vec::new(),
			current_active_scene: None,
			stream_status: StreamStatus::default(),
		}
	}

	/// Returns true if the orchestrator is in a terminal state
	pub fn is_terminal(&self) -> bool {
		self.mode.is_terminal()
	}

	/// Returns true if playback has completed (time reached end)
	pub fn is_complete(&self) -> bool {
		self.current_time >= self.total_duration
	}

	/// Returns true if currently playing
	pub fn is_running(&self) -> bool {
		self.mode == OrchestratorMode::Running
	}

	/// Returns true if paused
	pub fn is_paused(&self) -> bool {
		self.mode == OrchestratorMode::Paused
	}

	pub fn current_timecode(&self) -> Timecode {
		Timecode::from_ms(self.current_time)
	}
}

impl Default for OrchestratorState {
	fn default() -> Self {
		Self {
			mode: OrchestratorMode::Unconfigured,
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

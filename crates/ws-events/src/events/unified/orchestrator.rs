use crate::events::{OrchestratorCommandData, OrchestratorConfigData, OrchestratorMode, OrchestratorState};
use prost::Message;

/// Prost-compatible OrchestratorCommandData message
#[derive(Clone, PartialEq, Message)]
pub struct TickCommandMessage {
	#[prost(string, tag = "1")]
	pub stream_id: String,
	#[prost(oneof = "tick_command_message::Command", tags = "2, 3, 4, 5, 6, 7, 8, 9, 10")]
	pub command: Option<tick_command_message::Command>,
}

pub mod tick_command_message {
	use super::*;

	#[derive(Clone, PartialEq, prost::Oneof)]
	pub enum Command {
		#[prost(message, tag = "2")]
		Start(StartCommand),
		#[prost(message, tag = "3")]
		Stop(StopCommand),
		#[prost(message, tag = "4")]
		Pause(PauseCommand),
		#[prost(message, tag = "5")]
		Resume(ResumeCommand),
		#[prost(message, tag = "6")]
		Reset(ResetCommand),
		#[prost(message, tag = "7")]
		ForceScene(ForceSceneCommand),
		#[prost(message, tag = "8")]
		SkipCurrentScene(SkipCurrentSceneCommand),
		#[prost(message, tag = "9")]
		UpdateStreamStatus(UpdateStreamStatusCommand),
		#[prost(message, tag = "10")]
		Configure(ConfigureCommand),
	}

	#[derive(Clone, PartialEq, Message)]
	pub struct StartCommand {}

	#[derive(Clone, PartialEq, Message)]
	pub struct StopCommand {}

	#[derive(Clone, PartialEq, Message)]
	pub struct PauseCommand {}

	#[derive(Clone, PartialEq, Message)]
	pub struct ResumeCommand {}

	#[derive(Clone, PartialEq, Message)]
	pub struct ResetCommand {}

	#[derive(Clone, PartialEq, Message)]
	pub struct ForceSceneCommand {
		#[prost(string, tag = "1")]
		pub scene_name: String,
	}

	#[derive(Clone, PartialEq, Message)]
	pub struct SkipCurrentSceneCommand {}

	#[derive(Clone, PartialEq, Message)]
	pub struct UpdateStreamStatusCommand {
		#[prost(bool, tag = "1")]
		pub is_streaming: bool,
		#[prost(int64, tag = "2")]
		pub stream_time: i64,
		#[prost(string, tag = "3")]
		pub timecode: String,
	}

	#[derive(Clone, PartialEq, Message)]
	pub struct ConfigureCommand {
		/// JSON-encoded OrchestratorConfigData
		#[prost(bytes, tag = "1")]
		pub config_json: Vec<u8>,
	}
}

impl TickCommandMessage {
	pub fn from_tick_command(stream_id: String, cmd: OrchestratorCommandData) -> Result<Self, String> {
		use tick_command_message::*;

		let command = match cmd {
			OrchestratorCommandData::Start => Some(Command::Start(StartCommand {})),
			OrchestratorCommandData::Stop => Some(Command::Stop(StopCommand {})),
			OrchestratorCommandData::Pause => Some(Command::Pause(PauseCommand {})),
			OrchestratorCommandData::Resume => Some(Command::Resume(ResumeCommand {})),
			OrchestratorCommandData::Reset => Some(Command::Reset(ResetCommand {})),
			OrchestratorCommandData::ForceScene(scene_name) => Some(Command::ForceScene(ForceSceneCommand { scene_name })),
			OrchestratorCommandData::SkipCurrentScene => Some(Command::SkipCurrentScene(SkipCurrentSceneCommand {})),
			OrchestratorCommandData::UpdateStreamStatus {
				is_streaming,
				stream_time,
				timecode,
			} => Some(Command::UpdateStreamStatus(UpdateStreamStatusCommand {
				is_streaming,
				stream_time,
				timecode,
			})),
			OrchestratorCommandData::Configure(config_data) => {
				let config_json = serde_json::to_vec(&config_data).map_err(|e| format!("Failed to serialize config: {}", e))?;
				Some(Command::Configure(ConfigureCommand { config_json }))
			}
		};

		Ok(TickCommandMessage { stream_id, command })
	}

	pub fn to_tick_command(&self) -> Result<(String, OrchestratorCommandData), String> {
		use tick_command_message::Command;

		let cmd = match &self.command {
			Some(Command::Start(_)) => OrchestratorCommandData::Start,
			Some(Command::Stop(_)) => OrchestratorCommandData::Stop,
			Some(Command::Pause(_)) => OrchestratorCommandData::Pause,
			Some(Command::Resume(_)) => OrchestratorCommandData::Resume,
			Some(Command::Reset(_)) => OrchestratorCommandData::Reset,
			Some(Command::ForceScene(cmd)) => OrchestratorCommandData::ForceScene(cmd.scene_name.clone()),
			Some(Command::SkipCurrentScene(_)) => OrchestratorCommandData::SkipCurrentScene,
			Some(Command::UpdateStreamStatus(cmd)) => OrchestratorCommandData::UpdateStreamStatus {
				is_streaming: cmd.is_streaming,
				stream_time: cmd.stream_time,
				timecode: cmd.timecode.clone(),
			},
			Some(Command::Configure(cmd)) => {
				let config_data: OrchestratorConfigData = serde_json::from_slice(&cmd.config_json).map_err(|e| format!("Failed to deserialize config: {}", e))?;
				OrchestratorCommandData::Configure(config_data)
			}
			None => return Err("TickCommandMessage has no command variant".to_string()),
		};

		Ok((self.stream_id.clone(), cmd))
	}
}

/// Prost-compatible OrchestratorState message
#[derive(Clone, PartialEq, Message)]
pub struct OrchestratorStateMessage {
	#[prost(string, tag = "1")]
	pub stream_id: String,
	#[prost(enumeration = "OrchestratorModeProto", tag = "2")]
	pub mode: i32,
	#[prost(int64, tag = "3")]
	pub current_time: i64,
	#[prost(int64, tag = "4")]
	pub total_duration: i64,
	#[prost(double, tag = "5")]
	pub progress: f64,
	#[prost(int64, tag = "6")]
	pub time_remaining: i64,
	#[prost(bytes, tag = "7")]
	pub active_lifetimes_json: Vec<u8>,
	#[prost(string, optional, tag = "8")]
	pub current_active_scene: Option<String>,
	#[prost(bytes, tag = "9")]
	pub stream_status_json: Vec<u8>,
}

/// Protobuf enum for OrchestratorMode
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, prost::Enumeration)]
#[repr(i32)]
pub enum OrchestratorModeProto {
	Unconfigured = 0,
	Idle = 1,
	Running = 2,
	Paused = 3,
	Finished = 4,
	Stopped = 5,
	Error = 6,
}

impl From<OrchestratorMode> for OrchestratorModeProto {
	fn from(mode: OrchestratorMode) -> Self {
		match mode {
			OrchestratorMode::Unconfigured => OrchestratorModeProto::Unconfigured,
			OrchestratorMode::Idle => OrchestratorModeProto::Idle,
			OrchestratorMode::Running => OrchestratorModeProto::Running,
			OrchestratorMode::Paused => OrchestratorModeProto::Paused,
			OrchestratorMode::Finished => OrchestratorModeProto::Finished,
			OrchestratorMode::Stopped => OrchestratorModeProto::Stopped,
			OrchestratorMode::Error => OrchestratorModeProto::Error,
		}
	}
}

impl From<OrchestratorModeProto> for OrchestratorMode {
	fn from(mode: OrchestratorModeProto) -> Self {
		match mode {
			OrchestratorModeProto::Unconfigured => OrchestratorMode::Unconfigured,
			OrchestratorModeProto::Idle => OrchestratorMode::Idle,
			OrchestratorModeProto::Running => OrchestratorMode::Running,
			OrchestratorModeProto::Paused => OrchestratorMode::Paused,
			OrchestratorModeProto::Finished => OrchestratorMode::Finished,
			OrchestratorModeProto::Stopped => OrchestratorMode::Stopped,
			OrchestratorModeProto::Error => OrchestratorMode::Error,
		}
	}
}

impl OrchestratorStateMessage {
	pub fn from_orchestrator_state(stream_id: String, state: &OrchestratorState) -> Result<Self, String> {
		let stream_status_json = serde_json::to_vec(&state.stream_status).map_err(|e| format!("Failed to serialize stream_status: {}", e))?;

		let active_lifetimes_json = serde_json::to_vec(&state.active_lifetimes).map_err(|e| format!("Failed to serialize active_lifetimes: {}", e))?;

		Ok(Self {
			stream_id,
			mode: OrchestratorModeProto::from(state.mode) as i32,
			current_time: state.current_time,
			total_duration: state.total_duration,
			progress: state.progress.value(),
			time_remaining: state.time_remaining,
			active_lifetimes_json,
			current_active_scene: state.current_active_scene.clone(),
			stream_status_json,
		})
	}

	pub fn to_orchestrator_state(&self) -> Result<(String, OrchestratorState), String> {
		let stream_status = serde_json::from_slice(&self.stream_status_json).map_err(|e| format!("Failed to deserialize stream_status: {}", e))?;

		let active_lifetimes = serde_json::from_slice(&self.active_lifetimes_json).map_err(|e| format!("Failed to deserialize active_lifetimes: {}", e))?;

		let mode = OrchestratorModeProto::try_from(self.mode).map_err(|_| format!("Invalid mode value: {}", self.mode))?;

		let state = OrchestratorState {
			mode: mode.into(),
			current_time: self.current_time,
			total_duration: self.total_duration,
			progress: self.progress.into(),
			time_remaining: self.time_remaining,
			active_lifetimes,
			current_active_scene: self.current_active_scene.clone(),
			stream_status,
		};

		Ok((self.stream_id.clone(), state))
	}
}

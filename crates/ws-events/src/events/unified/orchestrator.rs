use crate::events::{OrchestratorState, TickCommand};
use prost::Message;

/// Prost-compatible TickCommand message
#[derive(Clone, PartialEq, Message)]
pub struct TickCommandMessage {
	#[prost(oneof = "tick_command_message::Command", tags = "1, 2, 3, 4, 5, 6, 7, 8, 9")]
	pub command: Option<tick_command_message::Command>,
}

pub mod tick_command_message {
	use super::*;

	#[derive(Clone, PartialEq, prost::Oneof)]
	pub enum Command {
		#[prost(message, tag = "1")]
		Start(StartCommand),
		#[prost(message, tag = "2")]
		Stop(StopCommand),
		#[prost(message, tag = "3")]
		Pause(PauseCommand),
		#[prost(message, tag = "4")]
		Resume(ResumeCommand),
		#[prost(message, tag = "5")]
		Reset(ResetCommand),
		#[prost(message, tag = "6")]
		ForceScene(ForceSceneCommand),
		#[prost(message, tag = "7")]
		SkipCurrentScene(SkipCurrentSceneCommand),
		#[prost(message, tag = "8")]
		UpdateStreamStatus(UpdateStreamStatusCommand),
		#[prost(message, tag = "9")]
		Reconfigure(ReconfigureCommand),
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
	pub struct ReconfigureCommand {
		#[prost(bytes, tag = "1")]
		pub config_json: Vec<u8>,
	}
}

impl TickCommandMessage {
	pub fn from_tick_command(cmd: TickCommand) -> Result<Self, String> {
		use tick_command_message::*;

		let command = match cmd {
			TickCommand::Start => Some(Command::Start(StartCommand {})),
			TickCommand::Stop => Some(Command::Stop(StopCommand {})),
			TickCommand::Pause => Some(Command::Pause(PauseCommand {})),
			TickCommand::Resume => Some(Command::Resume(ResumeCommand {})),
			TickCommand::Reset => Some(Command::Reset(ResetCommand {})),
			TickCommand::ForceScene(scene_name) => Some(Command::ForceScene(ForceSceneCommand { scene_name })),
			TickCommand::SkipCurrentScene => Some(Command::SkipCurrentScene(SkipCurrentSceneCommand {})),
			TickCommand::UpdateStreamStatus {
				is_streaming,
				stream_time,
				timecode,
			} => Some(Command::UpdateStreamStatus(UpdateStreamStatusCommand {
				is_streaming,
				stream_time,
				timecode,
			})),
			TickCommand::Reconfigure(config) => {
				let config_json = serde_json::to_vec(&config).map_err(|e| format!("Failed to serialize OrchestratorConfig: {}", e))?;
				Some(Command::Reconfigure(ReconfigureCommand { config_json }))
			}
		};

		Ok(TickCommandMessage { command })
	}

	pub fn to_tick_command(&self) -> Result<TickCommand, String> {
		use tick_command_message::Command;

		match &self.command {
			Some(Command::Start(_)) => Ok(TickCommand::Start),
			Some(Command::Stop(_)) => Ok(TickCommand::Stop),
			Some(Command::Pause(_)) => Ok(TickCommand::Pause),
			Some(Command::Resume(_)) => Ok(TickCommand::Resume),
			Some(Command::Reset(_)) => Ok(TickCommand::Reset),
			Some(Command::ForceScene(cmd)) => Ok(TickCommand::ForceScene(cmd.scene_name.clone())),
			Some(Command::SkipCurrentScene(_)) => Ok(TickCommand::SkipCurrentScene),
			Some(Command::UpdateStreamStatus(cmd)) => Ok(TickCommand::UpdateStreamStatus {
				is_streaming: cmd.is_streaming,
				stream_time: cmd.stream_time,
				timecode: cmd.timecode.clone(),
			}),
			Some(Command::Reconfigure(cmd)) => {
				let config = serde_json::from_slice(&cmd.config_json).map_err(|e| format!("Failed to deserialize OrchestratorConfig: {}", e))?;
				Ok(TickCommand::Reconfigure(config))
			}
			None => Err("TickCommandMessage has no command variant".to_string()),
		}
	}
}

/// Prost-compatible OrchestratorState message
#[derive(Clone, PartialEq, Message)]
pub struct OrchestratorStateMessage {
	#[prost(bool, tag = "1")]
	pub is_running: bool,
	#[prost(bool, tag = "2")]
	pub is_paused: bool,
	#[prost(string, optional, tag = "3")]
	pub current_active_scene: Option<String>,
	#[prost(int32, tag = "4")]
	pub current_scene_index: i32,
	#[prost(double, tag = "5")]
	pub progress: f64,
	#[prost(int64, tag = "6")]
	pub current_time: i64,
	#[prost(int64, tag = "7")]
	pub time_remaining: i64,
	#[prost(string, repeated, tag = "8")]
	pub active_elements: Vec<String>,
	#[prost(int64, tag = "9")]
	pub total_duration: i64,
	#[prost(bytes, tag = "10")]
	pub stream_status_json: Vec<u8>,
	#[prost(bytes, tag = "11")]
	pub scheduled_elements_json: Vec<u8>,
	#[prost(bytes, tag = "12")]
	pub scenes_json: Vec<u8>,
}

impl OrchestratorStateMessage {
	pub fn from_orchestrator_state(state: &OrchestratorState) -> Result<Self, String> {
		let stream_status_json = serde_json::to_vec(&state.stream_status).map_err(|e| format!("Failed to serialize stream_status: {}", e))?;

		let scheduled_elements_json = serde_json::to_vec(&state.scheduled_elements).map_err(|e| format!("Failed to serialize scheduled_elements: {}", e))?;

		let scenes_json = serde_json::to_vec(&state.scenes).map_err(|e| format!("Failed to serialize scenes: {}", e))?;

		Ok(Self {
			is_running: state.is_running,
			is_paused: state.is_paused,
			current_active_scene: state.current_active_scene.clone(),
			current_scene_index: state.current_scene_index,
			progress: state.progress.value(),
			current_time: state.current_time,
			time_remaining: state.time_remaining,
			active_elements: state.active_elements.clone(),
			total_duration: state.total_duration,
			stream_status_json,
			scheduled_elements_json,
			scenes_json,
		})
	}

	pub fn to_orchestrator_state(&self) -> Result<OrchestratorState, String> {
		let stream_status = serde_json::from_slice(&self.stream_status_json).map_err(|e| format!("Failed to deserialize stream_status: {}", e))?;

		let scheduled_elements = serde_json::from_slice(&self.scheduled_elements_json).map_err(|e| format!("Failed to deserialize scheduled_elements: {}", e))?;

		let scenes = serde_json::from_slice(&self.scenes_json).map_err(|e| format!("Failed to deserialize scenes: {}", e))?;

		Ok(OrchestratorState {
			is_running: self.is_running,
			is_paused: self.is_paused,
			current_active_scene: self.current_active_scene.clone(),
			current_scene_index: self.current_scene_index,
			progress: self.progress.into(),
			current_time: self.current_time,
			time_remaining: self.time_remaining,
			active_elements: self.active_elements.clone(),
			total_duration: self.total_duration,
			stream_status,
			scheduled_elements,
			scenes,
		})
	}
}

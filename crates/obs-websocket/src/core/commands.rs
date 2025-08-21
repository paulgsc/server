use super::*;
use crate::ObsRequestBuilder;
use serde_json::Value as JsonValue;

#[derive(Debug, Clone)]
pub enum ObsCommand {
	StartStream,
	StopStream,
	StartRecording,
	StopRecording,
	SwitchScene(String),
	SetInputMute(String, bool),
	SetInputVolume(String, f64),
	ToggleStudioMode,
	ToggleVirtualCamera,
	ToggleReplayBuffer,
	Custom(JsonValue),
}

#[derive(Debug)]
pub enum InternalCommand {
	Execute(ObsCommand),
	Disconnect,
}

/// Validates state and executes commands
pub struct CommandExecutor {
	state_handle: StateHandle,
	request_builder: ObsRequestBuilder,
}

impl CommandExecutor {
	pub fn new(state_handle: StateHandle) -> Self {
		Self {
			state_handle,
			request_builder: ObsRequestBuilder::new(),
		}
	}

	/// Execute command with state validation
	pub async fn execute(&self, command: ObsCommand) -> Result<(), StateError> {
		self.state_handle.execute_command(command).await
	}

	pub fn build_request(&self, command: &ObsCommand) -> JsonValue {
		match command {
			ObsCommand::StartStream => self.request_builder.start_stream(),
			ObsCommand::StopStream => self.request_builder.stop_stream(),
			ObsCommand::StartRecording => self.request_builder.start_recording(),
			ObsCommand::StopRecording => self.request_builder.stop_recording(),
			ObsCommand::SwitchScene(name) => self.request_builder.switch_scene(name),
			ObsCommand::SetInputMute(name, muted) => self.request_builder.set_input_mute(name, *muted),
			ObsCommand::SetInputVolume(name, volume) => self.request_builder.set_input_volume(name, *volume),
			ObsCommand::ToggleStudioMode => self.request_builder.toggle_studio_mode(),
			ObsCommand::ToggleVirtualCamera => self.request_builder.toggle_virtual_camera(),
			ObsCommand::ToggleReplayBuffer => self.request_builder.toggle_replay_buffer(),
			ObsCommand::Custom(json) => json.clone(),
		}
	}
}

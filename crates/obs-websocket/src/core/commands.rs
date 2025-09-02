use super::*;
use crate::messages::YouTubePrivacy;
use crate::ObsRequestBuilder;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "camelCase")]
pub enum ObsCommand {
	StartStream,
	StopStream,
	StartRecording,
	StopRecording,
	SwitchScene(String),
	SetInputMute(String, bool),
	SetInputVolume(String, f64),
	ToggleStudioMode(bool),
	StartVirtualCamera,
	StopVirtualCamera,
	StartReplayBuffer,
	StopReplayBuffer,
	GetInputMute(String),
	GetInputVolume(String),
	SetYouTubeStream {
		stream_key: String,
		title: String,
		description: String,
		category: String,
		privacy: YouTubePrivacy,
		unlisted: bool,
		tags: Vec<String>,
	},
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
}

impl CommandExecutor {
	pub fn new(state_handle: StateHandle) -> Self {
		Self { state_handle }
	}

	/// Execute command with state validation
	pub async fn execute(&self, command: ObsCommand) -> Result<(), StateError> {
		self.state_handle.execute_command(command).await
	}

	pub fn build_request(&self, command: &ObsCommand) -> JsonValue {
		match command {
			ObsCommand::StartStream => ObsRequestBuilder::start_stream(),
			ObsCommand::StopStream => ObsRequestBuilder::stop_stream(),
			ObsCommand::StartRecording => ObsRequestBuilder::start_recording(),
			ObsCommand::StopRecording => ObsRequestBuilder::stop_recording(),
			ObsCommand::SwitchScene(name) => ObsRequestBuilder::switch_scene(name),
			ObsCommand::SetInputMute(name, muted) => ObsRequestBuilder::set_input_mute(name, *muted),
			ObsCommand::SetInputVolume(name, volume) => ObsRequestBuilder::set_input_volume(name, *volume),
			ObsCommand::ToggleStudioMode(enabled) => ObsRequestBuilder::toggle_studio_mode(*enabled),
			ObsCommand::StartVirtualCamera => ObsRequestBuilder::start_virtual_camera(),
			ObsCommand::StopVirtualCamera => ObsRequestBuilder::stop_virtual_camera(),
			ObsCommand::StartReplayBuffer => ObsRequestBuilder::start_replay_buffer(),
			ObsCommand::StopReplayBuffer => ObsRequestBuilder::stop_replay_buffer(),
			ObsCommand::GetInputMute(name) => ObsRequestBuilder::get_input_mute(name),
			ObsCommand::GetInputVolume(name) => ObsRequestBuilder::get_input_volume(name),
			ObsCommand::SetYouTubeStream {
				stream_key,
				title,
				description,
				category,
				privacy,
				unlisted,
				tags,
			} => ObsRequestBuilder::set_youtube_stream(stream_key, title, description, category, *privacy, *unlisted, tags.clone()),
			ObsCommand::Custom(json) => json.clone(),
		}
	}
}

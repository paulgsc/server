//! Runs an [`ObsCommand`] as the matching typed `obws` request.

use crate::manager::CommandError;
use crate::types::{ObsCommand, StreamKey};
use obws::requests::inputs::{InputId, Volume};
use obws::Client;
use serde::Serialize;
use serde_json::{json, Value};

/// Primary RTMP ingest endpoint for `YouTube` live streams.
const YOUTUBE_RTMP_INGEST: &str = "rtmp://a.rtmp.youtube.com/live2";

/// Settings for OBS's `rtmp_custom` stream service, which reads only these two
/// fields (plus optional auth this crate doesn't use).
#[derive(Serialize)]
struct RtmpCustom<'a> {
	server: &'a str,
	key: &'a StreamKey,
}

/// Runs `command`, returning what OBS's `responseData` would hold for it:
/// `None` for requests that return nothing.
pub async fn execute(client: &Client, command: ObsCommand) -> Result<Option<Value>, CommandError> {
	let request = request_name(&command);
	run(client, command).await.map_err(|e| CommandError::from_obws(request, e))
}

async fn run(client: &Client, command: ObsCommand) -> Result<Option<Value>, obws::error::Error> {
	let none = |()| None;
	match command {
		ObsCommand::StartStream => client.streaming().start().await.map(none),
		ObsCommand::StopStream => client.streaming().stop().await.map(none),
		ObsCommand::StartRecording => client.recording().start().await.map(none),
		ObsCommand::StopRecording => {
			let path = client.recording().stop().await?;
			Ok(Some(json!({ "outputPath": path })))
		}
		ObsCommand::SwitchScene(name) => client.scenes().set_current_program_scene(name.as_str()).await.map(none),
		ObsCommand::SetInputMute(name, muted) => client.inputs().set_muted(InputId::Name(&name), muted).await.map(none),
		ObsCommand::SetInputVolume(name, multiplier) => {
			// OBS takes an f32 multiplier in 0.0..=20.0; f64 precision is moot there.
			#[allow(clippy::cast_possible_truncation)]
			let multiplier = multiplier as f32;
			client.inputs().set_volume(InputId::Name(&name), Volume::Mul(multiplier)).await.map(none)
		}
		ObsCommand::ToggleStudioMode(enabled) => client.ui().set_studio_mode_enabled(enabled).await.map(none),
		ObsCommand::StartVirtualCamera => client.virtual_cam().start().await.map(none),
		ObsCommand::StopVirtualCamera => client.virtual_cam().stop().await.map(none),
		ObsCommand::StartReplayBuffer => client.replay_buffer().start().await.map(none),
		ObsCommand::StopReplayBuffer => client.replay_buffer().stop().await.map(none),
		ObsCommand::GetInputMute(name) => {
			let muted = client.inputs().muted(InputId::Name(&name)).await?;
			Ok(Some(json!({ "inputMuted": muted })))
		}
		ObsCommand::GetInputVolume(name) => {
			let volume = client.inputs().volume(InputId::Name(&name)).await?;
			Ok(Some(json!({ "inputVolumeMul": volume.mul, "inputVolumeDb": volume.db })))
		}
		ObsCommand::SetYouTubeStream { stream_key } => client
			.config()
			.set_stream_service_settings(
				"rtmp_custom",
				&RtmpCustom {
					server: YOUTUBE_RTMP_INGEST,
					key: &stream_key,
				},
			)
			.await
			.map(none),
		// Rejected by the manager before reaching here; kept total for the compiler.
		ObsCommand::Custom(_) => Ok(None),
	}
}

/// The obs-websocket request a command sends, for error messages.
pub const fn request_name(command: &ObsCommand) -> &'static str {
	match command {
		ObsCommand::StartStream => "StartStream",
		ObsCommand::StopStream => "StopStream",
		ObsCommand::StartRecording => "StartRecord",
		ObsCommand::StopRecording => "StopRecord",
		ObsCommand::SwitchScene(_) => "SetCurrentProgramScene",
		ObsCommand::SetInputMute(..) => "SetInputMute",
		ObsCommand::SetInputVolume(..) => "SetInputVolume",
		ObsCommand::ToggleStudioMode(_) => "SetStudioModeEnabled",
		ObsCommand::StartVirtualCamera => "StartVirtualCam",
		ObsCommand::StopVirtualCamera => "StopVirtualCam",
		ObsCommand::StartReplayBuffer => "StartReplayBuffer",
		ObsCommand::StopReplayBuffer => "StopReplayBuffer",
		ObsCommand::GetInputMute(_) => "GetInputMute",
		ObsCommand::GetInputVolume(_) => "GetInputVolume",
		ObsCommand::SetYouTubeStream { .. } => "SetStreamServiceSettings",
		ObsCommand::Custom(_) => "Custom",
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn rtmp_custom_settings_hold_only_server_and_key() {
		let key = StreamKey::new("abcd-efgh".to_owned());
		let settings = serde_json::to_value(RtmpCustom {
			server: YOUTUBE_RTMP_INGEST,
			key: &key,
		})
		.unwrap();

		assert_eq!(settings, json!({ "server": YOUTUBE_RTMP_INGEST, "key": "abcd-efgh" }));
	}
}

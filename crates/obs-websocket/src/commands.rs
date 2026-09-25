//! Runs an [`ObsCommand`] as the matching typed `obws` requests.

use crate::manager::CommandError;
use crate::studio;
use crate::types::{MediaAction, ObsCommand, StreamKey};
use obws::requests::inputs::{InputId, SetSettings, Volume};
use obws::requests::scene_items::{Id as SceneItemLookup, SetEnabled as SetSceneItemEnabled};
use obws::requests::scenes::SceneId;
use obws::requests::sources::SourceId;
use obws::Client;
use serde::Serialize;
use serde_json::{json, Value};

type ObwsResult = Result<Option<Value>, obws::error::Error>;

/// Primary RTMP ingest endpoint for `YouTube` live streams.
const YOUTUBE_RTMP_INGEST: &str = "rtmp://a.rtmp.youtube.com/live2";

/// OBS's volume range in dB.
const MIN_VOLUME_DB: f64 = -100.0;
const MAX_VOLUME_DB: f64 = 26.0;

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

// One arm per command keeps this match exhaustive, so a new command can't
// compile without deciding what it does; multi-request commands delegate.
#[allow(clippy::too_many_lines)]
async fn run(client: &Client, command: ObsCommand) -> ObwsResult {
	let none = |()| None;
	match command {
		ObsCommand::StartStream => client.streaming().start().await.map(none),
		ObsCommand::StopStream => client.streaming().stop().await.map(none),
		ObsCommand::ToggleStream => {
			let active = client.streaming().toggle().await?;
			Ok(Some(json!({ "outputActive": active })))
		}
		ObsCommand::SendStreamCaption { text } => client.streaming().send_caption(&text).await.map(none),
		ObsCommand::SetYouTubeStream { stream_key } => set_youtube_stream(client, &stream_key).await.map(none),

		ObsCommand::StartRecording => client.recording().start().await.map(none),
		ObsCommand::StopRecording => {
			let path = client.recording().stop().await?;
			Ok(Some(json!({ "outputPath": path })))
		}
		ObsCommand::ToggleRecording => {
			let active = client.recording().toggle().await?;
			Ok(Some(json!({ "outputActive": active })))
		}
		ObsCommand::PauseRecording => client.recording().pause().await.map(none),
		ObsCommand::ResumeRecording => client.recording().resume().await.map(none),
		ObsCommand::TogglePauseRecording => {
			let paused = client.recording().toggle_pause().await?;
			Ok(Some(json!({ "outputPaused": paused })))
		}
		ObsCommand::SplitRecording => client.recording().split_file().await.map(none),
		ObsCommand::AddRecordingChapter { name } => client.recording().create_chapter(name.as_deref()).await.map(none),

		ObsCommand::StartReplayBuffer => client.replay_buffer().start().await.map(none),
		ObsCommand::StopReplayBuffer => client.replay_buffer().stop().await.map(none),
		ObsCommand::SaveReplay => client.replay_buffer().save().await.map(none),
		ObsCommand::StartVirtualCamera => client.virtual_cam().start().await.map(none),
		ObsCommand::StopVirtualCamera => client.virtual_cam().stop().await.map(none),

		ObsCommand::SwitchScene { scene } => client.scenes().set_current_program_scene(scene.as_str()).await.map(none),
		ObsCommand::SetPreviewScene { scene } => client.scenes().set_current_preview_scene(scene.as_str()).await.map(none),
		ObsCommand::SetStudioMode { enabled } => client.ui().set_studio_mode_enabled(enabled).await.map(none),
		ObsCommand::TriggerTransition => client.transitions().trigger().await.map(none),
		ObsCommand::SetTransition { transition } => client.transitions().set_current(&transition).await.map(none),
		ObsCommand::SetTransitionDuration { millis } => {
			let millis = i64::try_from(millis).unwrap_or(i64::MAX);
			client.transitions().set_current_duration(time::Duration::milliseconds(millis)).await.map(none)
		}
		ObsCommand::SetSourceVisible { source, scene, visible } => {
			let scene = scene_or_live(client, scene).await?;
			let item_id = scene_item_id(client, &scene, &source).await?;
			set_scene_item_enabled(client, &scene, item_id, visible).await.map(none)
		}
		ObsCommand::ToggleSourceVisible { source, scene } => {
			let visible = toggle_source_visible(client, &source, scene).await?;
			Ok(Some(json!({ "sceneItemEnabled": visible })))
		}

		ObsCommand::SetMute { input, muted } => client.inputs().set_muted(InputId::Name(&input), muted).await.map(none),
		ObsCommand::ToggleMute { input } => {
			let muted = client.inputs().toggle_mute(InputId::Name(&input)).await?;
			Ok(Some(json!({ "inputMuted": muted })))
		}
		ObsCommand::SetVolume { input, db } => set_volume_db(client, &input, db).await.map(none),
		ObsCommand::AdjustVolume { input, delta_db } => {
			let db = adjust_volume(client, &input, delta_db).await?;
			Ok(Some(json!({ "inputVolumeDb": db })))
		}
		ObsCommand::Media { input, action } => client.media_inputs().trigger_action(InputId::Name(&input), media_action(action)).await.map(none),
		ObsCommand::SetFilterEnabled { source, filter, enabled } => set_filter_enabled(client, &source, &filter, enabled).await.map(none),
		ObsCommand::SetText { input, text } => set_text(client, &input, &text).await.map(none),
		ObsCommand::TriggerHotkey { name } => client.hotkeys().trigger_by_name(&name, None).await.map(none),

		ObsCommand::GetStudio => {
			let snapshot = studio::snapshot(client).await?;
			Ok(serde_json::to_value(snapshot).ok())
		}
	}
}

async fn set_youtube_stream(client: &Client, stream_key: &StreamKey) -> Result<(), obws::error::Error> {
	let settings = RtmpCustom {
		server: YOUTUBE_RTMP_INGEST,
		key: stream_key,
	};
	client.config().set_stream_service_settings("rtmp_custom", &settings).await
}

async fn toggle_source_visible(client: &Client, source: &str, scene: Option<String>) -> Result<bool, obws::error::Error> {
	let scene = scene_or_live(client, scene).await?;
	let item_id = scene_item_id(client, &scene, source).await?;
	let visible = !client.scene_items().enabled(SceneId::Name(&scene), item_id).await?;
	set_scene_item_enabled(client, &scene, item_id, visible).await?;
	Ok(visible)
}

async fn adjust_volume(client: &Client, input: &str, delta_db: f64) -> Result<f64, obws::error::Error> {
	let current = client.inputs().volume(InputId::Name(input)).await?.db;
	let db = (f64::from(current) + delta_db).clamp(MIN_VOLUME_DB, MAX_VOLUME_DB);
	set_volume_db(client, input, db).await?;
	Ok(db)
}

async fn set_filter_enabled(client: &Client, source: &str, filter: &str, enabled: bool) -> Result<(), obws::error::Error> {
	client
		.filters()
		.set_enabled(obws::requests::filters::SetEnabled {
			source: SourceId::Name(source),
			filter,
			enabled,
		})
		.await
}

async fn set_text(client: &Client, input: &str, text: &str) -> Result<(), obws::error::Error> {
	client
		.inputs()
		.set_settings(SetSettings {
			input: InputId::Name(input),
			settings: &json!({ "text": text }),
			overlay: Some(true),
		})
		.await
}

async fn scene_or_live(client: &Client, scene: Option<String>) -> Result<String, obws::error::Error> {
	match scene {
		Some(scene) => Ok(scene),
		None => Ok(client.scenes().current_program_scene().await?.id.name),
	}
}

async fn scene_item_id(client: &Client, scene: &str, source: &str) -> Result<i64, obws::error::Error> {
	client
		.scene_items()
		.id(SceneItemLookup {
			scene: SceneId::Name(scene),
			source,
			search_offset: None,
		})
		.await
}

async fn set_scene_item_enabled(client: &Client, scene: &str, item_id: i64, enabled: bool) -> Result<(), obws::error::Error> {
	client
		.scene_items()
		.set_enabled(SetSceneItemEnabled {
			scene: SceneId::Name(scene),
			item_id,
			enabled,
		})
		.await
}

async fn set_volume_db(client: &Client, input: &str, db: f64) -> Result<(), obws::error::Error> {
	// OBS takes an f32 in -100.0..=26.0; clamping first keeps the cast in range.
	#[allow(clippy::cast_possible_truncation)]
	let db = db.clamp(MIN_VOLUME_DB, MAX_VOLUME_DB) as f32;
	client.inputs().set_volume(InputId::Name(input), Volume::Db(db)).await
}

const fn media_action(action: MediaAction) -> obws::common::MediaAction {
	match action {
		MediaAction::Play => obws::common::MediaAction::Play,
		MediaAction::Pause => obws::common::MediaAction::Pause,
		MediaAction::Stop => obws::common::MediaAction::Stop,
		MediaAction::Restart => obws::common::MediaAction::Restart,
		MediaAction::Next => obws::common::MediaAction::Next,
		MediaAction::Previous => obws::common::MediaAction::Previous,
	}
}

/// The obs-websocket request a command maps to, for error messages. Commands
/// built from several requests name the one that makes the change.
pub const fn request_name(command: &ObsCommand) -> &'static str {
	match command {
		ObsCommand::StartStream => "StartStream",
		ObsCommand::StopStream => "StopStream",
		ObsCommand::ToggleStream => "ToggleStream",
		ObsCommand::SendStreamCaption { .. } => "SendStreamCaption",
		ObsCommand::SetYouTubeStream { .. } => "SetStreamServiceSettings",
		ObsCommand::StartRecording => "StartRecord",
		ObsCommand::StopRecording => "StopRecord",
		ObsCommand::ToggleRecording => "ToggleRecord",
		ObsCommand::PauseRecording => "PauseRecord",
		ObsCommand::ResumeRecording => "ResumeRecord",
		ObsCommand::TogglePauseRecording => "ToggleRecordPause",
		ObsCommand::SplitRecording => "SplitRecordFile",
		ObsCommand::AddRecordingChapter { .. } => "CreateRecordChapter",
		ObsCommand::StartReplayBuffer => "StartReplayBuffer",
		ObsCommand::StopReplayBuffer => "StopReplayBuffer",
		ObsCommand::SaveReplay => "SaveReplayBuffer",
		ObsCommand::StartVirtualCamera => "StartVirtualCam",
		ObsCommand::StopVirtualCamera => "StopVirtualCam",
		ObsCommand::SwitchScene { .. } => "SetCurrentProgramScene",
		ObsCommand::SetPreviewScene { .. } => "SetCurrentPreviewScene",
		ObsCommand::SetStudioMode { .. } => "SetStudioModeEnabled",
		ObsCommand::TriggerTransition => "TriggerStudioModeTransition",
		ObsCommand::SetTransition { .. } => "SetCurrentSceneTransition",
		ObsCommand::SetTransitionDuration { .. } => "SetCurrentSceneTransitionDuration",
		ObsCommand::SetSourceVisible { .. } | ObsCommand::ToggleSourceVisible { .. } => "SetSceneItemEnabled",
		ObsCommand::SetMute { .. } => "SetInputMute",
		ObsCommand::ToggleMute { .. } => "ToggleInputMute",
		ObsCommand::SetVolume { .. } | ObsCommand::AdjustVolume { .. } => "SetInputVolume",
		ObsCommand::Media { .. } => "TriggerMediaInputAction",
		ObsCommand::SetFilterEnabled { .. } => "SetSourceFilterEnabled",
		ObsCommand::SetText { .. } => "SetInputSettings",
		ObsCommand::TriggerHotkey { .. } => "TriggerHotkeyByName",
		ObsCommand::GetStudio => "GetStudio",
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

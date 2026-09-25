use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Everything that can be done to OBS during a stream, one variant per intent.
///
/// Sources, scenes, inputs, filters and transitions are addressed by the names
/// OBS shows, since that is what a spoken command carries. Resolving a loose
/// name ("mic") to an exact one ("Mic/Aux") is the caller's job; the names that
/// exist come from [`ObsCommand::GetStudio`]. An unknown name fails with OBS's
/// own "not found" reason.
///
/// On the wire: `{"type": "setMute", "data": {"input": "Mic/Aux", "muted": true}}`;
/// variants without fields omit `data`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "camelCase")]
pub enum ObsCommand {
	// -- Streaming
	StartStream,
	StopStream,
	/// Replies `{"outputActive": bool}` with the new state.
	ToggleStream,
	/// Sends a CEA-608 caption over the running stream.
	SendStreamCaption {
		text: String,
	},
	/// Point OBS's stream output at the `YouTube` RTMP ingest using `stream_key`.
	///
	/// This only configures where OBS pushes video. A broadcast's title,
	/// description and privacy live on the platform side and are set through
	/// the `YouTube` Data API: OBS's `rtmp_custom` service ignores such fields.
	SetYouTubeStream {
		stream_key: StreamKey,
	},

	// -- Recording
	StartRecording,
	/// Replies `{"outputPath": string}`, the finished file.
	StopRecording,
	/// Replies `{"outputActive": bool}` with the new state.
	ToggleRecording,
	PauseRecording,
	ResumeRecording,
	/// Replies `{"outputPaused": bool}` with the new state.
	TogglePauseRecording,
	/// Finishes the current file and keeps recording into a new one.
	SplitRecording,
	/// Marks a chapter; OBS supports this only when recording to Hybrid MP4.
	AddRecordingChapter {
		name: Option<String>,
	},

	// -- Replay buffer and virtual camera
	StartReplayBuffer,
	StopReplayBuffer,
	/// Saves the last N seconds the replay buffer holds.
	SaveReplay,
	StartVirtualCamera,
	StopVirtualCamera,

	// -- Scenes and transitions
	/// Makes `scene` live (the program scene).
	SwitchScene {
		scene: String,
	},
	/// In studio mode, stages `scene` in the preview.
	SetPreviewScene {
		scene: String,
	},
	SetStudioMode {
		enabled: bool,
	},
	/// In studio mode, transitions the preview scene to program.
	TriggerTransition,
	SetTransition {
		transition: String,
	},
	SetTransitionDuration {
		millis: u64,
	},

	// -- Sources within a scene
	/// Shows or hides `source` in `scene`, or in the live scene when omitted.
	/// For a source inside a group, `scene` is the group's name (see
	/// [`StudioSnapshot::groups`]).
	SetSourceVisible {
		source: String,
		scene: Option<String>,
		visible: bool,
	},
	/// Replies `{"sceneItemEnabled": bool}` with the new state.
	ToggleSourceVisible {
		source: String,
		scene: Option<String>,
	},

	// -- Audio
	SetMute {
		input: String,
		muted: bool,
	},
	/// Replies `{"inputMuted": bool}` with the new state.
	ToggleMute {
		input: String,
	},
	/// Sets the volume in dB: `0.0` is unity, OBS's range is `-100.0..=26.0`.
	SetVolume {
		input: String,
		db: f64,
	},
	/// Changes the volume by `delta_db`, clamped to OBS's range. Replies
	/// `{"inputVolumeDb": number}` with the new level.
	AdjustVolume {
		input: String,
		delta_db: f64,
	},

	// -- Media sources (music, clips)
	Media {
		input: String,
		action: MediaAction,
	},

	// -- Filters, text, hotkeys
	SetFilterEnabled {
		source: String,
		filter: String,
		enabled: bool,
	},
	/// Replaces the text of a text source.
	SetText {
		input: String,
		text: String,
	},
	/// Presses an OBS hotkey by its internal name, as listed in
	/// [`StudioSnapshot::hotkeys`]: reaches anything bound to a hotkey in OBS,
	/// including plugins.
	TriggerHotkey {
		name: String,
	},

	// -- Query
	/// Replies with a [`StudioSnapshot`]: the current state and every name the
	/// other commands accept.
	GetStudio,
}

/// A playback action on a media source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MediaAction {
	Play,
	Pause,
	Stop,
	Restart,
	Next,
	Previous,
}

/// OBS's current state plus every name [`ObsCommand`] variants accept.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StudioSnapshot {
	pub stream: StreamState,
	pub recording: RecordingState,
	/// `None` when the replay buffer is disabled in OBS's output settings.
	pub replay_buffer_active: Option<bool>,
	pub virtual_camera_active: bool,
	pub studio_mode: bool,
	pub program_scene: String,
	/// Set only in studio mode.
	pub preview_scene: Option<String>,
	pub scenes: Vec<SceneState>,
	/// Source groups. A group also appears as a source in the scene holding it;
	/// its own sources are addressed with the group's name as `scene`.
	pub groups: Vec<SceneState>,
	/// Every input with audio.
	pub audio: Vec<AudioState>,
	/// Every media source (`ffmpeg_source`, `vlc_source`).
	pub media: Vec<MediaState>,
	pub filters: Vec<FilterState>,
	pub transitions: Vec<String>,
	pub current_transition: Option<String>,
	pub hotkeys: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamState {
	pub active: bool,
	pub reconnecting: bool,
	/// `HH:MM:SS.mmm` since the stream started.
	pub timecode: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingState {
	pub active: bool,
	pub paused: bool,
	/// `HH:MM:SS.mmm` of recorded time.
	pub timecode: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneState {
	pub name: String,
	/// In OBS's layer order, bottom layer first (the Sources panel shows it last).
	pub sources: Vec<SceneSourceState>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneSourceState {
	pub name: String,
	pub visible: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioState {
	pub input: String,
	pub muted: bool,
	pub volume_db: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaState {
	pub input: String,
	pub playback: MediaPlayback,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MediaPlayback {
	Idle,
	Loading,
	Playing,
	Paused,
	Stopped,
	Ended,
	Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FilterState {
	pub source: String,
	pub filter: String,
	pub enabled: bool,
}

/// A stream key: anyone holding one can publish to the channel it belongs to,
/// so `Debug` redacts it rather than letting it reach logs.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct StreamKey(String);

impl StreamKey {
	#[must_use]
	pub const fn new(key: String) -> Self {
		Self(key)
	}

	#[must_use]
	pub fn expose(&self) -> &str {
		&self.0
	}
}

impl std::fmt::Debug for StreamKey {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.write_str("StreamKey(***)")
	}
}

/// Represents different types of events from OBS
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "camelCase")]
pub enum ObsEvent {
	// Stream and Recording Status
	StreamStatusResponse(StreamStatusData),
	RecordingStatusResponse(RecordingStatusData),

	// Scene Management
	SceneListResponse(SceneListData),
	CurrentSceneResponse(CurrentSceneData),

	// Source Management
	SourcesListResponse(SourcesListData),
	InputListResponse(InputListData),

	// Audio Management
	AudioMuteResponse(AudioMuteData),
	AudioVolumeResponse(AudioVolumeData),

	// Profile and Collection Management
	ProfileListResponse(ProfileListData),
	CurrentProfileResponse(CurrentProfileData),
	SceneCollectionListResponse(SceneCollectionListData),
	CurrentCollectionResponse(CurrentCollectionData),

	// Virtual Camera
	VirtualCamStatusResponse(VirtualCamStatusData),

	// Replay Buffer
	ReplayBufferStatusResponse(ReplayBufferStatusData),

	// Studio Mode
	StudioModeResponse(StudioModeData),

	// Statistics
	StatsResponse(StatsData),

	// Transitions
	CurrentTransitionResponse(CurrentTransitionData),
	TransitionListResponse(TransitionListData),

	// Filters
	FilterListResponse(FilterListData),

	// Hotkeys
	HotkeyListResponse(HotkeyListData),

	// Version
	VersionResponse(VersionData),

	// Real-time events (op: 5)
	StreamStateChanged(StreamStateData),
	RecordStateChanged(RecordStateData),
	CurrentProgramSceneChanged(CurrentProgramSceneData),
	SceneItemEnableStateChanged(SceneItemEnableStateData),
	InputMuteStateChanged(InputMuteStateData),
	InputVolumeChanged(InputVolumeData),
	VirtualcamStateChanged(VirtualcamStateData),
	ReplayBufferStateChanged(ReplayBufferStateData),
	StudioModeStateChanged(StudioModeStateData),
	CurrentSceneTransitionChanged(CurrentSceneTransitionData),
	SceneTransitionStarted(SceneTransitionStartedData),
	SceneTransitionEnded(SceneTransitionEndedData),

	/// The whole studio, published on connect and after each burst of changes:
	/// a display can render this alone instead of folding individual events.
	StudioChanged(Box<StudioSnapshot>),

	// Generic events for unhandled cases
	UnknownResponse(UnknownResponseData),
	UnknownEvent(UnknownEventData),
}

// Data structures for each enum variant
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamStatusData {
	pub streaming: bool,
	pub timecode: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingStatusData {
	pub recording: bool,
	pub timecode: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneListData {
	pub scenes: Vec<SceneInfo>,
	pub current_scene: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CurrentSceneData {
	pub scene_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourcesListData {
	pub sources: Vec<SourceInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InputListData {
	pub inputs: Vec<InputInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioMuteData {
	pub input_name: String,
	pub muted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioVolumeData {
	pub input_name: String,
	pub volume_db: f64,
	pub volume_mul: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileListData {
	pub profiles: Vec<String>,
	pub current_profile: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CurrentProfileData {
	pub profile_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneCollectionListData {
	pub collections: Vec<String>,
	pub current_collection: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CurrentCollectionData {
	pub collection_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VirtualCamStatusData {
	pub active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplayBufferStatusData {
	pub active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StudioModeData {
	pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatsData {
	pub stats: ObsStats,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CurrentTransitionData {
	pub transition_name: String,
	pub transition_duration: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransitionListData {
	pub transitions: Vec<TransitionInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FilterListData {
	pub source_name: String,
	pub filters: Vec<FilterInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HotkeyListData {
	pub hotkeys: Vec<HotkeyInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionData {
	pub obs_version: String,
	pub websocket_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamStateData {
	pub streaming: bool,
	pub timecode: Option<String>,
	/// OBS's `outputState`, e.g. `OBS_WEBSOCKET_OUTPUT_STARTED`. A successful
	/// `StartStream` response only means OBS began starting the output; this
	/// is where a failed ingest connection shows up (`..._STOPPED`).
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub output_state: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordStateData {
	pub recording: bool,
	pub timecode: Option<String>,
	/// OBS's `outputState`; pausing shows up here as
	/// `OBS_WEBSOCKET_OUTPUT_PAUSED` while `recording` stays `true`.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub output_state: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CurrentProgramSceneData {
	pub scene_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneItemEnableStateData {
	pub scene_name: String,
	pub item_id: u32,
	pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InputMuteStateData {
	pub input_name: String,
	pub muted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InputVolumeData {
	pub input_name: String,
	pub volume_db: f64,
	pub volume_mul: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VirtualcamStateData {
	pub active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplayBufferStateData {
	pub active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StudioModeStateData {
	pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CurrentSceneTransitionData {
	pub transition_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneTransitionStartedData {
	pub transition_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneTransitionEndedData {
	pub transition_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnknownResponseData {
	pub request_type: String,
	pub data: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnknownEventData {
	pub event_type: String,
	pub data: Value,
}

/// Scene information structure
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneInfo {
	pub name: String,
	pub index: Option<u32>,
}

/// Source information structure
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceInfo {
	pub name: String,
	pub type_id: String,
	pub kind: String,
}

/// Input information structure
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InputInfo {
	pub name: String,
	pub kind: String,
	pub unversioned_kind: String,
}

/// Transition information structure
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransitionInfo {
	pub name: String,
	pub kind: String,
	pub fixed: bool,
}

/// Filter information structure
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FilterInfo {
	pub name: String,
	pub kind: String,
	pub index: u32,
	pub enabled: bool,
}

/// Hotkey information structure
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HotkeyInfo {
	pub name: String,
	pub description: String,
}

/// OBS Statistics structure
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObsStats {
	pub cpu_usage: f64,
	pub memory_usage: f64,
	pub available_disk_space: f64,
	pub active_fps: f64,
	pub average_frame_time: f64,
	pub render_total_frames: u64,
	pub render_missed_frames: u64,
	pub output_total_frames: u64,
	pub output_skipped_frames: u64,
	pub web_socket_session_incoming_messages: u64,
	pub web_socket_session_outgoing_messages: u64,
}

impl Default for ObsStats {
	fn default() -> Self {
		Self {
			cpu_usage: 0.0,
			memory_usage: 0.0,
			available_disk_space: 0.0,
			active_fps: 0.0,
			average_frame_time: 0.0,
			render_total_frames: 0,
			render_missed_frames: 0,
			output_total_frames: 0,
			output_skipped_frames: 0,
			web_socket_session_incoming_messages: 0,
			web_socket_session_outgoing_messages: 0,
		}
	}
}

impl ObsEvent {
	/// Check if this event should trigger a status broadcast
	#[must_use]
	pub const fn should_broadcast(&self) -> bool {
		matches!(
			self,
			Self::StreamStatusResponse(_)
				| Self::RecordingStatusResponse(_)
				| Self::SceneListResponse(_)
				| Self::CurrentSceneResponse(_)
				| Self::VirtualCamStatusResponse(_)
				| Self::ReplayBufferStatusResponse(_)
				| Self::StudioModeResponse(_)
				| Self::StreamStateChanged(_)
				| Self::RecordStateChanged(_)
				| Self::CurrentProgramSceneChanged(_)
				| Self::VirtualcamStateChanged(_)
				| Self::ReplayBufferStateChanged(_)
				| Self::StudioModeStateChanged(_)
				| Self::StudioChanged(_)
		)
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use serde_json::json;

	#[test]
	fn set_youtube_stream_accepts_the_old_payload_shape() {
		// Senders built against the earlier variant still send metadata fields;
		// they must keep deserializing, with the extras ignored.
		let command: ObsCommand = serde_json::from_value(json!({
			"type": "setYouTubeStream",
			"data": {
				"stream_key": "abcd-efgh",
				"title": "t", "description": "d", "category": "c",
				"privacy": "public", "unlisted": false, "tags": []
			}
		}))
		.unwrap();

		match command {
			ObsCommand::SetYouTubeStream { stream_key } => assert_eq!(stream_key.expose(), "abcd-efgh"),
			other => panic!("expected SetYouTubeStream, got {other:?}"),
		}
	}

	#[test]
	fn stream_key_is_redacted_in_debug_output() {
		let command = ObsCommand::SetYouTubeStream {
			stream_key: StreamKey::new("abcd-efgh".to_owned()),
		};
		let rendered = std::fmt::format(format_args!("{command:?}"));
		assert!(!rendered.contains("abcd-efgh"), "{rendered}");
	}

	#[test]
	fn commands_use_named_fields_on_the_wire() {
		let command: ObsCommand = serde_json::from_value(json!({
			"type": "setMute",
			"data": { "input": "Mic/Aux", "muted": true }
		}))
		.unwrap();
		assert!(matches!(command, ObsCommand::SetMute { ref input, muted: true } if input == "Mic/Aux"));

		let media: ObsCommand = serde_json::from_value(json!({
			"type": "media",
			"data": { "input": "Intro music", "action": "play" }
		}))
		.unwrap();
		assert!(matches!(media, ObsCommand::Media { action: MediaAction::Play, .. }));
	}

	#[test]
	fn fieldless_commands_need_no_data() {
		let command: ObsCommand = serde_json::from_value(json!({ "type": "pauseRecording" })).unwrap();
		assert!(matches!(command, ObsCommand::PauseRecording));
	}

	#[test]
	fn source_visibility_defaults_to_the_live_scene() {
		let command: ObsCommand = serde_json::from_value(json!({
			"type": "setSourceVisible",
			"data": { "source": "Terminal", "visible": true }
		}))
		.unwrap();
		assert!(matches!(command, ObsCommand::SetSourceVisible { scene: None, visible: true, .. }));
	}
}

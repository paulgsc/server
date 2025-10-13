use crate::messages::types::*;
use crate::messages::{JsonExtractor, ObsMessagesError};
use serde_json::{json, Value};
use tracing::{error, trace, warn};

type Result<T> = std::result::Result<T, ObsMessagesError>;

/// Handles parsing of OBS response messages (op: 7)
pub(crate) struct ResponseMessageParser;

impl ResponseMessageParser {
	pub fn parse(json: &Value) -> Result<ObsEvent> {
		let extractor = JsonExtractor::new(json, "Response message");
		let d = extractor.get_object("d")?;

		let val = Value::Object(d.clone());
		let d_extractor = JsonExtractor::new(&val, "Response data");
		let request_type_str = d_extractor.get_string("requestType")?;
		let request_type = ObsRequestType::from_str(request_type_str);

		trace!("Parsing response type: {}", request_type_str);

		// Check request status first
		if let Some(status) = d.get("requestStatus").and_then(Value::as_object) {
			let success = status.get("result").and_then(Value::as_bool).unwrap_or(false);
			tracing::info!("succes: {}, status: {:?}", success, status);
			if !success {
				let code = status.get("code").and_then(Value::as_u64).unwrap_or(0);
				let comment = status.get("comment").and_then(Value::as_str).unwrap_or("Unknown error");

				error!("OBS request failed - Type: {:?}, Code: {}, Comment: {}", request_type, code, comment);

				return Err(ObsMessagesError::ObsRequestFailed {
					request_type: request_type_str.to_string(),
					code,
					comment: comment.to_string(),
				});
			}
		}

		let response_data = d.get("responseData");
		if response_data.is_none() {
			trace!("No response data for request type: {}", request_type_str);
			return Ok(ObsEvent::UnknownResponse(UnknownResponseData {
				request_type: request_type_str.to_string(),
				data: json!({"success": true, "no_data": true}),
			}));
		}

		let response_data = response_data.unwrap();

		let event = match request_type {
			ObsRequestType::GetStreamStatus => Self::parse_stream_status_response(response_data)?,
			ObsRequestType::GetRecordStatus => Self::parse_record_status_response(response_data)?,
			ObsRequestType::GetSceneList => Self::parse_scene_list_response(response_data)?,
			ObsRequestType::GetCurrentProgramScene => Self::parse_current_scene_response(response_data)?,
			ObsRequestType::GetSourcesList => Self::parse_sources_list_response(response_data)?,
			ObsRequestType::GetInputList => Self::parse_input_list_response(response_data)?,
			ObsRequestType::GetInputMute => Self::parse_input_mute_response(response_data)?,
			ObsRequestType::GetInputVolume => Self::parse_input_volume_response(response_data)?,
			ObsRequestType::GetProfileList => Self::parse_profile_list_response(response_data)?,
			ObsRequestType::GetCurrentProfile => Self::parse_current_profile_response(response_data)?,
			ObsRequestType::GetSceneCollectionList => Self::parse_scene_collection_list_response(response_data)?,
			ObsRequestType::GetCurrentSceneCollection => Self::parse_current_collection_response(response_data)?,
			ObsRequestType::GetVirtualCamStatus => Self::parse_virtual_cam_status_response(response_data)?,
			ObsRequestType::GetReplayBufferStatus => Self::parse_replay_buffer_status_response(response_data)?,
			ObsRequestType::GetStudioModeEnabled => Self::parse_studio_mode_response(response_data)?,
			ObsRequestType::GetStats => Self::parse_stats_response(response_data)?,
			ObsRequestType::GetCurrentSceneTransition => Self::parse_current_transition_response(response_data)?,
			ObsRequestType::GetSceneTransitionList => Self::parse_transition_list_response(response_data)?,
			ObsRequestType::GetSourceFilterList => Self::parse_filter_list_response(response_data)?,
			ObsRequestType::GetHotkeyList => Self::parse_hotkey_list_response(response_data)?,
			ObsRequestType::GetVersion => Self::parse_version_response(response_data)?,
			_ => {
				warn!("Unknown request type: {}, creating UnknownResponse", request_type_str);
				ObsEvent::UnknownResponse(UnknownResponseData {
					request_type: request_type_str.to_string(),
					data: response_data.clone(),
				})
			}
		};

		trace!("Successfully parsed response: {:?}", event);
		Ok(event)
	}

	fn parse_stream_status_response(data: &Value) -> Result<ObsEvent> {
		let streaming = data.get("outputActive").and_then(Value::as_bool).unwrap_or(false);
		let timecode = data.get("outputTimecode").and_then(Value::as_str).unwrap_or("00:00:00.000").to_string();
		Ok(ObsEvent::StreamStatusResponse(StreamStatusData { streaming, timecode }))
	}

	fn parse_record_status_response(data: &Value) -> Result<ObsEvent> {
		let recording = data.get("outputActive").and_then(Value::as_bool).unwrap_or(false);
		let timecode = data.get("outputTimecode").and_then(Value::as_str).unwrap_or("00:00:00.000").to_string();
		Ok(ObsEvent::RecordingStatusResponse(RecordingStatusData { recording, timecode }))
	}

	fn parse_scene_list_response(data: &Value) -> Result<ObsEvent> {
		let scenes = data
			.get("scenes")
			.and_then(Value::as_array)
			.map(|arr| {
				arr
					.iter()
					.filter_map(|s| {
						Some(SceneInfo {
							name: s.get("sceneName").and_then(Value::as_str)?.to_string(),
							index: s.get("sceneIndex").and_then(Value::as_u64).map(|i| i as u32),
						})
					})
					.collect()
			})
			.unwrap_or_default();

		let current_scene = data.get("currentProgramSceneName").and_then(Value::as_str).unwrap_or("").to_string();

		Ok(ObsEvent::SceneListResponse(SceneListData { scenes, current_scene }))
	}

	fn parse_current_scene_response(data: &Value) -> Result<ObsEvent> {
		let scene_name = data.get("sceneName").and_then(Value::as_str).unwrap_or("").to_string();
		Ok(ObsEvent::CurrentSceneResponse(CurrentSceneData { scene_name }))
	}

	fn parse_sources_list_response(data: &Value) -> Result<ObsEvent> {
		let sources = data
			.get("sources")
			.and_then(Value::as_array)
			.map(|arr| {
				arr
					.iter()
					.filter_map(|s| {
						Some(SourceInfo {
							name: s.get("sourceName").and_then(Value::as_str)?.to_string(),
							type_id: s.get("sourceType").and_then(Value::as_str).unwrap_or("").to_string(),
							kind: s.get("sourceKind").and_then(Value::as_str).unwrap_or("").to_string(),
						})
					})
					.collect()
			})
			.unwrap_or_default();

		Ok(ObsEvent::SourcesListResponse(SourcesListData { sources }))
	}

	fn parse_input_list_response(data: &Value) -> Result<ObsEvent> {
		let inputs = data
			.get("inputs")
			.and_then(Value::as_array)
			.map(|arr| {
				arr
					.iter()
					.filter_map(|i| {
						Some(InputInfo {
							name: i.get("inputName").and_then(Value::as_str)?.to_string(),
							kind: i.get("inputKind").and_then(Value::as_str).unwrap_or("").to_string(),
							unversioned_kind: i.get("unversionedInputKind").and_then(Value::as_str).unwrap_or("").to_string(),
						})
					})
					.collect()
			})
			.unwrap_or_default();

		Ok(ObsEvent::InputListResponse(InputListData { inputs }))
	}

	fn parse_input_mute_response(data: &Value) -> Result<ObsEvent> {
		let input_name = data.get("inputName").and_then(Value::as_str).unwrap_or("").to_string();
		let muted = data.get("inputMuted").and_then(Value::as_bool).unwrap_or(false);
		Ok(ObsEvent::AudioMuteResponse(AudioMuteData { input_name, muted }))
	}

	fn parse_input_volume_response(data: &Value) -> Result<ObsEvent> {
		let input_name = data.get("inputName").and_then(Value::as_str).unwrap_or("").to_string();
		let volume_db = data.get("inputVolumeDb").and_then(Value::as_f64).unwrap_or(0.0);
		let volume_mul = data.get("inputVolumeMul").and_then(Value::as_f64).unwrap_or(1.0);
		Ok(ObsEvent::AudioVolumeResponse(AudioVolumeData {
			input_name,
			volume_db,
			volume_mul,
		}))
	}

	fn parse_profile_list_response(data: &Value) -> Result<ObsEvent> {
		let profiles = data
			.get("profiles")
			.and_then(Value::as_array)
			.map(|arr| arr.iter().filter_map(|p| p.as_str().map(String::from)).collect())
			.unwrap_or_default();
		let current_profile = data.get("currentProfileName").and_then(Value::as_str).unwrap_or("").to_string();
		Ok(ObsEvent::ProfileListResponse(ProfileListData { profiles, current_profile }))
	}

	fn parse_current_profile_response(data: &Value) -> Result<ObsEvent> {
		let profile_name = data.get("profileName").and_then(Value::as_str).unwrap_or("").to_string();
		Ok(ObsEvent::CurrentProfileResponse(CurrentProfileData { profile_name }))
	}

	fn parse_scene_collection_list_response(data: &Value) -> Result<ObsEvent> {
		let collections = data
			.get("sceneCollections")
			.and_then(Value::as_array)
			.map(|arr| arr.iter().filter_map(|c| c.as_str().map(String::from)).collect())
			.unwrap_or_default();
		let current_collection = data.get("currentSceneCollectionName").and_then(Value::as_str).unwrap_or("").to_string();
		Ok(ObsEvent::SceneCollectionListResponse(SceneCollectionListData { collections, current_collection }))
	}

	fn parse_current_collection_response(data: &Value) -> Result<ObsEvent> {
		let collection_name = data.get("sceneCollectionName").and_then(Value::as_str).unwrap_or("").to_string();
		Ok(ObsEvent::CurrentCollectionResponse(CurrentCollectionData { collection_name }))
	}

	fn parse_virtual_cam_status_response(data: &Value) -> Result<ObsEvent> {
		let active = data.get("outputActive").and_then(Value::as_bool).unwrap_or(false);
		Ok(ObsEvent::VirtualCamStatusResponse(VirtualCamStatusData { active }))
	}

	fn parse_replay_buffer_status_response(data: &Value) -> Result<ObsEvent> {
		let active = data.get("outputActive").and_then(Value::as_bool).unwrap_or(false);
		Ok(ObsEvent::ReplayBufferStatusResponse(ReplayBufferStatusData { active }))
	}

	fn parse_studio_mode_response(data: &Value) -> Result<ObsEvent> {
		let enabled = data.get("studioModeEnabled").and_then(Value::as_bool).unwrap_or(false);
		Ok(ObsEvent::StudioModeResponse(StudioModeData { enabled }))
	}

	fn parse_stats_response(data: &Value) -> Result<ObsEvent> {
		let stats = ObsStats {
			cpu_usage: data.get("cpuUsage").and_then(Value::as_f64).unwrap_or(0.0),
			memory_usage: data.get("memoryUsage").and_then(Value::as_f64).unwrap_or(0.0),
			available_disk_space: data.get("availableDiskSpace").and_then(Value::as_f64).unwrap_or(0.0),
			active_fps: data.get("activeFps").and_then(Value::as_f64).unwrap_or(0.0),
			average_frame_time: data.get("averageFrameTime").and_then(Value::as_f64).unwrap_or(0.0),
			render_total_frames: data.get("renderTotalFrames").and_then(Value::as_u64).unwrap_or(0),
			render_missed_frames: data.get("renderMissedFrames").and_then(Value::as_u64).unwrap_or(0),
			output_total_frames: data.get("outputTotalFrames").and_then(Value::as_u64).unwrap_or(0),
			output_skipped_frames: data.get("outputSkippedFrames").and_then(Value::as_u64).unwrap_or(0),
			web_socket_session_incoming_messages: data.get("webSocketSessionIncomingMessages").and_then(Value::as_u64).unwrap_or(0),
			web_socket_session_outgoing_messages: data.get("webSocketSessionOutgoingMessages").and_then(Value::as_u64).unwrap_or(0),
		};
		Ok(ObsEvent::StatsResponse(StatsData { stats }))
	}

	fn parse_current_transition_response(data: &Value) -> Result<ObsEvent> {
		let transition_name = data.get("transitionName").and_then(Value::as_str).unwrap_or("").to_string();
		let transition_duration = data.get("transitionDuration").and_then(Value::as_u64).unwrap_or(0) as u32;
		Ok(ObsEvent::CurrentTransitionResponse(CurrentTransitionData {
			transition_name,
			transition_duration,
		}))
	}

	fn parse_transition_list_response(data: &Value) -> Result<ObsEvent> {
		let transitions = data
			.get("transitions")
			.and_then(Value::as_array)
			.map(|arr| {
				arr
					.iter()
					.filter_map(|t| {
						Some(TransitionInfo {
							name: t.get("transitionName").and_then(Value::as_str)?.to_string(),
							kind: t.get("transitionKind").and_then(Value::as_str).unwrap_or("").to_string(),
							fixed: t.get("transitionFixed").and_then(Value::as_bool).unwrap_or(false),
						})
					})
					.collect()
			})
			.unwrap_or_default();

		Ok(ObsEvent::TransitionListResponse(TransitionListData { transitions }))
	}

	fn parse_filter_list_response(data: &Value) -> Result<ObsEvent> {
		let source_name = data.get("sourceName").and_then(Value::as_str).unwrap_or("").to_string();
		let filters = data
			.get("filters")
			.and_then(Value::as_array)
			.map(|arr| {
				arr
					.iter()
					.filter_map(|f| {
						Some(FilterInfo {
							name: f.get("filterName").and_then(Value::as_str)?.to_string(),
							kind: f.get("filterKind").and_then(Value::as_str).unwrap_or("").to_string(),
							index: f.get("filterIndex").and_then(Value::as_u64).unwrap_or(0) as u32,
							enabled: f.get("filterEnabled").and_then(Value::as_bool).unwrap_or(false),
						})
					})
					.collect()
			})
			.unwrap_or_default();

		Ok(ObsEvent::FilterListResponse(FilterListData { source_name, filters }))
	}

	fn parse_hotkey_list_response(data: &Value) -> Result<ObsEvent> {
		let hotkeys = data
			.get("hotkeys")
			.and_then(Value::as_array)
			.map(|arr| {
				arr
					.iter()
					.filter_map(|h| {
						Some(HotkeyInfo {
							name: h.get("hotkeyName").and_then(Value::as_str)?.to_string(),
							description: h.get("hotkeyDescription").and_then(Value::as_str).unwrap_or("").to_string(),
						})
					})
					.collect()
			})
			.unwrap_or_default();

		Ok(ObsEvent::HotkeyListResponse(HotkeyListData { hotkeys }))
	}

	fn parse_version_response(data: &Value) -> Result<ObsEvent> {
		let obs_version = data.get("obsVersion").and_then(Value::as_str).unwrap_or("").to_string();
		let websocket_version = data.get("obsWebSocketVersion").and_then(Value::as_str).unwrap_or("").to_string();
		Ok(ObsEvent::VersionResponse(VersionData { obs_version, websocket_version }))
	}
}

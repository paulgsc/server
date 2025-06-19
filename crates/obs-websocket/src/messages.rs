use crate::ObsStatus;
use futures_util::{sink::SinkExt, stream::SplitSink};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::error::Error;
use std::sync::Arc;
use tokio::net::TcpStream;
use tokio::sync::RwLock;
use tokio_tungstenite::{tungstenite::protocol::Message as TungsteniteMessage, MaybeTlsStream};
use tracing::{debug, info, warn};

/// Represents different types of events from OBS
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum ObsEvent {
	// Response events (op: 7)
	StreamStatusResponse { streaming: bool, timecode: String },
	RecordingStatusResponse { recording: bool, timecode: String },
	SceneListResponse { scenes: Vec<String>, current_scene: String },

	// Real-time events (op: 5)
	StreamStateChanged { streaming: bool, timecode: Option<String> },
	RecordStateChanged { recording: bool, timecode: Option<String> },
	CurrentProgramSceneChanged { scene_name: String },

	// Generic events for unhandled cases
	UnknownResponse { request_type: String, data: Value },
	UnknownEvent { event_type: String, data: Value },

	// Connection events
	Hello { obs_version: String },
	Identified,
}

impl ObsEvent {
	/// Check if this event should trigger a status broadcast
	pub fn should_broadcast(&self) -> bool {
		match self {
			ObsEvent::StreamStatusResponse { .. }
			| ObsEvent::RecordingStatusResponse { .. }
			| ObsEvent::SceneListResponse { .. }
			| ObsEvent::StreamStateChanged { .. }
			| ObsEvent::RecordStateChanged { .. }
			| ObsEvent::CurrentProgramSceneChanged { .. } => true,
			_ => false,
		}
	}
}

/// Wait for hello message from OBS and send initial state requests
pub async fn fetch_init_state(
	sink: &mut SplitSink<tokio_tungstenite::WebSocketStream<MaybeTlsStream<TcpStream>>, TungsteniteMessage>,
) -> Result<(), Box<dyn Error + Send + Sync>> {
	let requests = [("GetSceneList", "scenes-init"), ("GetStreamStatus", "stream-init"), ("GetRecordStatus", "recording-init")];

	for (request_type, request_id) in requests {
		let request = json!({
			"op": 6,
			"d": {
				"requestType": request_type,
				"requestId": request_id
			}
		});

		debug!("Requesting initial {}: {}", request_type, request);
		sink.send(TungsteniteMessage::Text(request.to_string().into())).await?;
	}

	sink.flush().await?;
	info!("Sent all initial state requests");

	Ok(())
}

/// Parse OBS message and return appropriate event
pub async fn process_obs_message(text: String, status: Arc<RwLock<ObsStatus>>) -> Result<ObsEvent, Box<dyn Error + Send + Sync>> {
	let json: Value = serde_json::from_str(&text)?;
	let op = json.get("op").and_then(Value::as_u64).unwrap_or(99);

	debug!("Processing OBS message with op: {}", op);

	let event = match op {
		0 => {
			// Hello message
			let d = json.get("d").and_then(Value::as_object).ok_or("Missing 'd' field in Hello message")?;
			let obs_version = d.get("obsWebSocketVersion").and_then(Value::as_str).unwrap_or("unknown").to_string();

			ObsEvent::Hello { obs_version }
		}
		2 => {
			// Identified
			ObsEvent::Identified
		}
		5 => {
			// Event from OBS
			parse_obs_event(&json)?
		}
		7 => {
			// Request response from OBS
			parse_obs_response(&json)?
		}
		_ => {
			warn!("Unknown OBS operation code: {}", op);
			return Err(format!("Unknown op code: {}", op).into());
		}
	};

	// Update status if needed
	if event.should_broadcast() {
		update_status_from_event(&event, status).await;
	}

	Ok(event)
}

/// Parse OBS event messages (op: 5)
fn parse_obs_event(json: &Value) -> Result<ObsEvent, Box<dyn Error + Send + Sync>> {
	let d = json.get("d").and_then(Value::as_object).ok_or("Missing 'd' field in event message")?;

	let event_type = d.get("eventType").and_then(Value::as_str).ok_or("Missing eventType in event message")?;

	let event = match event_type {
		"StreamStateChanged" => {
			let streaming = d.get("outputActive").and_then(Value::as_bool).unwrap_or(false);
			let timecode = if streaming {
				d.get("outputTimecode").and_then(Value::as_str).map(String::from)
			} else {
				Some("00:00:00.000".to_string())
			};

			ObsEvent::StreamStateChanged { streaming, timecode }
		}
		"RecordStateChanged" => {
			let recording = d.get("outputActive").and_then(Value::as_bool).unwrap_or(false);
			let timecode = if recording {
				d.get("outputTimecode").and_then(Value::as_str).map(String::from)
			} else {
				Some("00:00:00.000".to_string())
			};

			ObsEvent::RecordStateChanged { recording, timecode }
		}
		"CurrentProgramSceneChanged" => {
			let scene_name = d
				.get("sceneName")
				.and_then(Value::as_str)
				.ok_or("Missing sceneName in CurrentProgramSceneChanged event")?
				.to_string();

			ObsEvent::CurrentProgramSceneChanged { scene_name }
		}
		_ => {
			debug!("Unhandled event type: {}", event_type);
			ObsEvent::UnknownEvent {
				event_type: event_type.to_string(),
				data: d.clone().into(),
			}
		}
	};

	Ok(event)
}

/// Parse OBS response messages (op: 7)
fn parse_obs_response(json: &Value) -> Result<ObsEvent, Box<dyn Error + Send + Sync>> {
	let d = json.get("d").and_then(Value::as_object).ok_or("Missing 'd' field in response message")?;

	let request_type = d.get("requestType").and_then(Value::as_str).ok_or("Missing requestType in response message")?;

	let response_data = d.get("responseData").ok_or("Missing responseData in response message")?;

	let event = match request_type {
		"GetStreamStatus" => {
			let streaming = response_data.get("outputActive").and_then(Value::as_bool).unwrap_or(false);
			let timecode = response_data.get("outputTimecode").and_then(Value::as_str).unwrap_or("00:00:00.000").to_string();

			ObsEvent::StreamStatusResponse { streaming, timecode }
		}
		"GetRecordStatus" => {
			let recording = response_data.get("outputActive").and_then(Value::as_bool).unwrap_or(false);
			let timecode = response_data.get("outputTimecode").and_then(Value::as_str).unwrap_or("00:00:00.000").to_string();

			ObsEvent::RecordingStatusResponse { recording, timecode }
		}
		"GetSceneList" => {
			let scenes = response_data
				.get("scenes")
				.and_then(Value::as_array)
				.map(|arr| arr.iter().filter_map(|s| s.get("sceneName").and_then(Value::as_str).map(String::from)).collect())
				.unwrap_or_default();

			let current_scene = response_data.get("currentProgramSceneName").and_then(Value::as_str).unwrap_or("").to_string();

			ObsEvent::SceneListResponse { scenes, current_scene }
		}
		_ => {
			debug!("Unhandled response type: {}", request_type);
			ObsEvent::UnknownResponse {
				request_type: request_type.to_string(),
				data: response_data.clone(),
			}
		}
	};

	Ok(event)
}

/// Update ObsStatus based on the event
async fn update_status_from_event(event: &ObsEvent, status: Arc<RwLock<ObsStatus>>) {
	let mut status_guard = status.write().await;

	match event {
		ObsEvent::StreamStatusResponse { streaming, timecode }
		| ObsEvent::StreamStateChanged {
			streaming,
			timecode: Some(timecode),
		} => {
			status_guard.streaming = *streaming;
			status_guard.stream_timecode = timecode.clone();
			debug!("Updated stream status: streaming={}, timecode={}", streaming, timecode);
		}
		ObsEvent::StreamStateChanged { streaming, timecode: None } => {
			status_guard.streaming = *streaming;
			if !streaming {
				status_guard.stream_timecode = "00:00:00.000".to_string();
			}
			debug!("Updated stream status: streaming={}", streaming);
		}
		ObsEvent::RecordingStatusResponse { recording, timecode }
		| ObsEvent::RecordStateChanged {
			recording,
			timecode: Some(timecode),
		} => {
			status_guard.recording = *recording;
			status_guard.recording_timecode = timecode.clone();
			debug!("Updated recording status: recording={}, timecode={}", recording, timecode);
		}
		ObsEvent::RecordStateChanged { recording, timecode: None } => {
			status_guard.recording = *recording;
			if !recording {
				status_guard.recording_timecode = "00:00:00.000".to_string();
			}
			debug!("Updated recording status: recording={}", recording);
		}
		ObsEvent::SceneListResponse { scenes, current_scene } => {
			status_guard.scenes = scenes.clone();
			status_guard.current_scene = current_scene.clone();
			debug!("Updated scenes: {} scenes, current: {}", scenes.len(), current_scene);
		}
		ObsEvent::CurrentProgramSceneChanged { scene_name } => {
			status_guard.current_scene = scene_name.clone();
			debug!("Scene changed to: {}", scene_name);
		}
		_ => {
			// No status update needed for other events
		}
	}
}

/// Legacy function to process messages with broadcasting (for backward compatibility)
// pub async fn process_obs_message_with_broadcast(
// 	text: String,
// 	status: Arc<RwLock<ObsStatus>>,
// 	broadcaster: async_broadcast::Sender<ObsStatus>,
// ) -> Result<ObsEvent, Box<dyn Error + Send + Sync>> {
// 	let event = process_obs_message(text, status.clone()).await?;
//
// 	// Broadcast if needed
// 	if event.should_broadcast() {
// 		let status_snapshot = status.read().await.clone();
// 		if let Err(e) = broadcaster.broadcast(status_snapshot).await {
// 			warn!("Failed to broadcast status update: {}", e);
// 		}
// 	}
//
// 	Ok(event)
// }

#[cfg(test)]
mod tests {
	use super::*;
	use serde_json::json;

	#[test]
	fn test_parse_stream_state_changed() {
		let message = json!({
			"op": 5,
			"d": {
				"eventType": "StreamStateChanged",
				"outputActive": true,
				"outputTimecode": "00:01:23.456"
			}
		});

		let event = parse_obs_event(&message).unwrap();
		match event {
			ObsEvent::StreamStateChanged { streaming, timecode } => {
				assert!(streaming);
				assert_eq!(timecode, Some("00:01:23.456".to_string()));
			}
			_ => panic!("Wrong event type"),
		}
	}

	#[test]
	fn test_parse_stream_status_response() {
		let message = json!({
			"op": 7,
			"d": {
				"requestType": "GetStreamStatus",
				"responseData": {
					"outputActive": false,
					"outputTimecode": "00:00:00.000"
				}
			}
		});

		let event = parse_obs_response(&message).unwrap();
		match event {
			ObsEvent::StreamStatusResponse { streaming, timecode } => {
				assert!(!streaming);
				assert_eq!(timecode, "00:00:00.000");
			}
			_ => panic!("Wrong event type"),
		}
	}

	#[test]
	fn test_scene_list_response() {
		let message = json!({
			"op": 7,
			"d": {
				"requestType": "GetSceneList",
				"responseData": {
					"scenes": [
					{"sceneName": "Scene 1"},
					{"sceneName": "Scene 2"}
					],
					"currentProgramSceneName": "Scene 1"
				}
			}
		});

		let event = parse_obs_response(&message).unwrap();
		match event {
			ObsEvent::SceneListResponse { scenes, current_scene } => {
				assert_eq!(scenes, vec!["Scene 1", "Scene 2"]);
				assert_eq!(current_scene, "Scene 1");
			}
			_ => panic!("Wrong event type"),
		}
	}
}

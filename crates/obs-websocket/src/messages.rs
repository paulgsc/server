use crate::ObsStatus;
use futures_util::{sink::SinkExt, stream::SplitSink};
use serde_json::{json, Value};
use std::error::Error;
use std::sync::Arc;
use tokio::net::TcpStream;
use tokio::sync::RwLock;
use tokio_tungstenite::{tungstenite::protocol::Message as TungsteniteMessage, MaybeTlsStream};
use tracing::info;

// Wait for hello message from OBS
pub async fn fetch_init_state(
	sink: &mut SplitSink<tokio_tungstenite::WebSocketStream<MaybeTlsStream<TcpStream>>, TungsteniteMessage>,
) -> Result<(), Box<dyn Error + Send + Sync>> {
	let scene_req = json!({
		"op": 6,
		"d": {
			"requestType": "GetSceneList",
			"requestId": "scenes-1"
		}
	});
	info!("Requesting initial scene list...");
	sink.send(TungsteniteMessage::Text(scene_req.to_string().into())).await?;

	let stream_req = json!({
		"op": 6,
		"d": {
			"requestType": "GetStreamStatus",
			"requestId": "initial-stream-1"
		}
	});
	info!("Requesting initial stream status...");
	sink.send(TungsteniteMessage::Text(stream_req.to_string().into())).await?;

	let recording_req = json!({
		"op": 6,
		"d": {
			"requestType": "GetRecordStatus",
			"requestId": "initial-recording-1"
		}
	});
	info!("Requesting initial recording status...");
	sink.send(TungsteniteMessage::Text(recording_req.to_string().into())).await?;

	Ok(())
}

// Process messages from OBS WebSocket
pub async fn process_obs_message(text: String, status: Arc<RwLock<ObsStatus>>, broadcaster: async_broadcast::Sender<ObsStatus>) -> Result<(), Box<dyn Error + Send + Sync>> {
	let json: Value = serde_json::from_str(&text)?;
	let op = json.get("op").and_then(Value::as_u64).unwrap_or(99);

	match op {
		7 => {
			let d = json.get("d").and_then(Value::as_object).unwrap();
			let request_type = d.get("requestType").and_then(Value::as_str).unwrap_or("");

			match request_type {
				"GetStreamStatus" => {
					if let Some(data) = d.get("responseData") {
						let mut status_guard = status.write().await;
						status_guard.streaming = data.get("outputActive").and_then(Value::as_bool).unwrap_or(false);
						status_guard.stream_timecode = data.get("outputTimecode").and_then(Value::as_str).unwrap_or("00:00:00.000").to_string();

						// Broadcast updated status
						let _ = broadcaster.broadcast(status_guard.clone()).await;
					}
				}
				"GetRecordStatus" => {
					if let Some(data) = d.get("responseData") {
						let mut status_guard = status.write().await;
						status_guard.recording = data.get("outputActive").and_then(Value::as_bool).unwrap_or(false);
						status_guard.recording_timecode = data.get("outputTimecode").and_then(Value::as_str).unwrap_or("00:00:00.000").to_string();

						// Broadcast updated status
						let _ = broadcaster.broadcast(status_guard.clone()).await;
					}
				}
				"GetSceneList" => {
					if let Some(data) = d.get("responseData") {
						let mut status_guard = status.write().await;

						// Extract scenes
						if let Some(scenes) = data.get("scenes").and_then(Value::as_array) {
							status_guard.scenes = scenes.iter().filter_map(|s| s.get("sceneName").and_then(Value::as_str).map(String::from)).collect();
						}

						// Get current scene
						if let Some(current) = data.get("currentProgramSceneName").and_then(Value::as_str) {
							status_guard.current_scene = current.to_string();
						}

						// Broadcast updated status
						let _ = broadcaster.broadcast(status_guard.clone()).await;
					}
				}
				_ => {}
			}
		}
		5 => {
			// Event from OBS
			let d = json.get("d").and_then(Value::as_object).unwrap();
			let event_type = d.get("eventType").and_then(Value::as_str).unwrap_or("");

			match event_type {
				"StreamStateChanged" => {
					let mut status_guard = status.write().await;
					let output_active = d.get("outputActive").and_then(Value::as_bool).unwrap_or(false);
					status_guard.streaming = output_active;

					if !output_active {
						status_guard.stream_timecode = "00:00:00.000".to_string();
					}

					// Broadcast updated status
					let _ = broadcaster.broadcast(status_guard.clone()).await;
				}
				"RecordStateChanged" => {
					let mut status_guard = status.write().await;
					let output_active = d.get("outputActive").and_then(Value::as_bool).unwrap_or(false);
					status_guard.recording = output_active;

					if !output_active {
						status_guard.recording_timecode = "00:00:00.000".to_string();
					}

					// Broadcast updated status
					let _ = broadcaster.broadcast(status_guard.clone()).await;
				}
				"CurrentProgramSceneChanged" => {
					let mut status_guard = status.write().await;
					if let Some(scene_name) = d.get("sceneName").and_then(Value::as_str) {
						status_guard.current_scene = scene_name.to_string();

						// Broadcast updated status
						let _ = broadcaster.broadcast(status_guard.clone()).await;
					}
				}
				_ => {}
			}
		}
		_ => {}
	}

	Ok(())
}

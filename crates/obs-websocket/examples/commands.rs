#![allow(dead_code)]

use obs_websocket::*;
use serde_json::json;
use tokio::time::{sleep, Duration};
use tokio_util::sync::CancellationToken;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
	tracing_subscriber::fmt().with_max_level(tracing::Level::INFO).init();

	// Run the OBS commands showcase
	showcase_obs_commands().await?;
	Ok(())
}

pub async fn showcase_obs_commands() -> Result<(), Box<dyn std::error::Error>> {
	tracing::info!("🎬 Starting OBS Commands Showcase");

	let obs_config = ObsConfig::default();
	let obs_manager = ObsWebSocketManager::new(obs_config, RetryConfig::default());

	// Create cancellation token for graceful shutdown
	let cancel_token = CancellationToken::new();
	let cancel_clone = cancel_token.clone();

	// Setup shutdown handler
	tokio::spawn(async move {
		let _ = tokio::signal::ctrl_c().await;
		tracing::info!("🛑 Shutdown signal received");
		cancel_clone.cancel();
	});

	// Connect to OBS with basic polling configuration
	let polling_requests = PollingConfig::default();
	let request_slice: Box<[(ObsRequestType, PollingFrequency)]> = polling_requests.into();

	match obs_manager.connect(&request_slice).await {
		Ok(()) => {
			tracing::info!("✅ Connected to OBS WebSocket successfully");

			// Show connection info
			if let Ok(info) = obs_manager.connection_info().await {
				tracing::info!("📊 Connection Info: State={:?}", info.state);
			}

			// Run command demonstrations
			demonstrate_commands(&obs_manager, &cancel_token).await?;
		}
		Err(e) => {
			tracing::error!("❌ Failed to connect to OBS: {}", e);
			return Err(e.into());
		}
	}

	// Clean disconnect
	let _ = obs_manager.disconnect().await;
	tracing::info!("🏁 OBS Commands Showcase completed");

	Ok(())
}

async fn demonstrate_commands(obs_manager: &ObsWebSocketManager, cancel_token: &CancellationToken) -> Result<(), Box<dyn std::error::Error>> {
	tracing::info!("🎭 Starting command demonstrations...");

	// Run demonstrations sequentially to avoid function pointer type issues
	let demonstrations = [
		"🎥 Stream Controls",
		"📹 Recording Controls",
		"🎬 Scene Management",
		"🔊 Audio Controls",
		"🛠️ Studio Features",
		"🔧 Custom Commands",
		"📡 System Information",
	];

	for (i, category) in demonstrations.iter().enumerate() {
		if cancel_token.is_cancelled() {
			tracing::info!("⏹️ Demonstration cancelled by user");
			break;
		}

		tracing::info!("🔄 Running: {}", category);

		let result = match i {
			// 0 => demo_stream_controls(obs_manager).await,
			1 => demo_recording_controls(obs_manager).await,
			// 2 => demo_scene_management(obs_manager).await,
			// 3 => demo_audio_controls(obs_manager).await,
			// 4 => demo_studio_features(obs_manager).await,
			// 5 => demo_custom_commands(obs_manager).await,
			// 6 => demo_system_info(obs_manager).await,
			_ => Ok(()),
		};

		match result {
			Ok(()) => tracing::info!("✅ {} completed successfully", category),
			Err(e) => tracing::warn!("⚠️ {} encountered error: {}", category, e),
		}

		// Brief pause between demonstrations
		sleep(Duration::from_millis(500)).await;
	}

	Ok(())
}

// Stream control demonstrations
async fn demo_stream_controls(obs_manager: &ObsWebSocketManager) -> Result<(), Box<dyn std::error::Error>> {
	tracing::info!("  📺 Demonstrating stream controls...");

	// Check if we can start streaming (won't actually start without proper setup)
	match obs_manager.execute_command(ObsCommand::StartStream).await {
		Ok(()) => tracing::info!("  ✅ Start stream command executed"),
		Err(e) => tracing::debug!("  ℹ️ Start stream: {} (expected if not configured)", e),
	}

	sleep(Duration::from_millis(100)).await;

	// Stop streaming command
	match obs_manager.execute_command(ObsCommand::StopStream).await {
		Ok(()) => tracing::info!("  ✅ Stop stream command executed"),
		Err(e) => tracing::debug!("  ℹ️ Stop stream: {} (expected if not streaming)", e),
	}

	Ok(())
}

// Recording control demonstrations
async fn demo_recording_controls(obs_manager: &ObsWebSocketManager) -> Result<(), Box<dyn std::error::Error>> {
	tracing::info!("  🎬 Demonstrating recording controls...");

	// Start recording
	match obs_manager.execute_command(ObsCommand::StartRecording).await {
		Ok(()) => {
			tracing::info!("  ✅ Recording started");

			// Let it record for a brief moment
			sleep(Duration::from_millis(200)).await;

			// Stop recording
			match obs_manager.execute_command(ObsCommand::StopRecording).await {
				Ok(()) => tracing::info!("  ✅ Recording stopped"),
				Err(e) => tracing::warn!("  ⚠️ Stop recording failed: {}", e),
			}
		}
		Err(e) => tracing::debug!("  ℹ️ Start recording: {} (may need setup)", e),
	}

	Ok(())
}

// Scene management demonstrations
async fn demo_scene_management(obs_manager: &ObsWebSocketManager) -> Result<(), Box<dyn std::error::Error>> {
	tracing::info!("  🎭 Demonstrating scene management...");

	let test_scenes = ["Scene", "Scene 2", "Gaming Scene", "Webcam Scene"];

	for scene_name in test_scenes.iter() {
		match obs_manager.execute_command(ObsCommand::SwitchScene(scene_name.to_string())).await {
			Ok(()) => {
				tracing::info!("  ✅ Switched to scene: {}", scene_name);
				sleep(Duration::from_millis(100)).await;
			}
			Err(e) => tracing::debug!("  ℹ️ Scene '{}': {} (may not exist)", scene_name, e),
		}
	}

	Ok(())
}

// Audio control demonstrations
async fn demo_audio_controls(obs_manager: &ObsWebSocketManager) -> Result<(), Box<dyn std::error::Error>> {
	tracing::info!("  🔊 Demonstrating audio controls...");

	let audio_sources = ["Mic/Aux", "Desktop Audio", "Microphone"];

	for source in audio_sources.iter() {
		// Test muting
		match obs_manager.execute_command(ObsCommand::SetInputMute(source.to_string(), true)).await {
			Ok(()) => {
				tracing::info!("  🔇 Muted: {}", source);
				sleep(Duration::from_millis(50)).await;

				// Unmute
				match obs_manager.execute_command(ObsCommand::SetInputMute(source.to_string(), false)).await {
					Ok(()) => tracing::info!("  🔊 Unmuted: {}", source),
					Err(e) => tracing::debug!("  ℹ️ Unmute {}: {}", source, e),
				}
			}
			Err(e) => tracing::debug!("  ℹ️ Audio source '{}': {} (may not exist)", source, e),
		}

		// Test volume adjustment
		let volumes = [0.5, 0.8, 1.0];
		for &volume in volumes.iter() {
			match obs_manager.execute_command(ObsCommand::SetInputVolume(source.to_string(), volume)).await {
				Ok(()) => tracing::info!("  🎚️ Set {} volume to {:.1}", source, volume),
				Err(e) => tracing::debug!("  ℹ️ Volume {}: {} (may not exist)", source, e),
			}
			sleep(Duration::from_millis(30)).await;
		}
	}

	Ok(())
}

// Studio features demonstrations
async fn demo_studio_features(obs_manager: &ObsWebSocketManager) -> Result<(), Box<dyn std::error::Error>> {
	tracing::info!("  🛠️ Demonstrating studio features...");

	// Toggle Studio Mode
	match obs_manager.execute_command(ObsCommand::ToggleStudioMode).await {
		Ok(()) => {
			tracing::info!("  ✅ Toggled Studio Mode");
			sleep(Duration::from_millis(100)).await;

			// Toggle it back
			let _ = obs_manager.execute_command(ObsCommand::ToggleStudioMode).await;
			tracing::info!("  ✅ Toggled Studio Mode back");
		}
		Err(e) => tracing::debug!("  ℹ️ Studio Mode: {}", e),
	}

	// Toggle Virtual Camera
	match obs_manager.execute_command(ObsCommand::ToggleVirtualCamera).await {
		Ok(()) => {
			tracing::info!("  📷 Toggled Virtual Camera");
			sleep(Duration::from_millis(100)).await;

			// Toggle it back
			let _ = obs_manager.execute_command(ObsCommand::ToggleVirtualCamera).await;
			tracing::info!("  📷 Toggled Virtual Camera back");
		}
		Err(e) => tracing::debug!("  ℹ️ Virtual Camera: {}", e),
	}

	// Toggle Replay Buffer
	match obs_manager.execute_command(ObsCommand::ToggleReplayBuffer).await {
		Ok(()) => {
			tracing::info!("  ⏪ Toggled Replay Buffer");
			sleep(Duration::from_millis(100)).await;

			// Toggle it back
			let _ = obs_manager.execute_command(ObsCommand::ToggleReplayBuffer).await;
			tracing::info!("  ⏪ Toggled Replay Buffer back");
		}
		Err(e) => tracing::debug!("  ℹ️ Replay Buffer: {}", e),
	}

	Ok(())
}

// Custom command demonstrations
async fn demo_custom_commands(obs_manager: &ObsWebSocketManager) -> Result<(), Box<dyn std::error::Error>> {
	tracing::info!("  🔧 Demonstrating custom commands...");

	// Custom command to get version info
	let version_request = json!({
			"requestType": "GetVersion",
			"requestId": "version-check"
	});

	match obs_manager.execute_command(ObsCommand::Custom(version_request)).await {
		Ok(()) => tracing::info!("  ✅ Custom version request executed"),
		Err(e) => tracing::debug!("  ℹ️ Custom version: {}", e),
	}

	// Custom command to get stats
	let stats_request = json!({
			"requestType": "GetStats",
			"requestId": "stats-check"
	});

	match obs_manager.execute_command(ObsCommand::Custom(stats_request)).await {
		Ok(()) => tracing::info!("  ✅ Custom stats request executed"),
		Err(e) => tracing::debug!("  ℹ️ Custom stats: {}", e),
	}

	// Custom command to list scenes
	let scenes_request = json!({
			"requestType": "GetSceneList",
			"requestId": "scenes-list"
	});

	match obs_manager.execute_command(ObsCommand::Custom(scenes_request)).await {
		Ok(()) => tracing::info!("  ✅ Custom scene list request executed"),
		Err(e) => tracing::debug!("  ℹ️ Custom scenes: {}", e),
	}

	Ok(())
}

// System information demonstrations
async fn demo_system_info(obs_manager: &ObsWebSocketManager) -> Result<(), Box<dyn std::error::Error>> {
	tracing::info!("  📡 Checking system information...");

	// Check connection health
	match obs_manager.is_healthy().await {
		Ok(healthy) => tracing::info!("  💓 Connection healthy: {}", healthy),
		Err(e) => tracing::warn!("  ⚠️ Health check failed: {}", e),
	}

	// Get current state
	match obs_manager.current_state().await {
		Ok(state) => tracing::info!("  📊 Current state: {:?}", state),
		Err(e) => tracing::warn!("  ⚠️ State check failed: {}", e),
	}

	// Show connection info again
	match obs_manager.connection_info().await {
		Ok(info) => {
			tracing::info!("  📋 Final connection info:");
			tracing::info!("    State: {:?}", info.state);
		}
		Err(e) => tracing::warn!("  ⚠️ Connection info failed: {}", e),
	}

	Ok(())
}

// Helper function to demonstrate event streaming (commented out to avoid blocking)
#[allow(dead_code)]
async fn demo_event_streaming(obs_manager: &ObsWebSocketManager) -> Result<(), Box<dyn std::error::Error>> {
	tracing::info!("  📻 Starting event stream demonstration...");

	let mut event_count = 0;
	let max_events = 10;

	obs_manager
		.stream_events(move |event| {
			Box::pin(async move {
				event_count += 1;
				tracing::info!("  📨 Event {}/{}: {:?}", event_count, max_events, event);

				if event_count >= max_events {
					tracing::info!("  ✅ Event streaming demo completed");
					// In a real implementation, you'd want to break the stream here
				}
			})
		})
		.await?;

	Ok(())
}

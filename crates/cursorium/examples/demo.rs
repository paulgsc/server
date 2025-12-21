use cursorium::core::StreamOrchestrator;
use tokio::time::{sleep, Duration};
use tracing::Level;
use tracing_subscriber;
use ws_events::events::{OrchestratorCommandData, OrchestratorConfigData, SceneConfigData};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
	// Initialize tracing
	tracing_subscriber::fmt().with_max_level(Level::INFO).with_target(false).init();

	println!("\n🎬 Stream Orchestrator Demo\n");
	println!("Demonstrating discrete event projection onto continuous time.\n");

	// Run different demo scenarios
	demo_parallel_scenes().await?;
	demo_overlapping_scenes().await?;
	demo_lifetime_tracking().await?;
	demo_dynamic_pipeline().await?;
	demo_sparse_timeline().await?;
	demo_client_workflow().await?;
	demo_event_density().await?;

	println!("\n✅ All demos completed!\n");
	Ok(())
}

/// Demo 1: Parallel scenes - multiple events starting at same time
async fn demo_parallel_scenes() -> Result<(), Box<dyn std::error::Error>> {
	println!("🎭 Demo 1: Parallel Scene Execution");
	println!("Multiple discrete events projected at t=0, t=3000\n");

	// Create config data as would come from a client
	let config_data = OrchestratorConfigData {
		scenes: vec![
			// All start at t=0 (parallel execution)
			SceneConfigData {
				scene_name: "Main Camera".to_string(),
				duration: 5000,
				start_time: Some(0),
				metadata: Some(serde_json::json!({"layer": "video", "priority": 1})),
			},
			SceneConfigData {
				scene_name: "Lower Third".to_string(),
				duration: 5000,
				start_time: Some(0),
				metadata: Some(serde_json::json!({"layer": "overlay", "priority": 2})),
			},
			SceneConfigData {
				scene_name: "Audio Track".to_string(),
				duration: 5000,
				start_time: Some(0),
				metadata: Some(serde_json::json!({"layer": "audio", "priority": 3})),
			},
			// Second wave at t=3000
			SceneConfigData {
				scene_name: "Transition Effect".to_string(),
				duration: 2000,
				start_time: Some(3000),
				metadata: Some(serde_json::json!({"layer": "fx"})),
			},
		],
		tick_interval_ms: 50,
		loop_scenes: false,
	};

	// Start unconfigured, configure via command
	let orchestrator = StreamOrchestrator::new(None)?;
	let mut state_rx = orchestrator.subscribe();

	println!("📋 Timeline Structure:");
	println!("  t=0ms:    [Main Camera, Lower Third, Audio Track] start (N=3)");
	println!("  t=3000ms: [Transition Effect] starts (N=1)");
	println!("  t=5000ms: [Main Camera, Lower Third, Audio Track] end (N=3)");
	println!("  t=5000ms: [Transition Effect] ends (N=1)");
	println!("\n🔧 Configuring...\n");

	orchestrator.configure(OrchestratorCommandData::Configure(config_data)).await?;
	orchestrator.start().await?;

	// Monitor discrete event changes with active_lifetimes
	tokio::spawn(async move {
		let mut last_active_count = 0;

		while state_rx.changed().await.is_ok() {
			let state = state_rx.borrow().clone();
			let active_count = state.active_lifetimes.len();

			if active_count != last_active_count {
				let active_names: Vec<&str> = state.active_lifetimes.iter().filter_map(|lt| lt.scene_name()).collect();

				println!(
					"⚡ t={}ms: Active lifetimes: {} → {} | Scenes: [{}]",
					state.current_time,
					last_active_count,
					active_count,
					active_names.join(", ")
				);
				last_active_count = active_count;
			}

			if state.is_complete() {
				println!("\n✅ All discrete events drained");
				break;
			}
		}
	});

	sleep(Duration::from_secs(6)).await;
	orchestrator.shutdown().await;

	Ok(())
}

/// Demo 2: Overlapping scenes with different start times
async fn demo_overlapping_scenes() -> Result<(), Box<dyn std::error::Error>> {
	println!("\n🌊 Demo 2: Overlapping Event Lifetimes");
	println!("Scenes with staggered starts - showing projection at each t\n");

	let config_data = OrchestratorConfigData {
		scenes: vec![
			SceneConfigData {
				scene_name: "Background".to_string(),
				duration: 6000,
				start_time: Some(0),
				metadata: Some(serde_json::json!({"type": "background"})),
			},
			SceneConfigData {
				scene_name: "Speaker 1".to_string(),
				duration: 3000,
				start_time: Some(500), // Starts 500ms in
				metadata: Some(serde_json::json!({"speaker": "alice"})),
			},
			SceneConfigData {
				scene_name: "Speaker 2".to_string(),
				duration: 3000,
				start_time: Some(2000), // Starts 2s in
				metadata: Some(serde_json::json!({"speaker": "bob"})),
			},
			SceneConfigData {
				scene_name: "Credits".to_string(),
				duration: 2000,
				start_time: Some(4000), // Starts 4s in
				metadata: Some(serde_json::json!({"type": "credits"})),
			},
		],
		tick_interval_ms: 100,
		loop_scenes: false,
	};

	let orchestrator = StreamOrchestrator::new(None)?;
	let mut state_rx = orchestrator.subscribe();

	println!("📋 Discrete Event Schedule:");
	println!("  t=0ms:    Background starts");
	println!("  t=500ms:  Speaker 1 starts (Background continues)");
	println!("  t=2000ms: Speaker 2 starts (Background + Speaker 1)");
	println!("  t=3500ms: Speaker 1 ends");
	println!("  t=4000ms: Credits starts (Background + Speaker 2)");
	println!("  t=5000ms: Speaker 2 ends");
	println!("  t=6000ms: Background + Credits end\n");

	orchestrator.configure(OrchestratorCommandData::Configure(config_data)).await?;
	orchestrator.start().await?;

	// Monitor the continuous time projection with active_lifetimes detail
	tokio::spawn(async move {
		let mut last_snapshot = Vec::new();

		while state_rx.changed().await.is_ok() {
			let state = state_rx.borrow().clone();
			let current_snapshot: Vec<String> = state.active_lifetimes.iter().filter_map(|lt| lt.scene_name().map(String::from)).collect();

			if current_snapshot != last_snapshot {
				println!(
					"📸 t={}ms | Active lifetimes: N={} | [{}]",
					state.current_time,
					current_snapshot.len(),
					current_snapshot.join(", ")
				);
				last_snapshot = current_snapshot;
			}

			if state.is_complete() {
				break;
			}
		}
	});

	sleep(Duration::from_secs(7)).await;
	orchestrator.shutdown().await;

	Ok(())
}

/// Demo 3: Lifetime tracking and metadata
async fn demo_lifetime_tracking() -> Result<(), Box<dyn std::error::Error>> {
	println!("\n🔍 Demo 3: Active Lifetime Inspection");
	println!("Deep dive into active_lifetimes structure and metadata\n");

	let config_data = OrchestratorConfigData {
		scenes: vec![
			SceneConfigData {
				scene_name: "Video Primary".to_string(),
				duration: 4000,
				start_time: Some(0),
				metadata: Some(serde_json::json!({
						"layer": "video",
						"source": "camera_1",
						"resolution": "1080p"
				})),
			},
			SceneConfigData {
				scene_name: "Audio Track".to_string(),
				duration: 4000,
				start_time: Some(0),
				metadata: Some(serde_json::json!({
						"layer": "audio",
						"source": "mic_input",
						"bitrate": "192kbps"
				})),
			},
			SceneConfigData {
				scene_name: "Overlay Banner".to_string(),
				duration: 2000,
				start_time: Some(1000),
				metadata: Some(serde_json::json!({
						"layer": "overlay",
						"position": "bottom",
						"animation": "slide-in"
				})),
			},
		],
		tick_interval_ms: 100,
		loop_scenes: false,
	};

	let orchestrator = StreamOrchestrator::new(None)?;
	let mut state_rx = orchestrator.subscribe();

	orchestrator.configure(OrchestratorCommandData::Configure(config_data)).await?;
	orchestrator.start().await?;

	println!("📊 Tracking active lifetimes with metadata:\n");

	tokio::spawn(async move {
		let mut sample_count = 0;

		while state_rx.changed().await.is_ok() {
			let state = state_rx.borrow().clone();
			sample_count += 1;

			// Sample every 10 ticks to avoid spam
			if sample_count % 10 == 0 {
				println!("  ⏱️  t={}ms | Progress: {:.1}%", state.current_time, state.progress.percentage());

				for lifetime in &state.active_lifetimes {
					let duration = state.current_time - lifetime.started_at;
					if let Some(scene_name) = lifetime.scene_name() {
						println!(
							"    → Lifetime(id={:?}, scene='{}', started_at={}ms, duration={}ms)",
							lifetime.id, scene_name, lifetime.started_at, duration
						);
					}
				}

				if !state.active_lifetimes.is_empty() {
					println!();
				}
			}

			if state.is_complete() {
				println!("  ✅ Final state: {} lifetimes drained", state.active_lifetimes.len());
				break;
			}
		}
	});

	sleep(Duration::from_secs(5)).await;
	orchestrator.shutdown().await;

	Ok(())
}

/// Demo 4: Dynamic reconfiguration pipeline (simulating client updates)
async fn demo_dynamic_pipeline() -> Result<(), Box<dyn std::error::Error>> {
	println!("\n🔄 Demo 4: Dynamic Configuration Pipeline");
	println!("Simulating client sending config updates during runtime\n");

	let orchestrator = StreamOrchestrator::new(None)?;
	let mut state_rx = orchestrator.subscribe();

	// Monitor task
	let monitor = tokio::spawn(async move {
		while state_rx.changed().await.is_ok() {
			let state = state_rx.borrow().clone();
			if !state.active_lifetimes.is_empty() {
				let scene_names: Vec<&str> = state.active_lifetimes.iter().filter_map(|lt| lt.scene_name()).collect();
				println!(
					"  [{}ms] Active: {} | Progress: {:.1}%",
					state.current_time,
					scene_names.join(", "),
					state.progress.percentage()
				);
			}
		}
	});

	// Phase 1: Initial configuration from client
	println!("📥 Phase 1: Client sends initial config");
	let config1 = OrchestratorConfigData {
		scenes: vec![
			SceneConfigData {
				scene_name: "Intro".to_string(),
				duration: 2000,
				start_time: Some(0),
				metadata: None,
			},
			SceneConfigData {
				scene_name: "Segment A".to_string(),
				duration: 2000,
				start_time: Some(2000),
				metadata: None,
			},
		],
		tick_interval_ms: 100,
		loop_scenes: false,
	};

	orchestrator.configure(OrchestratorCommandData::Configure(config1)).await?;
	orchestrator.start().await?;
	sleep(Duration::from_millis(1500)).await;

	// Phase 2: Client sends stop + new config
	println!("\n📥 Phase 2: Client requests stop and reconfiguration");
	orchestrator.stop().await?;
	sleep(Duration::from_millis(200)).await;

	let config2 = OrchestratorConfigData {
		scenes: vec![
			SceneConfigData {
				scene_name: "Updated Intro".to_string(),
				duration: 1500,
				start_time: Some(0),
				metadata: Some(serde_json::json!({"version": 2})),
			},
			SceneConfigData {
				scene_name: "Segment B".to_string(),
				duration: 1500,
				start_time: Some(0), // Parallel with intro
				metadata: Some(serde_json::json!({"layer": "background"})),
			},
			SceneConfigData {
				scene_name: "Outro".to_string(),
				duration: 1000,
				start_time: Some(1500),
				metadata: None,
			},
		],
		tick_interval_ms: 100,
		loop_scenes: false,
	};

	orchestrator.configure(OrchestratorCommandData::Configure(config2)).await?;
	orchestrator.start().await?;
	sleep(Duration::from_secs(3)).await;

	monitor.abort();
	orchestrator.shutdown().await;

	Ok(())
}

/// Demo 5: Sparse timeline with gaps
async fn demo_sparse_timeline() -> Result<(), Box<dyn std::error::Error>> {
	println!("\n⏱️  Demo 5: Sparse Timeline with Gaps");
	println!("Demonstrating projection where some t have no events (N=0)\n");

	let config_data = OrchestratorConfigData {
		scenes: vec![
			SceneConfigData {
				scene_name: "Event 1".to_string(),
				duration: 1000,
				start_time: Some(0),
				metadata: None,
			},
			// Gap from t=1000 to t=3000 (N=0 active lifetimes)
			SceneConfigData {
				scene_name: "Event 2".to_string(),
				duration: 1000,
				start_time: Some(3000),
				metadata: None,
			},
			// Gap from t=4000 to t=6000
			SceneConfigData {
				scene_name: "Event 3".to_string(),
				duration: 1000,
				start_time: Some(6000),
				metadata: None,
			},
		],
		tick_interval_ms: 100,
		loop_scenes: false,
	};

	let orchestrator = StreamOrchestrator::new(None)?;
	let mut state_rx = orchestrator.subscribe();

	println!("📋 Timeline with Gaps:");
	println!("  t=0-1000ms:   Event 1 active (N=1)");
	println!("  t=1000-3000ms: GAP (N=0 active_lifetimes)");
	println!("  t=3000-4000ms: Event 2 active (N=1)");
	println!("  t=4000-6000ms: GAP (N=0 active_lifetimes)");
	println!("  t=6000-7000ms: Event 3 active (N=1)\n");

	orchestrator.configure(OrchestratorCommandData::Configure(config_data)).await?;
	orchestrator.start().await?;

	tokio::spawn(async move {
		let mut in_gap = false;

		while state_rx.changed().await.is_ok() {
			let state = state_rx.borrow().clone();
			let has_active = !state.active_lifetimes.is_empty();

			if !has_active && !in_gap {
				println!("  [{}ms] → Entering gap (N=0 active_lifetimes)", state.current_time);
				in_gap = true;
			} else if has_active && in_gap {
				println!("  [{}ms] ← Exiting gap (N={})", state.current_time, state.active_lifetimes.len());
				in_gap = false;
			}

			if has_active {
				let scene = state.active_lifetimes[0].scene_name().unwrap_or("unknown");
				println!("  [{}ms] Active: {}", state.current_time, scene);
			}

			if state.is_complete() {
				break;
			}
		}
	});

	sleep(Duration::from_secs(8)).await;
	orchestrator.shutdown().await;

	Ok(())
}

/// Demo 6: Complete client workflow simulation
async fn demo_client_workflow() -> Result<(), Box<dyn std::error::Error>> {
	println!("\n🌐 Demo 6: Complete Client Workflow");
	println!("Simulating full data pipeline: client → commands → orchestrator\n");

	let orchestrator = StreamOrchestrator::new(None)?;
	let mut state_rx = orchestrator.subscribe();

	// State monitor with detailed output
	let monitor = tokio::spawn(async move {
		let mut event_count = 0;

		while state_rx.changed().await.is_ok() {
			let state = state_rx.borrow().clone();
			event_count += 1;

			if event_count % 10 == 0 {
				// Sample every 10th event
				let timecode = state.current_timecode();
				println!(
					"  📊 [{}] Lifetimes: {} | Time: {}ms/{}ms | Remaining: {}ms",
					timecode.as_str(),
					state.active_lifetimes.len(),
					state.current_time,
					state.total_duration,
					state.time_remaining
				);
			}

			if state.is_complete() {
				println!("\n  ✅ Workflow complete at t={}ms", state.current_time);
				break;
			}
		}
	});

	println!("📱 Step 1: Client sends configuration");
	let client_config = OrchestratorConfigData {
		scenes: vec![
			// Opening sequence - multiple parallel layers
			SceneConfigData {
				scene_name: "Video Layer".to_string(),
				duration: 10000,
				start_time: Some(0),
				metadata: Some(serde_json::json!({
						"source": "camera_1",
						"resolution": "1080p"
				})),
			},
			SceneConfigData {
				scene_name: "Audio Layer".to_string(),
				duration: 10000,
				start_time: Some(0),
				metadata: Some(serde_json::json!({
						"source": "microphone",
						"volume": 0.8
				})),
			},
			// Mid-stream overlays
			SceneConfigData {
				scene_name: "Lower Third".to_string(),
				duration: 3000,
				start_time: Some(2000),
				metadata: Some(serde_json::json!({
						"text": "John Doe - Speaker",
						"style": "modern"
				})),
			},
			SceneConfigData {
				scene_name: "Logo Overlay".to_string(),
				duration: 8000,
				start_time: Some(1000),
				metadata: Some(serde_json::json!({
						"position": "top-right",
						"opacity": 0.7
				})),
			},
			// Closing
			SceneConfigData {
				scene_name: "Fade Out".to_string(),
				duration: 1000,
				start_time: Some(9000),
				metadata: Some(serde_json::json!({"effect": "fade"})),
			},
		],
		tick_interval_ms: 50,
		loop_scenes: false,
	};

	orchestrator.configure(OrchestratorCommandData::Configure(client_config)).await?;

	println!("\n▶️  Step 2: Client sends START command");
	orchestrator.start().await?;

	// Simulate client interactions during playback
	sleep(Duration::from_secs(3)).await;

	println!("\n⏸️  Step 3: Client sends PAUSE command");
	orchestrator.pause().await?;
	sleep(Duration::from_secs(1)).await;

	println!("\n▶️  Step 4: Client sends RESUME command");
	orchestrator.resume().await?;
	sleep(Duration::from_secs(2)).await;

	println!("\n⏭️  Step 5: Client sends SKIP command");
	orchestrator.skip_current_scene()?;
	sleep(Duration::from_secs(3)).await;

	println!("\n🛑 Step 6: Client sends STOP command");
	orchestrator.stop().await?;

	monitor.abort();
	orchestrator.shutdown().await;

	println!("\n💡 Key Concepts Demonstrated:");
	println!("  • Commands as serialized data (OrchestratorCommandData)");
	println!("  • Config as data pipeline input (OrchestratorConfigData)");
	println!("  • active_lifetimes: Vec<ActiveLifetime> tracking");
	println!("  • Multiple events at same t (parallel projection)");
	println!("  • Continuous time with discrete event lifetimes");
	println!("  • Dynamic control during playback");
	println!("  • current_active_scene derived from active_lifetimes");

	Ok(())
}

/// Demo 7: Event density and lifetime overlap analysis
async fn demo_event_density() -> Result<(), Box<dyn std::error::Error>> {
	println!("\n📈 Demo 7: Event Density & Lifetime Duration Analysis");
	println!("Analyzing active_lifetimes count and individual lifetime durations\n");

	let config_data = OrchestratorConfigData {
		scenes: vec![
			// Dense region at beginning (t=0-2000: 5 concurrent lifetimes)
			SceneConfigData {
				scene_name: "Layer1".to_string(),
				duration: 2000,
				start_time: Some(0),
				metadata: None,
			},
			SceneConfigData {
				scene_name: "Layer2".to_string(),
				duration: 2000,
				start_time: Some(0),
				metadata: None,
			},
			SceneConfigData {
				scene_name: "Layer3".to_string(),
				duration: 2000,
				start_time: Some(0),
				metadata: None,
			},
			SceneConfigData {
				scene_name: "Layer4".to_string(),
				duration: 2000,
				start_time: Some(0),
				metadata: None,
			},
			SceneConfigData {
				scene_name: "Layer5".to_string(),
				duration: 2000,
				start_time: Some(0),
				metadata: None,
			},
			// Sparse region (t=2000-4000: 1 lifetime)
			SceneConfigData {
				scene_name: "Solo".to_string(),
				duration: 2000,
				start_time: Some(2000),
				metadata: None,
			},
			// Medium density (t=4000-6000: 3 lifetimes)
			SceneConfigData {
				scene_name: "MedA".to_string(),
				duration: 2000,
				start_time: Some(4000),
				metadata: None,
			},
			SceneConfigData {
				scene_name: "MedB".to_string(),
				duration: 2000,
				start_time: Some(4000),
				metadata: None,
			},
			SceneConfigData {
				scene_name: "MedC".to_string(),
				duration: 2000,
				start_time: Some(4000),
				metadata: None,
			},
		],
		tick_interval_ms: 100,
		loop_scenes: false,
	};

	let orchestrator = StreamOrchestrator::new(None)?;
	let mut state_rx = orchestrator.subscribe();

	println!("📊 Expected Density:");
	println!("  t=0-2000ms:   HIGH (N=5 active_lifetimes)");
	println!("  t=2000-4000ms: LOW (N=1 active_lifetime)");
	println!("  t=4000-6000ms: MEDIUM (N=3 active_lifetimes)\n");

	orchestrator.configure(OrchestratorCommandData::Configure(config_data)).await?;
	orchestrator.start().await?;

	tokio::spawn(async move {
		let mut last_density = 0;

		while state_rx.changed().await.is_ok() {
			let state = state_rx.borrow().clone();
			let current_density = state.active_lifetimes.len();

			if current_density != last_density {
				let density_label = match current_density {
					0 => "NONE",
					1 => "LOW",
					2..=3 => "MEDIUM",
					_ => "HIGH",
				};

				println!(
					"  [t={}ms] Density change: {} → {} ({} - {} active_lifetimes)",
					state.current_time, last_density, current_density, density_label, current_density
				);

				// Show individual lifetime durations
				for lifetime in &state.active_lifetimes {
					let age = state.current_time - lifetime.started_at;
					if let Some(name) = lifetime.scene_name() {
						println!("    • {}: running for {}ms", name, age);
					}
				}

				last_density = current_density;
			}

			if state.is_complete() {
				break;
			}
		}
	});

	sleep(Duration::from_secs(7)).await;
	orchestrator.shutdown().await;

	Ok(())
}

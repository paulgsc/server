use tokio::time::{sleep, Duration};
use tracing::Level;
use tracing_subscriber;
use ws_events::events::{OrchestratorConfig, SceneConfig};
use ws_events::stream_orch::StreamOrchestrator;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
	// Initialize tracing
	tracing_subscriber::fmt().with_max_level(Level::INFO).with_target(false).init();

	println!("\n🎬 Stream Orchestrator Demo\n");
	println!("This demo shows the orchestrator managing a multi-scene livestream.\n");

	// Run different demo scenarios
	demo_basic_flow().await?;

	demo_scene_control().await?;

	demo_pause_resume().await?;

	demo_stream_sync().await?;

	println!("\n✅ All demos completed!\n");
	Ok(())
}

/// Demo 1: Basic orchestration flow
async fn demo_basic_flow() -> Result<(), Box<dyn std::error::Error>> {
	println!("📺 Demo 1: Basic Orchestration Flow");

	// Create a simple 3-scene configuration
	let config = OrchestratorConfig::new(vec![
		SceneConfig::new("Intro", 2000),        // 2 seconds
		SceneConfig::new("Main Content", 5000), // 5 seconds
		SceneConfig::new("Outro", 2000),        // 2 seconds
	])
	.with_tick_interval(50); // 50ms tick rate for smooth updates

	let orchestrator = StreamOrchestrator::new(config)?;
	let mut state_rx = orchestrator.subscribe();

	println!("\n📋 Scene Schedule:");
	println!("  1. Intro         (0:00 - 0:02)");
	println!("  2. Main Content  (0:02 - 0:07)");
	println!("  3. Outro         (0:07 - 0:09)");
	println!("\n▶️  Starting orchestrator...\n");

	orchestrator.start()?;

	// Monitor state changes
	let monitor_task = tokio::spawn(async move {
		let mut last_scene: Option<String> = None;

		while state_rx.changed().await.is_ok() {
			let state = state_rx.borrow().clone();

			// Log scene changes
			if state.current_active_scene != last_scene {
				if let Some(scene) = &state.current_active_scene {
					let timecode = state.current_timecode();
					println!("🎬 [{:02}] Scene Change: {} (Progress: {:.1}%)", timecode.as_str(), scene, state.progress.percentage());
				}
				last_scene = state.current_active_scene.clone();
			}

			// Log completion
			if state.is_complete() && state.is_running {
				println!("\n✅ Orchestration complete!");
				break;
			}
		}
	});

	// Wait for completion
	sleep(Duration::from_secs(10)).await;
	orchestrator.shutdown().await;
	let _ = monitor_task.await;

	Ok(())
}

/// Demo 2: Manual scene control
async fn demo_scene_control() -> Result<(), Box<dyn std::error::Error>> {
	println!("🎮 Demo 2: Manual Scene Control");

	let config = OrchestratorConfig::new(vec![
		SceneConfig::new("Camera 1", 3000),
		SceneConfig::new("Camera 2", 3000),
		SceneConfig::new("Camera 3", 3000),
		SceneConfig::new("Outro", 2000),
	])
	.with_tick_interval(100);

	let orchestrator = StreamOrchestrator::new(config)?;
	let mut state_rx = orchestrator.subscribe();

	println!("\n📋 Available Scenes:");
	println!("  • Camera 1");
	println!("  • Camera 2");
	println!("  • Camera 3");
	println!("  • Outro");
	println!("\n▶️  Starting with manual control...\n");

	orchestrator.start()?;

	// Manual scene switching demo
	tokio::spawn(async move {
		while state_rx.changed().await.is_ok() {
			let state = state_rx.borrow().clone();
			if let Some(scene) = &state.current_active_scene {
				let timecode = state.current_timecode();
				println!(
					"📹 [{:02}] Active: {} | Time: {}ms | Remaining: {}ms",
					timecode.as_str(),
					scene,
					state.current_time,
					state.time_remaining
				);
			}
		}
	});

	// Let Camera 1 play for 1 second
	sleep(Duration::from_secs(1)).await;
	println!("\n🎯 Forcing scene to 'Camera 3'...");
	orchestrator.force_scene("Camera 3")?;

	sleep(Duration::from_secs(2)).await;
	println!("\n⏭️  Skipping current scene...");
	orchestrator.skip_current_scene()?;

	sleep(Duration::from_secs(2)).await;
	println!("\n🛑 Stopping orchestrator early...");
	orchestrator.stop()?;

	sleep(Duration::from_millis(500)).await;
	orchestrator.shutdown().await;

	Ok(())
}

/// Demo 3: Pause and Resume
async fn demo_pause_resume() -> Result<(), Box<dyn std::error::Error>> {
	println!("⏸️  Demo 3: Pause and Resume");

	let config = OrchestratorConfig::new(vec![
		SceneConfig::new("Scene A", 3000),
		SceneConfig::new("Scene B", 3000),
		SceneConfig::new("Scene C", 3000),
	])
	.with_tick_interval(100);

	let orchestrator = StreamOrchestrator::new(config)?;
	let mut state_rx = orchestrator.subscribe();

	println!("\n▶️  Starting orchestrator...\n");
	orchestrator.start()?;

	// State monitor
	tokio::spawn(async move {
		while state_rx.changed().await.is_ok() {
			let state = state_rx.borrow().clone();

			let status = if state.is_paused {
				"⏸️  PAUSED"
			} else if state.is_running {
				"▶️  PLAYING"
			} else {
				"⏹️  STOPPED"
			};

			if let Some(scene) = &state.current_active_scene {
				println!("{} | Scene: {} | Progress: {:.1}%", status, scene, state.progress.percentage());
			}
		}
	});

	// Play for 1.5 seconds
	sleep(Duration::from_millis(1500)).await;

	println!("\n⏸️  Pausing...");
	orchestrator.pause()?;
	sleep(Duration::from_secs(2)).await;

	println!("\n▶️  Resuming...");
	orchestrator.resume()?;
	sleep(Duration::from_millis(1500)).await;

	println!("\n⏸️  Pausing again...");
	orchestrator.pause()?;
	sleep(Duration::from_secs(1)).await;

	println!("\n▶️  Resuming...");
	orchestrator.resume()?;
	sleep(Duration::from_secs(3)).await;

	orchestrator.stop()?;
	sleep(Duration::from_millis(500)).await;
	orchestrator.shutdown().await;

	Ok(())
}

/// Demo 4: Syncing with external stream timecode (simulating OBS)
async fn demo_stream_sync() -> Result<(), Box<dyn std::error::Error>> {
	println!("🔄 Demo 4: Stream Timecode Synchronization");
	println!("\nSimulating OBS streaming timecode updates...\n");

	let config = OrchestratorConfig::new(vec![
		SceneConfig::new("Starting Soon", 3000),
		SceneConfig::new("Live Show", 6000),
		SceneConfig::new("Credits", 2000),
	])
	.with_tick_interval(100);

	let orchestrator = StreamOrchestrator::new(config)?;
	let mut state_rx = orchestrator.subscribe();

	println!("📋 Scene Schedule:");
	println!("  1. Starting Soon  (0:00 - 0:03)");
	println!("  2. Live Show      (0:03 - 0:09)");
	println!("  3. Credits        (0:09 - 0:11)");
	println!("\n🎥 Starting stream...\n");

	orchestrator.start()?;

	// State monitor
	let state_monitor = tokio::spawn(async move {
		let mut last_scene: Option<String> = None;

		while state_rx.changed().await.is_ok() {
			let state = state_rx.borrow().clone();

			if state.current_active_scene != last_scene {
				if let Some(scene) = &state.current_active_scene {
					println!(
						"🎬 Scene: {} | OBS Timecode: {} | Stream: {}",
						scene,
						state.stream_status.timecode,
						if state.stream_status.is_streaming { "🔴 LIVE" } else { "⚫ OFFLINE" }
					);
				}
				last_scene = state.current_active_scene.clone();
			}
		}
	});

	// Simulate OBS sending timecode updates
	let obs_simulator = tokio::spawn({
		let orchestrator = orchestrator;
		async move {
			// Simulate stream starting
			orchestrator.update_stream_status(true, 0, "00:00:00.000".to_string()).ok();

			for i in 0..120 {
				let stream_time = i * 100; // 100ms increments
				let seconds = stream_time / 1000;
				let millis = stream_time % 1000;
				let timecode = format!("00:00:{:02}.{:03}", seconds, millis);

				orchestrator.update_stream_status(true, stream_time, timecode).ok();
				sleep(Duration::from_millis(100)).await;
			}

			// Simulate stream ending
			println!("\n🔴 Stream ending...");
			orchestrator.update_stream_status(false, 12000, "00:00:12.000".to_string()).ok();
			sleep(Duration::from_millis(500)).await;

			orchestrator.stop().ok();
			orchestrator
		}
	});

	let orchestrator = obs_simulator.await?;
	let _ = state_monitor.await;

	sleep(Duration::from_millis(500)).await;
	orchestrator.shutdown().await;

	Ok(())
}

/// Bonus: Interactive demo (optional - requires user input)
#[allow(dead_code)]
async fn demo_interactive() -> Result<(), Box<dyn std::error::Error>> {
	use std::io::{self, Write};

	println!("🎮 Interactive Demo");

	let config = OrchestratorConfig::new(vec![
		SceneConfig::new("Intro", 10000),
		SceneConfig::new("Main", 20000),
		SceneConfig::new("Q&A", 15000),
		SceneConfig::new("Outro", 5000),
	])
	.with_tick_interval(100);

	let orchestrator = StreamOrchestrator::new(config)?;
	let mut state_rx = orchestrator.subscribe();

	// State display task
	tokio::spawn(async move {
		while state_rx.changed().await.is_ok() {
			let state = state_rx.borrow().clone();

			print!(
				"\r🎬 Scene: {:15} | Progress: {:>5.1}% | Time: {:>5}ms | Status: {}  ",
				state.current_active_scene.as_deref().unwrap_or("None"),
				state.progress.percentage(),
				state.current_time,
				if state.is_paused {
					"⏸️ "
				} else if state.is_running {
					"▶️ "
				} else {
					"⏹️ "
				}
			);
			io::stdout().flush().unwrap();
		}
	});

	println!("\nCommands:");
	println!("  start - Start orchestration");
	println!("  stop  - Stop orchestration");
	println!("  pause - Pause orchestration");
	println!("  resume - Resume orchestration");
	println!("  skip  - Skip current scene");
	println!("  intro/main/q&a/outro - Jump to scene");
	println!("  quit  - Exit demo\n");

	// Simple command loop (blocking - just for demo)
	loop {
		print!("\n> ");
		io::stdout().flush()?;

		let mut input = String::new();
		io::stdin().read_line(&mut input)?;
		let cmd = input.trim().to_lowercase();

		match cmd.as_str() {
			"start" => orchestrator.start()?,
			"stop" => orchestrator.stop()?,
			"pause" => orchestrator.pause()?,
			"resume" => orchestrator.resume()?,
			"skip" => orchestrator.skip_current_scene()?,
			"intro" => orchestrator.force_scene("Intro")?,
			"main" => orchestrator.force_scene("Main")?,
			"q&a" => orchestrator.force_scene("Q&A")?,
			"outro" => orchestrator.force_scene("Outro")?,
			"quit" => break,
			_ => println!("Unknown command: {}", cmd),
		}
	}

	orchestrator.shutdown().await;
	Ok(())
}

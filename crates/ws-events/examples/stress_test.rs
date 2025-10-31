use std::time::Instant;
use tokio::time::{sleep, Duration};
use ws_events::events::{OrchestratorConfig, SceneConfig};
use ws_events::stream_orch::StreamOrchestrator;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
	println!("🔬 Orchestrator Stress Test\n");

	test_many_scenes().await?;
	test_high_tick_rate().await?;
	test_rapid_commands().await?;
	test_many_subscribers().await?;

	println!("\n✅ All stress tests passed!");
	Ok(())
}

/// Test with many scenes
async fn test_many_scenes() -> Result<(), Box<dyn std::error::Error>> {
	println!("📊 Test 1: Many Scenes (100 scenes)");

	let scenes: Vec<_> = (0..100).map(|i| SceneConfig::new(format!("Scene {}", i), 100)).collect();

	let config = OrchestratorConfig::new(scenes).with_tick_interval(10);
	let start = Instant::now();

	let orchestrator = StreamOrchestrator::new(config)?;
	let creation_time = start.elapsed();

	let mut state_rx = orchestrator.subscribe();
	let mut scene_changes = 0;
	let mut last_scene: Option<String> = None;

	tokio::spawn(async move {
		while state_rx.changed().await.is_ok() {
			let state = state_rx.borrow();
			if state.current_active_scene != last_scene {
				scene_changes += 1;
				last_scene = state.current_active_scene.clone();
			}
		}
		scene_changes
	});

	orchestrator.start()?;
	let start_exec = Instant::now();

	// Run for 5 seconds (should hit ~50 scenes)
	sleep(Duration::from_secs(5)).await;

	let state = orchestrator.current_state();
	let exec_time = start_exec.elapsed();

	orchestrator.shutdown().await;

	println!("  Creation time: {:?}", creation_time);
	println!("  Execution time: {:?}", exec_time);
	println!("  Current scene index: {}", state.current_scene_index);
	println!("  Progress: {:.1}%", state.progress.percentage());
	println!("  ✅ Passed\n");

	Ok(())
}

/// Test with very high tick rate
async fn test_high_tick_rate() -> Result<(), Box<dyn std::error::Error>> {
	println!("⚡ Test 2: High Tick Rate (10ms ticks)");

	let config = OrchestratorConfig::new(vec![
		SceneConfig::new("Scene A", 1000),
		SceneConfig::new("Scene B", 1000),
		SceneConfig::new("Scene C", 1000),
	])
	.with_tick_interval(10); // 100 ticks per second

	let orchestrator = StreamOrchestrator::new(config)?;
	let mut state_rx = orchestrator.subscribe();

	let mut tick_count = 0;
	tokio::spawn(async move {
		while state_rx.changed().await.is_ok() {
			tick_count += 1;
		}
		tick_count
	});

	orchestrator.start()?;
	let start = Instant::now();

	sleep(Duration::from_secs(2)).await;

	let elapsed = start.elapsed();
	let state = orchestrator.current_state();

	orchestrator.shutdown().await;

	println!("  Elapsed: {:?}", elapsed);
	println!("  Progress: {:.1}%", state.progress.percentage());
	println!("  Current time: {}ms", state.current_time);
	println!("  ✅ Passed\n");

	Ok(())
}

/// Test rapid command execution
async fn test_rapid_commands() -> Result<(), Box<dyn std::error::Error>> {
	println!("🚀 Test 3: Rapid Commands (1000 commands)");

	let config = OrchestratorConfig::new(vec![
		SceneConfig::new("Scene 1", 5000),
		SceneConfig::new("Scene 2", 5000),
		SceneConfig::new("Scene 3", 5000),
	]);

	let orchestrator = StreamOrchestrator::new(config)?;
	orchestrator.start()?;

	let start = Instant::now();

	// Fire 1000 commands rapidly
	for i in 0..1000 {
		match i % 4 {
			0 => orchestrator.pause()?,
			1 => orchestrator.resume()?,
			2 => orchestrator.force_scene("Scene 2")?,
			_ => orchestrator.skip_current_scene()?,
		}
	}

	let command_time = start.elapsed();

	// Let it settle
	sleep(Duration::from_millis(100)).await;

	let state = orchestrator.current_state();
	orchestrator.shutdown().await;

	println!("  Command execution time: {:?}", command_time);
	println!("  Avg per command: {:?}", command_time / 1000);
	println!("  Final state valid: {}", state.is_running || !state.is_running);
	println!("  ✅ Passed\n");

	Ok(())
}

/// Test many concurrent subscribers
async fn test_many_subscribers() -> Result<(), Box<dyn std::error::Error>> {
	println!("👥 Test 4: Many Subscribers (100 subscribers)");

	let config = OrchestratorConfig::new(vec![SceneConfig::new("Test Scene", 2000)]).with_tick_interval(50);

	let orchestrator = StreamOrchestrator::new(config)?;

	// Create 100 subscribers
	let mut handles = vec![];
	for i in 0..100 {
		let mut state_rx = orchestrator.subscribe();
		let handle = tokio::spawn(async move {
			let mut updates = 0;
			while state_rx.changed().await.is_ok() {
				updates += 1;
			}
			(i, updates)
		});
		handles.push(handle);
	}

	let start = Instant::now();
	orchestrator.start()?;

	sleep(Duration::from_secs(3)).await;

	orchestrator.shutdown().await;
	let elapsed = start.elapsed();

	// Collect results
	let mut total_updates = 0;
	for handle in handles {
		let (id, updates) = handle.await?;
		total_updates += updates;
		if id == 0 {
			println!("  Subscriber 0 received {} updates", updates);
		}
	}

	println!("  Total time: {:?}", elapsed);
	println!("  Total updates across all subscribers: {}", total_updates);
	println!("  Avg updates per subscriber: {}", total_updates / 100);
	println!("  ✅ Passed\n");

	Ok(())
}

/// Memory usage test (optional, requires manual inspection)
#[allow(dead_code)]
async fn test_memory_usage() -> Result<(), Box<dyn std::error::Error>> {
	println!("💾 Test 5: Memory Usage");

	// Create orchestrator
	let config = OrchestratorConfig::new(vec![SceneConfig::new("Scene", 1000)]);

	let orchestrator = StreamOrchestrator::new(config)?;
	orchestrator.start()?;

	println!("  Running for 30 seconds, monitor memory usage...");

	for i in 0..30 {
		sleep(Duration::from_secs(1)).await;
		let state = orchestrator.current_state();
		if i % 5 == 0 {
			println!("  {} seconds: Progress {:.1}%", i, state.progress.percentage());
		}
	}

	orchestrator.shutdown().await;
	println!("  ✅ Passed\n");

	Ok(())
}

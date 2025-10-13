use tokio::time::{sleep, Duration};
use ws_events::stream_orch::{OrchestratorConfig, SceneConfig, StreamOrchestrator};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
	// 1. Create configuration with scenes
	let config = OrchestratorConfig::new(vec![SceneConfig::new("Intro", 3000), SceneConfig::new("Main Show", 5000), SceneConfig::new("Outro", 2000)]);

	// 2. Create orchestrator
	let orchestrator = StreamOrchestrator::new(config)?;

	// 3. Subscribe to state updates
	let mut state_rx = orchestrator.subscribe();

	// 4. Spawn a task to monitor state changes
	tokio::spawn(async move {
		while state_rx.changed().await.is_ok() {
			let state = state_rx.borrow();

			if let Some(scene) = &state.current_active_scene {
				println!("Current Scene: {} | Progress: {:.1}%", scene, state.progress.percentage());
			}
		}
	});

	// 5. Start orchestration
	orchestrator.start()?;
	println!("▶️  Orchestrator started\n");

	// 6. Let it run for the full duration
	sleep(Duration::from_secs(11)).await;

	// 7. Clean shutdown
	orchestrator.shutdown().await;
	println!("\n✅ Done!");

	Ok(())
}

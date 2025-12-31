use cursorium::core::StreamOrchestrator;
use std::collections::HashMap;
use tokio::time::{sleep, Duration};
use tracing::Level;
use tracing_subscriber;
use ws_events::events::{ComponentPlacementData, FocusIntentData, OrchestratorCommandData, OrchestratorConfigData, PanelIntentData, SceneConfigData, UILayoutIntentData};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
	tracing_subscriber::fmt().with_max_level(Level::INFO).with_target(false).init();

	println!("\n🎬 Stream Orchestrator Demo\n");

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

/// Helper to create a UI layout list (Vec) as required by new types
fn create_demo_ui_list(title: &str, duration: i64) -> Vec<UILayoutIntentData> {
	let mut panels = HashMap::new();
	panels.insert(
		"main".to_string(),
		PanelIntentData {
			registry_key: "MainPanel".to_string(),
			props: None,
			focus: Some(FocusIntentData {
				region: "center".to_string(),
				intensity: 0.8,
			}),
			children: Some(vec![ComponentPlacementData {
				registry_key: "SceneTitle".to_string(),
				props: Some(serde_json::json!({ "title": title })),
				duration,
			}]),
		},
	);
	// Wrap in a Vec to match the new SceneConfigData requirement
	vec![UILayoutIntentData { panels }]
}

/// Demo 1: Parallel scenes
async fn demo_parallel_scenes() -> Result<(), Box<dyn std::error::Error>> {
	println!("🎭 Demo 1: Parallel Scene Execution");

	let config_data = OrchestratorConfigData {
		scenes: vec![
			SceneConfigData {
				scene_name: "Main Camera".to_string(),
				duration: 5000,
				start_time: Some(0),
				ui: create_demo_ui_list("Main Camera", 5000),
			},
			SceneConfigData {
				scene_name: "Lower Third".to_string(),
				duration: 5000,
				start_time: Some(0),
				ui: create_demo_ui_list("Lower Third", 5000),
			},
			SceneConfigData {
				scene_name: "Audio Track".to_string(),
				duration: 5000,
				start_time: Some(0),
				ui: vec![], // Empty vec instead of None
			},
		],
		tick_interval_ms: 50,
		loop_scenes: false,
	};

	let orchestrator = StreamOrchestrator::new(None)?;
	let mut state_rx = orchestrator.subscribe();

	orchestrator.configure(OrchestratorCommandData::Configure(config_data)).await?;
	orchestrator.start().await?;

	let monitor = tokio::spawn(async move {
		let mut last_count = 0;
		while state_rx.changed().await.is_ok() {
			let state = state_rx.borrow().clone();
			if state.active_lifetimes.len() != last_count {
				println!("⚡ t={}ms: Active lifetimes count: {}", state.current_time, state.active_lifetimes.len());
				last_count = state.active_lifetimes.len();
			}
			if state.is_complete() {
				break;
			}
		}
	});

	sleep(Duration::from_secs(6)).await;
	monitor.abort();
	orchestrator.shutdown().await;
	Ok(())
}

/// Demo 2: Overlapping scenes
async fn demo_overlapping_scenes() -> Result<(), Box<dyn std::error::Error>> {
	println!("\n🌊 Demo 2: Overlapping Event Lifetimes");

	let config_data = OrchestratorConfigData {
		scenes: vec![
			SceneConfigData {
				scene_name: "Background".to_string(),
				duration: 6000,
				start_time: Some(0),
				ui: create_demo_ui_list("Background", 6000),
			},
			SceneConfigData {
				scene_name: "Staggered Layer".to_string(),
				duration: 3000,
				start_time: Some(1000),
				ui: create_demo_ui_list("Staggered", 3000),
			},
		],
		tick_interval_ms: 100,
		loop_scenes: false,
	};

	let orchestrator = StreamOrchestrator::new(None)?;
	orchestrator.configure(OrchestratorCommandData::Configure(config_data)).await?;
	orchestrator.start().await?;

	sleep(Duration::from_secs(4)).await;
	orchestrator.shutdown().await;
	Ok(())
}

/// Demo 3: Lifetime tracking
async fn demo_lifetime_tracking() -> Result<(), Box<dyn std::error::Error>> {
	println!("\n🔍 Demo 3: Active Lifetime Inspection");

	let config_data = OrchestratorConfigData {
		scenes: vec![SceneConfigData {
			scene_name: "Primary".to_string(),
			duration: 3000,
			start_time: Some(0),
			ui: create_demo_ui_list("Primary", 3000),
		}],
		tick_interval_ms: 100,
		loop_scenes: false,
	};

	let orchestrator = StreamOrchestrator::new(None)?;
	let mut state_rx = orchestrator.subscribe();
	orchestrator.configure(OrchestratorCommandData::Configure(config_data)).await?;
	orchestrator.start().await?;

	tokio::spawn(async move {
		while state_rx.changed().await.is_ok() {
			let state = state_rx.borrow().clone();
			for lt in &state.active_lifetimes {
				if let Some(name) = lt.scene_name() {
					println!("   → [{}] id: {:?}, age: {}ms", name, lt.id, state.current_time - lt.started_at);
				}
			}
			if state.is_complete() {
				break;
			}
			sleep(Duration::from_millis(500)).await;
		}
	});

	sleep(Duration::from_secs(4)).await;
	orchestrator.shutdown().await;
	Ok(())
}

/// Demo 4: Dynamic Reconfiguration
async fn demo_dynamic_pipeline() -> Result<(), Box<dyn std::error::Error>> {
	println!("\n🔄 Demo 4: Dynamic Configuration Pipeline");

	let orchestrator = StreamOrchestrator::new(None)?;

	// Initial Config
	let config1 = OrchestratorConfigData {
		scenes: vec![SceneConfigData {
			scene_name: "Initial".to_string(),
			duration: 5000,
			start_time: Some(0),
			ui: create_demo_ui_list("Initial", 5000),
		}],
		tick_interval_ms: 100,
		loop_scenes: false,
	};

	orchestrator.configure(OrchestratorCommandData::Configure(config1)).await?;
	orchestrator.start().await?;
	sleep(Duration::from_secs(1)).await;

	println!("📥 Client Reconfiguring mid-stream...");
	let config2 = OrchestratorConfigData {
		scenes: vec![SceneConfigData {
			scene_name: "Hot Reloaded".to_string(),
			duration: 2000,
			start_time: Some(0),
			ui: create_demo_ui_list("New UI", 2000),
		}],
		tick_interval_ms: 100,
		loop_scenes: false,
	};

	orchestrator.configure(OrchestratorCommandData::Configure(config2)).await?;
	orchestrator.start().await?;
	sleep(Duration::from_secs(3)).await;

	orchestrator.shutdown().await;
	Ok(())
}

/// Demo 5: Sparse Timeline
async fn demo_sparse_timeline() -> Result<(), Box<dyn std::error::Error>> {
	println!("\n⏱️  Demo 5: Sparse Timeline with Gaps");

	let config_data = OrchestratorConfigData {
		scenes: vec![
			SceneConfigData {
				scene_name: "Burst 1".to_string(),
				duration: 1000,
				start_time: Some(0),
				ui: create_demo_ui_list("Burst 1", 1000),
			},
			SceneConfigData {
				scene_name: "Burst 2".to_string(),
				duration: 1000,
				start_time: Some(3000), // 2s gap
				ui: create_demo_ui_list("Burst 2", 1000),
			},
		],
		tick_interval_ms: 100,
		loop_scenes: false,
	};

	let orchestrator = StreamOrchestrator::new(None)?;
	orchestrator.configure(OrchestratorCommandData::Configure(config_data)).await?;
	orchestrator.start().await?;

	sleep(Duration::from_secs(5)).await;
	orchestrator.shutdown().await;
	Ok(())
}

/// Demo 6: Client Workflow
async fn demo_client_workflow() -> Result<(), Box<dyn std::error::Error>> {
	println!("\n🌐 Demo 6: Complete Client Workflow");

	let orchestrator = StreamOrchestrator::new(None)?;
	let config = OrchestratorConfigData {
		scenes: vec![SceneConfigData {
			scene_name: "Workflow Scene".to_string(),
			duration: 10000,
			start_time: Some(0),
			ui: create_demo_ui_list("Interactive", 10000),
		}],
		tick_interval_ms: 50,
		loop_scenes: false,
	};

	orchestrator.configure(OrchestratorCommandData::Configure(config)).await?;
	orchestrator.start().await?;
	sleep(Duration::from_secs(1)).await;

	orchestrator.pause().await?;
	println!("⏸ Paused...");
	sleep(Duration::from_secs(1)).await;

	orchestrator.resume().await?;
	println!("▶ Resumed...");
	sleep(Duration::from_secs(1)).await;

	orchestrator.stop().await?;
	println!("🛑 Stopped.");

	orchestrator.shutdown().await;
	Ok(())
}

/// Demo 7: Density analysis
async fn demo_event_density() -> Result<(), Box<dyn std::error::Error>> {
	println!("\n📈 Demo 7: Event Density Analysis");

	let config_data = OrchestratorConfigData {
		scenes: (0..5)
			.map(|i| SceneConfigData {
				scene_name: format!("Layer {}", i),
				duration: 2000,
				start_time: Some(0),
				ui: create_demo_ui_list(&format!("Layer {}", i), 2000),
			})
			.collect(),
		tick_interval_ms: 100,
		loop_scenes: false,
	};

	let orchestrator = StreamOrchestrator::new(None)?;
	orchestrator.configure(OrchestratorCommandData::Configure(config_data)).await?;
	orchestrator.start().await?;

	sleep(Duration::from_secs(3)).await;
	orchestrator.shutdown().await;
	Ok(())
}

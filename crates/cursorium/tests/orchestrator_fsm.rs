// tests/fsm_model_tests.rs
// Model-locking tests that enforce the FSM transition table

use cursorium::core::*;
use std::time::Duration;
use ws_events::events::{OrchestratorCommandData, OrchestratorConfigData, OrchestratorState, SceneConfigData};

// ============================================================================
// Test harness - deterministic engine wrapper
// ============================================================================

pub type Result<T> = std::result::Result<T, OrchestratorError>;

/// Simple async test engine
pub struct TestEngine {
	orchestrator: StreamOrchestrator,
}

impl TestEngine {
	pub fn new() -> Self {
		let orchestrator = StreamOrchestrator::new(None).unwrap();
		Self { orchestrator }
	}

	pub async fn configure(&self, config: OrchestratorCommandData) -> Result<()> {
		self.orchestrator.configure(config).await
	}
	pub async fn start(&self) -> Result<()> {
		self.orchestrator.start().await
	}
	pub async fn pause(&self) -> Result<()> {
		self.orchestrator.pause().await
	}
	pub async fn resume(&self) -> Result<()> {
		self.orchestrator.resume().await
	}
	pub async fn stop(&self) -> Result<()> {
		self.orchestrator.stop().await
	}
	pub async fn reset(&self) -> Result<()> {
		self.orchestrator.reset().await
	}
	pub fn force_scene(&self, scene: impl Into<String>) -> Result<()> {
		self.orchestrator.force_scene(scene)
	}
	pub fn skip_current_scene(&self) -> Result<()> {
		self.orchestrator.skip_current_scene()
	}

	pub async fn tick(&self, ms: u64) {
		tokio::time::sleep(Duration::from_millis(ms)).await;
	}

	pub fn state(&self) -> OrchestratorState {
		self.orchestrator.current_state()
	}
}

// Helper to generate a multi-scene config
fn test_config(looping: bool) -> OrchestratorCommandData {
	OrchestratorCommandData::Configure(OrchestratorConfigData {
		scenes: vec![
			SceneConfigData {
				scene_name: "A".into(),
				duration: 1000,
				start_time: None,
				metadata: None,
			},
			SceneConfigData {
				scene_name: "B".into(),
				duration: 1000,
				start_time: None,
				metadata: None,
			},
		],
		tick_interval_ms: 10,
		loop_scenes: looping,
	})
}

#[tokio::test]
async fn test_fsm_integrity_and_idempotency() {
	let engine = TestEngine::new();

	// 1. Unconfigured Invariants
	assert!(engine.start().await.is_err(), "Cannot start without config");

	engine.configure(test_config(false)).await.unwrap();

	// 2. Transition Guarding
	engine.start().await.unwrap();
	assert!(engine.start().await.is_err(), "Cannot start while running");

	// 3. Idempotency of Pause/Resume
	engine.pause().await.unwrap();
	assert!(engine.pause().await.is_ok(), "Pause must be idempotent");

	engine.resume().await.unwrap();
	assert!(engine.resume().await.is_ok(), "Resume must be idempotent");

	// 4. Reset vs Stop
	engine.stop().await.unwrap();
	assert!(!engine.state().is_running && engine.state().current_time == 0);
}

#[tokio::test]
async fn test_temporal_reconstruction_on_forced_seek() {
	let engine = TestEngine::new();
	engine.configure(test_config(false)).await.unwrap();

	// SCENARIO: Seek to Scene B while Idle
	// This tests if the engine can "catch up" state without actually playing
	engine.force_scene("B").unwrap();

	// Use a small sleep to let the unbuffered command process
	tokio::time::sleep(Duration::from_millis(50)).await;

	let state = engine.state();
	assert_eq!(state.current_active_scene.as_deref(), Some("B"));
	assert!(state.current_time >= 1000, "Time should have jumped to start of Scene B");
	assert!(!state.is_running, "Seek should not trigger auto-start");
}

#[tokio::test]
async fn test_reconfiguration_mid_stream() {
	let engine = TestEngine::new();
	engine.configure(test_config(false)).await.unwrap();
	engine.start().await.unwrap();

	// Advance into the middle of the first scene
	tokio::time::sleep(Duration::from_millis(200)).await;

	// SCENARIO: Hot-reconfigure while running
	// Invariant: New config must kill the old session and reset to Idle
	engine.configure(test_config(false)).await.unwrap();

	let state = engine.state();
	assert_eq!(state.current_time, 0);
	assert!(!state.is_running, "Engine must stop running after reconfig");
	assert!(state.active_lifetimes.is_empty(), "State must be purged");
}

#[tokio::test]
async fn test_looping_boundary_conditions() {
	let engine = TestEngine::new();
	// Set very fast interval for testing
	let mut config = test_config(true);
	if let OrchestratorCommandData::Configure(ref mut d) = config {
		d.tick_interval_ms = 1;
	}

	engine.configure(config).await.unwrap();
	engine.start().await.unwrap();

	// SCENARIO: Cross the finish line
	// Invariant: If loop_scenes=true, time should wrap and Cursor must reset
	tokio::time::sleep(Duration::from_millis(2500)).await;

	let state = engine.state();
	assert!(state.current_time < 2000, "Time should have wrapped around");
	assert!(state.is_running, "Should still be running while looping");
	// Check if we are back in Scene A or B (depending on exact timing)
	assert!(state.current_active_scene.is_some());
}

#[tokio::test]
async fn test_skip_behavior_at_terminal_scene() {
	let engine = TestEngine::new();
	engine.configure(test_config(false)).await.unwrap();
	engine.start().await.unwrap();

	// Jump to last scene
	engine.force_scene("B").unwrap();
	tokio::time::sleep(Duration::from_millis(50)).await;

	// SCENARIO: Skip when there is no "Next"
	// Invariant: Should handle gracefully (usually a no-op or warning)
	let result = engine.skip_current_scene();
	assert!(result.is_ok(), "Skip should not crash if no next scene exists");

	let state = engine.state();
	assert_eq!(state.current_active_scene.as_deref(), Some("B"), "Should remain on last scene");
}

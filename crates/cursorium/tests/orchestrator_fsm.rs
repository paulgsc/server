// tests/fsm_model_tests.rs
// Model-locking tests that enforce the FSM transition table

use cursorium::core::*;
use std::collections::HashMap;
use std::time::Duration;
use ws_events::events::{ComponentPlacementData, FocusIntentData, UILayoutIntentData};
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

	pub async fn configure(&self, config: OrchestratorConfigData) -> Result<()> {
		self.orchestrator.configure(OrchestratorCommandData::Configure(config)).await
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
fn test_config(looping: bool) -> OrchestratorConfigData {
	let mut ui_scene_a = HashMap::new();
	ui_scene_a.insert(
		"header".to_string(),
		ComponentPlacementData {
			registry_key: "HeaderComponent".to_string(),
			props: Some(serde_json::json!({"title": "Welcome to Scene A"})),
		},
	);

	let mut ui_scene_b = HashMap::new();
	ui_scene_b.insert(
		"header".to_string(),
		ComponentPlacementData {
			registry_key: "HeaderComponent".to_string(),
			props: Some(serde_json::json!({"title": "Welcome to Scene B"})),
		},
	);

	OrchestratorConfigData {
		scenes: vec![
			SceneConfigData {
				scene_name: "A".into(),
				duration: 1000,
				start_time: None,
				ui: Some(UILayoutIntentData {
					content: ui_scene_a,
					focus: Some(FocusIntentData {
						region: "main".to_string(),
						intensity: 0.8,
					}),
				}),
			},
			SceneConfigData {
				scene_name: "B".into(),
				duration: 1500,
				start_time: None,
				ui: Some(UILayoutIntentData {
					content: ui_scene_b,
					focus: Some(FocusIntentData {
						region: "main".to_string(),
						intensity: 0.5,
					}),
				}),
			},
		],
		tick_interval_ms: 10,
		loop_scenes: looping,
	}
}

// ============================================================================
// FSM Transition Tests
// ============================================================================

#[tokio::test]
async fn test_fsm_transition_guards_and_idempotency() {
	let engine = TestEngine::new();

	// Unconfigured -> Start should fail
	assert!(engine.start().await.is_err(), "FSM violation: Start from Unconfigured");

	engine.configure(test_config(false)).await.unwrap();

	// Idle -> Start -> Start should fail
	engine.start().await.unwrap();
	assert!(engine.start().await.is_err(), "FSM violation: Start from Running");

	// Running -> Pause (idempotent)
	engine.pause().await.unwrap();
	assert!(engine.pause().await.is_ok(), "FSM violation: Pause not idempotent");

	// Paused -> Resume (idempotent)
	engine.resume().await.unwrap();
	assert!(engine.resume().await.is_ok(), "FSM violation: Resume not idempotent");

	// Running -> Stop -> Idle
	engine.stop().await.unwrap();
	let state = engine.state();
	assert!(!state.is_running, "State violation: is_running after Stop");
	assert_eq!(state.current_time, 0, "State violation: time not reset after Stop");

	// Idle -> Pause should fail
	assert!(engine.pause().await.is_err(), "FSM violation: Pause from Idle");

	// Idle -> Resume should fail
	assert!(engine.resume().await.is_err(), "FSM violation: Resume from Idle");
}

#[tokio::test]
async fn test_reconfiguration_kills_active_session() {
	let engine = TestEngine::new();
	engine.configure(test_config(false)).await.unwrap();
	engine.start().await.unwrap();

	// Advance into scene A
	tokio::time::sleep(Duration::from_millis(300)).await;

	let pre_state = engine.state();
	assert!(pre_state.is_running, "Precondition: should be running");
	assert!(pre_state.current_time > 0, "Precondition: time should advance");

	// Hot-reconfigure while running - should kill session
	engine.configure(test_config(false)).await.unwrap();

	let post_state = engine.state();
	assert!(!post_state.is_running, "Invariant: reconfig must stop engine");
	assert_eq!(post_state.current_time, 0, "Invariant: reconfig must reset time");
	assert!(post_state.active_lifetimes.is_empty(), "Invariant: reconfig must clear state");
}

#[tokio::test]
async fn test_temporal_reconstruction_via_force_scene() {
	let engine = TestEngine::new();
	engine.configure(test_config(false)).await.unwrap();

	// Force scene while Idle - tests reconstruction without playback
	engine.force_scene("B").unwrap();
	tokio::time::sleep(Duration::from_millis(50)).await;

	let state = engine.state();
	assert_eq!(state.current_active_scene.as_deref(), Some("B"), "Invariant: force_scene must activate target scene");
	assert!(state.current_time >= 1000, "Invariant: force_scene must reconstruct time to scene start");
	assert!(!state.is_running, "Invariant: force_scene must not auto-start playback");
}

#[tokio::test]
async fn test_loop_wrapping_and_cursor_reset() {
	let engine = TestEngine::new();

	// Fast tick for rapid loop testing
	let mut config = test_config(true);
	config.tick_interval_ms = 1;

	engine.configure(config).await.unwrap();
	engine.start().await.unwrap();

	// Run past total duration (A:1000ms + B:1500ms = 2500ms)
	tokio::time::sleep(Duration::from_millis(2700)).await;

	let state = engine.state();
	assert!(state.current_time < 2500, "Invariant: loop_scenes must wrap time");
	assert!(state.is_running, "Invariant: loop_scenes must keep running");
	assert!(state.current_active_scene.is_some(), "Invariant: loop_scenes must maintain active scene");
}

#[tokio::test]
async fn test_skip_at_terminal_scene_boundary() {
	let engine = TestEngine::new();
	engine.configure(test_config(false)).await.unwrap();
	engine.start().await.unwrap();

	// Jump to last scene
	engine.force_scene("B").unwrap();
	tokio::time::sleep(Duration::from_millis(50)).await;

	let pre_scene = engine.state().current_active_scene.clone();

	// Skip when no next scene exists
	let result = engine.skip_current_scene();
	assert!(result.is_ok(), "Invariant: skip at boundary must not panic");

	let post_scene = engine.state().current_active_scene;
	assert_eq!(pre_scene, post_scene, "Invariant: skip at terminal must be no-op");
}

#[tokio::test]
async fn test_pause_resume_preserves_temporal_position() {
	let engine = TestEngine::new();
	engine.configure(test_config(false)).await.unwrap();
	engine.start().await.unwrap();

	// Advance partway through scene A
	tokio::time::sleep(Duration::from_millis(500)).await;

	let time_before_pause = engine.state().current_time;

	engine.pause().await.unwrap();
	tokio::time::sleep(Duration::from_millis(300)).await; // Wait while paused

	let time_during_pause = engine.state().current_time;
	assert!((time_during_pause - time_before_pause).abs() < 100, "Invariant: time must not advance during pause");

	engine.resume().await.unwrap();
	tokio::time::sleep(Duration::from_millis(200)).await;

	let time_after_resume = engine.state().current_time;
	assert!(time_after_resume > time_during_pause, "Invariant: time must advance after resume");
}

#[tokio::test]
async fn test_reset_clears_state_but_preserves_config() {
	let engine = TestEngine::new();
	engine.configure(test_config(false)).await.unwrap();
	engine.start().await.unwrap();

	// Advance and accumulate state
	tokio::time::sleep(Duration::from_millis(400)).await;

	let pre_reset = engine.state();
	assert!(pre_reset.current_time > 0, "Precondition: time advanced");
	assert!(pre_reset.is_running, "Precondition: running");

	engine.reset().await.unwrap();

	let post_reset = engine.state();
	assert_eq!(post_reset.current_time, 0, "Invariant: reset clears time");
	assert!(!post_reset.is_running, "Invariant: reset stops playback");
	assert!(post_reset.active_lifetimes.is_empty(), "Invariant: reset clears lifetimes");

	// Config should be preserved - can start again
	assert!(engine.start().await.is_ok(), "Invariant: reset preserves config");
}

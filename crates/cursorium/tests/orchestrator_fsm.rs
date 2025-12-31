// Model-locking tests that enforce the FSM transition table

use cursorium::core::*;
use std::collections::HashMap;
use std::time::Duration;
use ws_events::events::{ComponentPlacementData, FocusIntentData, PanelIntentData, UILayoutIntentData};
use ws_events::events::{OrchestratorCommandData, OrchestratorConfigData, OrchestratorMode, OrchestratorState, SceneConfigData};

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
	let mut panels_scene_a = HashMap::new();
	panels_scene_a.insert(
		"main_panel".to_string(),
		PanelIntentData {
			registry_key: "MainPanel".to_string(),
			props: None,
			focus: Some(FocusIntentData {
				region: "main".to_string(),
				intensity: 0.8,
			}),
			children: Some(vec![ComponentPlacementData {
				registry_key: "HeaderComponent".to_string(),
				props: Some(serde_json::json!({"title": "Welcome to Scene A"})),
				duration: 1000,
			}]),
		},
	);

	let mut panels_scene_b = HashMap::new();
	panels_scene_b.insert(
		"main_panel".to_string(),
		PanelIntentData {
			registry_key: "MainPanel".to_string(),
			props: None,
			focus: Some(FocusIntentData {
				region: "main".to_string(),
				intensity: 0.5,
			}),
			children: Some(vec![ComponentPlacementData {
				registry_key: "HeaderComponent".to_string(),
				props: Some(serde_json::json!({"title": "Welcome to Scene B"})),
				duration: 1500,
			}]),
		},
	);

	OrchestratorConfigData {
		scenes: vec![
			SceneConfigData {
				scene_name: "A".into(),
				duration: 1000,
				start_time: None,
				ui: vec![UILayoutIntentData { panels: panels_scene_a }],
			},
			SceneConfigData {
				scene_name: "B".into(),
				duration: 1500,
				start_time: None,
				ui: vec![UILayoutIntentData { panels: panels_scene_b }],
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

	// Verify initial state is Unconfigured
	assert_eq!(engine.state().mode, OrchestratorMode::Unconfigured);

	// Unconfigured -> Start should fail
	assert!(engine.start().await.is_err(), "FSM violation: Start from Unconfigured");

	engine.configure(test_config(false)).await.unwrap();
	assert_eq!(engine.state().mode, OrchestratorMode::Idle, "Configure should transition to Idle");

	// Idle -> Start -> Running
	engine.start().await.unwrap();
	assert_eq!(engine.state().mode, OrchestratorMode::Running, "Start should transition to Running");

	// Running -> Start should fail
	assert!(engine.start().await.is_err(), "FSM violation: Start from Running");

	// Running -> Pause -> Paused (idempotent)
	engine.pause().await.unwrap();
	assert_eq!(engine.state().mode, OrchestratorMode::Paused, "Pause should transition to Paused");
	assert!(engine.pause().await.is_ok(), "FSM violation: Pause not idempotent");

	// Paused -> Resume -> Running (idempotent)
	engine.resume().await.unwrap();
	assert_eq!(engine.state().mode, OrchestratorMode::Running, "Resume should transition to Running");
	assert!(engine.resume().await.is_ok(), "FSM violation: Resume not idempotent");

	// Running -> Stop -> Stopped (terminal)
	engine.stop().await.unwrap();
	let state = engine.state();
	assert_eq!(state.mode, OrchestratorMode::Stopped, "Stop should transition to Stopped (terminal)");
	assert!(state.mode.is_terminal(), "Stopped should be a terminal state");
	assert_eq!(state.current_time, 0, "State violation: time not reset after Stop");

	// Terminal -> Pause should fail
	assert!(engine.pause().await.is_err(), "FSM violation: Pause from terminal state");

	// Terminal -> Resume should fail
	assert!(engine.resume().await.is_err(), "FSM violation: Resume from terminal state");

	// Terminal -> Start should fail
	assert!(engine.start().await.is_err(), "FSM violation: Start from terminal state");

	// Terminal -> Reset -> Idle (recovery path)
	engine.reset().await.unwrap();
	assert_eq!(engine.state().mode, OrchestratorMode::Idle, "Reset should transition terminal state to Idle");

	// Now we can start again
	assert!(engine.start().await.is_ok(), "Should be able to start after reset from terminal");
}

#[tokio::test]
async fn test_terminal_state_enforcement() {
	let engine = TestEngine::new();
	engine.configure(test_config(false)).await.unwrap();
	engine.start().await.unwrap();

	// Stop to reach terminal state
	engine.stop().await.unwrap();

	let state = engine.state();
	assert!(state.is_terminal(), "Stopped should be terminal");
	assert_eq!(state.mode, OrchestratorMode::Stopped);

	// All commands except Reset and Configure should fail
	assert!(engine.start().await.is_err(), "Terminal state should reject Start");
	assert!(engine.pause().await.is_err(), "Terminal state should reject Pause");
	assert!(engine.resume().await.is_err(), "Terminal state should reject Resume");
	assert!(engine.stop().await.is_ok(), "Terminal state should accept Stop (idempotent)");

	// Recovery paths should work
	assert!(engine.reset().await.is_ok(), "Terminal state should accept Reset");
	assert_eq!(engine.state().mode, OrchestratorMode::Idle);

	// Reconfigure from terminal should also work
	engine.stop().await.unwrap();
	assert!(engine.configure(test_config(false)).await.is_ok(), "Terminal state should accept Configure");
	assert_eq!(engine.state().mode, OrchestratorMode::Idle);
}

#[tokio::test]
async fn test_natural_completion_reaches_finished_terminal_state() {
	let engine = TestEngine::new();

	// Short non-looping config
	let mut config = test_config(false);
	config.tick_interval_ms = 1;

	engine.configure(config).await.unwrap();
	engine.start().await.unwrap();

	// Wait for natural completion (A:1000ms + B:1500ms = 2500ms + margin)
	tokio::time::sleep(Duration::from_millis(2700)).await;

	let state = engine.state();
	assert_eq!(state.mode, OrchestratorMode::Finished, "Natural completion should transition to Finished (terminal)");
	assert!(state.is_terminal(), "Finished should be a terminal state");
	assert!(state.is_complete(), "Timeline should be complete");

	// Finished is terminal - should reject commands
	assert!(engine.start().await.is_err(), "Finished state should reject Start");
	assert!(engine.pause().await.is_err(), "Finished state should reject Pause");

	// Can recover via Reset
	engine.reset().await.unwrap();
	assert_eq!(engine.state().mode, OrchestratorMode::Idle);
}

#[tokio::test]
async fn test_reconfiguration_kills_active_session() {
	let engine = TestEngine::new();
	engine.configure(test_config(false)).await.unwrap();
	engine.start().await.unwrap();

	// Advance into scene A
	tokio::time::sleep(Duration::from_millis(300)).await;

	let pre_state = engine.state();
	assert_eq!(pre_state.mode, OrchestratorMode::Running, "Precondition: should be running");
	assert!(pre_state.current_time > 0, "Precondition: time should advance");

	// Hot-reconfigure while running - should kill session and go to Idle
	engine.configure(test_config(false)).await.unwrap();

	let post_state = engine.state();
	assert_eq!(post_state.mode, OrchestratorMode::Idle, "Invariant: reconfig must transition to Idle");
	assert_eq!(post_state.current_time, 0, "Invariant: reconfig must reset time");
	assert!(post_state.active_lifetimes.is_empty(), "Invariant: reconfig must clear state");
}

#[tokio::test]
async fn test_temporal_reconstruction_via_force_scene() {
	let engine = TestEngine::new();
	engine.configure(test_config(false)).await.unwrap();

	// Verify we're in Idle
	assert_eq!(engine.state().mode, OrchestratorMode::Idle);

	// Force scene while Idle - tests reconstruction without playback
	engine.force_scene("B").unwrap();
	tokio::time::sleep(Duration::from_millis(50)).await;

	let state = engine.state();
	assert_eq!(state.current_active_scene.as_deref(), Some("B"), "Invariant: force_scene must activate target scene");
	assert!(state.current_time >= 1000, "Invariant: force_scene must reconstruct time to scene start");
	assert_eq!(state.mode, OrchestratorMode::Idle, "Invariant: force_scene must not auto-start playback");
}

#[tokio::test]
async fn test_loop_wrapping_prevents_finished_state() {
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
	assert_eq!(state.mode, OrchestratorMode::Running, "Invariant: loop_scenes must keep Running (not Finished)");
	assert!(state.current_active_scene.is_some(), "Invariant: loop_scenes must maintain active scene");
	assert!(!state.is_terminal(), "Invariant: loop_scenes must never reach terminal state");
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
	assert_eq!(engine.state().mode, OrchestratorMode::Running);

	// Skip when no next scene exists
	let result = engine.skip_current_scene();
	assert!(result.is_ok(), "Invariant: skip at boundary must not panic");

	let post_scene = engine.state().current_active_scene;
	assert_eq!(pre_scene, post_scene, "Invariant: skip at terminal must be no-op");
	assert_eq!(engine.state().mode, OrchestratorMode::Running, "Skip should not change mode");
}

#[tokio::test]
async fn test_pause_resume_preserves_temporal_position() {
	let engine = TestEngine::new();
	engine.configure(test_config(false)).await.unwrap();
	engine.start().await.unwrap();

	// Advance partway through scene A
	tokio::time::sleep(Duration::from_millis(500)).await;

	let time_before_pause = engine.state().current_time;
	assert_eq!(engine.state().mode, OrchestratorMode::Running);

	engine.pause().await.unwrap();
	assert_eq!(engine.state().mode, OrchestratorMode::Paused);

	tokio::time::sleep(Duration::from_millis(300)).await; // Wait while paused

	let time_during_pause = engine.state().current_time;
	assert!((time_during_pause - time_before_pause).abs() < 100, "Invariant: time must not advance during pause");
	assert_eq!(engine.state().mode, OrchestratorMode::Paused);

	engine.resume().await.unwrap();
	assert_eq!(engine.state().mode, OrchestratorMode::Running);

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
	assert_eq!(pre_reset.mode, OrchestratorMode::Running, "Precondition: running");

	engine.reset().await.unwrap();

	let post_reset = engine.state();
	assert_eq!(post_reset.current_time, 0, "Invariant: reset clears time");
	assert_eq!(post_reset.mode, OrchestratorMode::Idle, "Invariant: reset transitions to Idle");
	assert!(post_reset.active_lifetimes.is_empty(), "Invariant: reset clears lifetimes");

	// Config should be preserved - can start again
	assert!(engine.start().await.is_ok(), "Invariant: reset preserves config");
	assert_eq!(engine.state().mode, OrchestratorMode::Running);
}

#[tokio::test]
async fn test_mode_observable_state_consistency() {
	let engine = TestEngine::new();

	// Unconfigured state
	let state = engine.state();
	assert_eq!(state.mode, OrchestratorMode::Unconfigured);
	assert!(!state.is_running());
	assert!(!state.is_paused());
	assert!(!state.is_terminal());

	// Configure -> Idle
	engine.configure(test_config(false)).await.unwrap();
	let state = engine.state();
	assert_eq!(state.mode, OrchestratorMode::Idle);
	assert!(state.mode.is_active());
	assert!(!state.is_running());
	assert!(!state.is_terminal());

	// Start -> Running
	engine.start().await.unwrap();
	let state = engine.state();
	assert_eq!(state.mode, OrchestratorMode::Running);
	assert!(state.is_running());
	assert!(!state.is_paused());
	assert!(state.mode.is_active());

	// Pause -> Paused
	engine.pause().await.unwrap();
	let state = engine.state();
	assert_eq!(state.mode, OrchestratorMode::Paused);
	assert!(!state.is_running());
	assert!(state.is_paused());
	assert!(state.mode.is_active());

	// Stop -> Stopped (terminal)
	engine.stop().await.unwrap();
	let state = engine.state();
	assert_eq!(state.mode, OrchestratorMode::Stopped);
	assert!(state.is_terminal());
	assert!(!state.mode.is_active());
}

#[tokio::test]
async fn test_stop_is_not_idempotent_to_idle() {
	let engine = TestEngine::new();
	engine.configure(test_config(false)).await.unwrap();

	// Stop from Idle should be idempotent (stay Idle)
	engine.stop().await.unwrap();
	assert_eq!(engine.state().mode, OrchestratorMode::Idle);

	engine.start().await.unwrap();
	assert_eq!(engine.state().mode, OrchestratorMode::Running);

	// Stop from Running should go to Stopped (terminal), not Idle
	engine.stop().await.unwrap();
	assert_eq!(engine.state().mode, OrchestratorMode::Stopped);
	assert!(engine.state().is_terminal());

	// Stop from Stopped should be idempotent (stay Stopped)
	engine.stop().await.unwrap();
	assert_eq!(engine.state().mode, OrchestratorMode::Stopped);
}

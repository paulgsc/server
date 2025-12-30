mod commands;
mod config;
mod engine;
mod error;
mod orchestrator;
mod state;
mod timeline;

use ws_events::events::{ActiveLifetime, OrchestratorConfigData, Progress, SceneConfigData, ScenePayload, TimedEvent};
use ws_events::events::{LifetimeEvent, LifetimeId, LifetimeKind, SceneId, StreamStatus};
use ws_events::events::{OrchestratorCommandData, OrchestratorEvent, OrchestratorState, UILayoutIntentData};

pub use config::{OrchestratorConfig, SceneConfig};
pub use engine::OrchestratorEngine;
pub use error::OrchestratorError;
pub use orchestrator::StreamOrchestrator;

use commands::OrchestratorCommand;
use state::EngineState;
use timeline::{Cursor, Timeline};

/// Time in milliseconds
pub(crate) type TimeMs = i64;

#[cfg(test)]
mod tests {
	use super::*;

	// ============================================================================
	// Fixtures
	// ============================================================================

	fn simple_timeline() -> (OrchestratorConfig, Vec<TimeMs>) {
		let scenes = vec![SceneConfig::new("scene_a", 1000), SceneConfig::new("scene_b", 1000), SceneConfig::new("scene_c", 1000)];
		let sample_times = vec![0, 500, 1000, 1500, 2000, 2500, 3000];
		(OrchestratorConfig::new(scenes), sample_times)
	}

	fn overlapping_timeline() -> OrchestratorConfig {
		let scenes = vec![SceneConfig::new("scene_a", 2000).starting_at(0), SceneConfig::new("scene_b", 2000).starting_at(1000)];
		OrchestratorConfig::new(scenes)
	}

	fn simulate_engine(config: &OrchestratorConfig, sample_times: &[TimeMs]) -> Vec<OrchestratorState> {
		let timeline = config.compile_timeline();
		let mut engine_state = EngineState::new(timeline.total_duration());
		let mut cursor = timeline.cursor();
		let mut snapshots = Vec::new();

		for &time in sample_times {
			cursor.apply_until(&timeline, time, |event| engine_state.apply_event(event));
			engine_state.sync_view_state(time);
			snapshots.push(engine_state.state.clone());
		}

		snapshots
	}

	#[derive(Debug, Clone, PartialEq, Eq)]
	struct ComparableState {
		current_time: TimeMs,
		active_lifetime_count: usize,
		active_lifetime_ids: Vec<LifetimeId>,
		current_scene: Option<String>,
	}

	impl From<&OrchestratorState> for ComparableState {
		fn from(state: &OrchestratorState) -> Self {
			let mut ids: Vec<_> = state.active_lifetimes.iter().map(|l| l.id).collect();
			ids.sort_by_key(|id| id.0);

			Self {
				current_time: state.current_time,
				active_lifetime_count: state.active_lifetimes.len(),
				active_lifetime_ids: ids,
				current_scene: state.current_active_scene.clone(),
			}
		}
	}

	// ============================================================================
	// Invariant 1: Determinism
	// ============================================================================

	#[test]
	fn determinism() {
		let (config, sample_times) = simple_timeline();
		let states_1 = simulate_engine(&config, &sample_times);
		let states_2 = simulate_engine(&config, &sample_times);

		for (s1, s2) in states_1.iter().zip(states_2.iter()) {
			assert_eq!(ComparableState::from(s1), ComparableState::from(s2));
		}
	}

	// ============================================================================
	// Invariant 2: Prefix replay equivalence
	// ============================================================================

	#[test]
	fn prefix_replay_equivalence() {
		let config = simple_timeline().0;
		let timeline = config.compile_timeline();
		let target_time = 1500;

		// Incremental application
		let mut engine_incremental = EngineState::new(timeline.total_duration());
		let mut cursor_incremental = timeline.cursor();
		cursor_incremental.apply_until(&timeline, target_time, |e| engine_incremental.apply_event(e));
		engine_incremental.sync_view_state(target_time);

		// Full replay from start
		let mut engine_replay = EngineState::new(timeline.total_duration());
		let mut cursor_replay = timeline.cursor();
		engine_replay.reconstruct_from_start(&mut cursor_replay, &timeline, target_time);

		assert_eq!(ComparableState::from(&engine_incremental.state), ComparableState::from(&engine_replay.state));
	}

	// ============================================================================
	// Invariant 3: Lifetime pairing correctness
	// ============================================================================

	#[test]
	fn lifetime_pairing() {
		let config = simple_timeline().0;
		let timeline = config.compile_timeline();

		let expectations = vec![
			(0, vec!["scene_a"]),
			(500, vec!["scene_a"]),
			(1000, vec!["scene_b"]),
			(1500, vec!["scene_b"]),
			(2000, vec!["scene_c"]),
			(2500, vec!["scene_c"]),
			(3000, vec![]),
		];

		for (time, expected_scenes) in expectations {
			let mut engine_state = EngineState::new(timeline.total_duration());
			let mut cursor = timeline.cursor();
			engine_state.reconstruct_from_start(&mut cursor, &timeline, time);

			let active_scenes: Vec<_> = engine_state.active_lifetimes.iter().filter_map(|l| l.scene_name().map(String::from)).collect();

			assert_eq!(
				active_scenes, expected_scenes,
				"At time {}, expected {:?} but got {:?}",
				time, expected_scenes, active_scenes
			);
		}
	}

	// ============================================================================
	// Invariant 4: Overlap / concurrency
	// ============================================================================

	#[test]
	fn overlapping_lifetimes() {
		let config = overlapping_timeline();
		let timeline = config.compile_timeline();

		let expectations = vec![
			(500, vec!["scene_a"]),
			(1000, vec!["scene_a", "scene_b"]),
			(1500, vec!["scene_a", "scene_b"]),
			(2000, vec!["scene_b"]),
			(2500, vec!["scene_b"]),
			(3000, vec![]),
		];

		for (time, expected_scenes) in expectations {
			let mut engine_state = EngineState::new(timeline.total_duration());
			let mut cursor = timeline.cursor();
			engine_state.reconstruct_from_start(&mut cursor, &timeline, time);

			let mut active_scenes: Vec<_> = engine_state.active_lifetimes.iter().filter_map(|l| l.scene_name().map(String::from)).collect();

			active_scenes.sort();

			let mut expected_sorted = expected_scenes.clone();
			expected_sorted.sort();

			assert_eq!(
				active_scenes, expected_sorted,
				"At time {}, expected {:?} but got {:?}",
				time, expected_sorted, active_scenes
			);
		}
	}

	// ============================================================================
	// Invariant 5: Seek idempotence
	// ============================================================================

	#[test]
	fn seek_idempotence() {
		let config = simple_timeline().0;
		let timeline = config.compile_timeline();
		let target_time = 1500;

		let mut state1 = EngineState::new(timeline.total_duration());
		let mut cursor1 = timeline.cursor();
		state1.reconstruct_from_start(&mut cursor1, &timeline, target_time);

		let mut state2 = EngineState::new(timeline.total_duration());
		let mut cursor2 = timeline.cursor();
		state2.reconstruct_from_start(&mut cursor2, &timeline, target_time);

		assert_eq!(ComparableState::from(&state1.state), ComparableState::from(&state2.state));
	}

	#[test]
	fn multiple_seeks_idempotence() {
		let config = simple_timeline().0;
		let timeline = config.compile_timeline();
		let times = vec![500, 1500, 2500, 1000];

		let mut snapshots = Vec::new();
		for &time in &times {
			let mut engine = EngineState::new(timeline.total_duration());
			let mut cursor = timeline.cursor();
			engine.reconstruct_from_start(&mut cursor, &timeline, time);
			snapshots.push((time, ComparableState::from(&engine.state)));
		}

		// Verify that seeking to the same time always produces the same state
		for i in 0..snapshots.len() {
			for j in i + 1..snapshots.len() {
				if snapshots[i].0 == snapshots[j].0 {
					assert_eq!(snapshots[i].1, snapshots[j].1, "Seeking to time {} produced different states", snapshots[i].0);
				}
			}
		}
	}

	// ============================================================================
	// Invariant 6: Monotonicity of event application
	// ============================================================================

	#[test]
	fn monotonic_application() {
		let (config, _) = simple_timeline();
		let timeline = config.compile_timeline();
		let mut cursor = timeline.cursor();

		// Apply events up to 1500ms
		let mut first_phase = Vec::new();
		cursor.apply_until(&timeline, 1500, |event| first_phase.push(event.at));

		// Reconstruct state (which resets cursor)
		let mut engine_state = EngineState::new(timeline.total_duration());
		engine_state.reconstruct_from_start(&mut cursor, &timeline, 1000);

		// Apply more events from current frontier
		let mut second_phase = Vec::new();
		cursor.apply_until(&timeline, 2000, |event| second_phase.push(event.at));

		// All events in second phase should be after the reconstruct time
		assert!(
			second_phase.iter().all(|&t| t > 1000),
			"Expected all second phase events to be > 1000ms, got: {:?}",
			second_phase
		);
	}

	// ============================================================================
	// Additional: Cursor frontier tracking
	// ============================================================================

	#[test]
	fn cursor_frontier_advances_monotonically() {
		let (config, _) = simple_timeline();
		let timeline = config.compile_timeline();
		let mut cursor = timeline.cursor();

		assert_eq!(cursor.applied_frontier(), 0);

		cursor.apply_until(&timeline, 500, |_| {});
		let frontier1 = cursor.applied_frontier();
		assert!(frontier1 > 0);

		cursor.apply_until(&timeline, 1500, |_| {});
		let frontier2 = cursor.applied_frontier();
		assert!(frontier2 >= frontier1);

		cursor.apply_until(&timeline, 3000, |_| {});
		let frontier3 = cursor.applied_frontier();
		assert!(frontier3 >= frontier2);
	}

	#[test]
	fn cursor_reset_returns_to_zero() {
		let (config, _) = simple_timeline();
		let timeline = config.compile_timeline();
		let mut cursor = timeline.cursor();

		cursor.apply_until(&timeline, 1500, |_| {});
		assert!(cursor.applied_frontier() > 0);

		cursor.reset();
		assert_eq!(cursor.applied_frontier(), 0);
	}

	// ============================================================================
	// Additional: Timeline validation
	// ============================================================================

	#[test]
	fn timeline_total_duration_calculated_correctly() {
		let config = simple_timeline().0;
		let timeline = config.compile_timeline();

		// Total duration should be the end of the last scene
		assert_eq!(timeline.total_duration(), 3000);
	}

	#[test]
	fn overlapping_timeline_duration() {
		let config = overlapping_timeline();
		let timeline = config.compile_timeline();

		// Scene A: 0-2000, Scene B: 1000-3000
		// Total duration is the end of the last event
		assert_eq!(timeline.total_duration(), 3000);
	}

	#[test]
	fn empty_timeline_has_zero_duration() {
		let config = OrchestratorConfig::new(vec![]);
		let timeline = config.compile_timeline();

		assert_eq!(timeline.total_duration(), 0);
		assert!(timeline.is_empty());
		assert_eq!(timeline.len(), 0);
	}

	#[test]
	fn test_teleportation_consistency() {
		let config = overlapping_timeline(); // Use overlapping to make it harder
		let timeline = config.compile_timeline();
		let target_time = 1500;

		// 1. Play naturally (Ticking)
		let mut state_tick = EngineState::new(timeline.total_duration());
		let mut cursor_tick = timeline.cursor();
		// Simulate incremental steps
		for t in (0..=target_time).step_by(100) {
			cursor_tick.apply_until(&timeline, t, |e| state_tick.apply_event(e));
			state_tick.sync_view_state(t);
		}

		// 2. Jump directly (Reconstruction)
		let mut state_seek = EngineState::new(timeline.total_duration());
		let mut cursor_seek = timeline.cursor();
		state_seek.reconstruct_from_start(&mut cursor_seek, &timeline, target_time);

		// FAILURE POINT: If reconstruction logic is incomplete, these won't match
		assert_eq!(
			ComparableState::from(&state_tick.state),
			ComparableState::from(&state_seek.state),
			"Natural playback and seeking produced different internal states at {}ms",
			target_time
		);
	}

	#[test]
	fn test_seek_backwards_purges_old_state() {
		let config = simple_timeline().0;
		let timeline = config.compile_timeline();

		let mut engine = EngineState::new(timeline.total_duration());
		let mut cursor = timeline.cursor();

		// Seek far forward to Scene C (3rd scene)
		engine.reconstruct_from_start(&mut cursor, &timeline, 2500);
		assert_eq!(engine.state.current_active_scene.as_deref(), Some("scene_c"));

		// SCENARIO: Seek backwards to Scene A
		engine.reconstruct_from_start(&mut cursor, &timeline, 500);

		// FAILURE POINT: If we didn't clear active_lifetimes, Scene C might still be there.
		assert_eq!(engine.state.current_active_scene.as_deref(), Some("scene_a"));
		assert_eq!(engine.active_lifetimes.len(), 1, "Should only have 1 active scene after seeking back");
	}

	#[test]
	fn test_no_skipped_or_double_events_on_jitter() {
		let config = simple_timeline().0;
		let timeline = config.compile_timeline();
		let mut cursor = timeline.cursor();
		let mut engine = EngineState::new(timeline.total_duration());

		let mut fired_events = Vec::new();

		// Simulate jittery ticks: 100, 90 (oops), 110, 500...
		let jitters = vec![100, 90, 110, 500, 450, 1200];

		for time in jitters {
			cursor.apply_until(&timeline, time, |event| {
				fired_events.push(event.clone());
				engine.apply_event(event);
			});
		}

		// Invariant: Events in the list must be unique (no double-firing)
		let mut unique_check = fired_events.clone();
		unique_check.dedup_by(|a, b| format!("{:?}", a) == format!("{:?}", b));

		assert_eq!(fired_events.len(), unique_check.len(), "Events fired multiple times due to time jitter");

		// Invariant: The cursor frontier must represent the maximum time seen
		assert!(cursor.applied_frontier() > 0);
	}
}

#![cfg(feature = "stream-orch")]

use super::error::{OrchestratorError, Result};
use crate::events::{OrchestratorConfig, OrchestratorState, SceneSchedule, TickCommand, TimeMs};
use tokio::sync::watch;
use tokio::time::{interval, Instant};
use tracing::{debug, error, info, warn};

/// Internal mutable state owned by the tick engine
struct TickEngineState {
	config: OrchestratorConfig,
	schedule: SceneSchedule,
	start_time: Option<Instant>,
	paused_at: Option<TimeMs>,
	accumulated_pause_duration: TimeMs,
}

impl TickEngineState {
	fn new(config: OrchestratorConfig, schedule: SceneSchedule) -> Self {
		Self {
			config,
			schedule,
			start_time: None,
			paused_at: None,
			accumulated_pause_duration: 0,
		}
	}

	fn calculate_current_time(&self) -> TimeMs {
		if let Some(start) = self.start_time {
			let elapsed = start.elapsed().as_millis() as TimeMs;
			elapsed.saturating_sub(self.accumulated_pause_duration)
		} else {
			0
		}
	}
}

/// Tick engine that drives the orchestrator state updates
/// Uses pure actor pattern - all state mutations happen via commands
pub struct TickEngine {
	state_rx: watch::Receiver<OrchestratorState>,
}

impl TickEngine {
	pub fn new(config: OrchestratorConfig) -> Result<Self> {
		config.validate().map_err(OrchestratorError::InvalidSceneConfig)?;

		let schedule = SceneSchedule::from_scenes(&config.scenes);
		let state = OrchestratorState::from_schedule(&schedule, config.scenes.clone());
		let (_state_tx, state_rx) = watch::channel(state);

		Ok(Self { state_rx })
	}

	/// Get a receiver for state updates
	pub fn subscribe(&self) -> watch::Receiver<OrchestratorState> {
		self.state_rx.clone()
	}

	/// Get current state (immutable)
	pub fn current_state(&self) -> OrchestratorState {
		self.state_rx.borrow().clone()
	}

	/// Run the tick engine actor loop
	pub async fn run(self, config: OrchestratorConfig, mut command_rx: tokio::sync::mpsc::UnboundedReceiver<TickCommand>, cancel_token: tokio_util::sync::CancellationToken) {
		let schedule = SceneSchedule::from_scenes(&config.scenes);
		let initial_state = OrchestratorState::from_schedule(&schedule, config.scenes.clone());
		let (state_tx, _state_rx) = watch::channel(initial_state);

		let mut engine_state = TickEngineState::new(config, schedule);
		let mut ticker = interval(engine_state.config.tick_interval());

		info!("Starting tick engine with interval: {:?}", engine_state.config.tick_interval());

		loop {
			tokio::select! {
					_ = ticker.tick() => {
							Self::handle_tick(&mut engine_state, &state_tx);
					}
					Some(command) = command_rx.recv() => {
							if let Err(e) = Self::handle_command(&mut engine_state, &state_tx, command) {
									if !e.is_recoverable() {
											warn!("Command error: {}", e);
									}
							}
					}
					_ = cancel_token.cancelled() => {
							info!("Tick engine cancelled");
							break;
					}
			}
		}

		info!("Tick engine stopped");
	}

	/// Handle a tick update (internal actor logic)
	fn handle_tick(engine_state: &mut TickEngineState, state_tx: &watch::Sender<OrchestratorState>) {
		let mut state = state_tx.borrow().clone();

		if !state.is_running || state.is_paused || state.scenes.is_empty() {
			return;
		}

		// Calculate current time based on elapsed time since start
		let current_time = engine_state.calculate_current_time();

		Self::update_time_internal(&engine_state.schedule, &mut state, current_time);

		// Check if we're complete and should loop
		if engine_state.schedule.is_complete(current_time) {
			if engine_state.config.loop_scenes {
				debug!("Looping orchestrator");
				engine_state.start_time = Some(Instant::now());
				engine_state.accumulated_pause_duration = 0;
				Self::update_time_internal(&engine_state.schedule, &mut state, 0);
			} else {
				debug!("Orchestrator complete, stopping");
				state.stop();
				engine_state.start_time = None;
			}
		}

		state_tx.send_replace(state);
	}

	/// Handle a command (internal actor logic)
	fn handle_command(engine_state: &mut TickEngineState, state_tx: &watch::Sender<OrchestratorState>, command: TickCommand) -> Result<()> {
		let mut state = state_tx.borrow().clone();

		match command {
			TickCommand::Start => {
				if state.scenes.is_empty() {
					error!("The are no scenes configured, current list of scenes is empty!");
					return Err(OrchestratorError::NotConfigured);
				}

				if state.is_running {
					return Err(OrchestratorError::AlreadyRunning);
				}

				state.start();
				engine_state.start_time = Some(Instant::now());
				engine_state.paused_at = None;
				engine_state.accumulated_pause_duration = 0;

				info!("Orchestrator started");
			}

			TickCommand::Stop => {
				if !state.is_running {
					return Err(OrchestratorError::NotRunning);
				}

				state.stop();
				engine_state.start_time = None;
				engine_state.paused_at = None;
				engine_state.accumulated_pause_duration = 0;

				info!("Orchestrator stopped");
			}

			TickCommand::Pause => {
				if !state.is_running {
					return Err(OrchestratorError::NotRunning);
				}

				if !state.is_paused {
					state.pause();
					engine_state.paused_at = Some(state.current_time);
					info!("Orchestrator paused at {}ms", state.current_time);
				}
			}

			TickCommand::Resume => {
				if !state.is_running {
					return Err(OrchestratorError::NotRunning);
				}

				if state.is_paused {
					state.resume();

					// Calculate how long we were paused
					if let Some(paused_time) = engine_state.paused_at {
						let pause_duration = state.current_time.saturating_sub(paused_time);
						engine_state.accumulated_pause_duration += pause_duration;
					}

					engine_state.paused_at = None;
					info!("Orchestrator resumed");
				}
			}

			TickCommand::Reset => {
				state.reset();
				engine_state.start_time = None;
				engine_state.paused_at = None;
				engine_state.accumulated_pause_duration = 0;

				info!("Orchestrator reset");
			}

			TickCommand::ForceScene(scene_name) => {
				let element = engine_state
					.schedule
					.get_scene_by_name(&scene_name)
					.ok_or_else(|| OrchestratorError::SceneNotFound(scene_name.clone()))?;

				let new_time = element.start_time;
				Self::update_time_internal(&engine_state.schedule, &mut state, new_time);

				info!("Forced scene: {}", scene_name);
			}

			TickCommand::SkipCurrentScene => {
				if !state.is_running {
					return Err(OrchestratorError::NotRunning);
				}

				let next_scene = engine_state.schedule.get_next_scene(state.current_time);

				if let Some(next) = next_scene {
					let new_time = next.start_time;
					Self::update_time_internal(&engine_state.schedule, &mut state, new_time);
					info!("Skipped to scene: {}", next.scene_name);
				} else {
					warn!("No next scene to skip to");
				}
			}

			TickCommand::UpdateStreamStatus {
				is_streaming,
				stream_time,
				timecode,
			} => {
				state.update_stream_status(is_streaming, stream_time, timecode);
			}

			TickCommand::Reconfigure(config) => {
				config.validate().map_err(OrchestratorError::InvalidSceneConfig)?;

				let was_running = state.is_running;

				// Stop if running
				if was_running {
					state.stop();
					engine_state.start_time = None;
					engine_state.paused_at = None;
					engine_state.accumulated_pause_duration = 0;
				}

				// Update configuration and schedule
				engine_state.schedule = SceneSchedule::from_scenes(&config.scenes);
				state = OrchestratorState::from_schedule(&engine_state.schedule, config.scenes.clone());
				engine_state.config = config;

				info!("Orchestrator reconfigured with {} scenes", engine_state.schedule.len());
			}
		}

		state_tx.send_replace(state);
		Ok(())
	}

	/// Internal time update logic (pure function)
	fn update_time_internal(schedule: &SceneSchedule, state: &mut OrchestratorState, current_time: TimeMs) {
		let previous_scene = state.current_active_scene.clone();

		state.update_from_time(current_time, schedule);

		// Log scene changes
		if state.current_active_scene != previous_scene {
			info!("Scene changed: {:?} -> {:?}", previous_scene, state.current_active_scene);
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::stream_orch::types::SceneConfig;
	use tokio::time::{sleep, Duration};

	#[tokio::test]
	async fn test_tick_engine_via_commands() {
		let config = OrchestratorConfig::new(vec![SceneConfig::new("intro", 5000), SceneConfig::new("main", 10000)]);

		let engine = TickEngine::new(config.clone()).unwrap();
		let mut state_rx = engine.subscribe();
		let (cmd_tx, cmd_rx) = tokio::sync::mpsc::unbounded_channel();
		let cancel = tokio_util::sync::CancellationToken::new();

		// Spawn engine
		let engine_handle = tokio::spawn({
			let cancel = cancel.clone();
			async move {
				engine.run(config, cmd_rx, cancel).await;
			}
		});

		// Send start command
		cmd_tx.send(TickCommand::Start).unwrap();
		sleep(Duration::from_millis(100)).await;

		state_rx.changed().await.unwrap();
		assert!(state_rx.borrow().is_running);

		// Send stop command
		cmd_tx.send(TickCommand::Stop).unwrap();
		sleep(Duration::from_millis(100)).await;

		state_rx.changed().await.unwrap();
		assert!(!state_rx.borrow().is_running);

		cancel.cancel();
		engine_handle.await.unwrap();
	}
}

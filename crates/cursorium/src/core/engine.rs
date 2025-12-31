use tokio::sync::{mpsc, watch};
use tokio::time::{interval, Interval};
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use super::error::{OrchestratorError, Result};
use super::EngineState;
use super::{Cursor, LifetimeEvent, LifetimeKind, StreamStatus};
use super::{OrchestratorCommand, OrchestratorCommandData, OrchestratorConfig, OrchestratorEvent, OrchestratorMode, OrchestratorState, Timeline};

// ============================================================================
// Session - Owns all timeline-related state
// ============================================================================

struct Session {
	timeline: Timeline,
	cursor: Cursor,
	engine_state: EngineState,
	ticker: Interval,
	loop_scenes: bool,
}

impl Session {
	fn from_config(config: OrchestratorConfig) -> Result<Self> {
		config.validate().map_err(|e| OrchestratorError::InvalidSceneConfig(e.to_string()))?;

		let timeline = config.compile_timeline();
		let total_duration = timeline.total_duration();

		let mut engine_state = EngineState::new(total_duration);
		engine_state.sync_view_state(0);

		Ok(Self {
			timeline,
			cursor: Cursor::new(),
			engine_state,
			ticker: interval(config.tick_interval()),
			loop_scenes: config.loop_scenes,
		})
	}
}

// ============================================================================
// Pure FSM - Returns only the next mode
// ============================================================================

/// Pure FSM: validates transitions only
fn transition(mode: OrchestratorMode, cmd: &OrchestratorCommand) -> Result<OrchestratorMode> {
	use OrchestratorCommand::*;
	use OrchestratorMode::*;

	Ok(match (mode, cmd) {
		// Configure always creates session and moves to Idle
		(_, Configure { .. }) => Idle,

		// Start only from Idle
		(Idle, Start { .. }) => Running,
		(Unconfigured, Start { .. }) => return Err(OrchestratorError::NotConfigured),
		(Running | Paused, Start { .. }) => return Err(OrchestratorError::AlreadyRunning),
		(Finished | Stopped | Error, Start { .. }) => return Err(OrchestratorError::TerminalState),

		// Pause only from Running
		(Running, Pause { .. }) => Paused,
		(Paused, Pause { .. }) => Paused, // Idempotent
		(Unconfigured | Idle, Pause { .. }) => return Err(OrchestratorError::NotRunning),
		(Finished | Stopped | Error, Pause { .. }) => return Err(OrchestratorError::TerminalState),

		// Resume only from Paused
		(Paused, Resume { .. }) => Running,
		(Running, Resume { .. }) => Running, // Idempotent
		(Unconfigured | Idle, Resume { .. }) => return Err(OrchestratorError::NotRunning),
		(Finished | Stopped | Error, Resume { .. }) => return Err(OrchestratorError::TerminalState),

		// Stop from Running or Paused → Terminal Stopped state
		(Running | Paused, Stop { .. }) => Stopped,
		(Unconfigured | Idle, Stop { .. }) => mode,        // Idempotent
		(Finished | Stopped | Error, Stop { .. }) => mode, // Already terminal

		// Reset from active states → Idle (can recover from terminal states)
		(Idle | Running | Paused | Finished | Stopped | Error, Reset { .. }) => Idle,
		(Unconfigured, Reset { .. }) => Unconfigured, // Can't reset without config

		// Non-FSM commands don't affect mode
		(_, ForceScene(_) | SkipCurrentScene | UpdateStreamStatus { .. }) => mode,
	})
}

// ============================================================================
// OrchestratorEngine
// ============================================================================

pub struct OrchestratorEngine {
	state_tx: watch::Sender<OrchestratorState>,
	state_rx: watch::Receiver<OrchestratorState>,
}

impl OrchestratorEngine {
	pub fn new() -> Self {
		let (state_tx, state_rx) = watch::channel(OrchestratorState::default());
		info!("OrchestratorEngine created (unconfigured)");
		Self { state_tx, state_rx }
	}

	pub fn subscribe(&self) -> watch::Receiver<OrchestratorState> {
		self.state_rx.clone()
	}

	pub fn current_state(&self) -> OrchestratorState {
		self.state_rx.borrow().clone()
	}

	pub async fn run(self, initial_config: Option<OrchestratorConfig>, mut command_rx: mpsc::UnboundedReceiver<OrchestratorCommand>, cancel: CancellationToken) {
		let mut mode = OrchestratorMode::Unconfigured;
		let mut session: Option<Session> = None;

		// Initialize with config if provided
		if let Some(cfg) = initial_config {
			if let Ok(s) = Session::from_config(cfg) {
				let mut state = s.engine_state.state.clone();
				state.mode = OrchestratorMode::Idle;
				self.state_tx.send_replace(state);
				session = Some(s);
				mode = OrchestratorMode::Idle;
			}
		}

		info!("Orchestrator engine started");

		loop {
			tokio::select! {
				// Tick only when Running
				_ = async {
					if let (OrchestratorMode::Running, Some(s)) = (mode, &mut session) {
						s.ticker.tick().await;
					} else {
						std::future::pending::<()>().await;
					}
				} => {
					if let Some(s) = &mut session {
						mode = Self::handle_tick(s, mode, &self.state_tx);
					}
				}

				Some(cmd) = command_rx.recv() => {
					let result = match &cmd {
						OrchestratorCommand::Configure { config, .. } => {
							if let OrchestratorCommandData::Configure(config_data) = config {
								match Session::from_config(config_data.clone().into()) {
									Ok(s) => {
										session = Some(s);
										mode = OrchestratorMode::Idle;
										if let Some(s) = &session {
											let mut state = s.engine_state.state.clone();
											state.mode = mode;
											self.state_tx.send_replace(state);
										}
										info!("Session configured");
										Ok(())
									},
									Err(e) => Err(OrchestratorError::InvalidSceneConfig(e.to_string())),
								}
							} else {
								Err(OrchestratorError::InvalidSceneConfig(
										"Expected Configure command data".to_string(),
								))
							}
						}
						OrchestratorCommand::Start { .. }
						| OrchestratorCommand::Pause { .. }
						| OrchestratorCommand::Resume { .. }
						| OrchestratorCommand::Stop { .. }
						| OrchestratorCommand::Reset { .. } => {
							match transition(mode, &cmd) {
								Ok(new_mode) => {
									Self::apply_mode_change(mode, new_mode, &mut session, &self.state_tx);
									mode = new_mode;
									Self::handle_operations(&cmd, &mut session, &self.state_tx);
									Ok(())
								}
								Err(e) => Err(e),
							}
						}
						_ => {
							Self::handle_operations(&cmd, &mut session, &self.state_tx);
							Ok(())
						}
					};

					// Send result if command has oneshot
					match cmd {
						OrchestratorCommand::Configure { response, .. }
						| OrchestratorCommand::Start { response }
						| OrchestratorCommand::Pause { response }
						| OrchestratorCommand::Resume { response }
						| OrchestratorCommand::Stop { response }
						| OrchestratorCommand::Reset { response } => {
							let _ = response.send(result);
						}
						_ => {}
					}
				}

				_ = cancel.cancelled() => {
					info!("Orchestrator engine cancelled");
					break;
				}
			}
		}
	}

	fn handle_tick(session: &mut Session, mode: OrchestratorMode, state_tx: &watch::Sender<OrchestratorState>) -> OrchestratorMode {
		let current_time = session.engine_state.calculate_current_time();

		// Apply timeline events
		session.cursor.apply_until(&session.timeline, current_time, |event| session.engine_state.apply_event(event));

		session.engine_state.sync_view_state(current_time);

		// Check for natural completion
		let new_mode = if session.engine_state.state.is_complete() {
			if session.loop_scenes {
				debug!("Looping orchestrator");
				session.engine_state.start_instant = Some(std::time::Instant::now());
				session.engine_state.accumulated_pause_duration = 0;
				session.cursor.reset();
				session.engine_state.active_lifetimes.clear();
				session.engine_state.sync_view_state(0);
				OrchestratorMode::Running
			} else {
				info!("Orchestrator completed naturally");
				OrchestratorMode::Finished
			}
		} else {
			mode
		};

		// Update state with current mode
		let mut state = session.engine_state.state.clone();
		state.mode = new_mode;
		state_tx.send_replace(state);

		new_mode
	}

	/// Apply side effects based on mode transitions
	fn apply_mode_change(from: OrchestratorMode, to: OrchestratorMode, session: &mut Option<Session>, state_tx: &watch::Sender<OrchestratorState>) {
		let Some(s) = session else { return };

		match (from, to) {
			// Starting playback
			(OrchestratorMode::Idle, OrchestratorMode::Running) => {
				s.engine_state.start_instant = Some(std::time::Instant::now());
				s.engine_state.paused_at = None;
				s.engine_state.accumulated_pause_duration = 0;
				info!("Playback started");
			}

			// Pausing
			(OrchestratorMode::Running, OrchestratorMode::Paused) => {
				s.engine_state.paused_at = Some(s.engine_state.state.current_time);
				info!("Playback paused");
			}

			// Resuming
			(OrchestratorMode::Paused, OrchestratorMode::Running) => {
				if let Some(paused_time) = s.engine_state.paused_at {
					s.engine_state.accumulated_pause_duration += s.engine_state.state.current_time.saturating_sub(paused_time);
				}
				s.engine_state.paused_at = None;
				info!("Playback resumed");
			}

			// Stopping (terminal transition)
			(OrchestratorMode::Running | OrchestratorMode::Paused, OrchestratorMode::Stopped) => {
				s.engine_state.start_instant = None;
				s.engine_state.paused_at = None;
				info!("Orchestrator stopped by user");
			}

			// Resetting to Idle
			(_, OrchestratorMode::Idle) if from != OrchestratorMode::Idle => {
				s.engine_state.start_instant = None;
				s.engine_state.paused_at = None;
				s.engine_state.accumulated_pause_duration = 0;
				s.cursor.reset();
				s.engine_state.active_lifetimes.clear();
				s.engine_state.sync_view_state(0);
				info!("Timeline reset to idle");
			}

			_ => {} // No state change needed
		}

		// Update state with new mode
		let mut state = s.engine_state.state.clone();
		state.mode = to;
		state_tx.send_replace(state);
	}

	/// Handle non-FSM operational commands
	fn handle_operations(command: &OrchestratorCommand, session: &mut Option<Session>, state_tx: &watch::Sender<OrchestratorState>) {
		let Some(s) = session else { return };

		match command {
			OrchestratorCommand::ForceScene(scene_name) => {
				let target_event = s.timeline.events().iter().find(|e| {
					matches!(
						e.event,
						OrchestratorEvent::Lifetime(LifetimeEvent::Start {
							kind: LifetimeKind::Scene(ref scene),
							..
						}) if scene.scene_name == *scene_name
					)
				});

				if let Some(event) = target_event {
					let target_time = event.at;
					s.engine_state.reconstruct_from_start(&mut s.cursor, &s.timeline, target_time);
					let state = s.engine_state.state.clone();
					state_tx.send_replace(state);
					info!("Forced scene: {}", scene_name);
				} else {
					warn!("Scene not found: {}", scene_name);
				}
			}

			OrchestratorCommand::SkipCurrentScene => {
				let next_scene = s.timeline.events()[s.cursor.applied_frontier()..]
					.iter()
					.find(|e| matches!(e.event, OrchestratorEvent::Lifetime(LifetimeEvent::Start { kind: LifetimeKind::Scene(_), .. })));

				if let Some(next) = next_scene {
					let target_time = next.at;
					s.engine_state.reconstruct_from_start(&mut s.cursor, &s.timeline, target_time);
					let state = s.engine_state.state.clone();
					state_tx.send_replace(state);
					info!("Skipped to next scene");
				} else {
					warn!("No next scene to skip to");
				}
			}

			OrchestratorCommand::UpdateStreamStatus {
				is_streaming,
				stream_time,
				timecode,
			} => {
				s.engine_state.state.stream_status = StreamStatus {
					is_streaming: *is_streaming,
					stream_time: *stream_time,
					timecode: timecode.clone(),
				};
				state_tx.send_replace(s.engine_state.state.clone());
			}

			_ => {} // FSM commands already handled
		}
	}
}

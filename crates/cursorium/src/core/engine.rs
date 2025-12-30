use tokio::sync::{mpsc, watch};
use tokio::time::{interval, Interval};
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use super::error::{OrchestratorError, Result};
use super::EngineState;
use super::{Cursor, LifetimeEvent, LifetimeKind, StreamStatus};
use super::{OrchestratorCommand, OrchestratorCommandData, OrchestratorConfig, OrchestratorEvent, OrchestratorState, Timeline};

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

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
enum EngineMode {
	Unconfigured,
	Idle,
	Running,
	Paused,
}

/// Pure FSM: validates transitions only
/// Pure FSM: validates transitions only
fn transition(mode: EngineMode, cmd: &OrchestratorCommand) -> Result<EngineMode> {
	use EngineMode::*;
	use OrchestratorCommand::*;

	Ok(match (mode, cmd) {
		// Configure always creates session and moves to Idle
		(_, Configure { .. }) => Idle,

		// Start only from Idle
		(Idle, Start { .. }) => Running,
		(Unconfigured, Start { .. }) => return Err(OrchestratorError::NotConfigured),
		(Running | Paused, Start { .. }) => return Err(OrchestratorError::AlreadyRunning),

		// Pause only from Running
		(Running, Pause { .. }) => Paused,
		(Paused, Pause { .. }) => Paused, // Idempotent
		(Unconfigured | Idle, Pause { .. }) => return Err(OrchestratorError::NotRunning),

		// Resume only from Paused
		(Paused, Resume { .. }) => Running,
		(Running, Resume { .. }) => Running, // Idempotent
		(Unconfigured | Idle, Resume { .. }) => return Err(OrchestratorError::NotRunning),

		// Stop from Running or Paused
		(Running | Paused, Stop { .. }) => Idle,
		(Unconfigured | Idle, Stop { .. }) => mode, // Idempotent

		// Reset preserves config, resets time
		(Idle | Running | Paused, Reset { .. }) => Idle,
		(Unconfigured, Reset { .. }) => Unconfigured,

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
		let mut mode = EngineMode::Unconfigured;
		let mut session: Option<Session> = None;

		// Initialize with config if provided
		if let Some(cfg) = initial_config {
			if let Ok(s) = Session::from_config(cfg) {
				self.state_tx.send_replace(s.engine_state.state.clone());
				session = Some(s);
				mode = EngineMode::Idle;
			}
		}

		info!("Orchestrator engine started");

		loop {
			tokio::select! {
				// Tick only when Running
				_ = async {
					if let (EngineMode::Running, Some(s)) = (mode, &mut session) {
						s.ticker.tick().await;
					} else {
						std::future::pending::<()>().await;
					}
				} => {
					if let Some(s) = &mut session {
						Self::handle_tick(s, &self.state_tx);
					}
				}

				Some(cmd) = command_rx.recv() => {
					let result = match &cmd {
						OrchestratorCommand::Configure { config, .. } => {
							if let OrchestratorCommandData::Configure(config_data) = config {
								match Session::from_config(config_data.clone().into()) {
									Ok(s) => {
										session = Some(s);
										mode = EngineMode::Idle;
										if let Some(s) = &session {
											self.state_tx.send_replace(s.engine_state.state.clone());
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

	fn handle_tick(session: &mut Session, state_tx: &watch::Sender<OrchestratorState>) {
		let current_time = session.engine_state.calculate_current_time();

		// Apply timeline events
		session.cursor.apply_until(&session.timeline, current_time, |event| session.engine_state.apply_event(event));

		session.engine_state.sync_view_state(current_time);

		// Handle completion
		if session.engine_state.state.is_complete() {
			if session.loop_scenes {
				debug!("Looping orchestrator");
				session.engine_state.start_instant = Some(std::time::Instant::now());
				session.engine_state.accumulated_pause_duration = 0;
				session.cursor.reset();
				session.engine_state.active_lifetimes.clear();
				session.engine_state.sync_view_state(0);
			} else {
				debug!("Orchestrator complete");
				session.engine_state.state.is_running = false;
				session.engine_state.start_instant = None;
			}
		}

		state_tx.send_replace(session.engine_state.state.clone());
	}

	/// Apply side effects based on mode transitions
	fn apply_mode_change(from: EngineMode, to: EngineMode, session: &mut Option<Session>, state_tx: &watch::Sender<OrchestratorState>) {
		let Some(s) = session else { return };

		match (from, to) {
			// Starting playback
			(EngineMode::Idle, EngineMode::Running) => {
				s.engine_state.state.is_running = true;
				s.engine_state.state.is_paused = false;
				s.engine_state.start_instant = Some(std::time::Instant::now());
				s.engine_state.paused_at = None;
				s.engine_state.accumulated_pause_duration = 0;
				info!("Playback started");
			}

			// Pausing
			(EngineMode::Running, EngineMode::Paused) => {
				s.engine_state.state.is_paused = true;
				s.engine_state.paused_at = Some(s.engine_state.state.current_time);
				info!("Playback paused");
			}

			// Resuming
			(EngineMode::Paused, EngineMode::Running) => {
				s.engine_state.state.is_paused = false;
				if let Some(paused_time) = s.engine_state.paused_at {
					s.engine_state.accumulated_pause_duration += s.engine_state.state.current_time.saturating_sub(paused_time);
				}
				s.engine_state.paused_at = None;
				info!("Playback resumed");
			}

			// Stopping or resetting to Idle
			(_, EngineMode::Idle) if from != EngineMode::Idle => {
				s.engine_state.state.is_running = false;
				s.engine_state.state.is_paused = false;
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

		state_tx.send_replace(s.engine_state.state.clone());
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
					state_tx.send_replace(s.engine_state.state.clone());
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
					state_tx.send_replace(s.engine_state.state.clone());
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

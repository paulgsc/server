#![cfg(feature = "stream-orch")]

use super::config::OrchestratorConfig;
use super::error::{OrchestratorError, Result};
use super::state::OrchestratorState;
use super::tick::{TickCommand, TickEngine};
use super::types::TimeMs;
use std::sync::Arc;
use tokio::sync::{mpsc, watch, Mutex};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::info;

/// Commands that can be sent to the orchestrator
#[derive(Debug, Clone)]
pub enum OrchestratorCommand {
	Configure { config: OrchestratorConfig },
	Start,
	Stop,
	Reset,
	Pause,
	Resume,
	ForceScene { scene_name: String },
	SkipCurrentScene,
	UpdateStreamStatus { is_streaming: bool, stream_time: TimeMs, timecode: String },
}

/// Main orchestrator that manages the tick engine
/// Pure actor pattern - all methods are immutable (&self)
pub struct StreamOrchestrator {
	command_tx: mpsc::UnboundedSender<TickCommand>,
	state_rx: watch::Receiver<OrchestratorState>,
	task_handle: Arc<Mutex<Option<JoinHandle<()>>>>,
	cancel_token: CancellationToken,
}

impl StreamOrchestrator {
	/// Create a new orchestrator with configuration
	pub fn new(config: OrchestratorConfig) -> Result<Self> {
		let cancel_token = CancellationToken::new();
		let (command_tx, command_rx) = mpsc::unbounded_channel();

		let tick_engine = TickEngine::new(config.clone())?;
		let state_rx = tick_engine.subscribe();

		let task_handle = tokio::spawn(tick_engine.run(config, command_rx, cancel_token.clone()));

		Ok(Self {
			command_tx,
			state_rx,
			task_handle: Arc::new(Mutex::new(Some(task_handle))),
			cancel_token,
		})
	}

	/// Create without initial configuration
	pub fn new_unconfigured() -> Result<Self> {
		let default_config = OrchestratorConfig::default();
		Self::new(default_config)
	}

	/// Send a command to the orchestrator (immutable &self)
	fn send_command(&self, command: TickCommand) -> Result<()> {
		self.command_tx.send(command).map_err(|_| OrchestratorError::Internal("Failed to send command".to_string()))
	}

	/// Configure the orchestrator (immutable &self)
	pub fn configure(&self, config: OrchestratorConfig) -> Result<()> {
		self.send_command(TickCommand::Reconfigure(config))
	}

	/// Start the orchestrator (immutable &self)
	pub fn start(&self) -> Result<()> {
		self.send_command(TickCommand::Start)
	}

	/// Stop the orchestrator (immutable &self)
	pub fn stop(&self) -> Result<()> {
		self.send_command(TickCommand::Stop)
	}

	/// Reset the orchestrator (immutable &self)
	pub fn reset(&self) -> Result<()> {
		self.send_command(TickCommand::Reset)
	}

	/// Pause the orchestrator (immutable &self)
	pub fn pause(&self) -> Result<()> {
		self.send_command(TickCommand::Pause)
	}

	/// Resume the orchestrator (immutable &self)
	pub fn resume(&self) -> Result<()> {
		self.send_command(TickCommand::Resume)
	}

	/// Force a specific scene (immutable &self)
	pub fn force_scene(&self, scene_name: impl Into<String>) -> Result<()> {
		self.send_command(TickCommand::ForceScene(scene_name.into()))
	}

	/// Skip the current scene (immutable &self)
	pub fn skip_current_scene(&self) -> Result<()> {
		self.send_command(TickCommand::SkipCurrentScene)
	}

	/// Update stream status from external source (OBS) (immutable &self)
	pub fn update_stream_status(&self, is_streaming: bool, stream_time: TimeMs, timecode: impl Into<String>) -> Result<()> {
		self.send_command(TickCommand::UpdateStreamStatus {
			is_streaming,
			stream_time,
			timecode: timecode.into(),
		})
	}

	/// Subscribe to state updates (immutable &self)
	pub fn subscribe(&self) -> watch::Receiver<OrchestratorState> {
		self.state_rx.clone()
	}

	/// Get current state snapshot (immutable &self)
	pub fn current_state(&self) -> OrchestratorState {
		self.state_rx.borrow().clone()
	}

	/// Shutdown the orchestrator
	/// This consumes self but uses interior mutability for the handle
	pub async fn shutdown(self) {
		info!("Shutting down orchestrator");
		self.cancel_token.cancel();

		if let Some(handle) = self.task_handle.lock().await.take() {
			let _ = handle.await;
		}

		info!("Orchestrator shut down complete");
	}
}

impl Drop for StreamOrchestrator {
	fn drop(&mut self) {
		self.cancel_token.cancel();
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::stream_orch::types::SceneConfig;
	use tokio::time::{sleep, Duration};

	#[tokio::test]
	async fn test_orchestrator_basic() {
		let config = OrchestratorConfig::new(vec![SceneConfig::new("intro", 1000), SceneConfig::new("main", 2000)]);

		let orchestrator = StreamOrchestrator::new(config).unwrap();

		orchestrator.start().unwrap();
		sleep(Duration::from_millis(100)).await;

		let state = orchestrator.current_state();
		assert!(state.is_running);

		orchestrator.shutdown().await;
	}

	#[tokio::test]
	async fn test_orchestrator_state_updates() {
		let config = OrchestratorConfig::new(vec![SceneConfig::new("intro", 500), SceneConfig::new("main", 1000)]);

		let orchestrator = StreamOrchestrator::new(config).unwrap();
		let mut state_rx = orchestrator.subscribe();

		orchestrator.start().unwrap();

		// Wait for state update
		state_rx.changed().await.unwrap();
		let state = state_rx.borrow().clone();
		assert!(state.is_running);

		orchestrator.shutdown().await;
	}

	#[tokio::test]
	async fn test_orchestrator_scene_forcing() {
		let config = OrchestratorConfig::new(vec![SceneConfig::new("intro", 1000), SceneConfig::new("main", 2000)]);

		let orchestrator = StreamOrchestrator::new(config).unwrap();

		orchestrator.start().unwrap();
		sleep(Duration::from_millis(50)).await;

		orchestrator.force_scene("main").unwrap();
		sleep(Duration::from_millis(50)).await;

		let state = orchestrator.current_state();
		assert_eq!(state.current_active_scene, Some("main".to_string()));

		orchestrator.shutdown().await;
	}

	#[tokio::test]
	async fn test_immutable_orchestrator_methods() {
		let config = OrchestratorConfig::new(vec![SceneConfig::new("test", 1000)]);

		let orchestrator = StreamOrchestrator::new(config).unwrap();

		// All these methods take &self, not &mut self
		orchestrator.start().unwrap();
		orchestrator.pause().unwrap();
		orchestrator.resume().unwrap();
		orchestrator.force_scene("test").unwrap();
		orchestrator.stop().unwrap();

		// Can still call methods after previous calls
		orchestrator.start().unwrap();
		let _state = orchestrator.current_state();
		let _sub = orchestrator.subscribe();

		orchestrator.shutdown().await;
	}
}

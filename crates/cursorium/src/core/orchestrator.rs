use std::sync::Arc;
use tokio::sync::{mpsc, oneshot, watch, Mutex};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::info;

use super::error::Result;
use super::{OrchestratorCommand, OrchestratorCommandData, OrchestratorConfig, OrchestratorEngine, OrchestratorError, OrchestratorState};

/// The orchestrator actor façade
pub struct StreamOrchestrator {
	command_tx: mpsc::UnboundedSender<OrchestratorCommand>,
	state_rx: watch::Receiver<OrchestratorState>,
	task_handle: Arc<Mutex<Option<JoinHandle<()>>>>,
	cancel_token: CancellationToken,
}

impl StreamOrchestrator {
	/// Create a new orchestrator, optionally with a config
	pub fn new(config: Option<OrchestratorConfig>) -> Result<Self> {
		let cancel_token = CancellationToken::new();
		let (command_tx, command_rx) = mpsc::unbounded_channel();

		let engine = OrchestratorEngine::new();
		let state_rx = engine.subscribe();

		let task_handle = tokio::spawn(engine.run(config, command_rx, cancel_token.clone()));

		info!("StreamOrchestrator created");

		Ok(Self {
			command_tx,
			state_rx,
			task_handle: Arc::new(Mutex::new(Some(task_handle))),
			cancel_token,
		})
	}

	/// Send FSM command and await result
	async fn send_fsm(&self, cmd: OrchestratorCommand) -> Result<()> {
		let (tx, rx) = oneshot::channel();
		let cmd = match cmd {
			OrchestratorCommand::Configure { config, .. } => OrchestratorCommand::Configure { config, response: tx },
			OrchestratorCommand::Start { .. } => OrchestratorCommand::Start { response: tx },
			OrchestratorCommand::Pause { .. } => OrchestratorCommand::Pause { response: tx },
			OrchestratorCommand::Resume { .. } => OrchestratorCommand::Resume { response: tx },
			OrchestratorCommand::Stop { .. } => OrchestratorCommand::Stop { response: tx },
			OrchestratorCommand::Reset { .. } => OrchestratorCommand::Reset { response: tx },
			_ => panic!("send_fsm called with non-FSM command"),
		};

		self.command_tx.send(cmd).map_err(|_| OrchestratorError::Internal("Failed to send command".into()))?;

		rx.await.map_err(|_| OrchestratorError::Internal("Engine dropped".into()))?
	}

	// FSM façade methods
	pub async fn configure(&self, config: OrchestratorCommandData) -> Result<()> {
		self
			.send_fsm(OrchestratorCommand::Configure {
				config,
				response: oneshot::channel().0,
			})
			.await
	}
	pub async fn start(&self) -> Result<()> {
		self.send_fsm(OrchestratorCommand::Start { response: oneshot::channel().0 }).await
	}
	pub async fn pause(&self) -> Result<()> {
		self.send_fsm(OrchestratorCommand::Pause { response: oneshot::channel().0 }).await
	}
	pub async fn resume(&self) -> Result<()> {
		self.send_fsm(OrchestratorCommand::Resume { response: oneshot::channel().0 }).await
	}
	pub async fn stop(&self) -> Result<()> {
		self.send_fsm(OrchestratorCommand::Stop { response: oneshot::channel().0 }).await
	}
	pub async fn reset(&self) -> Result<()> {
		self.send_fsm(OrchestratorCommand::Reset { response: oneshot::channel().0 }).await
	}

	// Fire-and-forget commands
	pub fn force_scene(&self, scene: impl Into<String>) -> Result<()> {
		self
			.command_tx
			.send(OrchestratorCommand::ForceScene(scene.into()))
			.map_err(|_| OrchestratorError::Internal("Failed to send command".into()))
	}

	pub fn skip_current_scene(&self) -> Result<()> {
		self
			.command_tx
			.send(OrchestratorCommand::SkipCurrentScene)
			.map_err(|_| OrchestratorError::Internal("Failed to send command".into()))
	}

	// Access state
	pub fn subscribe(&self) -> watch::Receiver<OrchestratorState> {
		self.state_rx.clone()
	}
	pub fn current_state(&self) -> OrchestratorState {
		self.state_rx.borrow().clone()
	}

	// Shutdown the orchestrator
	pub async fn shutdown(&self) {
		self.cancel_token.cancel();
		if let Some(handle) = self.task_handle.lock().await.take() {
			let _ = handle.await;
		}
	}
}

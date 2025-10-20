use crate::core::{InternalCommand, ObsCommand};
use crate::{ObsConfig, ObsEvent};
use std::time::Instant;
use thiserror::Error;
use tokio::sync::{mpsc, oneshot};

#[derive(Debug, Clone)]
pub enum ConnectionState {
	Disconnected,
	Connecting { started_at: Instant },
	Connected { connected_at: Instant },
	Disconnecting { started_at: Instant },
	Failed { error: String, failed_at: Instant },
}

#[derive(Debug, Clone)]
pub enum StateTransition {
	StartConnecting,
	ConnectionEstablished,
	StartDisconnecting,
	ConnectionLost,
	ConnectionFailed(String),
}

#[derive(Debug, Error)]
pub enum StateError {
	#[error("Invalid state transition: {from:?} -> {to:?}")]
	InvalidTransition { from: ConnectionState, to: ConnectionState },
	#[error("Connection error: {0}")]
	ConnectionError(String),
	#[error("Transition error: {0}")]
	TransitionError(String),
	#[error("Command execution failed: {0}")]
	CommandFailed(String),
	#[error("Event processing failed: {0}")]
	EventFailed(String),
	#[error("Not connected")]
	NotConnected,
	#[error("No Recievers")]
	NoReceivers,
	#[error("State actor unavailable")]
	ActorUnavailable,
	#[error("Channel Overflow {0}")]
	ChannelOverflow(String),
}

/// Messages sent to the state actor
#[derive(Debug)]
pub enum StateMessage {
	// Queries with response
	GetConnectionState(oneshot::Sender<ConnectionState>),
	GetConfig(oneshot::Sender<ObsConfig>),
	IsConnected(oneshot::Sender<bool>),
	CanExecuteCommands(oneshot::Sender<bool>),
	GetCommandSender(oneshot::Sender<Option<tokio::sync::mpsc::Sender<InternalCommand>>>),

	// State transitions
	Transition(StateTransition, oneshot::Sender<Result<(), StateError>>),

	// Resource management
	SetCommandSender(tokio::sync::mpsc::Sender<InternalCommand>),
	SetEventReceiver(async_broadcast::Receiver<ObsEvent>),
	SetEventSender(async_broadcast::Sender<ObsEvent>),
	SetConnectionHandle(tokio::task::JoinHandle<()>),
	TakeCommandSender(oneshot::Sender<Option<tokio::sync::mpsc::Sender<InternalCommand>>>),
	TakeEventReceiver(oneshot::Sender<Option<async_broadcast::Receiver<ObsEvent>>>),
	TakeConnectionHandle(oneshot::Sender<Option<tokio::task::JoinHandle<()>>>),
	UpdateConfig(ObsConfig),

	// Command execution (for convenience)
	ExecuteCommand(ObsCommand, oneshot::Sender<Result<(), StateError>>),
}

/// Core state container - now owned exclusively by the actor
#[derive(Debug)]
pub struct ObsState {
	config: ObsConfig,
	connection_state: ConnectionState,
	command_sender: Option<tokio::sync::mpsc::Sender<InternalCommand>>,
	event_receiver: Option<async_broadcast::Receiver<ObsEvent>>,
	event_sender: Option<async_broadcast::Sender<ObsEvent>>,
	connection_handle: Option<tokio::task::JoinHandle<()>>,
}

impl ObsState {
	const fn new(config: ObsConfig) -> Self {
		Self {
			config,
			connection_state: ConnectionState::Disconnected,
			command_sender: None,
			event_receiver: None,
			event_sender: None,
			connection_handle: None,
		}
	}

	/// Validate and execute state transition
	fn transition(&mut self, transition: StateTransition) -> Result<(), StateError> {
		let new_state = self.validate_transition(&self.connection_state, &transition)?;
		self.connection_state = new_state;
		Ok(())
	}

	/// Validate if a transition is allowed
	fn validate_transition(&self, current: &ConnectionState, transition: &StateTransition) -> Result<ConnectionState, StateError> {
		use ConnectionState::*;
		use StateTransition::*;

		let new_state = match (current, transition) {
			(Disconnected, StartConnecting) => Connecting { started_at: Instant::now() },
			(Connecting { .. }, ConnectionEstablished) => Connected { connected_at: Instant::now() },
			(Connecting { .. }, ConnectionFailed(err)) => Failed {
				error: err.clone(),
				failed_at: Instant::now(),
			},
			(Connected { .. }, StartDisconnecting) => Disconnecting { started_at: Instant::now() },
			(Connected { .. }, ConnectionLost) => Disconnected,
			(Disconnecting { .. }, ConnectionLost) => Disconnected,
			(Failed { .. }, StartConnecting) => Connecting { started_at: Instant::now() },
			_ => {
				return Err(StateError::InvalidTransition {
					from: current.clone(),
					to: self.expected_state_for_transition(transition),
				})
			}
		};

		Ok(new_state)
	}

	fn expected_state_for_transition(&self, transition: &StateTransition) -> ConnectionState {
		use StateTransition::*;
		match transition {
			StartConnecting => ConnectionState::Connecting { started_at: Instant::now() },
			ConnectionEstablished => ConnectionState::Connected { connected_at: Instant::now() },
			StartDisconnecting => ConnectionState::Disconnecting { started_at: Instant::now() },
			ConnectionLost => ConnectionState::Disconnected,
			ConnectionFailed(err) => ConnectionState::Failed {
				error: err.clone(),
				failed_at: Instant::now(),
			},
		}
	}

	pub fn is_connected(&self) -> bool {
		matches!(self.connection_state, ConnectionState::Connected { .. })
	}

	fn can_execute_commands(&self) -> bool {
		self.is_connected() && self.command_sender.is_some()
	}
}

/// The state actor that owns and manages the ObsState
pub struct StateActor {
	state: ObsState,
	receiver: mpsc::Receiver<StateMessage>,
}

impl StateActor {
	/// Create a new state actor and its handle
	pub fn new(config: ObsConfig) -> (Self, StateHandle) {
		let (sender, receiver) = mpsc::channel(100);
		let actor = Self {
			state: ObsState::new(config),
			receiver,
		};
		let handle = StateHandle { sender };
		(actor, handle)
	}

	/// Run the actor event loop
	pub async fn run(mut self) {
		while let Some(msg) = self.receiver.recv().await {
			match msg {
				StateMessage::GetConnectionState(reply) => {
					let _ = reply.send(self.state.connection_state.clone());
				}
				StateMessage::GetConfig(reply) => {
					let _ = reply.send(self.state.config.clone());
				}
				StateMessage::IsConnected(reply) => {
					let _ = reply.send(self.state.is_connected());
				}
				StateMessage::CanExecuteCommands(reply) => {
					let _ = reply.send(self.state.can_execute_commands());
				}
				StateMessage::GetCommandSender(reply) => {
					let _ = reply.send(self.state.command_sender.clone());
				}
				StateMessage::Transition(transition, reply) => {
					let result = self.state.transition(transition);
					let _ = reply.send(result);
				}
				StateMessage::SetCommandSender(sender) => {
					self.state.command_sender = Some(sender);
				}
				StateMessage::SetEventReceiver(receiver) => {
					self.state.event_receiver = Some(receiver);
				}
				StateMessage::SetEventSender(sender) => {
					self.state.event_sender = Some(sender);
				}
				StateMessage::SetConnectionHandle(handle) => {
					self.state.connection_handle = Some(handle);
				}
				StateMessage::TakeCommandSender(reply) => {
					let sender = self.state.command_sender.take();
					let _ = reply.send(sender);
				}
				StateMessage::TakeEventReceiver(reply) => {
					let receiver = self.state.event_receiver.take();
					let _ = reply.send(receiver);
				}
				StateMessage::TakeConnectionHandle(reply) => {
					let handle = self.state.connection_handle.take();
					let _ = reply.send(handle);
				}
				StateMessage::UpdateConfig(config) => {
					self.state.config = config;
				}
				StateMessage::ExecuteCommand(command, reply) => {
					let result = self.execute_command_internal(command).await;
					let _ = reply.send(result);
				}
			}
		}
	}

	/// Internal command execution logic
	async fn execute_command_internal(&self, command: ObsCommand) -> Result<(), StateError> {
		if !self.state.can_execute_commands() {
			return Err(StateError::NotConnected);
		}

		let sender = self.state.command_sender.as_ref().ok_or(StateError::NotConnected)?;

		sender.try_send(InternalCommand::Execute(command)).map_err(|e| StateError::CommandFailed(e.to_string()))?;

		Ok(())
	}
}

/// Handle for communicating with the state actor
#[derive(Clone)]
pub struct StateHandle {
	sender: mpsc::Sender<StateMessage>,
}

impl StateHandle {
	/// Get current connection state
	pub async fn connection_state(&self) -> Result<ConnectionState, StateError> {
		let (tx, rx) = oneshot::channel();
		self.sender.send(StateMessage::GetConnectionState(tx)).await.map_err(|_| StateError::ActorUnavailable)?;
		rx.await.map_err(|_| StateError::ActorUnavailable)
	}

	/// Get current config
	pub async fn config(&self) -> Result<ObsConfig, StateError> {
		let (tx, rx) = oneshot::channel();
		self.sender.send(StateMessage::GetConfig(tx)).await.map_err(|_| StateError::ActorUnavailable)?;
		rx.await.map_err(|_| StateError::ActorUnavailable)
	}

	/// Check if connected
	pub async fn is_connected(&self) -> Result<bool, StateError> {
		let (tx, rx) = oneshot::channel();
		self
			.sender
			.send(StateMessage::IsConnected(tx))
			.await
			.map_err(|e| StateError::ConnectionError(e.to_string()))?;
		rx.await.map_err(|e| StateError::ConnectionError(e.to_string()))
	}

	/// Check if can execute commands
	pub async fn can_execute_commands(&self) -> Result<bool, StateError> {
		let (tx, rx) = oneshot::channel();
		self.sender.send(StateMessage::CanExecuteCommands(tx)).await.map_err(|_| StateError::ActorUnavailable)?;
		rx.await.map_err(|e| StateError::CommandFailed(e.to_string()))
	}

	/// Get command sender (for internal use)
	pub async fn command_sender(&self) -> Result<Option<tokio::sync::mpsc::Sender<InternalCommand>>, StateError> {
		let (tx, rx) = oneshot::channel();
		self.sender.send(StateMessage::GetCommandSender(tx)).await.map_err(|_| StateError::ActorUnavailable)?;
		rx.await.map_err(|_| StateError::ActorUnavailable)
	}

	/// Execute a state transition
	pub async fn transition(&self, transition: StateTransition) -> Result<(), StateError> {
		let (tx, rx) = oneshot::channel();
		self
			.sender
			.send(StateMessage::Transition(transition, tx))
			.await
			.map_err(|e| StateError::TransitionError(e.to_string()))?;
		rx.await.map_err(|e| StateError::TransitionError(e.to_string()))?
	}

	/// Convenience methods for common transitions
	pub async fn transition_to_connecting(&self) -> Result<(), StateError> {
		self.transition(StateTransition::StartConnecting).await
	}

	pub async fn transition_to_connected(&self) -> Result<(), StateError> {
		self.transition(StateTransition::ConnectionEstablished).await
	}

	pub async fn transition_to_disconnecting(&self) -> Result<(), StateError> {
		self.transition(StateTransition::StartDisconnecting).await
	}

	pub async fn transition_to_disconnected(&self) -> Result<(), StateError> {
		tracing::warn!("transitioned to disconnected");
		self.transition(StateTransition::ConnectionLost).await
	}

	pub async fn transition_to_failed(&self, error: String) -> Result<(), StateError> {
		self.transition(StateTransition::ConnectionFailed(error)).await
	}

	/// Execute a command
	pub async fn execute_command(&self, command: ObsCommand) -> Result<(), StateError> {
		let (tx, rx) = oneshot::channel();
		self
			.sender
			.send(StateMessage::ExecuteCommand(command, tx))
			.await
			.map_err(|_| StateError::ActorUnavailable)?;
		rx.await.map_err(|_| StateError::ActorUnavailable)?
	}

	/// Resource management methods
	pub async fn set_command_sender(&self, sender: tokio::sync::mpsc::Sender<InternalCommand>) -> Result<(), StateError> {
		self.sender.send(StateMessage::SetCommandSender(sender)).await.map_err(|_| StateError::ActorUnavailable)
	}

	pub async fn set_event_receiver(&self, receiver: async_broadcast::Receiver<ObsEvent>) -> Result<(), StateError> {
		self.sender.send(StateMessage::SetEventReceiver(receiver)).await.map_err(|_| StateError::ActorUnavailable)
	}

	pub async fn set_event_sender(&self, sender: async_broadcast::Sender<ObsEvent>) -> Result<(), StateError> {
		self.sender.send(StateMessage::SetEventSender(sender)).await.map_err(|_| StateError::ActorUnavailable)
	}

	pub async fn set_connection_handle(&self, handle: tokio::task::JoinHandle<()>) -> Result<(), StateError> {
		self.sender.send(StateMessage::SetConnectionHandle(handle)).await.map_err(|_| StateError::ActorUnavailable)
	}

	pub async fn take_command_sender(&self) -> Result<Option<tokio::sync::mpsc::Sender<InternalCommand>>, StateError> {
		let (tx, rx) = oneshot::channel();
		self.sender.send(StateMessage::TakeCommandSender(tx)).await.map_err(|_| StateError::ActorUnavailable)?;
		rx.await.map_err(|_| StateError::ActorUnavailable)
	}

	pub async fn take_event_receiver(&self) -> Result<Option<async_broadcast::Receiver<ObsEvent>>, StateError> {
		let (tx, rx) = oneshot::channel();
		self.sender.send(StateMessage::TakeEventReceiver(tx)).await.map_err(|_| StateError::ActorUnavailable)?;
		rx.await.map_err(|_| StateError::ActorUnavailable)
	}

	pub async fn take_connection_handle(&self) -> Result<Option<tokio::task::JoinHandle<()>>, StateError> {
		let (tx, rx) = oneshot::channel();
		self.sender.send(StateMessage::TakeConnectionHandle(tx)).await.map_err(|_| StateError::ActorUnavailable)?;
		rx.await.map_err(|_| StateError::ActorUnavailable)
	}

	pub async fn update_config(&self, config: ObsConfig) -> Result<(), StateError> {
		self.sender.send(StateMessage::UpdateConfig(config)).await.map_err(|_| StateError::ActorUnavailable)
	}
}

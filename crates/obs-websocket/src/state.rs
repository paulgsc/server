use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

// Connection state machine
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConnectionState {
	Init,
	Connecting,
	HandshakeSent,
	Authenticating,
	Ready,
	Failed,
	Disconnected,
	Recovering,
}

// Events that can trigger state transitions
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
	ConnectRequested,
	HandshakeAckReceived,
	AuthChallengeReceived,
	AuthSuccess,
	AuthFailure,
	ConnectionError,
	DisconnectRequested,
	UnexpectedDisconnect,
	RetryTimeout,
}

impl ConnectionState {
	// Pure function for state transitions
	pub fn transition(self, event: Event) -> ConnectionState {
		use ConnectionState::*;
		use Event::*;

		match (self, event) {
			(Init, ConnectRequested) => Connecting,
			(Connecting, HandshakeAckReceived) => HandshakeSent,
			(HandshakeSent, AuthChallengeReceived) => Authenticating,
			(Authenticating, AuthSuccess) => Ready,
			(Authenticating, AuthFailure) => Failed,
			(_, ConnectionError) => Failed,
			(Ready, DisconnectRequested) => Disconnected,
			(Ready, UnexpectedDisconnect) => Recovering,
			(Failed, RetryTimeout) => Connecting,
			(Recovering, RetryTimeout) => Connecting,
			(s, _) => s, // no-op if event invalid in current state
		}
	}

	// Helper method to check if the state is a terminal state
	pub fn is_terminal(&self) -> bool {
		matches!(self, ConnectionState::Disconnected)
	}

	// Helper method to check if the state requires reconnection
	pub fn requires_reconnect(&self) -> bool {
		matches!(self, ConnectionState::Failed | ConnectionState::Recovering)
	}
}

// State manager to handle state transitions with RwLock
pub struct StateManager {
	state: Arc<RwLock<ConnectionState>>,
	// Optional callback for state changes
	on_state_change: Option<Box<dyn Fn(ConnectionState, ConnectionState) + Send + Sync>>,
}

impl StateManager {
	// Create a new state manager
	pub fn new() -> Self {
		Self {
			state: Arc::new(RwLock::new(ConnectionState::Init)),
			on_state_change: None,
		}
	}

	// Get a clone of the state for read-only operations
	pub fn get_state_handle(&self) -> Arc<RwLock<ConnectionState>> {
		Arc::clone(&self.state)
	}

	// Get the current state (blocking)
	pub async fn current_state(&self) -> ConnectionState {
		*self.state.read().await
	}

	// Process an event and transition to the new state
	pub async fn process_event(&self, event: Event) -> ConnectionState {
		let mut state_guard = self.state.write().await;
		let old_state = *state_guard;
		let new_state = old_state.transition(event);

		// Only log and update if state actually changed
		if old_state != new_state {
			debug!("State transition: {:?} -> {:?} triggered by {:?}", old_state, new_state, event);
			*state_guard = new_state;

			// Call state change callback if set
			if let Some(callback) = &self.on_state_change {
				callback(old_state, new_state);
			}
		}

		new_state
	}

	// Set a callback to be called on state changes
	pub fn on_state_change<F>(&mut self, callback: F)
	where
		F: Fn(ConnectionState, ConnectionState) + Send + Sync + 'static,
	{
		self.on_state_change = Some(Box::new(callback));
	}
}

// Example integration with the ObsWebSocketClient
pub fn integrate_state_machine(client: &mut ObsWebSocketClient) {
	let mut state_manager = StateManager::new();

	// Example callback for state changes
	state_manager.on_state_change(|old_state, new_state| {
		info!("OBS WebSocket connection state changed: {:?} -> {:?}", old_state, new_state);
	});

	// Store the state manager in the client
	client.state_manager = Some(state_manager);
}

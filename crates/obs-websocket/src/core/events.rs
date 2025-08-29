use super::{ConnectionState, StateError, StateHandle};
use crate::ObsEvent;
use futures_util::future;
use std::time::Duration;
use tracing::{debug, error, trace, warn};

pub struct EventHandler {
	state_handle: StateHandle,
}

impl EventHandler {
	pub fn new(state_handle: StateHandle) -> Self {
		Self { state_handle }
	}

	/// Get next event with timeout and state validation
	pub async fn next_event(&self) -> Result<ObsEvent, StateError> {
		// First check if we're connected
		if !self.state_handle.is_connected().await? {
			return Err(StateError::NotConnected);
		}

		// Take the event receiver temporarily
		let mut receiver = match self.state_handle.take_event_receiver().await? {
			Some(receiver) => receiver,
			None => return Err(StateError::NotConnected),
		};

		// Try to receive an event with timeout
		let result = match tokio::time::timeout(Duration::from_secs(30), receiver.recv()).await {
			Ok(Ok(event)) => {
				// Success - put the receiver back and return the event
				let _ = self.state_handle.set_event_receiver(receiver).await;
				Ok(event)
			}
			Ok(Err(_)) => {
				// Channel closed, transition to disconnected
				let _ = self.state_handle.transition_to_disconnected().await;
				Err(StateError::EventFailed("Event channel closed".into()))
			}
			Err(_) => {
				// Timeout - put the receiver back
				let _ = self.state_handle.set_event_receiver(receiver).await;
				Err(StateError::EventFailed("Timeout waiting for events".into()))
			}
		};

		result
	}

	/// Stream events continuously until disconnected
	pub async fn stream_events<F>(&self, mut handler: F) -> Result<(), StateError>
	where
		F: FnMut(ObsEvent) -> future::BoxFuture<'static, ()>,
	{
		trace!("starting event stream loop");
		loop {
			match self.next_event().await {
				Ok(event) => {
					handler(event).await;
				}
				Err(StateError::NotConnected) => {
					warn!("disconnected cleanly (NotConnected)");
					break Ok(());
				}
				Err(StateError::EventFailed(msg)) if msg.contains("channel closed") => {
					warn!(%msg, "channel closed, exiting cleanly");
					break Err(StateError::EventFailed(msg));
				}
				Err(StateError::EventFailed(msg)) if msg.contains("Timeout") => {
					debug!(%msg, "timeout, continuing loop");
					continue;
				}
				Err(e) => {
					error!(error = ?e, "fatal error, aborting event stream");
					break Err(e);
				}
			}
		}
	}

	/// Get multiple events with a batch timeout
	pub async fn next_events_batch(&self, max_events: usize, timeout: Duration) -> Result<Vec<ObsEvent>, StateError> {
		if !self.state_handle.is_connected().await? {
			return Err(StateError::NotConnected);
		}

		let mut receiver = match self.state_handle.take_event_receiver().await? {
			Some(receiver) => receiver,
			None => return Err(StateError::NotConnected),
		};

		let mut events = Vec::new();
		let start_time = tokio::time::Instant::now();

		while events.len() < max_events && start_time.elapsed() < timeout {
			let remaining_timeout = timeout.saturating_sub(start_time.elapsed());

			match tokio::time::timeout(remaining_timeout, receiver.recv()).await {
				Ok(Ok(event)) => {
					events.push(event);
				}
				Ok(Err(_)) => {
					// Channel closed
					let _ = self.state_handle.transition_to_disconnected().await;
					return Err(StateError::EventFailed("Event channel closed".into()));
				}
				Err(_) => {
					// Timeout reached
					break;
				}
			}
		}

		// Put the receiver back
		let _ = self.state_handle.set_event_receiver(receiver).await;

		if events.is_empty() {
			Err(StateError::EventFailed("No events received within timeout".into()))
		} else {
			Ok(events)
		}
	}

	/// Check if the event system is healthy (connected and has receiver)
	pub async fn is_healthy(&self) -> Result<bool, StateError> {
		let connected = self.state_handle.is_connected().await?;
		if !connected {
			return Ok(false);
		}

		// Check if we have an event receiver
		let has_receiver = self.state_handle.take_event_receiver().await?.is_some();
		if has_receiver {
			// Put it back if we took it
			if let Ok(Some(receiver)) = self.state_handle.take_event_receiver().await {
				let _ = self.state_handle.set_event_receiver(receiver).await;
			}
		}

		Ok(has_receiver)
	}

	/// Get the current connection state for event handling context
	pub async fn connection_state(&self) -> Result<ConnectionState, StateError> {
		self.state_handle.connection_state().await
	}
}

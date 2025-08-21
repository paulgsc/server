use super::*;
use crate::ObsEvent;
use async_broadcast::{Receiver, Sender};
use thiserror::Error;
use tracing::error;

/// Broadcast-specific error types
#[derive(Error, Debug)]
pub enum BroadcastError {
	#[error("Connection error: {0}")]
	Connection(#[from] ConnectionError),

	#[error("State error: {0}")]
	State(#[from] StateError),

	#[error("Event processing failed: {0}")]
	EventProcessing(String),

	#[error("Broadcast channel error: {0}")]
	Channel(#[from] async_broadcast::SendError<ObsEvent>),

	#[error("Retry limit exceeded after {attempts} attempts")]
	RetryExhausted { attempts: u32 },

	#[error("Manager unavailable or shutdown")]
	ManagerUnavailable,

	#[error("Broadcast loop was cancelled")]
	Cancelled,
}

pub struct EventBroadcaster {
	pub(crate) sender: Sender<ObsEvent>,
}

impl EventBroadcaster {
	pub fn new() -> Self {
		let (mut sender, _) = async_broadcast::broadcast(3);
		sender.set_overflow(true);
		sender.set_await_active(true);

		Self { sender }
	}

	pub fn subscribe(&self) -> Receiver<crate::messages::ObsEvent> {
		self.sender.new_receiver()
	}
}

pub struct BroadcastHandle {
	task_handle: tokio::task::JoinHandle<()>,
	broadcaster: Sender<ObsEvent>,
}

impl BroadcastHandle {
	pub fn subscribe(&self) -> Receiver<ObsEvent> {
		self.broadcaster.new_receiver()
	}

	pub async fn stop(self) -> Result<(), BroadcastError> {
		self.task_handle.abort();

		match self.task_handle.await {
			Ok(()) => Ok(()),
			Err(e) if e.is_cancelled() => Ok(()), // Expected when we abort
			Err(e) => Err(BroadcastError::EventProcessing(format!("Task join error: {}", e))),
		}
	}

	pub fn is_running(&self) -> bool {
		!self.task_handle.is_finished()
	}

	/// Get a clone of the broadcaster for creating additional subscribers
	pub fn broadcaster(&self) -> &Sender<ObsEvent> {
		&self.broadcaster
	}

	/// Check if the broadcast handle is healthy (task running and channel open)
	pub fn is_healthy(&self) -> bool {
		self.is_running() && !self.broadcaster.is_closed()
	}
}

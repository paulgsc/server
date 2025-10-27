#![cfg(feature = "mpsc_utils")]

/// Utilities for working with tokio mpsc channels with graceful error handling

/// Result type for send operations
#[derive(Debug)]
pub enum SendResult<T> {
	/// Message sent successfully
	Sent,
	/// Receiver has been dropped (connection closed)
	ReceiverDropped(T),
	/// Channel is full (for bounded channels)
	ChannelFull(T),
}

impl<T> SendResult<T> {
	pub fn is_ok(&self) -> bool {
		matches!(self, SendResult::Sent)
	}

	pub fn is_receiver_dropped(&self) -> bool {
		matches!(self, SendResult::ReceiverDropped(_))
	}

	pub fn is_channel_full(&self) -> bool {
		matches!(self, SendResult::ChannelFull(_))
	}
}

/// Result type for receive operations
#[derive(Debug)]
pub enum RecvResult<T> {
	/// Message received successfully
	Message(T),
	/// Sender has been dropped (no more messages)
	SenderDropped,
	/// Receive timeout elapsed
	Timeout,
}

impl<T> RecvResult<T> {
	pub fn is_message(&self) -> bool {
		matches!(self, RecvResult::Message(_))
	}

	pub fn is_closed(&self) -> bool {
		matches!(self, RecvResult::SenderDropped)
	}

	pub fn into_option(self) -> Option<T> {
		match self {
			RecvResult::Message(msg) => Some(msg),
			_ => None,
		}
	}
}

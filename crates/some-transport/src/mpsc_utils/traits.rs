#![cfg(feature = "mpsc_utils")]

use super::{RecvResult, SendResult};
use tokio::sync::mpsc::{self, error::SendError};
use tracing::{debug, warn};

/// Extension trait for unbounded sender with graceful error handling and logging.
///
/// This trait provides methods for sending messages through unbounded channels
/// with automatic error handling and structured logging via tracing.
///
/// # Examples
///
/// ```
/// use tokio::sync::mpsc;
/// use your_crate::mpsc_utils::{UnboundedSenderExt, SendResult};
///
/// #[tokio::main]
/// async fn main() {
///     let (tx, mut rx) = mpsc::unbounded_channel();
///     
///     // Send with graceful error handling
///     match tx.send_graceful(42, "processor") {
///         SendResult::Sent => println!("Message sent"),
///         SendResult::ReceiverDropped(msg) => {
///             println!("Receiver gone, message: {}", msg);
///         }
///         _ => unreachable!(),
///     }
/// }
/// ```
pub trait UnboundedSenderExt<T> {
	/// Send a message with graceful error handling and automatic logging.
	///
	/// This method sends a message through the channel and logs the result.
	/// If the receiver has been dropped, it returns the message in the error variant.
	///
	/// # Arguments
	///
	/// * `msg` - The message to send
	/// * `context` - A context string for logging (e.g., "worker_pool", "event_handler")
	///
	/// # Returns
	///
	/// * `SendResult::Sent` - Message was sent successfully
	/// * `SendResult::ReceiverDropped(T)` - Receiver was dropped, returns the message
	///
	/// # Examples
	///
	/// ```
	/// use tokio::sync::mpsc;
	/// use your_crate::mpsc_utils::{UnboundedSenderExt, SendResult};
	///
	/// #[tokio::main]
	/// async fn main() {
	///     let (tx, mut rx) = mpsc::unbounded_channel();
	///     
	///     if tx.send_graceful(42, "my_component").is_ok() {
	///         println!("Message sent successfully");
	///     }
	/// }
	/// ```
	fn send_graceful(&self, msg: T, context: &str) -> SendResult<T>;

	/// Send a message with a custom error handler.
	///
	/// This method allows you to provide a custom closure that will be called
	/// if the send operation fails. Useful for custom logging, metrics, or error recovery.
	///
	/// # Arguments
	///
	/// * `msg` - The message to send
	/// * `handler` - A closure that receives the `SendError` if the send fails
	///
	/// # Examples
	///
	/// ```
	/// use tokio::sync::mpsc;
	/// use your_crate::mpsc_utils::{UnboundedSenderExt, SendResult};
	///
	/// #[tokio::main]
	/// async fn main() {
	///     let (tx, rx) = mpsc::unbounded_channel();
	///     drop(rx); // Simulate receiver drop
	///     
	///     let result = tx.send_with_handler(42, |err| {
	///         eprintln!("Send failed: {:?}", err);
	///         // Could log metrics, trigger alerts, etc.
	///     });
	///     
	///     assert!(result.is_receiver_dropped());
	/// }
	/// ```
	fn send_with_handler<F>(&self, msg: T, handler: F) -> SendResult<T>
	where
		F: FnOnce(&SendError<T>);
}

impl<T> UnboundedSenderExt<T> for mpsc::UnboundedSender<T>
where
	T: std::fmt::Debug + Clone,
{
	fn send_graceful(&self, msg: T, context: &str) -> SendResult<T> {
		match self.send(msg) {
			Ok(_) => {
				debug!(context = context, "Message sent successfully");
				SendResult::Sent
			}
			Err(SendError(msg)) => {
				warn!(context = context, "Failed to send message: receiver dropped");
				SendResult::ReceiverDropped(msg)
			}
		}
	}

	fn send_with_handler<F>(&self, msg: T, handler: F) -> SendResult<T>
	where
		F: FnOnce(&SendError<T>),
	{
		match self.send(msg) {
			Ok(_) => SendResult::Sent,
			Err(e) => {
				handler(&e);
				SendResult::ReceiverDropped(e.0)
			}
		}
	}
}

/// Extension trait for bounded sender with graceful error handling and backpressure detection.
///
/// This trait provides methods for sending messages through bounded channels with
/// timeout support, backpressure detection, and graceful error handling.
///
/// # Examples
///
/// ```
/// use tokio::sync::mpsc;
/// use std::time::Duration;
/// use your_crate::mpsc_utils::{SenderExt, SendResult};
///
/// #[tokio::main]
/// async fn main() {
///     let (tx, mut rx) = mpsc::channel(10);
///     
///     // Try to send without blocking
///     match tx.try_send_graceful(42, "worker") {
///         SendResult::Sent => println!("Sent immediately"),
///         SendResult::ChannelFull(msg) => println!("Channel full: {}", msg),
///         SendResult::ReceiverDropped(msg) => println!("Receiver gone: {}", msg),
///     }
/// }
/// ```
pub trait SenderExt<T> {
	/// Try to send a message without blocking.
	///
	/// This method attempts to send a message immediately. If the channel is full
	/// or the receiver is dropped, it returns the message in the error variant.
	///
	/// # Arguments
	///
	/// * `msg` - The message to send
	/// * `context` - A context string for logging
	///
	/// # Returns
	///
	/// * `SendResult::Sent` - Message was sent successfully
	/// * `SendResult::ChannelFull(T)` - Channel is full, returns the message
	/// * `SendResult::ReceiverDropped(T)` - Receiver was dropped, returns the message
	///
	/// # Examples
	///
	/// ```
	/// use tokio::sync::mpsc;
	/// use your_crate::mpsc_utils::{SenderExt, SendResult};
	///
	/// #[tokio::main]
	/// async fn main() {
	///     let (tx, mut rx) = mpsc::channel(1);
	///     
	///     // Fill the channel
	///     tx.try_send(1).unwrap();
	///     
	///     // This will fail with ChannelFull
	///     match tx.try_send_graceful(2, "producer") {
	///         SendResult::ChannelFull(msg) => {
	///             println!("Channel full, dropped message: {}", msg);
	///         }
	///         _ => {}
	///     }
	/// }
	/// ```
	fn try_send_graceful(&self, msg: T, context: &str) -> SendResult<T>;

	/// Send a message with a timeout.
	///
	/// This method attempts to send a message within the specified timeout duration.
	/// If the timeout elapses, it treats the situation as a full channel.
	///
	/// # Arguments
	///
	/// * `msg` - The message to send
	/// * `timeout` - Maximum duration to wait for the send to complete
	/// * `context` - A context string for logging
	///
	/// # Returns
	///
	/// * `SendResult::Sent` - Message was sent successfully
	/// * `SendResult::ChannelFull(T)` - Timeout elapsed, returns the message
	/// * `SendResult::ReceiverDropped(T)` - Receiver was dropped, returns the message
	///
	/// # Examples
	///
	/// ```
	/// use tokio::sync::mpsc;
	/// use std::time::Duration;
	/// use your_crate::mpsc_utils::{SenderExt, SendResult};
	///
	/// #[tokio::main]
	/// async fn main() {
	///     let (tx, mut rx) = mpsc::channel(1);
	///     
	///     // Send with 100ms timeout
	///     match tx.send_graceful_timeout(42, Duration::from_millis(100), "api_handler").await {
	///         SendResult::Sent => println!("Sent within timeout"),
	///         SendResult::ChannelFull(msg) => {
	///             println!("Timeout - receiver too slow");
	///         }
	///         SendResult::ReceiverDropped(msg) => {
	///             println!("Receiver dropped");
	///         }
	///     }
	/// }
	/// ```
	fn send_graceful_timeout(&self, msg: T, timeout: std::time::Duration, context: &str) -> impl std::future::Future<Output = SendResult<T>> + Send;

	/// Send a message with backpressure warnings.
	///
	/// This method sends a message and logs a warning if the send takes longer than 100ms,
	/// which may indicate backpressure or a slow receiver.
	///
	/// # Arguments
	///
	/// * `msg` - The message to send
	/// * `context` - A context string for logging
	///
	/// # Returns
	///
	/// * `SendResult::Sent` - Message was sent successfully
	/// * `SendResult::ReceiverDropped(T)` - Receiver was dropped, returns the message
	///
	/// # Examples
	///
	/// ```
	/// use tokio::sync::mpsc;
	/// use your_crate::mpsc_utils::{SenderExt, SendResult};
	///
	/// #[tokio::main]
	/// async fn main() {
	///     let (tx, mut rx) = mpsc::channel(100);
	///     
	///     // Send with automatic backpressure detection
	///     // Logs a warning if send takes > 100ms
	///     tx.send_with_backpressure_warn(42, "event_processor").await;
	/// }
	/// ```
	fn send_with_backpressure_warn(&self, msg: T, context: &str) -> impl std::future::Future<Output = SendResult<T>> + Send;
}

impl<T> SenderExt<T> for mpsc::Sender<T>
where
	T: std::fmt::Debug + Clone + Send,
{
	fn try_send_graceful(&self, msg: T, context: &str) -> SendResult<T> {
		match self.try_send(msg) {
			Ok(_) => {
				debug!(context = context, "Message sent successfully");
				SendResult::Sent
			}
			Err(mpsc::error::TrySendError::Full(msg)) => {
				warn!(context = context, capacity = self.capacity(), "Channel full, message dropped");
				SendResult::ChannelFull(msg)
			}
			Err(mpsc::error::TrySendError::Closed(msg)) => {
				warn!(context = context, "Failed to send: receiver dropped");
				SendResult::ReceiverDropped(msg)
			}
		}
	}

	async fn send_graceful_timeout(&self, msg: T, timeout_duration: std::time::Duration, context: &str) -> SendResult<T> {
		match tokio::time::timeout(timeout_duration, self.send(msg.clone())).await {
			Ok(Ok(_)) => {
				debug!(context = context, "Message sent successfully");
				SendResult::Sent
			}
			Ok(Err(SendError(msg))) => {
				warn!(context = context, "Failed to send: receiver dropped");
				SendResult::ReceiverDropped(msg)
			}
			Err(_) => {
				warn!(
					context = context,
					timeout_ms = timeout_duration.as_millis(),
					"Send timeout: channel likely full or receiver slow"
				);
				// Timeout means we still have the message, treat as full
				SendResult::ChannelFull(self.try_send(msg).err().unwrap().into_inner())
			}
		}
	}

	async fn send_with_backpressure_warn(&self, msg: T, context: &str) -> SendResult<T> {
		let start = std::time::Instant::now();

		match self.send(msg).await {
			Ok(_) => {
				let elapsed = start.elapsed();
				if elapsed.as_millis() > 100 {
					warn!(
						context = context,
						delay_ms = elapsed.as_millis(),
						capacity = self.capacity(),
						"Slow send detected - possible backpressure"
					);
				}
				SendResult::Sent
			}
			Err(SendError(msg)) => {
				warn!(context = context, "Failed to send: receiver dropped");
				SendResult::ReceiverDropped(msg)
			}
		}
	}
}

/// Extension trait for receiver with graceful error handling and timeout support.
///
/// This trait provides methods for receiving messages with automatic logging,
/// timeout support, and custom closed handlers.
///
/// # Examples
///
/// ```
/// use tokio::sync::mpsc;
/// use std::time::Duration;
/// use your_crate::mpsc_utils::{ReceiverExt, RecvResult};
///
/// #[tokio::main]
/// async fn main() {
///     let (tx, mut rx) = mpsc::channel(10);
///     
///     tx.send(42).await.unwrap();
///     
///     match rx.recv_graceful("worker").await {
///         RecvResult::Message(msg) => println!("Received: {}", msg),
///         RecvResult::SenderDropped => println!("No more messages"),
///         RecvResult::Timeout => unreachable!(),
///     }
/// }
/// ```
pub trait ReceiverExt<T> {
	/// Receive a message with graceful error handling.
	///
	/// This method receives a message from the channel and logs the result.
	/// If the sender has been dropped and no messages remain, it returns `SenderDropped`.
	///
	/// # Arguments
	///
	/// * `context` - A context string for logging
	///
	/// # Returns
	///
	/// * `RecvResult::Message(T)` - Message was received successfully
	/// * `RecvResult::SenderDropped` - Sender was dropped and no messages remain
	///
	/// # Examples
	///
	/// ```
	/// use tokio::sync::mpsc;
	/// use your_crate::mpsc_utils::{ReceiverExt, RecvResult};
	///
	/// #[tokio::main]
	/// async fn main() {
	///     let (tx, mut rx) = mpsc::channel(10);
	///     
	///     tokio::spawn(async move {
	///         tx.send(1).await.unwrap();
	///         tx.send(2).await.unwrap();
	///     });
	///     
	///     while let RecvResult::Message(msg) = rx.recv_graceful("processor").await {
	///         println!("Processing: {}", msg);
	///     }
	/// }
	/// ```
	fn recv_graceful(&mut self, context: &str) -> impl std::future::Future<Output = RecvResult<T>> + Send;

	/// Receive a message with a timeout.
	///
	/// This method attempts to receive a message within the specified timeout duration.
	/// If the timeout elapses before a message arrives, it returns `Timeout`.
	///
	/// # Arguments
	///
	/// * `timeout` - Maximum duration to wait for a message
	/// * `context` - A context string for logging
	///
	/// # Returns
	///
	/// * `RecvResult::Message(T)` - Message was received successfully
	/// * `RecvResult::SenderDropped` - Sender was dropped and no messages remain
	/// * `RecvResult::Timeout` - Timeout elapsed before receiving a message
	///
	/// # Examples
	///
	/// ```
	/// use tokio::sync::mpsc;
	/// use std::time::Duration;
	/// use your_crate::mpsc_utils::{ReceiverExt, RecvResult};
	///
	/// #[tokio::main]
	/// async fn main() {
	///     let (tx, mut rx) = mpsc::channel(10);
	///     
	///     // Try to receive with 100ms timeout
	///     match rx.recv_timeout(Duration::from_millis(100), "api_handler").await {
	///         RecvResult::Message(msg) => println!("Got message: {}", msg),
	///         RecvResult::Timeout => println!("No message within 100ms"),
	///         RecvResult::SenderDropped => println!("Sender closed"),
	///     }
	/// }
	/// ```
	fn recv_timeout(&mut self, timeout: std::time::Duration, context: &str) -> impl std::future::Future<Output = RecvResult<T>> + Send;

	/// Receive a message with a custom closed handler.
	///
	/// This method allows you to provide a custom closure that will be called
	/// if the channel is closed (sender dropped and no messages remain).
	///
	/// # Arguments
	///
	/// * `handler` - A closure to call if the channel is closed
	///
	/// # Examples
	///
	/// ```
	/// use tokio::sync::mpsc;
	/// use your_crate::mpsc_utils::{ReceiverExt, RecvResult};
	///
	/// #[tokio::main]
	/// async fn main() {
	///     let (tx, mut rx) = mpsc::channel(10);
	///     drop(tx); // Close the channel
	///     
	///     let result = rx.recv_with_closed_handler(|| {
	///         println!("Channel closed, performing cleanup...");
	///     }).await;
	///     
	///     assert!(result.is_closed());
	/// }
	/// ```
	fn recv_with_closed_handler<F>(&mut self, handler: F) -> impl std::future::Future<Output = RecvResult<T>> + Send
	where
		F: FnOnce() + Send;
}

impl<T> ReceiverExt<T> for mpsc::Receiver<T>
where
	T: Send,
{
	async fn recv_graceful(&mut self, context: &str) -> RecvResult<T> {
		match self.recv().await {
			Some(msg) => {
				debug!(context = context, "Message received");
				RecvResult::Message(msg)
			}
			None => {
				debug!(context = context, "Channel closed: sender dropped");
				RecvResult::SenderDropped
			}
		}
	}

	async fn recv_timeout(&mut self, timeout_duration: std::time::Duration, context: &str) -> RecvResult<T> {
		match tokio::time::timeout(timeout_duration, self.recv()).await {
			Ok(Some(msg)) => {
				debug!(context = context, "Message received");
				RecvResult::Message(msg)
			}
			Ok(None) => {
				debug!(context = context, "Channel closed: sender dropped");
				RecvResult::SenderDropped
			}
			Err(_) => {
				debug!(context = context, timeout_ms = timeout_duration.as_millis(), "Receive timeout");
				RecvResult::Timeout
			}
		}
	}

	async fn recv_with_closed_handler<F>(&mut self, handler: F) -> RecvResult<T>
	where
		F: FnOnce() + Send,
	{
		match self.recv().await {
			Some(msg) => RecvResult::Message(msg),
			None => {
				handler();
				RecvResult::SenderDropped
			}
		}
	}
}

/// Extension trait for unbounded receiver with graceful error handling and timeout support.
///
/// This trait provides methods for receiving messages from unbounded channels with
/// automatic logging, timeout support, and custom closed handlers.
///
/// # Examples
///
/// ```
/// use tokio::sync::mpsc;
/// use your_crate::mpsc_utils::{UnboundedReceiverExt, RecvResult};
///
/// #[tokio::main]
/// async fn main() {
///     let (tx, mut rx) = mpsc::unbounded_channel();
///     
///     tx.send(42).unwrap();
///     
///     match rx.recv_graceful("consumer").await {
///         RecvResult::Message(msg) => println!("Received: {}", msg),
///         RecvResult::SenderDropped => println!("No more messages"),
///         RecvResult::Timeout => unreachable!(),
///     }
/// }
/// ```
pub trait UnboundedReceiverExt<T> {
	/// Receive a message with graceful error handling.
	///
	/// This method receives a message from the unbounded channel and logs the result.
	/// If the sender has been dropped and no messages remain, it returns `SenderDropped`.
	///
	/// # Arguments
	///
	/// * `context` - A context string for logging
	///
	/// # Returns
	///
	/// * `RecvResult::Message(T)` - Message was received successfully
	/// * `RecvResult::SenderDropped` - Sender was dropped and no messages remain
	///
	/// # Examples
	///
	/// ```
	/// use tokio::sync::mpsc;
	/// use your_crate::mpsc_utils::{UnboundedReceiverExt, RecvResult};
	///
	/// #[tokio::main]
	/// async fn main() {
	///     let (tx, mut rx) = mpsc::unbounded_channel();
	///     
	///     tokio::spawn(async move {
	///         for i in 0..10 {
	///             tx.send(i).unwrap();
	///         }
	///     });
	///     
	///     while let RecvResult::Message(msg) = rx.recv_graceful("worker").await {
	///         println!("Got: {}", msg);
	///     }
	/// }
	/// ```
	fn recv_graceful(&mut self, context: &str) -> impl std::future::Future<Output = RecvResult<T>> + Send;

	/// Receive a message with a timeout.
	///
	/// This method attempts to receive a message within the specified timeout duration.
	/// If the timeout elapses before a message arrives, it returns `Timeout`.
	///
	/// # Arguments
	///
	/// * `timeout` - Maximum duration to wait for a message
	/// * `context` - A context string for logging
	///
	/// # Returns
	///
	/// * `RecvResult::Message(T)` - Message was received successfully
	/// * `RecvResult::SenderDropped` - Sender was dropped and no messages remain
	/// * `RecvResult::Timeout` - Timeout elapsed before receiving a message
	///
	/// # Examples
	///
	/// ```
	/// use tokio::sync::mpsc;
	/// use std::time::Duration;
	/// use your_crate::mpsc_utils::{UnboundedReceiverExt, RecvResult};
	///
	/// #[tokio::main]
	/// async fn main() {
	///     let (tx, mut rx) = mpsc::unbounded_channel();
	///     
	///     // Try to receive with 50ms timeout
	///     match rx.recv_timeout(Duration::from_millis(50), "timeout_test").await {
	///         RecvResult::Message(msg) => println!("Got: {}", msg),
	///         RecvResult::Timeout => println!("Timed out waiting for message"),
	///         RecvResult::SenderDropped => println!("Channel closed"),
	///     }
	/// }
	/// ```
	fn recv_timeout(&mut self, timeout: std::time::Duration, context: &str) -> impl std::future::Future<Output = RecvResult<T>> + Send;

	/// Receive a message with a custom closed handler.
	///
	/// This method allows you to provide a custom closure that will be called
	/// if the channel is closed (sender dropped and no messages remain).
	///
	/// # Arguments
	///
	/// * `handler` - A closure to call if the channel is closed
	///
	/// # Examples
	///
	/// ```
	/// use tokio::sync::mpsc;
	/// use your_crate::mpsc_utils::{UnboundedReceiverExt, RecvResult};
	///
	/// #[tokio::main]
	/// async fn main() {
	///     let (tx, mut rx) = mpsc::unbounded_channel();
	///     drop(tx);
	///     
	///     let result = rx.recv_with_closed_handler(|| {
	///         println!("Cleanup on channel close");
	///     }).await;
	///     
	///     assert!(result.is_closed());
	/// }
	/// ```
	fn recv_with_closed_handler<F>(&mut self, handler: F) -> impl std::future::Future<Output = RecvResult<T>> + Send
	where
		F: FnOnce() + Send;
}

impl<T> UnboundedReceiverExt<T> for mpsc::UnboundedReceiver<T>
where
	T: Send,
{
	async fn recv_graceful(&mut self, context: &str) -> RecvResult<T> {
		match self.recv().await {
			Some(msg) => {
				debug!(context = context, "Message received");
				RecvResult::Message(msg)
			}
			None => {
				debug!(context = context, "Channel closed: sender dropped");
				RecvResult::SenderDropped
			}
		}
	}

	async fn recv_timeout(&mut self, timeout_duration: std::time::Duration, context: &str) -> RecvResult<T> {
		match tokio::time::timeout(timeout_duration, self.recv()).await {
			Ok(Some(msg)) => {
				debug!(context = context, "Message received");
				RecvResult::Message(msg)
			}
			Ok(None) => {
				debug!(context = context, "Channel closed: sender dropped");
				RecvResult::SenderDropped
			}
			Err(_) => {
				debug!(context = context, timeout_ms = timeout_duration.as_millis(), "Receive timeout");
				RecvResult::Timeout
			}
		}
	}

	async fn recv_with_closed_handler<F>(&mut self, handler: F) -> RecvResult<T>
	where
		F: FnOnce() + Send,
	{
		match self.recv().await {
			Some(msg) => RecvResult::Message(msg),
			None => {
				handler();
				RecvResult::SenderDropped
			}
		}
	}
}

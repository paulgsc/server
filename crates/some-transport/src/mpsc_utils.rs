#![cfg(feature = "mpsc_utils")]

mod traits;
mod types;

pub use traits::{ReceiverExt, SenderExt, UnboundedReceiverExt, UnboundedSenderExt};
pub use types::{RecvResult, SendResult};

/// Convenience macros for common patterns
#[macro_export]
macro_rules! send_or_warn {
	($sender:expr, $msg:expr, $context:expr) => {{
		use $crate::mpsc_utils::UnboundedSenderExt;
		$sender.send_graceful($msg, $context)
	}};
}

#[macro_export]
macro_rules! send_or_break {
	($sender:expr, $msg:expr, $context:expr) => {{
		use $crate::mpsc_utils::UnboundedSenderExt;
		match $sender.send_graceful($msg, $context) {
			$crate::mpsc_utils::SendResult::Sent => {}
			_ => break,
		}
	}};
}

#[macro_export]
macro_rules! recv_or_break {
	($receiver:expr, $context:expr) => {{
		use $crate::mpsc_utils::UnboundedReceiverExt;
		match $receiver.recv_graceful($context).await {
			$crate::mpsc_utils::RecvResult::Message(msg) => msg,
			_ => break,
		}
	}};
}

#[cfg(test)]
mod tests {
	use super::*;
	use tokio::time::Duration;

	#[tokio::test]
	async fn test_unbounded_send_graceful() {
		let (tx, mut rx) = mpsc::unbounded_channel();

		let result = tx.send_graceful(42, "test_send");
		assert!(result.is_ok());

		let received = rx.recv().await.unwrap();
		assert_eq!(received, 42);
	}

	#[tokio::test]
	async fn test_unbounded_send_after_drop() {
		let (tx, rx) = mpsc::unbounded_channel::<i32>();
		drop(rx);

		let result = tx.send_graceful(42, "test_drop");
		assert!(result.is_receiver_dropped());
	}

	#[tokio::test]
	async fn test_bounded_try_send_full() {
		let (tx, _rx) = mpsc::channel(1);

		tx.try_send(1).unwrap();
		let result = tx.try_send_graceful(2, "test_full");

		assert!(result.is_channel_full());
	}

	#[tokio::test]
	async fn test_recv_graceful() {
		let (tx, mut rx) = mpsc::unbounded_channel();

		tx.send(42).unwrap();
		let result = rx.recv_graceful("test_recv").await;

		assert!(result.is_message());
		assert_eq!(result.into_option(), Some(42));
	}

	#[tokio::test]
	async fn test_recv_timeout() {
		let (_tx, mut rx) = mpsc::unbounded_channel::<i32>();

		let result = rx.recv_timeout(Duration::from_millis(10), "test_timeout").await;

		assert!(matches!(result, RecvResult::Timeout));
	}

	#[tokio::test]
	async fn test_macro_send_or_break() {
		let (tx, mut rx) = mpsc::unbounded_channel();

		tokio::spawn(async move {
			loop {
				send_or_break!(tx, 42, "macro_test");
				tokio::time::sleep(Duration::from_millis(10)).await;
			}
		});

		let received = rx.recv().await.unwrap();
		assert_eq!(received, 42);
	}

	#[tokio::test]
	async fn test_macro_recv_or_break() {
		let (tx, mut rx) = mpsc::unbounded_channel();

		tx.send(1).unwrap();
		tx.send(2).unwrap();
		drop(tx);

		let mut sum = 0;
		loop {
			let val = recv_or_break!(rx, "macro_recv_test");
			sum += val;
		}

		assert_eq!(sum, 3);
	}
}

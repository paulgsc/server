//! In-memory transport implementation using async_broadcast
//!
//! This module provides a high-performance, in-process message transport
//! suitable for testing, single-process applications, or as a local pubsub.
//!
//! # Features
//!
//! - Zero network overhead
//! - Lock-free using async_broadcast channels
//! - Per-connection channels for isolated communication
//! - Global broadcast support
//! - Overflow handling with configurable behavior
//!
//! # Example
//!
//! ```rust,no_run
//! use transport::inmem::InMemTransport;
//! use transport::traits::Transport;
//!
//! #[tokio::main]
//! async fn main() {
//!     // Create transport with buffer size
//!     let (transport, mut main_rx) = InMemTransport::<String>::with_receiver(100);
//!     
//!     // Subscribe to broadcasts
//!     tokio::spawn(async move {
//!         while let Ok(msg) = main_rx.recv().await {
//!             println!("Broadcast: {}", msg);
//!         }
//!     });
//!     
//!     // Open a dedicated channel
//!     let mut channel_rx = transport.open_channel("user_123").await;
//!     
//!     // Send to specific channel
//!     transport.send("user_123", "Hello user!".to_string()).await.ok();
//!     
//!     // Broadcast to all subscribers
//!     transport.broadcast("Hello everyone!".to_string()).await.ok();
//!     
//!     // Receive from channel
//!     if let Ok(msg) = channel_rx.recv().await {
//!         println!("Channel msg: {}", msg);
//!     }
//! }
//! ```

#![cfg(feature = "inmem")]

mod receiver;
mod transport;

// Re-export public types
pub use receiver::InMemReceiver;
pub use transport::InMemTransport;

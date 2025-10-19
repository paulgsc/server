//! Transport Layer Crate
//!
//! Provides a clean abstraction for managing per-connection transport tasks
//! with channels for communication between transport and coordinator layers.
//!
//! # Features
//!
//! - `inmem` - Enable in-memory transport using async_broadcast
//! - `nats` - Enable NATS-based distributed transport
//!
//! # Architecture
//!
//! This crate uses a trait-based design where all transports implement the
//! `Transport` trait, and all receivers implement `ReceiverTrait`. This allows
//! for seamless swapping between transport implementations.
//!
//! # Example
//!
//! ```rust,no_run
//! use some_transport::{Transport, TransportReceiver};
//!
//! #[cfg(feature = "inmem")]
//! async fn example_inmem() {
//!     use transport::InMemTransport;
//!     
//!     let (transport, mut rx) = InMemTransport::<String>::with_receiver(100);
//!     
//!     transport.broadcast("Hello!".to_string()).await.ok();
//!     
//!     if let Ok(msg) = rx.recv().await {
//!         println!("Received: {}", msg);
//!     }
//! }
//!
//! #[cfg(feature = "nats")]
//! async fn example_nats() {
//!     use some_transport::NatsTransport;
//!     
//!     let transport = NatsTransport::<String>::connect_pooled("nats://localhost:4222")
//!         .await
//!         .unwrap();
//!     
//!     let mut rx = transport.subscribe().await;
//!     
//!     transport.broadcast("Hello!".to_string()).await.ok();
//!     
//!     if let Ok(msg) = rx.recv().await {
//!         println!("Received: {}", msg);
//!     }
//! }
//! ```

// Core modules (always available)
pub mod error;
pub mod receiver;
pub mod traits;

// Re-export core types
pub use error::TransportError;
pub use receiver::{ReceiverTrait, TransportReceiver};
pub use traits::Transport;

// Feature-gated transport implementations
#[cfg(feature = "inmem")]
pub mod inmem;

#[cfg(feature = "nats")]
pub mod nats;

// Re-export transport types
#[cfg(feature = "inmem")]
pub use inmem::{InMemReceiver, InMemTransport};

#[cfg(feature = "nats")]
pub use nats::{NatsConnectionPool, NatsReceiver, NatsTransport};

// Type aliases for convenience and ergonomics
#[cfg(feature = "inmem")]
pub type InMemTransportReceiver<E> = TransportReceiver<E, InMemReceiver<E>>;

#[cfg(feature = "nats")]
pub type NatsTransportReceiver<E> = TransportReceiver<E, NatsReceiver<E>>;

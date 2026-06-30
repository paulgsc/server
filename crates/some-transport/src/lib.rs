#![allow(clippy::doc_markdown)]
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::missing_panics_doc)]
#![allow(clippy::missing_const_for_fn)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_lossless)]
#![allow(clippy::disallowed_macros)]
#![allow(clippy::disallowed_methods)]
#![allow(clippy::must_use_candidate)]
#![allow(clippy::return_self_not_must_use)]
#![allow(clippy::redundant_clone)]
#![allow(clippy::expect_used)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::match_same_arms)]
#![allow(clippy::future_not_send)]
#![allow(clippy::multiple_crate_versions)]
#![allow(clippy::wildcard_imports)]
#![allow(clippy::enum_glob_use)]
#![allow(clippy::ignored_unit_patterns)]
#![allow(clippy::option_if_let_else)]
#![allow(clippy::single_match)]
#![allow(clippy::single_match_else)]
#![allow(clippy::too_long_first_doc_paragraph)]
#![allow(clippy::manual_let_else)]
#![allow(clippy::result_large_err)]
#![allow(clippy::needless_pass_by_value)]
#![allow(clippy::unused_self)]
#![allow(clippy::match_wildcard_for_single_variants)]
//! Transport Layer Crate
//!
//! Provides a clean abstraction for managing per-connection transport tasks
//! with channels for communication between transport and coordinator layers.
//!
//! # Features
//!
//! - `inmem` - Enable in-memory transport using `async_broadcast`
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

#[cfg(feature = "mpsc_utils")]
pub mod mpsc_utils;

#[cfg(feature = "mpsc_utils")]
pub use mpsc_utils::{RecvResult, SendResult, SenderExt, UnboundedReceiverExt, UnboundedSenderExt};

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

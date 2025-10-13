//! Transport Layer Crate
//!
//! Provides a clean abstraction for managing per-connection transport tasks
//! with channels for communication between transport and coordinator layers.

mod error;
mod receiver;
mod traits;

pub use error::TransportError;
pub use receiver::TransportReceiver;
pub use traits::Transport;

#[cfg(feature = "inmem")]
pub mod inmem;

#[cfg(feature = "nats")]
pub mod nats;

// Type aliases to hide the receiver implementation detail
#[cfg(feature = "inmem")]
pub type InMemTransportReceiver<E> = TransportReceiver<E, crate::receiver::InMemReceiver<E>>;

#[cfg(feature = "inmem")]
pub use inmem::InMemTransport;

// #[cfg(all(feature = "nats", not(any(feature = "inmem", feature = "kafka"))))]
// pub use nats::NatsTransport

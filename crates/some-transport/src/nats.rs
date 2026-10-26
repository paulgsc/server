#![cfg(feature = "nats")]

mod jetstream;
mod pool;
mod receiver;
mod transport;

/// The NATS a binary connects to when its `NATS_URL` is unset.
///
/// HOST-SPECIFIC (#406): a NATS that publishes 4222 on the same machine,
/// which is right for a `cargo run` beside the compose stack on the dev host
/// and wrong anywhere else; every container sets `NATS_URL`. This is the one
/// declaration of it. Each binary's clap config names it as its default, and
/// this crate never reads the environment itself (#484).
pub const DEV_HOST_NATS_URL: &str = "nats://localhost:4222";

pub use jetstream::{AckHandle, DurableConsumer, JetStreamConfig, JetStreamPublisher};
pub use pool::NatsConnectionPool;
pub use receiver::NatsReceiver;
pub use transport::NatsTransport;

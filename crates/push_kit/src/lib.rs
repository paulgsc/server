//! Web Push actuation, and nothing else.
//!
//! This crate knows how to encrypt a payload for a browser and prove who sent
//! it. It does not know what a study session is, what decided to send, or what
//! HTTP framework is hosting the routes that collect subscriptions — those are
//! three separate concerns and they are three separate crates.
//!
//! ## The seam
//!
//! [`Sender`] turns `(subscription, bytes)` into a [`PreparedPush`]: a URL, a
//! handful of headers, and an opaque body. Putting that on the wire is a
//! [`PushTransport`], which is a **generic parameter, never a trait object** —
//! the transport in a given binary is known at compile time, and paying for a
//! vtable to reach one implementation is paying for nothing.
//!
//! ```ignore
//! let sender = Sender::new(identity, ReqwestTransport::default());
//! match sender.deliver(&subscription, payload_bytes).await {
//!     SendOutcome::Expired => repo.delete(&subscription.endpoint).await?,
//!     outcome => repo.record(&subscription.endpoint, outcome).await?,
//! }
//! ```
//!
//! ## What is deliberately absent
//!
//! **The payload shape.** `send` takes `&[u8]`. `{ title, body, url, tag }` is
//! a contract with one specific service worker, which makes it domain
//! vocabulary; it lives with the domain. A crate that knew that shape could not
//! be reused by anything that notifies about something else.
//!
//! **Any notion of *when*.** Nothing here schedules, throttles, or decides.
//! `deliver` sends when told to. Deciding is `intervention`'s job.

pub mod identity;
pub mod outcome;
pub mod sender;
pub mod subscription;
pub mod transport;

pub use identity::{VapidError, VapidIdentity};
pub use outcome::SendOutcome;
pub use sender::{PreparedPush, Sender, MAX_PAYLOAD_BYTES};
pub use subscription::{PushSubscription, SubscriptionKeys};
pub use transport::{PushResponse, PushTransport};

#[cfg(feature = "reqwest-transport")]
pub use transport::reqwest_transport::ReqwestTransport;

//! Where push subscriptions live.
//!
//! Storage for the actuation layer, and nothing else. The subscription *model*
//! and everything that knows how to encrypt for it are in `push_kit`; this
//! crate persists them, records what the push service said, and enforces the
//! one rule that cannot be enforced anywhere else: **a row exists only because
//! someone was asked and agreed.**
//!
//! Layout and `SQLite` habits follow `capture_repo`.

pub mod repository;

pub use repository::{Consent, PushSubscriptionRepository, StoredSubscription, Topic};

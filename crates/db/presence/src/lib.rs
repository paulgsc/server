//! Durable presence leases.
//!
//! Storage only, and deliberately thin: a lease is `(subject_id, context_key,
//! observed_at)`, and this crate has no opinion about what a context is, how
//! long a lease stays fresh, or who is allowed to write one. It stores the
//! triple and answers "what has this subject observed", and nothing else.
//!
//! In particular, freshness is not this crate's question. `observed_at` is
//! stored as-is; deciding whether it is still fresh enough to matter is the
//! caller's job, computed at read time against whatever `now` and `ttl` the
//! caller is holding. That keeps this crate's one table free of any column
//! that would need a background job to keep honest.

pub mod repository;

pub use repository::{PresenceLeaseRepository, PresenceLeaseRow};

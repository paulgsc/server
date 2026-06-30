//! `some-cache` — shared Redis caching contract for all bins.
//!
//! ## Why this crate exists
//!
//! The axum server and the pipeline daemon each connect to the same Redis
//! instance.  The axum handler writes `capture:session:*` keys; the pipeline
//! daemon reads them.  When each bin owned its own Redis client and
//! serialization logic the formats diverged — the axum store wraps values in
//! a gzip-compressed `CacheEntry<T>` envelope while the pipeline was reading
//! raw bytes — producing silent deserialization failures at runtime.
//!
//! This crate is the single source of truth for:
//!
//! * `CacheEntry<T>` — the on-wire envelope stored in Redis.
//! * `CacheStore`    — generic get/set/delete with retry + compression.
//! * `DedupCache`    — in-process dedup layer over `CacheStore`.
//! * `CacheConfig`   — construction parameters (no bin-specific deps).
//! * `CacheError` / `DedupCacheError` — error types without axum/SDK deps.
//! * Prometheus metrics and recording macros.
//!
//! ## Bin responsibilities
//!
//! Each bin is still responsible for:
//!
//! * Constructing `CacheConfig` from its own config type (via a local `From`
//!   impl — not defined here to avoid circular deps).
//! * Defining its own richer error enum and adding `#[from] DedupCacheError`
//!   (axum) or `#[from] CacheError` (pipeline) as appropriate.
//! * Domain-specific Redis operations that don't belong in a generic cache
//!   (e.g. the pipeline's `write_artifact` / `push_dlq`).

pub mod config;
pub mod dedup;
pub mod entry;
pub mod error;
pub mod inproc;
pub mod metrics;
pub mod store;
pub mod stream;

pub use config::CacheConfig;
pub use dedup::DedupCache;
pub use entry::CacheEntry;
pub use error::{CacheError, DedupCacheError};
pub use store::CacheStore;
pub use stream::StreamHandle;

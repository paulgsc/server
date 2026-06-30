#![allow(clippy::future_not_send)]
#![allow(clippy::disallowed_methods)]
#![allow(clippy::disallowed_macros)]
#![allow(clippy::type_complexity)]
#![allow(clippy::unused_self)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_lossless)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::manual_let_else)]
#![allow(clippy::non_std_lazy_statics)]
#![allow(clippy::must_use_candidate)]
#![allow(clippy::return_self_not_must_use)]
#![allow(clippy::option_if_let_else)]
#![allow(clippy::multiple_crate_versions)]
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::missing_panics_doc)]
#![allow(clippy::doc_markdown)]
#![allow(clippy::missing_const_for_fn)]
#![allow(clippy::result_large_err)]
#![allow(clippy::needless_pass_by_value)]
#![allow(clippy::single_match)]
#![allow(clippy::single_match_else)]
#![allow(clippy::ignored_unit_patterns)]
#![allow(clippy::match_wildcard_for_single_variants)]
#![allow(clippy::too_long_first_doc_paragraph)]
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
pub mod metrics;
pub mod store;
pub mod stream;

pub use config::CacheConfig;
pub use dedup::DedupCache;
pub use entry::CacheEntry;
pub use error::{CacheError, DedupCacheError};
pub use store::CacheStore;
pub use stream::StreamHandle;

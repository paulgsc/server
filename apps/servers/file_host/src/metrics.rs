// #226 (C1): these were all live already but sat behind `#[allow(dead_code)]`
// dating from before #213 gave each one a real call site — see
// `websocket/connection/instrument.rs`'s header for the sibling module that
// had the opposite problem (declared, never called). Removed here so the
// lint gate covers this module again.
pub mod build_info;
pub mod http;
pub mod instruments;
pub mod observability;
pub mod periodic;
pub mod pool;
pub mod rate_limit;
pub mod refusals;

#[allow(unused_imports)]
pub use observability::{ObservabilityError, OtelGuard};

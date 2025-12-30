#[allow(dead_code)]
pub mod observability;
#[allow(dead_code)]
pub mod otel;

#[allow(unused_imports)]
pub use observability::{ObservabilityError, OtelGuard};

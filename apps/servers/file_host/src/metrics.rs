#[allow(dead_code)]
pub mod build_info;
#[allow(dead_code)]
pub mod http;
#[allow(dead_code)]
pub mod instruments;
#[allow(dead_code)]
pub mod observability;
#[allow(dead_code)]
pub mod refusals;

#[allow(unused_imports)]
pub use observability::{ObservabilityError, OtelGuard};

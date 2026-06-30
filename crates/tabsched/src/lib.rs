#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_possible_wrap)]
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::doc_markdown)]
#![allow(clippy::expect_used)]
#![allow(clippy::map_unwrap_or)]
#![allow(clippy::missing_const_for_fn)]
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::missing_panics_doc)]
#![allow(clippy::multiple_crate_versions)]
#![allow(clippy::must_use_candidate)]
#![allow(clippy::option_if_let_else)]
#![allow(clippy::redundant_closure_for_method_calls)]
#![allow(clippy::redundant_pub_crate)]
#![allow(clippy::return_self_not_must_use)]
#![allow(clippy::uninlined_format_args)]
#![allow(clippy::use_self)]
pub mod adapters;
mod core;
mod domain;
mod runtime;

/// Re-export the minimal surface needed by external callers (CLI, tests).
///
/// Everything else is accessible via the module path for callers that
/// need more granular access.
pub use core::{
	state::{apply, next_session, record_outcome, State},
	window::SlidingWindow,
};
pub use domain::{
	ids::{ResourceId, TrackId},
	resource::Resource,
	session::{Outcome, Session},
	topology::Topology,
	track::Track,
};
pub use runtime::engine::Engine;

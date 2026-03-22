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

#![allow(clippy::similar_names)]
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::missing_panics_doc)]
#![allow(clippy::must_use_candidate)]
#![allow(clippy::return_self_not_must_use)]
#![allow(clippy::ignored_unit_patterns)]
#![allow(clippy::missing_const_for_fn)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_lossless)]
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::new_without_default)]
#![allow(clippy::multiple_crate_versions)]
#![allow(clippy::derivable_impls)]
#![allow(clippy::use_self)]
#![allow(clippy::disallowed_macros)]
#![allow(clippy::disallowed_methods)]
#![allow(clippy::doc_markdown)]
#![allow(clippy::match_wildcard_for_single_variants)]
#![allow(clippy::option_if_let_else)]
#![allow(clippy::single_match)]
#![allow(clippy::single_match_else)]
#![allow(clippy::map_unwrap_or)]
#![allow(clippy::uninlined_format_args)]
pub mod actor;
pub mod core;
pub mod errors;
pub mod types;

pub use actor::{ConnectionHandle, ConnectionState};
pub use core::conn::Connection;
pub use core::store::ConnectionStore;
pub use core::subscription::{EventKey, SubscriptionManager};
pub use types::{ClientId, ConnectionId};

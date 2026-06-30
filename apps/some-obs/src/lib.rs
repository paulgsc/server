#![allow(clippy::ignored_unit_patterns)]
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::missing_panics_doc)]
#![allow(clippy::multiple_crate_versions)]
#![allow(clippy::must_use_candidate)]
#![allow(clippy::needless_borrow)]
#![allow(clippy::needless_continue)]
#![allow(clippy::result_large_err)]
#![allow(clippy::use_self)]
pub mod config;
pub mod error;
pub mod service;

pub use config::Config;
pub use error::{Error, Result};
pub use service::ObsNatsService;

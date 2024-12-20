pub mod error;

pub use error::{Error, ResultExt};

pub type Result<T, E = Error> = std::result::Result<T, E>;

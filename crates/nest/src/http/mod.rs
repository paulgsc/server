pub mod handlers;
pub mod schema;

pub mod serve;

pub mod error;

pub mod routes;


pub use error::{Error, ResultExt};

pub type Result<T, E = Error> = std::result::Result<T, E>;

mod handlers;
mod schema;

mod serve;

mod error;

mod routes;


pub use error::{Error, ResultExt};

pub type Result<T, E = Error> = std::result::Result<T, E>;

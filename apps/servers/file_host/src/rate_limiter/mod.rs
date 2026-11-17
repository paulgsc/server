pub mod middleware;

pub use middleware::{rate_limit_middleware, HttpLimiter};

use some_services::rate_limiter::Quota;
use std::num::NonZeroU32;
use std::time::Duration;

/// The quota each client gets for `RATE_LIMIT` requests a minute: all of
/// them at once, refilling one each `60 s / rate_limit`. The one place it is
/// built, so what the limiter enforces and what `GET /compat` tells a client
/// (`rate_limit`) cannot differ. `None` for a limit of 0.
#[must_use]
pub fn quota(rate_limit: u32) -> Option<Quota> {
	NonZeroU32::new(rate_limit).map(|limit| Quota::per_period(limit, Duration::from_secs(60)))
}

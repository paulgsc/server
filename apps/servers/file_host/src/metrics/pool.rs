//! `sqlite_pool_size` / `sqlite_pool_idle` — see #228 (C3).
//!
//! The shared SQLite pool (`main.rs`) is `max_connections(5)`, split across
//! six repository crates, with a 5-second `busy_timeout`. Pool exhaustion is
//! reachable — #203 documents a repository taking a second connection while
//! holding a write transaction against that same pool of five — and today
//! it is observable only as requests mysteriously taking five seconds and
//! then failing. `sqlx::Pool::size()`/`num_idle()` are already tracked by
//! the pool itself, so this needs no instrumentation of any individual
//! query: idle hitting zero while size sits at the configured max is
//! visible before the `busy_timeout` turns it into a user-visible failure.

use metrics::gauge;
use sqlx::SqlitePool;

pub fn record(pool: &SqlitePool) {
	gauge!("sqlite_pool_size").set(f64::from(pool.size()));
	#[allow(clippy::cast_precision_loss)]
	gauge!("sqlite_pool_idle").set(pool.num_idle() as f64);
}

//! `service_info` and `process_start_time_seconds` — see #213 (F4).
//!
//! The build identity that makes every other panel trustworthy: when a
//! panel goes red, "how long has it been like that" and "is this the build
//! I think it is" are the first two questions, and neither is answerable
//! from `env!("CARGO_PKG_VERSION")` alone — every crate in this workspace
//! pins `version = "0.0.0"` (`Cargo.toml`'s `[workspace.package]`), so that
//! string has never changed and never will. `GIT_SHA` (stamped by
//! `build.rs`) is the identifier that actually distinguishes one deployed
//! binary from the next.

use metrics::gauge;
use std::time::{SystemTime, UNIX_EPOCH};

pub const SERVICE: &str = "file_host";
pub const GIT_SHA: &str = env!("GIT_SHA");
pub const GIT_DIRTY: &str = env!("GIT_DIRTY");
pub const BUILT_AT: &str = env!("BUILD_TIMESTAMP");
pub const RUSTC_VERSION: &str = env!("RUSTC_VERSION");

/// `git_sha`, with a `-dirty` suffix when appropriate.
///
/// Set when `build.rs` ran against a working tree that had uncommitted
/// changes — the same convention `git describe --dirty` uses, so it reads
/// the same way in a log line as on the dashboard.
#[must_use]
pub fn git_sha_with_dirty_marker() -> String {
	if GIT_DIRTY == "dirty" {
		GIT_SHA.to_string() + "-dirty"
	} else {
		GIT_SHA.to_string()
	}
}

/// Call once at startup, after `some_metrics::install_with` — mirrors that
/// function's own one-call-per-process contract.
///
/// Emits `service_info{service,version,git_sha,built_at,rust}` fixed at `1`
/// (the conventional "info gauge" shape: the interesting data lives in the
/// labels, not the value) and `process_start_time_seconds` set once to the
/// startup instant, so uptime is `time() - process_start_time_seconds` and a
/// restart is a visible discontinuity rather than an inference from a gap in
/// an unrelated series.
pub fn record() {
	gauge!(
		"service_info",
		"service" => SERVICE,
		"version" => env!("CARGO_PKG_VERSION"),
		"git_sha" => git_sha_with_dirty_marker(),
		"built_at" => BUILT_AT,
		"rust" => RUSTC_VERSION,
	)
	.set(1.0);

	let start = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0);
	gauge!("process_start_time_seconds").set(start);
}

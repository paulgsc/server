//! Stamps build identity into compile-time env vars — see #213 (F4).
//!
//! `handlers::readiness`/`metrics::instruments` read these back via
//! `env!(...)`. The point: `docker compose up -d` after a code change that
//! failed to rebuild leaves the old image running, and the dashboard looks
//! identical either way — the metric you just added being absent reads as
//! "the feature isn't firing" rather than "you're looking at yesterday's
//! binary". `service_info`'s `git_sha` label is what tells them apart.
//!
//! Prefers `VCS_REF`/`GIT_DIRTY`/`BUILD_DATE` env vars when set — that's the
//! path a Docker build takes, since `.git` isn't copied into any of this
//! workspace's build contexts (see `infra/docker/Dockerfile.server`) and
//! `git` genuinely isn't answerable in there. Falls back to running `git`
//! directly, which is what a local `cargo build`/`cargo test` gets: `.git`
//! is right there, and CI's `actions/checkout` provides it too. Either way,
//! absence is `"unknown"`, never a build failure — a missing identity
//! string is not a reason to refuse to compile the binary that would
//! report it.

// A build script talks to cargo over stdout by protocol — `cargo:rustc-env=…`
// lines are how this file's whole job gets done, not a logging shortcut the
// workspace's tracing-only convention has an alternative for.
#![allow(clippy::disallowed_macros)]

use std::env;
use std::process::Command;

fn main() {
	println!("cargo:rerun-if-changed=migrations");
	println!("cargo:rerun-if-changed=../../../.git/HEAD");
	println!("cargo:rerun-if-env-changed=VCS_REF");
	println!("cargo:rerun-if-env-changed=GIT_DIRTY");
	println!("cargo:rerun-if-env-changed=BUILD_DATE");

	// `VCS_REF`/`BUILD_DATE`: the same names `infra/docker/Dockerfile.server`
	// already declared as `ARG`s for its OCI `LABEL`s (previously wired to
	// the runtime stage only, so those labels were always literally
	// "unknown" — see `.github/actions/docker-build/action.yml`, which now
	// passes them as build args reaching this stage too). Reusing the names
	// means one git-derived value backs both the image's own metadata and
	// what the running binary reports about itself, rather than two
	// independent, driftable sources for the same fact.
	let git_sha = env::var("VCS_REF").ok().filter(|v| !v.is_empty() && v != "unknown").unwrap_or_else(|| git(&["rev-parse", "--short=12", "HEAD"]).unwrap_or_else(|| "unknown".to_string()));

	let dirty = env::var("GIT_DIRTY").ok().filter(|v| !v.is_empty()).unwrap_or_else(|| {
		if git(&["status", "--porcelain"]).is_some_and(|s| !s.trim().is_empty()) {
			"dirty".to_string()
		} else {
			"clean".to_string()
		}
	});

	let built_at = env::var("BUILD_DATE").ok().filter(|v| !v.is_empty() && v != "unknown").unwrap_or_else(|| {
		// `SOURCE_DATE_EPOCH`-independent: a build timestamp, unlike a git
		// SHA, has no reproducible-builds convention to defer to here, and
		// "when this binary was actually compiled" is exactly what F4 wants.
		humantime_seconds_now()
	});

	let rustc_version = Command::new(env::var("RUSTC").unwrap_or_else(|_| "rustc".to_string()))
		.arg("--version")
		.output()
		.ok()
		.filter(|o| o.status.success())
		.and_then(|o| String::from_utf8(o.stdout).ok())
		.map_or_else(|| "unknown".to_string(), |s| s.trim().to_string());

	println!("cargo:rustc-env=GIT_SHA={git_sha}");
	println!("cargo:rustc-env=GIT_DIRTY={dirty}");
	println!("cargo:rustc-env=BUILD_TIMESTAMP={built_at}");
	println!("cargo:rustc-env=RUSTC_VERSION={rustc_version}");
}

fn git(args: &[&str]) -> Option<String> {
	let output = Command::new("git").args(args).output().ok()?;
	if !output.status.success() {
		return None;
	}
	let text = String::from_utf8(output.stdout).ok()?;
	let trimmed = text.trim();
	(!trimmed.is_empty()).then(|| trimmed.to_string())
}

/// An RFC 3339 UTC timestamp without pulling in a `chrono`/`time` build
/// dependency just for this — `build.rs` runs on the host doing the
/// compiling, so `SystemTime::now()` plus fixed civil-calendar arithmetic is
/// enough; leap seconds are not a concern at "which day was this built" a
/// resolution.
///
/// `days as i64`: `days` is `secs / 86_400` off the current wall clock —
/// nowhere near `i64::MAX` days since the epoch, so this cannot wrap in
/// practice; see `civil_from_days`'s own doc comment for the same allowance.
#[allow(clippy::cast_possible_wrap)]
fn humantime_seconds_now() -> String {
	let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);

	let days = secs / 86_400;
	let time_of_day = secs % 86_400;
	let (hour, minute, second) = (time_of_day / 3600, (time_of_day % 3600) / 60, time_of_day % 60);

	let (year, month, day) = civil_from_days(days as i64);
	format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

/// Howard Hinnant's `civil_from_days`: days-since-epoch to a proleptic
/// Gregorian (year, month, day), valid over the entire range `i64` can
/// represent. <https://howardhinnant.github.io/date_algorithms.html>
///
/// The `i64`/`u64`/`u32` casts throughout are the algorithm's own —
/// `doe`/`yoe`/`doy`/`mp` are always non-negative by construction (each is a
/// remainder or difference bounded within one 400-year era), and `d`/`m` are
/// calendar day-of-month/month numbers, always small. None of clippy's
/// pedantic cast lints can see those invariants, so they're allowed here
/// rather than restructured with fallible `try_from` a build script's
/// bounded, always-in-range date math doesn't need.
#[allow(clippy::cast_possible_wrap, clippy::cast_sign_loss, clippy::cast_possible_truncation)]
const fn civil_from_days(z: i64) -> (i64, u32, u32) {
	let z = z + 719_468;
	let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
	let doe = (z - era * 146_097) as u64;
	let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
	let y = yoe as i64 + era * 400;
	let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
	let mp = (5 * doy + 2) / 153;
	let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
	let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
	(if m <= 2 { y + 1 } else { y }, m, d)
}

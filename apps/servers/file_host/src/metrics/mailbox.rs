//! Liveness and work metrics for the Jobs mailbox poller (`crate::mailbox`).
//!
//! Shaped like `metrics::waker`, for the same reason: a pass that finds
//! nothing new is the healthy case, so "nothing kept" must not read the same
//! as "nothing running". `mailbox_poll_last_attempt_timestamp_seconds` moves
//! when a pass starts, so stale means the task died or a pass hangs;
//! `mailbox_poll_failing{error}` says the last pass ended early, and why.
//! Both are read by the MAILBOX dashboard (`infra/grafana/dashboards/mailbox.jsonnet`).

use crate::mailbox::pass::Outcome;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Every value `mailbox_poll_failing`'s `error` label can take.
///
/// A `mailbox_kit::MailboxError::kind` that ends a pass, `storage` for the
/// database, or `no_account` when `MAILBOX_SUBJECT` names no account. Fixed,
/// so every series exists from spawn onward and a class that never failed
/// reads 0 rather than absent.
pub const PASS_ERROR_CLASSES: [&str; 10] = [
	"unauthorized",
	"forbidden",
	"not_found",
	"refused",
	"too_big",
	"http",
	"transport",
	"decode",
	"storage",
	"no_account",
];

/// Every value `mailbox_poll_items_refused_total`'s `why` label can take: see
/// `mailbox::pass::Outcome::refused`.
pub const REFUSALS: [&str; 4] = ["mismatch", "bad_name", "not_json", "rewritten"];

/// Every value `mailbox_poll_items_handed_total`'s `answer` label can take:
/// `mailbox_repo::Answer`.
pub const ANSWERS: [&str; 4] = ["added", "unchanged", "conflict", "refused"];

/// Whether this deployment runs the poller: 1 or 0, set once at boot on both
/// branches of `main.rs`'s mailbox gate, so "switched off" and "task missing"
/// read apart.
pub fn record_enabled(enabled: bool) {
	metrics::gauge!("mailbox_poll_enabled").set(if enabled { 1.0 } else { 0.0 });
}

/// Publish the configured interval as soon as the task is spawned, and start
/// every failure class and refusal at 0.
pub fn record_interval(interval: Duration) {
	metrics::gauge!("mailbox_poll_interval_seconds").set(interval.as_secs_f64());
	set_failing(None);
	for why in REFUSALS {
		metrics::counter!("mailbox_poll_items_refused_total", "why" => why).increment(0);
	}
	metrics::counter!("mailbox_poll_items_kept_total").increment(0);
	for answer in ANSWERS {
		metrics::counter!("mailbox_poll_items_handed_total", "answer" => answer).increment(0);
	}
}

/// Mark the start of a pass.
pub fn record_pass_started() {
	let timestamp = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0.0, |elapsed| elapsed.as_secs_f64());
	metrics::gauge!("mailbox_poll_last_attempt_timestamp_seconds").set(timestamp);
}

/// How long the last pass took, including its requests to the mailbox.
pub fn record_pass_duration(duration: Duration) {
	metrics::gauge!("mailbox_poll_last_pass_duration_seconds").set(duration.as_secs_f64());
}

/// What a pass did, and the class it failed in, if it did. A failure stays
/// set until the next pass succeeds.
pub fn record_pass(outcome: &Outcome, failed: Option<&'static str>) {
	metrics::counter!("mailbox_poll_items_kept_total").increment(outcome.kept as u64);
	for why in &outcome.refused {
		metrics::counter!("mailbox_poll_items_refused_total", "why" => *why).increment(1);
	}
	for answer in &outcome.handed {
		metrics::counter!("mailbox_poll_items_handed_total", "answer" => *answer).increment(1);
	}
	// A subject with no account keeps nothing, pass after pass: the
	// configured `MAILBOX_SUBJECT` names nobody. Read as a failure.
	set_failing(failed.or(if outcome.no_account { Some("no_account") } else { None }));
}

fn set_failing(class: Option<&'static str>) {
	for candidate in PASS_ERROR_CLASSES {
		metrics::gauge!("mailbox_poll_failing", "error" => candidate).set(if Some(candidate) == class { 1.0 } else { 0.0 });
	}
}

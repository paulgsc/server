//! Liveness and work metrics for the Jobs mailbox poller (`crate::mailbox`).
//!
//! Shaped like `metrics::waker`, for the same reason: a pass that finds
//! nothing new is the healthy case, so "nothing kept" must not read the same
//! as "nothing running". `mailbox_poll_last_attempt_timestamp_seconds` moves
//! when a pass starts, so stale means the task died or a pass hangs;
//! `mailbox_poll_failing{error}` says the last pass ended early, and every
//! reason it did: one series at 1 per failure class, so a second failing
//! phase is not hidden behind the first.
//! Both are read by the MAILBOX dashboard (`infra/grafana/dashboards/mailbox.jsonnet`).

use crate::mailbox::pass::Outcome;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Every value `mailbox_poll_failing`'s `error` label can take.
///
/// A `mailbox_kit::MailboxError::kind` that ends a pass, `storage` for the
/// database, or `no_account` when there is no account to keep the files
/// for: `MAILBOX_SUBJECT` names none, or it is unset and the box has no
/// account yet, or several (`mailbox::pass_subject`). Fixed,
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
pub const REFUSALS: [&str; 7] = ["mismatch", "bad_name", "not_json", "rewritten", "unreadable", "no_file", "too_big"];

/// Every value `mailbox_poll_items_handed_total`'s `answer` label can take:
/// `mailbox_repo::Answer`.
pub const ANSWERS: [&str; 4] = ["added", "unchanged", "conflict", "refused"];

/// Every value `mailbox_poll_items_archived_total`'s `answer` label can take.
///
/// Archived (or already), or why the file stayed live: its bytes were not
/// what the listing said (`mismatch`), its name could not be asked for
/// (`bad_name`), home holds other bytes under it (`rewritten`), it is a
/// digest home has not kept for the phone (`unread`), the mailbox refused it
/// as too young (`refused`), or it was gone (`gone`).
pub const ARCHIVE_ANSWERS: [&str; 8] = ["archived", "unchanged", "mismatch", "bad_name", "rewritten", "unread", "refused", "gone"];

/// Every value `mailbox_poll_items_restored_total`'s `answer` label can take.
///
/// Put back (or already live), the mailbox archived other bytes
/// (`conflict`), it refused them (`refused`), or home holds nothing for the
/// ask (`missing`).
pub const RESTORE_ANSWERS: [&str; 5] = ["restored", "unchanged", "conflict", "refused", "missing"];

/// Whether this deployment runs the poller, and whether it archives: 1 or 0
/// each, set once at boot on both branches of `main.rs`'s mailbox gate, so
/// "switched off" and "task missing" read apart.
pub fn record_enabled(enabled: bool, archiving: bool) {
	metrics::gauge!("mailbox_poll_enabled").set(if enabled { 1.0 } else { 0.0 });
	metrics::gauge!("mailbox_archive_enabled").set(if archiving { 1.0 } else { 0.0 });
}

/// Publish the configured interval as soon as the task is spawned, and start
/// every failure class and refusal at 0.
pub fn record_interval(interval: Duration) {
	metrics::gauge!("mailbox_poll_interval_seconds").set(interval.as_secs_f64());
	set_failing(&[]);
	for why in REFUSALS {
		metrics::counter!("mailbox_poll_items_refused_total", "why" => why).increment(0);
	}
	metrics::counter!("mailbox_poll_items_kept_total").increment(0);
	for answer in ANSWERS {
		metrics::counter!("mailbox_poll_items_handed_total", "answer" => answer).increment(0);
	}
	for answer in ARCHIVE_ANSWERS {
		metrics::counter!("mailbox_poll_items_archived_total", "answer" => answer).increment(0);
	}
	for answer in RESTORE_ANSWERS {
		metrics::counter!("mailbox_poll_items_restored_total", "answer" => answer).increment(0);
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

/// What a pass did, and every class it failed in (none when it succeeded). A
/// failure stays set until the next pass succeeds.
pub fn record_pass(outcome: &Outcome, failed: &[&'static str]) {
	metrics::counter!("mailbox_poll_items_kept_total").increment(outcome.kept as u64);
	for why in &outcome.refused {
		metrics::counter!("mailbox_poll_items_refused_total", "why" => *why).increment(1);
	}
	for answer in outcome.handed.iter().chain(&outcome.indexed) {
		metrics::counter!("mailbox_poll_items_handed_total", "answer" => *answer).increment(1);
	}
	for answer in &outcome.archived {
		metrics::counter!("mailbox_poll_items_archived_total", "answer" => *answer).increment(1);
	}
	for answer in &outcome.restored {
		metrics::counter!("mailbox_poll_items_restored_total", "answer" => *answer).increment(1);
	}
	// A pass with no account keeps nothing, pass after pass: `MAILBOX_SUBJECT`
	// names nobody, or it is unset and the box has no account yet or several.
	// Read as a failure.
	let no_account: &[&'static str] = if outcome.no_account { &["no_account"] } else { &[] };
	set_failing(&[failed, no_account].concat());
}

fn set_failing(classes: &[&'static str]) {
	for candidate in PASS_ERROR_CLASSES {
		metrics::gauge!("mailbox_poll_failing", "error" => candidate).set(if classes.contains(&candidate) { 1.0 } else { 0.0 });
	}
}

/// Every value `mailbox_bell_rings_total`'s `folder` label can take: the
/// folders a pass reads (`mailbox::pass::READS`), and `other` for a ring
/// naming any other, so a mailbox can't grow the label.
pub const RING_FOLDERS: [&str; 6] = ["brief", "scene", "drama", "run", "leetype", "other"];

/// The bell's task has started: it is not connected yet, and every failure
/// kind, ring folder and pass cause starts at 0.
pub fn record_bell_started() {
	record_bell_connected(false);
	for why in crate::mailbox::bell::FAILURES {
		metrics::counter!("mailbox_bell_failures_total", "why" => why).increment(0);
	}
	for folder in RING_FOLDERS {
		metrics::counter!("mailbox_bell_rings_total", "folder" => folder).increment(0);
	}
	for cause in crate::mailbox::bell::Cause::ALL {
		metrics::counter!("mailbox_passes_total", "cause" => cause.name()).increment(0);
	}
}

/// Whether home holds a connection to the mailbox's bell: 1 or 0. At 0 every
/// file still arrives, by the backstop timer.
pub fn record_bell_connected(connected: bool) {
	metrics::gauge!("mailbox_bell_connected").set(if connected { 1.0 } else { 0.0 });
}

/// A connection to the bell failed or ended, by kind
/// (`mailbox::bell::FAILURES`).
pub fn record_bell_failure(why: &'static str) {
	metrics::counter!("mailbox_bell_failures_total", "why" => why).increment(1);
}

/// The bell rang for `folder`.
pub fn record_ring(folder: &str) {
	let folder = RING_FOLDERS.into_iter().find(|known| *known == folder).unwrap_or("other");
	metrics::counter!("mailbox_bell_rings_total", "folder" => folder).increment(1);
}

/// A pass is starting, for `cause`.
pub fn record_pass_cause(cause: crate::mailbox::bell::Cause) {
	metrics::counter!("mailbox_passes_total", "cause" => cause.name()).increment(1);
}

/// The pass loop is alive.
///
/// Stamped every `mailbox::HEARTBEAT` while idle and
/// after each pass, never during one, so a dead task or a hung pass reads as
/// stale in minutes though the backstop runs hourly (`MailboxPollerStale`).
pub fn record_alive() {
	let timestamp = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0.0, |elapsed| elapsed.as_secs_f64());
	metrics::gauge!("mailbox_poll_alive_timestamp_seconds").set(timestamp);
}

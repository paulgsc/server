//! Liveness metrics for the engagement waker (#216/P4, #236).
//!
//! A successful pass can legitimately find no due work and emit no domain
//! events.  These gauges make that quiet success distinguishable from a task
//! which stopped running.  The interval is exported too: the dashboard must
//! compare the pass age with the deployment's actual configuration rather
//! than with the default compiled into `Config`.
//!
//! Two gauges, because "not succeeding" has two causes LOOPS has to tell
//! apart. `nudge_waker_last_attempt_timestamp_seconds` moves when a pass
//! *starts*: stale means the task died or a pass is hung (#264) — nothing is
//! running to fail. `nudge_waker_pass_failing{error}` says the last pass that
//! did finish failed, and in which [`PASS_ERROR_CLASSES`] bucket — the task
//! is alive and something it depends on isn't. A single "last success"
//! timestamp read the same for both, which is how a database missing a
//! migration once looked identical to a dead loop.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Every value `nudge_waker_pass_failing`'s `error` label can take.
///
/// See `nudge::waker::classify_pass_error` for what lands in each. Fixed, so
/// every series exists from spawn onward: a class that has never failed
/// reads 0 rather than absent, which SIGNAL would otherwise call blind.
pub const PASS_ERROR_CLASSES: [&str; 5] = ["schema", "locked", "io", "pool", "other"];

/// Publish the configured interval as soon as the task is spawned, and
/// start every failure class at 0.
pub fn record_interval(interval: Duration) {
	metrics::gauge!("nudge_waker_interval_seconds").set(interval.as_secs_f64());
	set_failing(None);
}

/// Mark the start of a pass.
pub fn record_pass_started() {
	let timestamp = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0.0, |elapsed| elapsed.as_secs_f64());
	metrics::gauge!("nudge_waker_last_attempt_timestamp_seconds").set(timestamp);
}

/// Mark a completed, successful pass, including an empty one.
pub fn record_successful_pass() {
	set_failing(None);
}

/// Mark a pass that returned an error, by class. Stays set until the next
/// pass succeeds — the dashboard reads "the last pass failed", not a rate.
pub fn record_failed_pass(class: &'static str) {
	set_failing(Some(class));
}

fn set_failing(class: Option<&'static str>) {
	for candidate in PASS_ERROR_CLASSES {
		metrics::gauge!("nudge_waker_pass_failing", "error" => candidate).set(if Some(candidate) == class { 1.0 } else { 0.0 });
	}
}

/// The terminal outcome one due subject reached this pass, and — for
/// `verdict == "suppressed"` only — which `nudge::constraints::Suppressed`
/// reason it was. `reason` is `"n/a"` for every other verdict so the label
/// set stays fixed regardless of which branch fired, rather than growing an
/// optional label Prometheus would show as absent on some series and
/// present on others.
///
/// This is the gap LOOPS (`nudge_waker_last_attempt_timestamp_seconds`) cannot
/// close: a tick that runs on schedule and a tick that finds every due
/// subject suppressed, or claims one and then has every device reject the
/// push, both leave LOOPS green. Feeds the NUDGE row's outcomes panel
/// (`infra/grafana/dashboards/lib/nudge-panels.libsonnet`) — see that
/// file's header for why a breakdown panel is the fix rather than a
/// pass/fail invariant: a subject correctly suppressed by quiet hours is
/// not a fault, and only a human reading the shape of the breakdown can
/// tell the two apart.
pub fn record_verdict(verdict: &'static str, reason: &'static str) {
	metrics::counter!("nudge_waker_verdicts_total", "verdict" => verdict, "reason" => reason).increment(1);
}

/// How many subjects `due` returned this pass, including zero. Paired with
/// `record_verdict`'s `"sent"` outcome so a dashboard can compare them
/// directly: subjects due with nothing landing as `"sent"`, sustained, is
/// exactly the silence this feature exists to prevent for whoever needs it
/// most.
pub fn record_due(count: usize) {
	#[allow(clippy::cast_precision_loss)] // a due-subject batch is never near f64's precision limit
	metrics::gauge!("nudge_waker_due_subjects").set(count as f64);
}

/// #262 (SLI1): every row `SessionRepository::list()` hands back, summed
/// across every due subject one pass considers.
///
/// `list()` has no `WHERE`/`LIMIT` — see its own docstring — so this grows
/// with `BATCH × |sessions|` on an accumulated table rather than staying flat
/// as the fleet grows. This counter is the measurement half of that story:
/// it makes the cost visible on a dashboard the same way the characterisation
/// test in `nudge::waker`'s test module makes it visible in CI. Fixing the
/// read pattern is #263 (SLI2); until then this counter is expected to climb
/// with usage, and #263's job is to make it stop.
pub fn record_session_rows_read(count: usize) {
	#[allow(clippy::cast_possible_truncation)] // a session table is never near u64::MAX rows
	metrics::counter!("nudge_waker_session_rows_read_total").increment(count as u64);
}

/// #285 (RCM8): every pass that ends with a subject the arithmetic said to
/// interrupt and nothing to say to them.
///
/// Deliberately its own series rather than only a label on
/// `nudge_waker_verdicts_total`. That counter is a *breakdown* — "which of
/// the terminal outcomes did each due subject reach this pass" — and every
/// one of its other values is an ordinary thing for a healthy deployment to
/// do. This one is not: after `#257`'s cold-start epic, reaching it means the
/// catalogue produced nothing to propose (a content or deployment bug), or a
/// class was added to `EngagementClass` without a matching `StudySelector`
/// arm (a build-time one). A non-zero value is a bug, not a quiet day — which
/// is a sentence an alert rule can be written against without depending on a
/// label value, and which `docs/study-nudge.md` says in exactly those words.
///
/// Exempted in `scripts/check_metric_contract.py`'s `EXEMPT_EMITTED` rather
/// than given a Grafana panel, with the reason written out there: its healthy
/// value is zero forever, and a panel whose healthy state is a flat line at
/// zero is the "detector that always reports fine" that check exists to
/// prevent. The conditions behind it are already on the NUDGE row's outcomes
/// breakdown by label; what this name adds is something to alert on.
///
/// Not incremented for a `Wait`, a `Suppressed`, or any pass where an
/// invitation still went out: a subject who got `GetStarted` because the
/// catalogue was empty was told *something*, and the failed proposal behind
/// it is already in the logs and in `verdict="storage_error"`/
/// `"nothing_to_provision"`.
pub fn record_nothing_to_say() {
	metrics::counter!("nudge_waker_nothing_to_say_total").increment(1);
}

/// #264 (SLI3): publish the configured pass deadline once, at spawn.
///
/// The dashboard draws it beside [`record_pass_duration`] — the same reason
/// `record_interval` exports the interval rather than letting a panel assume
/// `Config`'s default.
pub fn record_pass_deadline(deadline: Duration) {
	metrics::gauge!("nudge_waker_pass_deadline_seconds").set(deadline.as_secs_f64());
}

/// #264 (SLI3): how long the last pass took.
///
/// Successful or not, empty or not. A gauge rather than a histogram, like
/// the rest of this module: one pass per interval is too few observations
/// for a distribution to say more than "the last one took this long", and
/// read next to `nudge_waker_pass_deadline_seconds` that is the question —
/// is a pass getting close to its bound?
pub fn record_pass_duration(elapsed: Duration) {
	metrics::gauge!("nudge_waker_last_pass_duration_seconds").set(elapsed.as_secs_f64());
}

/// #264 (SLI3): every pass that stopped early because its deadline ran out.
///
/// Healthy value is zero; a pass that hits it has left due
/// subjects for the next pass, which is correct behaviour once and a sign of
/// a stalling push provider (or a `BATCH` the deployment has outgrown) when
/// sustained.
pub fn record_deadline_exceeded() {
	metrics::counter!("nudge_waker_pass_deadline_exceeded_total").increment(1);
}

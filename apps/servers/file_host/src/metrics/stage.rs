//! Every shared stage's series (`crate::stage`, paulgsc/some-ui#3333).
//!
//! One set of names for every stage, told apart by the `stage` label alone,
//! so `StageDegraded`, `StageBroken` and the overview's "Shared stages" row
//! read each stage with the same expression. Labels are a stage's name, its
//! objective's id and how an attempt ended, each from a closed set; never an
//! item or a person.

use crate::stage::{Ended, Stage};

#[allow(clippy::cast_precision_loss)] // counts and Unix seconds, far below 2^52
const fn as_f64(value: i64) -> f64 {
	value as f64
}

/// The stages started: each one's promise, every way an attempt ends at 0
/// (so a stage that has never failed reads 0, not absent), and nothing yet
/// known to be waiting.
pub fn record_started() {
	for stage in Stage::ALL {
		let promise = stage.promise();
		metrics::gauge!("stage_promise_seconds", "stage" => stage.name(), "objective" => promise.objective).set(promise.within.as_secs_f64());
		metrics::gauge!("stage_attempts_per_item", "stage" => stage.name()).set(as_f64(stage.attempts_per_item()));
		for ended in Ended::ALL {
			metrics::counter!("stage_attempts_total", "stage" => stage.name(), "result" => ended.name()).increment(0);
		}
	}
}

/// What a pass found waiting, over the stage's whole queue.
///
/// When the oldest item reached the stage, as ISO-8601 (none waiting reads
/// 0), and how many wait.
/// A time that does not parse reads as now, so it is never older than it is.
pub fn record_waiting(stage: Stage, oldest: Option<&str>, waiting: i64) {
	let since = since(oldest, chrono::Utc::now().timestamp());
	metrics::gauge!("stage_oldest_waiting_since_timestamp_seconds", "stage" => stage.name()).set(as_f64(since));
	metrics::gauge!("stage_waiting", "stage" => stage.name()).set(as_f64(waiting));
}

/// When the oldest item arrived, in Unix seconds: 0 when nothing waits, and
/// `now` when its time does not parse.
fn since(oldest: Option<&str>, now: i64) -> i64 {
	oldest.map_or(0, |at| chrono::DateTime::parse_from_rfc3339(at).map_or(now, |at| at.timestamp()))
}

/// One attempt at an item, and how it ended.
pub fn record_attempt(stage: Stage, ended: Ended) {
	metrics::counter!("stage_attempts_total", "stage" => stage.name(), "result" => ended.name()).increment(1);
}

#[cfg(test)]
mod tests {
	use super::since;

	#[test]
	fn nothing_waiting_is_zero_and_an_unreadable_time_is_now() {
		assert_eq!(since(None, 99), 0);
		assert_eq!(since(Some("2026-11-21T00:00:00.000Z"), 99), 1_795_219_200);
		assert_eq!(since(Some("yesterday"), 99), 99, "never older than it is");
	}
}

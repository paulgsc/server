//! Shared stages: the nodes other pipelines wait on (paulgsc/some-ui#3333;
//! `docs/shared-stages.md` holds the contract's invariants).
//!
//! A shared stage is one piece of home's work that several pipelines join at
//! (the transcriber hears every note: interview answers, reflections, jobs
//! notes and Studio takes alike), so when it is slow or broken everything
//! downstream waits, and nothing downstream says why. Each stage is a case of
//! [`Stage`], and each case declares the same four things, so a new stage is
//! a new case or the matches below don't build:
//!
//! 1. **Its promise** ([`Stage::promise`]): an objective in some-ui's
//!    assurance ledger, in the owner's words, whose bound is a policy
//!    constant here, moved only by a call.
//! 2. **One headline signal**: the age of the oldest item still waiting
//!    ([`crate::metrics::stage::record_waiting`]). It catches stopped,
//!    slow, looping, stuck-on-one-item and backlogged at once.
//! 3. **Two alerts**, the same rules for every stage
//!    (`infra/prometheus/rules/home.yml`, `StageDegraded` and
//!    `StageBroken`): degraded while the oldest waiting is older than the
//!    promise; broken while more than one item's worth of attempts
//!    ([`Stage::attempts_per_item`]) were made in an hour and almost none
//!    ended ([`Ended`]). Each alert takes its `objective` from the promise's own
//!    series (`stage_promise_seconds{stage, objective}`), so no rule names a
//!    stage, and the bound is read from here, never written in YAML.
//! 4. **Who waits on it** ([`Stage::waiters`]): what is blocked downstream,
//!    each a [`Waiter`] with the series that counts it, for the "Shared
//!    stages" row at the top of home's overview dashboard.
//!
//! **Why the age is exported as a time, not a number of seconds.** A stage
//! is measured where its pass reads its queue, which is once a pass, and a
//! broken stage's pass is long: the transcriber's batch is 20 notes, each
//! allowed 600 s, so a pass where every note runs to its deadline lasts over
//! three hours. A seconds gauge set at the pass's start would stand still
//! that whole time, at the one moment it matters. So the stage exports when
//! the oldest item reached it, and Prometheus's recording rule
//! `stage:oldest_waiting:seconds` ages it on every evaluation.
//!
//! **Over every item waiting, not the pass's batch.** A pass may hear its
//! batch in any order (fewest attempts first, say), so the batch's first is
//! not the oldest. Each stage reads its whole queue once a pass for the
//! oldest arrival and the count (for the transcriber,
//! `voice_repo::transcript::backlog`: the queue's own predicate, the same
//! indexed lookups per note that `waiting_first` makes over every candidate
//! before it sorts, so no more work than the queue's own read). Labels are the stage's and waiter's
//! names only, from these closed sets, never an item or a person.
//!
//! **The cases this contract will take next**, declared and not built
//! (paulgsc/some-ui#3333, "Cases"): the render lane (oldest render job
//! waiting, from the render worker's status folder), Kokoro's voicing (oldest
//! line waiting to be voiced) and the mailbox (oldest item the Worker holds
//! that home has not filed). Each is a new case here, with its own promise.

use std::time::Duration;

/// Every shared stage. Closed: a stage added here must say its promise, how
/// its attempts end and who waits on it, or this module does not build.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
	/// Home's transcriber (`crate::transcribe`): every voice note sent home
	/// waits on it before anything reads its words.
	Transcriber,
}

/// A stage's promise: the ledger objective that states it, and its bound.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Promise {
	/// The objective's id in paulgsc/some-ui's
	/// `packages/assurance/src/ledger.json`, the alerts' `objective` label.
	pub objective: &'static str,
	/// The oldest item may wait at most this long, or the stage is degraded.
	pub within: Duration,
}

/// How one attempt at an item ended, as a stage counts it.
///
/// `Refused` is the stage working: the item itself can't be done (audio
/// that doesn't decode, a note past the longest heard), said and kept.
/// `Failed` is the stage not working: it ran out of time, or broke.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ended {
	Done,
	Refused,
	Failed,
}

impl Ended {
	pub const ALL: [Self; 3] = [Self::Done, Self::Refused, Self::Failed];

	/// The `result` label of `stage_attempts_total`.
	#[must_use]
	pub const fn name(self) -> &'static str {
		match self {
			Self::Done => "done",
			Self::Refused => "refused",
			Self::Failed => "failed",
		}
	}
}

/// What waits downstream of a stage, each counted by a series `file_host`
/// exports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Waiter {
	/// Cards essays held back because a take a spoken card names is not
	/// heard yet (`cards_essays_waiting_on_takes`).
	CardsEssays,
	/// The batch lane held for interactive work, by kind
	/// (`lane_busy_ticks_total{kind}`): renders wait while a note is heard.
	RenderLane,
}

impl Waiter {
	/// The series that says how much is waiting.
	#[must_use]
	pub const fn series(self) -> &'static str {
		match self {
			Self::CardsEssays => "cards_essays_waiting_on_takes",
			Self::RenderLane => "lane_busy_ticks_total",
		}
	}
}

impl Stage {
	pub const ALL: [Self; 1] = [Self::Transcriber];

	/// The `stage` label.
	#[must_use]
	pub const fn name(self) -> &'static str {
		match self {
			Self::Transcriber => "transcriber",
		}
	}

	/// What the stage promises the owner.
	#[must_use]
	pub const fn promise(self) -> Promise {
		match self {
			Self::Transcriber => Promise {
				objective: "VH7",
				within: crate::transcribe::HEARD_WITHIN,
			},
		}
	}

	/// The most attempts the stage makes on one item, so `StageBroken` needs
	/// more than one item's worth of failures: one bad item is its own.
	#[must_use]
	pub const fn attempts_per_item(self) -> i64 {
		match self {
			Self::Transcriber => crate::transcribe::pass::ATTEMPTS,
		}
	}

	/// What waits on it.
	#[must_use]
	pub const fn waiters(self) -> &'static [Waiter] {
		match self {
			Self::Transcriber => &[Waiter::CardsEssays, Waiter::RenderLane],
		}
	}
}

#[cfg(test)]
mod tests {
	use super::{Ended, Stage, Waiter};

	const RULES: &str = include_str!("../../../../infra/prometheus/rules/home.yml");
	const TESTS: &str = include_str!("../../../../infra/prometheus/rules/home.test.yml");
	const OVERVIEW: &str = include_str!("../../../../infra/grafana/dashboards/lib/stage-panels.libsonnet");

	/// Every stage is listed once, with a promise the alerts can carry.
	#[test]
	fn every_stage_is_listed_once_with_a_promise() {
		for stage in Stage::ALL {
			match stage {
				Stage::Transcriber => {}
			}
			let promise = stage.promise();
			assert!(promise.within.as_secs() > 0, "{}", stage.name());
			assert!(stage.attempts_per_item() > 0, "{}", stage.name());
			assert!(!promise.objective.is_empty(), "{}", stage.name());
			assert!(!stage.waiters().is_empty(), "{} says who waits on it", stage.name());
		}
		let names: std::collections::BTreeSet<_> = Stage::ALL.iter().map(|stage| stage.name()).collect();
		assert_eq!(names.len(), Stage::ALL.len());
		for ended in Ended::ALL {
			match ended {
				Ended::Done | Ended::Refused | Ended::Failed => {}
			}
		}
	}

	/// The alerts carry each stage's objective, their tests prove both fire
	/// for each stage, and the overview has a tile for it and reads each of
	/// its waiters: a stage added in code alone fails here.
	#[test]
	fn every_stage_has_its_alerts_their_tests_and_its_tile() {
		for rule in ["alert: StageDegraded", "alert: StageBroken", "record: stage:oldest_waiting:seconds"] {
			assert!(RULES.contains(rule), "{rule}");
		}
		for stage in Stage::ALL {
			for alert in ["StageDegraded", "StageBroken"] {
				// The test's own lines, as home.test.yml indents them.
				let fired = [
					"alertname: ",
					alert,
					"\n        exp_alerts:\n          - exp_labels:\n              stage: ",
					stage.name(),
					"\n              objective: ",
					stage.promise().objective,
				]
				.concat();
				assert!(TESTS.contains(&fired), "home.test.yml proves {alert} fires for {} under its objective", stage.name());
			}
			let tile = ["stage: '", stage.name(), "'"].concat();
			assert!(OVERVIEW.contains(&tile), "the overview has a tile for {}", stage.name());
			for waiter in stage.waiters() {
				match waiter {
					Waiter::CardsEssays | Waiter::RenderLane => {}
				}
				assert!(OVERVIEW.contains(waiter.series()), "the overview reads {}", waiter.series());
			}
		}
	}
}

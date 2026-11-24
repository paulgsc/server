//! A failure is counted where it happens (#1189, epic #1174).
//!
//! The owner's rule: a failure is like energy, never destroyed. However deep
//! the call, a step that fails is one atomic increase in an error count, and
//! the bookkeeping must reconcile. Before this, it held only by discipline:
//! about 140 sites turned a failure into an ordinary value (a refusal stored
//! as an answer, a `false`, an empty list), and their callers read it as
//! nothing having gone wrong (#1172).
//!
//! [`Failure`] is the proof that one was counted. Only [`Failure::record`]
//! makes one, and it does three things at once:
//!
//! - increments `failures_total{process, step}`;
//! - logs at `error!`, naming the process, the step, the cause and the line
//!   that recorded it. So `tracing_events_total{level="error"}` counts it too
//!   and `file-host-errors.yml` hears of it, under this module's target: the
//!   process, step and line say where it came from;
//! - returns the token.
//!
//! An outcome that is a failure is meant to carry the token (#1172 makes the
//! mailbox's `Answer::Refused` the first), so storing the outcome needs the
//! count first. The type is `#[must_use]` and
//! has no other constructor, no `Default`, no `Clone` and no `From`, so it can
//! be neither forged nor copied into a second outcome, and nothing converts
//! into it: a yield (the lane's pause, stop or give) is a different type and
//! stays one (docs/home-compute.md, invariant 3). Once the outcome is stored,
//! the token has done its work and is dropped there. `#[must_use]` does not
//! see `let _ =`; the erasure ratchet (#1189) is what will.
//!
//! What it does not change: what is stored, and the shape of any function.
//! The database still stores `"refused"`. The count is event-driven and adds
//! no schedule, and a deterministic refusal is counted once and held, never
//! retried on a clock (docs/home-compute.md, invariants 6 and 7).
//!
//! The labels are bounded: [`Step`] is a closed table, and each step names
//! its [`Process`], so no value a person or a request supplies becomes a
//! label (docs/identity.md, "Privacy invariants"). `cause` is logged through
//! `Display`, and the type does not stop a caller passing anything: it is the
//! caller's duty to pass only what may be logged, never a peer's address, a
//! token, an endpoint or a note's text.
//!
//! `HomeFailures` (`infra/prometheus/rules/home.yml`) sends any increase to
//! the owner's Slack, beside the per-process `*Failing` alerts.

use metrics::counter;
use std::fmt;
use std::panic::Location;

/// One process home runs: the `process` label.
///
/// Each is its id in paulgsc/some-ui's assurance ledger
/// (`packages/assurance/src/ledger.json`, `processes`). A process the ledger
/// does not have yet gets its entry there (OA1) and its variant here.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Process {
	BiteMatched,
	CallWeeks,
	CardsEssays,
	Clips,
	GameFeed,
	HomeDeploy,
	HomeTranscription,
	MailboxPasses,
	PdFills,
	ReportWeeks,
	RustEssays,
	RustRoundsRuns,
	StudioVideoQueue,
	StudyNudge,
	TemplatePreview,
}

impl Process {
	/// Every process, for tests and for anything that lists them.
	pub const ALL: [Self; 15] = [
		Self::BiteMatched,
		Self::CallWeeks,
		Self::CardsEssays,
		Self::Clips,
		Self::GameFeed,
		Self::HomeDeploy,
		Self::HomeTranscription,
		Self::MailboxPasses,
		Self::PdFills,
		Self::ReportWeeks,
		Self::RustEssays,
		Self::RustRoundsRuns,
		Self::StudioVideoQueue,
		Self::StudyNudge,
		Self::TemplatePreview,
	];

	/// The ledger's id, which is the `process` label.
	#[must_use]
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::BiteMatched => "bite-matched",
			Self::CallWeeks => "call-weeks",
			Self::CardsEssays => "cards-essays",
			Self::Clips => "clips",
			Self::GameFeed => "game-feed",
			Self::HomeDeploy => "home-deploy",
			Self::HomeTranscription => "home-transcription",
			Self::MailboxPasses => "mailbox-passes",
			Self::PdFills => "pd-fills",
			Self::ReportWeeks => "report-weeks",
			Self::RustEssays => "rust-essays",
			Self::RustRoundsRuns => "rust-rounds-runs",
			Self::StudioVideoQueue => "studio-video-queue",
			Self::StudyNudge => "study-nudge",
			Self::TemplatePreview => "template-preview",
		}
	}
}

impl fmt::Display for Process {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.write_str(self.as_str())
	}
}

/// Declares every step a failure can be recorded at, once: the enum, its list
/// and its labels come from the one table, so a step cannot be left off the
/// list [`register_all`] starts at zero.
macro_rules! steps {
	($($(#[$doc:meta])* $variant:ident => $process:ident, $name:literal;)+) => {
		/// Where in a process a failure happened: the `step` label.
		///
		/// A closed set, not a free string, so that every `(process, step)`
		/// series exists at 0 from boot ([`register_all`]) and a failure's
		/// first count is an increase `HomeFailures` can see, after a restart
		/// too. A new step is one line in this table, named for what failed,
		/// in a few words.
		#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
		pub enum Step {
			$($(#[$doc])* $variant,)+
		}

		impl Step {
			/// Every step, in the table's order.
			pub const ALL: &'static [Self] = &[$(Self::$variant,)+];

			/// The process this step belongs to: the `process` label.
			#[must_use]
			pub const fn process(self) -> Process {
				match self {
					$(Self::$variant => Process::$process,)+
				}
			}

			/// The `step` label.
			#[must_use]
			pub const fn as_str(self) -> &'static str {
				match self {
					$(Self::$variant => $name,)+
				}
			}
		}
	};
}

steps! {
	/// The Jobs mailbox refused a file home handed in (#1172).
	MailboxHandOver => MailboxPasses, "hand_over";
}

/// Starts every `failures_total{process, step}` series at 0, once at boot.
///
/// `increase()` never sees a series' first sample, so a series born at its
/// first failure would hide that failure from `HomeFailures`, and a restart
/// that dropped the series would hide the next one. The repo's other counters
/// that page on any increase do the same (`refusals::register_all`).
pub fn register_all() {
	for step in Step::ALL {
		counter!("failures_total", "process" => step.process().as_str(), "step" => step.as_str()).increment(0);
	}
}

/// Proof that a failure was counted and logged. Only [`Failure::record`]
/// makes one.
#[must_use = "a failure must reach an outcome: return it or store it"]
#[derive(Debug, PartialEq, Eq)]
pub struct Failure {
	step: Step,
}

impl Failure {
	/// Counts `failures_total{process, step}`, logs at error, and returns the
	/// proof. `cause` is why, and must be loggable (see the module).
	#[track_caller]
	pub fn record(step: Step, cause: &dyn fmt::Display) -> Self {
		let at = Location::caller();
		let (process, name) = (step.process().as_str(), step.as_str());
		counter!("failures_total", "process" => process, "step" => name).increment(1);
		tracing::error!(
			process,
			step = name,
			at = %format_args!("{}:{}", at.file(), at.line()),
			cause = %cause,
			"{process} failed at {name}: {cause}"
		);
		Self { step }
	}

	#[must_use]
	pub const fn step(&self) -> Step {
		self.step
	}
}

#[cfg(test)]
mod tests {
	use super::{register_all, Failure, Process, Step};
	use metrics_util::debugging::{DebugValue, DebuggingRecorder, Snapshotter};
	use std::collections::HashSet;

	fn count(snapshotter: &Snapshotter, step: Step) -> Option<u64> {
		snapshotter.snapshot().into_vec().into_iter().find_map(|(key, _, _, value)| {
			let key = key.key();
			let is_it = key.name() == "failures_total"
				&& key.labels().any(|l| l.key() == "process" && l.value() == step.process().as_str())
				&& key.labels().any(|l| l.key() == "step" && l.value() == step.as_str());
			match (is_it, value) {
				(true, DebugValue::Counter(n)) => Some(n),
				_ => None,
			}
		})
	}

	/// A freshly booted home with no failures exports every series at 0, so
	/// the first failure, after a restart too, is an increase.
	#[test]
	fn every_step_exists_at_zero_after_register_all() {
		let recorder = DebuggingRecorder::new();
		let snapshotter = recorder.snapshotter();
		metrics::with_local_recorder(&recorder, register_all);
		for step in Step::ALL {
			assert_eq!(count(&snapshotter, *step), Some(0), "{step:?}");
		}
	}

	/// Each record is one increase of its own process and step.
	#[test]
	fn each_record_is_one_increase_of_its_process_and_step() {
		let recorder = DebuggingRecorder::new();
		let snapshotter = recorder.snapshotter();
		let tokens = metrics::with_local_recorder(&recorder, || {
			register_all();
			[Failure::record(Step::MailboxHandOver, &"refused"), Failure::record(Step::MailboxHandOver, &"refused again")]
		});

		assert_eq!(count(&snapshotter, Step::MailboxHandOver), Some(2));
		assert_eq!(tokens[1].step(), Step::MailboxHandOver);
	}

	/// The log is at error, so `tracing_events_total{level="error"}` counts it
	/// and file-host-errors.yml hears of it, and it names what failed, why,
	/// and the line that recorded it.
	#[test]
	fn a_record_logs_at_error_naming_the_process_step_cause_and_line() {
		let (captured, _guard) = crate::privacy::capture_where(|meta| *meta.level() == tracing::Level::ERROR);
		let _failure = Failure::record(Step::MailboxHandOver, &"the mailbox said: no such folder");

		let lines = captured.lines();
		for expected in [
			"process=mailbox-passes",
			"step=hand_over",
			"cause=the mailbox said: no such folder",
			"at=apps/servers/file_host/src/failure.rs:",
		] {
			assert!(lines.iter().any(|line| line.starts_with(expected)), "{expected} in {lines:#?}");
		}
	}

	/// The `process` label is the ledger's id (lowercase words joined by
	/// hyphens), one per variant; a step's label is lowercase words joined by
	/// underscores, one per step within its process.
	#[test]
	fn every_label_is_bounded_and_its_own() {
		let ids: HashSet<&str> = Process::ALL.iter().map(|p| p.as_str()).collect();
		assert_eq!(ids.len(), Process::ALL.len());
		for id in ids {
			assert!(id.split('-').all(|word| !word.is_empty() && word.chars().all(|c| c.is_ascii_lowercase())), "{id}");
		}
		let steps: HashSet<(&str, &str)> = Step::ALL.iter().map(|s| (s.process().as_str(), s.as_str())).collect();
		assert_eq!(steps.len(), Step::ALL.len());
		for (_, name) in steps {
			assert!(name.split('_').all(|word| !word.is_empty() && word.chars().all(|c| c.is_ascii_lowercase())), "{name}");
		}
	}
}

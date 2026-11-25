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
//! makes one, and it does three things at once ([`Failure::alone`] does the
//! first two, for a failure whose only outcome is its count and its log):
//!
//! - increments `failures_total{process, step}`;
//! - logs at `error!`, naming the process, the step, the cause and the line
//!   that recorded it. So `tracing_events_total{level="error"}` counts it too
//!   and `file-host-errors.yml` hears of it, under this module's target: the
//!   process, step and line say where it came from;
//! - returns the token.
//!
//! An outcome that is a failure carries the token (the mailbox pass's
//! `Handing`, #1172, is the first), so storing the outcome needs the count
//! first. The type is `#[must_use]` and
//! has no other constructor, no `Default`, no `Clone` and no `From`, so it can
//! be neither forged nor copied into a second outcome, and nothing converts
//! into it: a yield (the lane's pause, stop or give) is a different type and
//! stays one (docs/home-compute.md, invariant 3). Once the outcome is stored,
//! the token has done its work and is dropped there. `#[must_use]` does not
//! see `let _ =`; the erasure ratchet (#1189) is what will.
//!
//! What it does not change: the shape of any function, or what is stored for
//! an outcome stored truly. The mailbox still stores `"refused"` for its own
//! refusal; the one new stored value is `unfit`, for a file it can never take,
//! which was stored as `refused` and so handed again (#1172, the coordinator's
//! call on #1192's review). The count is event-driven and adds no schedule,
//! and a deterministic refusal is counted once and held, never retried on a
//! clock (docs/home-compute.md, invariants 6 and 7).
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
//!
//! One event is one count, so a failure that persists, which every pass or
//! every restart meets again until it is fixed, is not counted each time it
//! is met. It is a level instead: [`open`] sets `failures_open{process, step}`
//! to how many are open now, afresh each pass, at a step marked `open` in the
//! same table, and `HomeFailuresOpen` tells the owner while one stays open
//! (#1201, the owner's choice on 2026-11-24).

use metrics::{counter, gauge};
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
	JobsNotice,
	MailboxPasses,
	PdFills,
	ReportWeeks,
	RustEssays,
	RustRoundsRuns,
	SnipsPickup,
	StudioVideoQueue,
	StudyNudge,
	TemplatePreview,
}

impl Process {
	/// Every process, for tests and for anything that lists them.
	pub const ALL: [Self; 17] = [
		Self::BiteMatched,
		Self::CallWeeks,
		Self::CardsEssays,
		Self::Clips,
		Self::GameFeed,
		Self::HomeDeploy,
		Self::HomeTranscription,
		Self::JobsNotice,
		Self::MailboxPasses,
		Self::PdFills,
		Self::ReportWeeks,
		Self::RustEssays,
		Self::RustRoundsRuns,
		Self::SnipsPickup,
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
			Self::JobsNotice => "jobs-notice",
			Self::MailboxPasses => "mailbox-passes",
			Self::PdFills => "pd-fills",
			Self::ReportWeeks => "report-weeks",
			Self::RustEssays => "rust-essays",
			Self::RustRoundsRuns => "rust-rounds-runs",
			Self::SnipsPickup => "snips-pickup",
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
	($($(#[$doc:meta])* $variant:ident => $process:ident, $name:literal $(, $open:ident)?;)+) => {
		/// Where in a process a failure happened: the `step` label.
		///
		/// A closed set, not a free string, so that every `(process, step)`
		/// series exists at 0 from boot ([`register_all`]) and a failure's
		/// first count is an increase `HomeFailures` can see, after a restart
		/// too. A new step is one line in this table, named for what failed,
		/// in a few words. A step marked `open` is a failure that persists,
		/// held as a level ([`open`]), never counted.
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

			/// Whether this step is a failure that persists, held as a level
			/// in `failures_open` ([`open`]) rather than counted in
			/// `failures_total`.
			#[must_use]
			pub const fn is_open(self) -> bool {
				match self {
					$(Self::$variant => steps!(@open $($open)?),)+
				}
			}
		}
	};
	(@open open) => { true };
	(@open) => { false };
}

steps! {
	/// The Jobs mailbox refused a file home handed in, in its own words (a
	/// `422`); it is handed again (#1172, MI8).
	MailboxHandOver => MailboxPasses, "hand_over";
	/// A file home handed in can never be taken as it is: a name that cannot
	/// be put in a URL, or a body over the mailbox's ceiling (#1172).
	MailboxHandOverUnfit => MailboxPasses, "hand_over_unfit";
	/// The Jobs mailbox holds other bytes under a file's name, which it never
	/// replaces, so the file home handed in is never taken (#1172).
	MailboxHandOverConflict => MailboxPasses, "hand_over_conflict";
	/// A file kept as a plain brief because its body does not read as its
	/// name says: a Drive takes list, a Rust Rounds week, a mock interview's
	/// week, or a pickup's answer. What it would have queued or moved is not,
	/// and a week's essay is never owed (#1178). Counted when it is first
	/// kept (#1175): nothing rewrites a kept file, so it never clears.
	MailboxReadBody => MailboxPasses, "read_body";
	/// A file read from the mailbox and not kept (`read_incoming`) that a
	/// newer file kept in its folder passes: the cursor moves past it, so it
	/// is never read again. A final loss, counted once, by the pass whose keep
	/// passes it (#1201).
	MailboxPassedOver => MailboxPasses, "passed_over";
	/// Open: an archived file the routine asked back that home cannot put
	/// back: it holds no copy (`missing`), or the mailbox would not take the
	/// one it holds (`conflict`, `refused`). The ask stays open, so every
	/// pass meets it again (#1201).
	MailboxRestore => MailboxPasses, "restore", open;
	/// Open: a file read from the mailbox and not kept (its bytes are not
	/// what the listing said, its name is bad, it is not JSON, or its name
	/// already holds other bytes) that is newer than every file kept in its
	/// folder. The cursor is the newest file kept, so every pass fetches it
	/// again until a newer one is kept (#1201).
	MailboxReadIncoming => MailboxPasses, "read_incoming", open;
	/// Open: a routine's index the mailbox refused, was too big for, or
	/// conflicted on, which is not put again until what it counts moves.
	/// Held in memory, so a restart puts it again (#1201).
	MailboxPutIndex => MailboxPasses, "put_index", open;
	/// The Rust Rounds runner refused a program as malformed or too large,
	/// and the refusal is kept as the program's run, never asked again
	/// (#1182). Counted when it is kept.
	RustRoundsRunnerRefused => RustRoundsRuns, "runner_refused";
	/// A Rust Rounds program's run was lost (the runner dropped it or did not
	/// answer in time), kept as one loss; the last loss it is allowed keeps it
	/// as `runner-lost`. Counted once per loss kept.
	RustRoundsRunLost => RustRoundsRuns, "run_lost";
	/// A study nudge was claimed and sent, no device confirmed it, and a
	/// delivery timed out: the intervention is spent and kept, never sent
	/// again (#1180). Counted when it is kept; a process that dies between
	/// the claim and the verdict leaves the claim spent and uncounted.
	StudyNudgeDeliveryTimedOut => StudyNudge, "delivery_timed_out";
	/// Open: the call's render signals a tick of the call weeks worker could
	/// not read: what the worker made of the weeks held (a storage error),
	/// the worker's `last-pass`, each job's cache warnings, and the drive's
	/// free and total bytes (`statvfs`). Each one's gauges are unknown
	/// (`NaN`) while it is open, except the newest cache warning, which
	/// combines six jobs' folders: it keeps a warning time read in another
	/// folder, a lower bound. With a folder unreadable and no warning read in
	/// any other (none read, or those that read hold none), it is `NaN`
	/// (#1179, #1207, C1512).
	CallWeeksObserve => CallWeeks, "observe", open;
	/// Open: whether the call's next week is overdue, which a pass could not
	/// read, so `call_weeks_next_overdue` is unknown (`NaN`) and
	/// `CallWeekOverdue` is silent while it is open (#1179, #1207).
	CallWeeksNextOverdue => CallWeeks, "next_overdue", open;
	/// Open: what the render worker made of the Playoff Delta fills held,
	/// which a tick could not read (a storage error), so the `pd_render_*`
	/// gauges are not what the drive holds (#1179).
	PdFillsObserve => PdFills, "observe", open;
	/// A study nudge claimed and tried on its subject's devices, refused by
	/// each, none timing out. The claim is handed back, and the subject is
	/// tried again only at its next eligibility, a new attempt, so each
	/// attempt is counted once (#1180, Codex on #1221).
	StudyNudgeNoDeviceAccepted => StudyNudge, "no_device_accepted";
	/// A device that did not take an intervention another device accepted:
	/// its push service refused it or did not answer in time. The
	/// intervention is spent, so that device never gets it. Counted once per
	/// device, when the intervention is marked sent (#1180).
	StudyNudgeDeviceMissed => StudyNudge, "device_missed";
	/// Open: clip frames and walk sides a clips pass could not take (an
	/// index or frame that does not read, a frame whose bytes are not its
	/// digest, the drive), each asked again by the next pass, except a walk
	/// the pass rejected (#809).
	ClipsTake => Clips, "take", open;
	/// A walk side the render worker drew that the pass rejected and removed:
	/// it names other snips than the index (`mismatch`, drawn the same way
	/// until the render image moves), or its `walk.json` or snips do not
	/// read. Removed, it is walked again, a new render, so each render
	/// rejected is counted once (#809, Codex on #1221).
	ClipsWalkRejected => Clips, "walk_rejected";
	/// Open: home's deploy record (`state.json`) is there and can't be read
	/// or parsed. Every sweep meets it again until a deploy writes it anew or
	/// someone repairs it; one envctl can't replace either (a directory in
	/// its place, no permission) never heals by itself (HD3, #1208).
	HomeDeployReadState => HomeDeploy, "read_state", open;
	/// Open: home's deploy marker is there and its time can't be read, so
	/// how long a deploy has waited is unknown (HD3, #1208).
	HomeDeployReadMarker => HomeDeploy, "read_marker", open;
	/// Open: Jobs run records no phone took this pass because every phone
	/// met a final refusal (a key or JWT home must fix, a token FCM calls
	/// invalid). Each is tried again every pass until a phone takes it or
	/// it leaves the window (#1176, #1206).
	JobsNoticeRefused => JobsNotice, "refused", open;
	/// Open: Jobs run records no phone took this pass because FCM said to
	/// try later (a `429`, a `5xx`) or never answered; tried again every
	/// pass within the window (#1176).
	JobsNoticeUnanswered => JobsNotice, "unanswered", open;
	/// A Jobs run record that left the notice's window before any phone was
	/// told of it, while a phone had asked: a loss, stored `expired` and
	/// counted once, by the pass that stores it (#1176).
	JobsNoticeUntold => JobsNotice, "untold";
	/// A Snips commit or run marked gone (no run, an expired artifact, a
	/// bundle refused or too big): its essay is refused `snip-gone` until a
	/// newer run clears the mark. Counted once, when the mark is written
	/// (#1177).
	SnipsPickupGone => SnipsPickup, "gone";
	/// Open: Snips keys an essay waits on whose last fetch failed in a way
	/// that may clear (a timeout, the drive), each asked again next pass
	/// (#1177).
	SnipsPickupRetry => SnipsPickup, "retry", open;
	/// A browser the study nudge set aside after failures of its own in a
	/// row (`strikes::SET_ASIDE_AFTER`; the owner's "a few timeouts" on
	/// #1180): no longer sent to but for a daily probe, until it accepts one
	/// or subscribes again. Counted once, when the mark is written.
	StudyNudgeDeviceSetAside => StudyNudge, "device_set_aside";
	/// A phone the Jobs notice set aside after failures of its own in a row
	/// (#1222, `strikes`), as `device_set_aside`. Counted once, when the
	/// mark is written.
	JobsNoticePhoneSetAside => JobsNotice, "phone_set_aside";
	/// Open: what the render worker made of the Report weeks, which a tick
	/// could not read (a storage error), or the worker's `last-pass` there
	/// and unreadable; the `report_render_*` gauges are unknown (`NaN`) while
	/// it is open (#1227).
	ReportWeeksObserve => ReportWeeks, "observe", open;
	/// Open: what the render worker made of the Cards essays, which a tick
	/// could not read (a storage error); the `cards_essay_render_*` gauges are
	/// unknown (`NaN`) while it is open (#1227).
	CardsEssaysObserve => CardsEssays, "observe", open;
}

/// Starts every `failures_total{process, step}` series at 0, once at boot.
///
/// `increase()` never sees a series' first sample, so a series born at its
/// first failure would hide that failure from `HomeFailures`, and a restart
/// that dropped the series would hide the next one. (A failure before the
/// first scrape after a boot is `HomeFailures`' second arm, on
/// `process_start_time_seconds`.) The repo's other counters
/// that page on any increase do the same (`refusals::register_all`).
pub fn register_all() {
	for step in Step::ALL.iter().filter(|step| !step.is_open()) {
		counter!("failures_total", "process" => step.process().as_str(), "step" => step.as_str()).increment(0);
	}
}

/// Sets `failures_open{process, step}` to how many of `step`'s failures that
/// persist are open now, as one pass found them (#1201, epic #1174).
///
/// A failure that persists (an ask home cannot answer, a file refused that is
/// the newest in its folder) is met again by every pass, or every restart,
/// until it is fixed. Counting it each time would count one condition many
/// times, so it is a level: computed afresh by the pass that looks, never
/// added to, so a restart cannot count it twice and a fixed one clears
/// itself. It is neither counted in `failures_total` nor logged at error:
/// what the pass met is in its own outcome, logged with it.
/// `HomeFailuresOpen` (`infra/prometheus/rules/home.yml`) tells the owner
/// while one stays open. A pass that could not look leaves the level as the
/// last one that did set it; the pass's own failure is told by its process's
/// `*Failing` alert.
///
/// # Panics
/// In a debug build, when `step` is not marked `open`: a step's failure is
/// either an event, counted once, or a level, never both.
pub fn open(step: Step, now: usize) {
	debug_assert!(step.is_open(), "{step:?} is counted, not held open");
	#[allow(clippy::cast_precision_loss)] // a count of files and asks, far below 2^52
	let now = now as f64;
	gauge!("failures_open", "process" => step.process().as_str(), "step" => step.as_str()).set(now);
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
		count(step, cause, Location::caller());
		Self { step }
	}

	/// As [`Failure::record`], for a failure whose only outcome is its count
	/// and its log: nothing stores it (a file kept as a plain brief because
	/// its body does not read as its name says, say). Where an outcome is
	/// stored, `record` it and carry the token there instead. Either way, one
	/// event is one count: a condition that persists is not counted again
	/// each time it is seen.
	#[track_caller]
	pub fn alone(step: Step, cause: &dyn fmt::Display) {
		count(step, cause, Location::caller());
	}

	#[must_use]
	pub const fn step(&self) -> Step {
		self.step
	}
}

/// The count and the log both makers share; `at` is their caller's line.
fn count(step: Step, cause: &dyn fmt::Display, at: &Location<'_>) {
	debug_assert!(!step.is_open(), "{step:?} is held open, never counted");
	let (process, name) = (step.process().as_str(), step.as_str());
	counter!("failures_total", "process" => process, "step" => name).increment(1);
	tracing::error!(
		process,
		step = name,
		at = %format_args!("{}:{}", at.file(), at.line()),
		cause = %cause,
		"{process} failed at {name}: {cause}"
	);
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
		for step in Step::ALL.iter().filter(|step| !step.is_open()) {
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

	fn level(snapshotter: &Snapshotter, step: Step) -> Option<f64> {
		snapshotter.snapshot().into_vec().into_iter().find_map(|(key, _, _, value)| {
			let key = key.key();
			let is_it = key.name() == "failures_open"
				&& key.labels().any(|l| l.key() == "process" && l.value() == step.process().as_str())
				&& key.labels().any(|l| l.key() == "step" && l.value() == step.as_str());
			match (is_it, value) {
				(true, DebugValue::Gauge(n)) => Some(n.into_inner()),
				_ => None,
			}
		})
	}

	/// A step held open is a level, never a count: `register_all` starts no
	/// `failures_total` series for it, and each pass's level replaces the
	/// last, so two passes over the same condition hold it rather than add to
	/// it, and a fixed one clears.
	#[test]
	fn an_open_step_is_a_level_set_afresh_and_never_counted() {
		let recorder = DebuggingRecorder::new();
		let snapshotter = recorder.snapshotter();
		metrics::with_local_recorder(&recorder, || {
			register_all();
			super::open(Step::MailboxRestore, 2);
		});
		assert_eq!(level(&snapshotter, Step::MailboxRestore), Some(2.0));
		// Twice between reads, since a read drains: set, not added to.
		metrics::with_local_recorder(&recorder, || {
			super::open(Step::MailboxRestore, 2);
			super::open(Step::MailboxRestore, 2);
		});
		assert_eq!(level(&snapshotter, Step::MailboxRestore), Some(2.0), "the same two, met again, are still two");
		metrics::with_local_recorder(&recorder, || super::open(Step::MailboxRestore, 0));
		assert_eq!(level(&snapshotter, Step::MailboxRestore), Some(0.0), "fixed, it clears");
		for step in Step::ALL {
			assert_eq!(count(&snapshotter, *step).is_some(), !step.is_open(), "{step:?}");
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

/// What other modules' tests read back: how many times a step was counted.
#[cfg(test)]
pub(crate) mod counted {
	use super::Step;
	use metrics_util::debugging::{DebugValue, Snapshotter};

	/// `failures_total` for `step` in `snapshotter`'s recorder, or `None` if
	/// the series was never touched.
	pub(crate) fn failures(snapshotter: &Snapshotter, step: Step) -> Option<u64> {
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
}

/// What other modules' tests read back: steps' levels and counts, all from
/// one snapshot, since a snapshot drains every gauge and counter it reads.
#[cfg(test)]
pub(crate) mod held {
	use super::Step;
	use metrics_util::debugging::{DebugValue, Snapshotter};

	/// One snapshot of `failures_open` and `failures_total`.
	pub(crate) struct Seen(Vec<(String, String, String, DebugValue)>);

	/// Everything `snapshotter`'s recorder holds now.
	pub(crate) fn seen(snapshotter: &Snapshotter) -> Seen {
		Seen(
			snapshotter
				.snapshot()
				.into_vec()
				.into_iter()
				.map(|(key, _, _, value)| {
					let key = key.key();
					let label = |name: &str| key.labels().find(|l| l.key() == name).map_or_else(String::new, |l| l.value().to_owned());
					(key.name().to_owned(), label("process"), label("step"), value)
				})
				.collect(),
		)
	}

	impl Seen {
		fn find(&self, name: &str, step: Step) -> Option<&DebugValue> {
			self
				.0
				.iter()
				.find(|(n, process, label, _)| n == name && process == step.process().as_str() && label == step.as_str())
				.map(|(.., value)| value)
		}

		/// `failures_open` for `step`, if it was set since the last snapshot.
		pub(crate) fn open(&self, step: Step) -> Option<f64> {
			match self.find("failures_open", step)? {
				DebugValue::Gauge(n) => Some(n.into_inner()),
				_ => None,
			}
		}

		/// `failures_total` for `step` since the last snapshot, if touched.
		pub(crate) fn counted(&self, step: Step) -> Option<u64> {
			match self.find("failures_total", step)? {
				DebugValue::Counter(n) => Some(*n),
				_ => None,
			}
		}
	}

	/// `failures_open` for `step` alone (the snapshot drains the rest).
	pub(crate) fn open(snapshotter: &Snapshotter, step: Step) -> Option<f64> {
		seen(snapshotter).open(step)
	}
}

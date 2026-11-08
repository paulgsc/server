//! Home records each Rust Rounds program on arrival (paulgsc/some-ui#2234,
//! RR3).
//!
//! The phone puts a round (`PUT /api/v1/rust-rounds/:id`, kept in
//! `rust_round`, each distinct program queued in `rust_round_source`), and
//! this worker hands each program to the rust-runner, the sandboxed
//! container that compiles and runs it, and keeps how it ended in
//! `rust_round_run`. The phone reads the runs back with the round
//! (`GET /api/v1/rust-rounds/:id`).
//!
//! **Why this is a poll,** as the transcriber is: a round can arrive while
//! the runner is busy or down, and a restart must pick up whatever was left,
//! so the worker asks the database every [`INTERVAL`] (a minute) for the programs the runner's toolchain has not recorded. An
//! idle pass is the runner's toolchain and one indexed read. No request
//! waits on it.
//!
//! What it promises is RP1–RP4 in `docs/leetype-execution.md`, summarised in
//! [`pass`]. **Nothing a program says reaches a log line**: a pass logs
//! counts and the kind of stop, never a source, a recording or a round.
//!
//! **Always started; the operator pauses it.** Whether home records is a
//! lever, `settings::RustRounds` (on by default), set from the web app
//! (`PUT /api/v1/admin/settings/rust-rounds`, paulgsc/some-ui#2357), not an
//! environment variable: `RUST_ROUNDS_ENABLED` is gone (owner, 2026-11-08).
//! Each pass reads it once, first. Paused, the pass asks nothing: rounds and
//! weeks are still kept and their programs wait, and the first pass after it
//! is turned back on records them from the oldest. Pausing and resuming are
//! logged once, when the recorder first sees them; a lever it cannot read
//! is an error, and that pass is skipped rather than run on a guess.
//!
//! **No runner on this machine.** Whether the runner is here is detected,
//! not declared: a pass that finds nothing at [`SOCKET`] stops as
//! [`pass::Stop::NoRunner`], a deployment fact logged once at `WARN` when it
//! is first seen and once at `INFO` when a socket is there again, never an
//! error each pass. A runner that is deployed but down leaves its socket
//! behind, so it is an outage ([`pass::Stop::Unreachable`]), reported at
//! `ERROR` every pass it lasts, since the runner restarts on its own.

pub mod pass;

use crate::call_weeks::lane::Held;
use crate::settings::{self, RustRounds};
use crate::AppState;
use pass::{Runner, Socket};
use rust_rounds_wire::protocol::{AskError, Request, Response};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tracing::{error, info, warn};

// The recorder's engineering values: source constants, not variables
// (#617). Each was its variable's default.

/// The runner's socket: the volume compose shares between the two
/// containers (`infra/compose/rust_runner.yml`), and the runner's own
/// default (`rust-runner`'s `SOCKET`) (`RUST_ROUNDS_SOCKET`).
pub const SOCKET: &str = "/run/rust-runner/runner.sock";

/// The shortest interval a pass may run at.
pub const MIN_INTERVAL: Duration = Duration::from_secs(10);

/// How often the worker looks for rounds to record (`RUST_ROUNDS_SECONDS`).
pub const INTERVAL: Duration = Duration::from_secs(60);

/// The shortest deadline a pass may get: the toolchain ask's wait and one
/// program's (142 s).
///
/// A program's ask waits `ASK_TIMEOUT`, past the runner's watchdog deadline
/// for one job (its ceilings, clean-up, client deadlines and sweep
/// allowance, 132 s), so a job the watchdog ends is seen as lost (RP3); the
/// toolchain ask, `CLIENT_TIMEOUT`, comes first.
pub const MIN_DEADLINE: Duration = pass::Waits::SERVER.least_deadline();

/// How long one pass may take, every ask included
/// (`RUST_ROUNDS_PASS_DEADLINE_SECONDS`).
pub const PASS_DEADLINE: Duration = Duration::from_secs(600);

const _: () = assert!(INTERVAL.as_secs() >= MIN_INTERVAL.as_secs() && PASS_DEADLINE.as_secs() >= MIN_DEADLINE.as_secs());

/// Everything the worker needs.
#[derive(Clone)]
pub struct RustRoundsContext {
	pub runner: Arc<Socket>,
	pub interval: Duration,
	/// How long one pass may take, every ask included.
	pub deadline: Duration,
}

impl RustRoundsContext {
	/// The recorder's context. Its values are constants, held to their floors
	/// at compile time.
	#[must_use]
	pub fn new() -> Self {
		Self {
			runner: Arc::new(Socket(PathBuf::from(SOCKET))),
			interval: INTERVAL,
			deadline: PASS_DEADLINE,
		}
	}
}

impl Default for RustRoundsContext {
	fn default() -> Self {
		Self::new()
	}
}

/// Start the worker. It always runs; the operator's lever pauses its passes.
pub fn start(state: &AppState) {
	spawn(state, &state.rust_rounds);
}

/// What the recorder last said about the lever and the runner, so each is
/// logged when it changes, not every pass.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Seen {
	/// Whether it was last paused; `None` before the first pass.
	paused: Option<bool>,
	/// Whether the last pass found no runner on this machine.
	no_runner: bool,
}

/// What a pass should say about a change it saw.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Change {
	Paused,
	Resumed,
	NoRunner,
	RunnerFound,
}

impl Seen {
	/// The lever as this pass read it: `Paused` or `Resumed` when it moved
	/// (the first pass says `Paused` only if it starts paused).
	pub(crate) const fn lever(&mut self, paused: bool) -> Option<Change> {
		let was = self.paused.replace(paused);
		match (was, paused) {
			(Some(false) | None, true) => Some(Change::Paused),
			(Some(true), false) => Some(Change::Resumed),
			(Some(false) | None, false) | (Some(true), true) => None,
		}
	}

	/// The runner as this pass found it.
	pub(crate) fn runner(&mut self, stopped: Option<pass::Stop>) -> Option<Change> {
		let absent = stopped == Some(pass::Stop::NoRunner);
		let was = std::mem::replace(&mut self.no_runner, absent);
		match (was, absent) {
			(false, true) => Some(Change::NoRunner),
			(true, false) => Some(Change::RunnerFound),
			(false, false) | (true, true) => None,
		}
	}
}

fn say(change: Change) {
	match change {
		Change::Paused => info!("Rust Rounds recording is paused by the operator: rounds and weeks are kept, and their programs wait"),
		Change::Resumed => info!("Rust Rounds recording is on again: the programs that waited are recorded from the oldest"),
		Change::NoRunner => warn!(
			socket = SOCKET,
			"no Rust Rounds runner on this machine: nothing is at its socket, so programs wait until one answers; said once, not every pass"
		),
		Change::RunnerFound => info!(socket = SOCKET, "the Rust Rounds runner's socket is there again"),
	}
}

/// The runner, holding interactive work while it records a program, so the
/// batch lane pauses a render for it (`call_weeks::lane`, LA1). The
/// toolchain ask each pass makes first holds nothing: it is asked every
/// minute, whether or not a program waits, and held it would keep a render
/// from ever passing its hold-down.
impl<R: Runner> Runner for Held<R> {
	fn exchange(&self, request: &Request, timeout: Duration) -> Result<Response, AskError> {
		let _hold = matches!(request, Request::Record { .. }).then(|| self.interactive.hold());
		self.inner.exchange(request, timeout)
	}
}

/// Spawn the worker, cancelled through the shared token. Its first pass runs
/// at once, so a restart catches up without waiting an interval.
fn spawn(state: &AppState, context: &RustRoundsContext) {
	let cancel = state.core.cancel_token.clone();
	let db = state.core.shared_db.clone();
	let runner = Arc::new(Held {
		inner: Arc::clone(&context.runner),
		interactive: state.interactive.clone(),
	});
	let (every, deadline) = (context.interval, context.deadline);
	tokio::spawn(async move {
		info!(interval_secs = every.as_secs(), "Rust Rounds recorder started");
		let mut ticker = tokio::time::interval(every);
		ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
		let mut seen = Seen::default();
		loop {
			tokio::select! {
				() = cancel.cancelled() => {
					info!("Rust Rounds recorder cancelled");
					return;
				}
				_ = ticker.tick() => {
					// The lever, once a pass (`settings::RustRounds`).
					let lever = match settings::read::<RustRounds>(&db).await {
						Ok(standing) => standing.value,
						Err(err) => {
							crate::metrics::instruments::record_error("rust_rounds", "setting");
							error!(error = %err, "Rust Rounds pass skipped: its recording setting could not be read");
							continue;
						}
					};
					if let Some(change) = seen.lever(!lever.recording) {
						say(change);
					}
					if !lever.recording {
						continue;
					}
					let result = pass::run_once(&db, &runner, deadline, pass::Waits::SERVER, &cancel, &crate::mailbox::pass::now).await;
					if let Some(change) = result.as_ref().ok().and_then(|done| seen.runner(done.stopped)) {
						say(change);
					}
					log_pass(&result);
				}
			}
		}
	});
}

/// The pass's log line, when it did anything or stopped early: counts and
/// the kind of stop. Never a source, a recording or a round.
///
/// Every stop is an error, logged and counted on `operation_errors_total`
/// (`rust_rounds`, the stop's kind), every pass it lasts, but two: a
/// deadline reached after at least one program was asked about, which is a
/// warning, and no runner on this machine, which the recorder says once
/// when it changes ([`Seen::runner`]) and this does not repeat
/// ([`pass::Pass::failed`]).
pub(crate) fn log_pass(result: &Result<pass::Pass, sqlx::Error>) {
	let Ok(done) = result else {
		crate::metrics::instruments::record_error("rust_rounds", "storage");
		error!(error = "storage", "Rust Rounds pass failed");
		return;
	};
	match done.stopped {
		Some(pass::Stop::NoRunner) => {}
		Some(stop) if done.failed() => {
			crate::metrics::instruments::record_error("rust_rounds", stop.kind());
			error!(
				stop = stop.kind(),
				asked = done.asked,
				recorded = done.recorded,
				refused = done.refused,
				lost = done.lost,
				gave_up = done.gave_up,
				"Rust Rounds pass stopped: every program not yet recorded waits for the next pass"
			);
		}
		Some(stop) => warn!(
			stop = stop.kind(),
			asked = done.asked,
			recorded = done.recorded,
			refused = done.refused,
			"Rust Rounds pass ran out of time; what is left waits for the next"
		),
		None if done.asked > 0 => info!(recorded = done.recorded, refused = done.refused, cancelled = done.cancelled, "Rust Rounds pass"),
		None => {}
	}
}

#[cfg(test)]
mod tests {
	use super::{pass::Runner, pass::Stop, Change, Held, Seen};
	use crate::call_weeks::lane::Interactive;
	use rust_rounds_wire::protocol::{AskError, Request, Response};
	use std::sync::{Arc, Mutex};
	use std::time::Duration;

	/// A runner that notes whether interactive work was held at each ask.
	struct Watch {
		interactive: Interactive,
		held: Mutex<Vec<bool>>,
	}

	impl Runner for Watch {
		fn exchange(&self, _: &Request, _: Duration) -> Result<Response, AskError> {
			self.held.lock().unwrap().push(self.interactive.is_held());
			Err(AskError::TimedOut)
		}
	}

	#[test]
	fn a_program_recorded_is_interactive_work_and_the_toolchain_ask_is_not() {
		let interactive = Interactive::default();
		let watch = Arc::new(Watch {
			interactive: interactive.clone(),
			held: Mutex::new(Vec::new()),
		});
		let runner = Held {
			inner: Arc::clone(&watch),
			interactive: interactive.clone(),
		};
		let record = Request::Record {
			toolchain: String::from("1.0.0"),
			source: String::from("fn main() {}"),
		};
		for request in [Request::Toolchain {}, record] {
			assert!(runner.exchange(&request, Duration::from_secs(1)).is_err());
		}
		assert_eq!(*watch.held.lock().unwrap(), [false, true]);
		assert!(!interactive.is_held(), "let go once the program is recorded");
	}

	#[test]
	fn the_lever_is_said_when_it_moves_and_not_every_pass() {
		let mut seen = Seen::default();
		assert_eq!(seen.lever(false), None, "on from the start says nothing");
		assert_eq!(seen.lever(false), None);
		assert_eq!(seen.lever(true), Some(Change::Paused));
		assert_eq!(seen.lever(true), None);
		assert_eq!(seen.lever(false), Some(Change::Resumed));
		assert_eq!(Seen::default().lever(true), Some(Change::Paused), "paused from the start is said once");
	}

	#[test]
	fn no_runner_is_said_once_and_its_end_once() {
		let mut seen = Seen::default();
		assert_eq!(seen.runner(None), None);
		assert_eq!(seen.runner(Some(Stop::NoRunner)), Some(Change::NoRunner));
		assert_eq!(seen.runner(Some(Stop::NoRunner)), None, "not every pass");
		assert_eq!(
			seen.runner(Some(Stop::Unreachable)),
			Some(Change::RunnerFound),
			"a socket is back, though the runner is down"
		);
		assert_eq!(seen.runner(Some(Stop::Unreachable)), None);
		assert_eq!(
			Seen::default().runner(Some(Stop::NoRunner)),
			Some(Change::NoRunner),
			"a box with no runner says so at its first pass"
		);
	}
}

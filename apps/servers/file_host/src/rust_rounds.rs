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
//! so the worker asks the database every `RUST_ROUNDS_SECONDS` (60 by
//! default) for the programs the runner's toolchain has not recorded. An
//! idle pass is the runner's toolchain and one indexed read. No request
//! waits on it.
//!
//! What it promises is RP1–RP4 in `docs/leetype-execution.md`, summarised in
//! [`pass`]. **Nothing a program says reaches a log line**: a pass logs
//! counts and the kind of stop, never a source, a recording or a round.
//!
//! Off unless `RUST_ROUNDS_ENABLED`. On, the runner's socket must be mounted
//! where `RUST_ROUNDS_SOCKET` says; a runner that is not there is not a
//! startup failure but an outage each pass reports, since the runner
//! restarts on its own.

pub mod pass;

use crate::{AppState, Config};
use pass::Socket;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tracing::{error, info, warn};

/// The shortest interval `RUST_ROUNDS_SECONDS` may set.
pub const MIN_INTERVAL: Duration = Duration::from_secs(10);

/// The shortest pass deadline `RUST_ROUNDS_PASS_DEADLINE_SECONDS` may set:
/// the toolchain ask's wait and one program's (142 s).
///
/// A program's ask waits `ASK_TIMEOUT`, past the runner's watchdog deadline
/// for one job (its ceilings, clean-up, client deadlines and sweep
/// allowance, 132 s), so a job the watchdog ends is seen as lost (RP3); the
/// toolchain ask, `CLIENT_TIMEOUT`, comes first.
pub const MIN_DEADLINE: Duration = pass::Waits::SERVER.least_deadline();

/// Everything the worker needs, or `None` when the deployment has not turned
/// it on.
#[derive(Clone)]
pub struct RustRoundsContext {
	pub runner: Arc<Socket>,
	pub interval: Duration,
	/// How long one pass may take, every ask included.
	pub deadline: Duration,
}

impl RustRoundsContext {
	/// Validate the recorder's configuration once, at startup. Off is fine
	/// and logged.
	///
	/// # Errors
	/// `RUST_ROUNDS_ENABLED` with `RUST_ROUNDS_SECONDS` below 10 or
	/// `RUST_ROUNDS_PASS_DEADLINE_SECONDS` below [`MIN_DEADLINE`].
	pub fn from_config(config: &Config) -> anyhow::Result<Option<Self>> {
		if !config.rust_rounds_enabled {
			info!("Rust Rounds recording is not enabled; home keeps rounds without recording their programs");
			return Ok(None);
		}
		let interval = Duration::from_secs(config.rust_rounds_seconds);
		anyhow::ensure!(interval >= MIN_INTERVAL, "RUST_ROUNDS_SECONDS must be at least 10");
		let deadline = Duration::from_secs(config.rust_rounds_pass_deadline_seconds);
		anyhow::ensure!(
			deadline >= MIN_DEADLINE,
			"RUST_ROUNDS_PASS_DEADLINE_SECONDS must be at least 142: the toolchain's wait and one program's"
		);
		Ok(Some(Self {
			runner: Arc::new(Socket(PathBuf::from(&config.rust_rounds_socket))),
			interval,
			deadline,
		}))
	}
}

/// Start the worker if the deployment turned it on.
pub fn start(state: &AppState) {
	if let Some(context) = state.rust_rounds.clone() {
		spawn(state, context);
	}
}

/// Spawn the worker, cancelled through the shared token. Its first pass runs
/// at once, so a restart catches up without waiting an interval.
fn spawn(state: &AppState, context: RustRoundsContext) {
	let cancel = state.core.cancel_token.clone();
	let db = state.core.shared_db.clone();
	tokio::spawn(async move {
		info!(interval_secs = context.interval.as_secs(), "Rust Rounds recorder started");
		let mut ticker = tokio::time::interval(context.interval);
		ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
		loop {
			tokio::select! {
				() = cancel.cancelled() => {
					info!("Rust Rounds recorder cancelled");
					return;
				}
				_ = ticker.tick() => {
					let result = pass::run_once(&db, &context.runner, context.deadline, pass::Waits::SERVER, &cancel, &crate::mailbox::pass::now).await;
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
/// (`rust_rounds`, the stop's kind), every pass it lasts, but one: a
/// deadline reached after at least one program was asked about, which is a
/// warning ([`pass::Pass::failed`]).
pub(crate) fn log_pass(result: &Result<pass::Pass, sqlx::Error>) {
	let Ok(done) = result else {
		crate::metrics::instruments::record_error("rust_rounds", "storage");
		error!(error = "storage", "Rust Rounds pass failed");
		return;
	};
	match done.stopped {
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

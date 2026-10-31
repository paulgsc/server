//! One pass of Rust Rounds' recorder (paulgsc/some-ui#2234, RR3).
//!
//! The programs not yet recorded under the runner's toolchain, oldest first,
//! one at a time, each handed to the rust-runner over its socket and kept.
//!
//! RP1–RP4 in `docs/leetype-execution.md` are this module's invariants:
//!
//! - **RP1: never on a request's path, and `file_host` spawns no process.**
//!   The runner compiles and runs; a pass only asks it, on a blocking thread
//!   ([`Runner`]), from the background task `rust_rounds::start` spawns.
//! - **RP2: keyed to bytes and toolchain, read by range.** A pass asks the
//!   runner its toolchain, reads what waits under it
//!   (`rust_round_repo::waiting`, an indexed range read) and keeps each run
//!   under `key` of that toolchain and the source it sent, computed here.
//! - **RP3: an outage records nothing; a program that takes the runner down
//!   is recorded as that.** A runner that cannot be reached leaves every
//!   program waiting for the next pass, and says so on the log and on
//!   `operation_errors_total`. A runner that goes away while it holds a
//!   program (the whole request sent, no whole answer) is kept as a drop;
//!   at the [`DROPS`]th drop of one program the program is kept as
//!   `runner-restarted`, so one hostile program cannot take the runner down
//!   pass after pass, and the programs behind it are not held up for good.
//! - **RP4: bounded by [`BATCH`] and the pass deadline alone.** A pass takes
//!   at most `BATCH` programs, asks nothing once its deadline has passed,
//!   and gives each ask what is left of it. Any failure but a recording ends
//!   the pass: the next starts from the same program.

use rust_round_repo::{Ended, Kept, Waiting};
use rust_rounds_wire::protocol::{exchange, AskError, Refusal, Request, Response};
use rust_rounds_wire::recording::{key, release, RustcRun, Why};
use sqlx::SqlitePool;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

/// The most programs one pass records.
pub const BATCH: i64 = 20;

/// How many times the runner may go away while holding one program before
/// the program is kept as `runner-restarted` under that toolchain.
pub const DROPS: i64 = 2;

/// Who records a program: the rust-runner, asked one thing per connection.
/// A blocking call, made on a blocking thread.
pub trait Runner: Send + Sync + 'static {
	/// Ask `request`, waiting up to `timeout` for the answer.
	///
	/// # Errors
	/// [`AskError`], by how far the exchange got.
	fn exchange(&self, request: &Request, timeout: Duration) -> Result<Response, AskError>;
}

/// The runner on its Unix socket.
#[derive(Debug, Clone)]
pub struct Socket(pub PathBuf);

impl Runner for Socket {
	fn exchange(&self, request: &Request, timeout: Duration) -> Result<Response, AskError> {
		exchange(&self.0, request, timeout)
	}
}

/// Why a pass ended before it had nothing left to record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stop {
	/// The runner could not be reached, or never took the request: an outage.
	Unreachable,
	/// The runner went away while it held a program; kept as a drop.
	Dropped,
	/// The runner did not answer within what was left of the pass.
	TimedOut,
	/// The pass's deadline passed between programs.
	Deadline,
	/// The runner's toolchain is not the one this pass asked for: it was
	/// replaced mid-pass. The next pass asks again.
	ToolchainChanged,
	/// The runner refused the request (`malformed`, `too-large`, `peer`).
	Refused(Refusal),
	/// The runner answered something that is not an answer to what was asked.
	Invalid,
}

impl Stop {
	/// The stop's kind, as the log and `operation_errors_total` name it.
	#[must_use]
	pub const fn kind(self) -> &'static str {
		match self {
			Self::Unreachable => "runner_unreachable",
			Self::Dropped => "runner_dropped",
			Self::TimedOut => "runner_timed_out",
			Self::Deadline => "pass_deadline",
			Self::ToolchainChanged => "toolchain_changed",
			Self::Refused(_) => "runner_refused",
			Self::Invalid => "runner_invalid",
		}
	}
}

/// What a pass did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Pass {
	/// Programs the runner recorded and the pass kept.
	pub recorded: usize,
	/// Drops kept: the runner went away holding a program.
	pub dropped: usize,
	/// Programs kept as `runner-restarted`, at their last allowed drop.
	pub restarted: usize,
	/// Why it ended early, if it did.
	pub stopped: Option<Stop>,
	/// The server began shutting down during the pass.
	pub cancelled: bool,
}

/// Ask `runner` `request` on a blocking thread, waiting up to `timeout`.
async fn ask<R: Runner>(runner: &Arc<R>, request: Request, timeout: Duration) -> Result<Response, AskError> {
	let runner = Arc::clone(runner);
	tokio::task::spawn_blocking(move || runner.exchange(&request, timeout))
		.await
		.unwrap_or_else(|_| Err(AskError::NotSent(std::io::Error::other("the client's thread ended"))))
}

/// The stop an exchange that failed calls for.
const fn stop_of(err: &AskError) -> Stop {
	match err {
		AskError::Unreachable(_) | AskError::NotSent(_) => Stop::Unreachable,
		AskError::Dropped(_) => Stop::Dropped,
		AskError::TimedOut => Stop::TimedOut,
		AskError::Invalid(_) => Stop::Invalid,
	}
}

/// Records up to [`BATCH`] waiting programs with `runner`, within `deadline`.
///
/// # Errors
/// A storage failure, which ends the pass; what was kept before it stays
/// kept, and the next pass carries on from it.
pub async fn run_once<R: Runner>(
	db: &SqlitePool,
	runner: &Arc<R>,
	deadline: Duration,
	cancel: &CancellationToken,
	now: &(dyn Fn() -> String + Sync),
) -> Result<Pass, sqlx::Error> {
	let until = Instant::now() + deadline;
	let mut pass = Pass::default();
	let toolchain = match ask(runner, Request::Toolchain {}, deadline).await {
		Ok(Response::Toolchain { toolchain }) => toolchain,
		Ok(_) => {
			pass.stopped = Some(Stop::Invalid);
			return Ok(pass);
		}
		Err(err) => {
			pass.stopped = Some(stop_of(&err));
			return Ok(pass);
		}
	};
	let waiting = {
		let mut conn = db.acquire().await?;
		rust_round_repo::waiting(&mut conn, &toolchain, BATCH).await?
	};
	for program in waiting {
		if cancel.is_cancelled() {
			pass.cancelled = true;
			break;
		}
		let left = until.saturating_duration_since(Instant::now());
		if left.is_zero() {
			pass.stopped = Some(Stop::Deadline);
			break;
		}
		let request = Request::Record {
			toolchain: toolchain.clone(),
			source: program.source.clone(),
		};
		let stop = match ask(runner, request, left).await {
			Ok(Response::Recorded { recording, rustc }) if recording.toolchain == toolchain => {
				// Disallowed for tracing; this is the record itself, stored.
				#[allow(clippy::disallowed_methods)]
				let ended = Ended::Recorded {
					recording: serde_json::to_string(&recording).map_err(|err| sqlx::Error::Encode(Box::new(err)))?,
					rustc: serde_json::to_string(&rustc).map_err(|err| sqlx::Error::Encode(Box::new(err)))?,
				};
				if keep(db, &toolchain, &program, &ended, now).await? {
					pass.recorded += 1;
				}
				continue;
			}
			// A recording made under another toolchain is not this key's (RX4).
			Ok(Response::Recorded { .. } | Response::Toolchain { .. }) => Stop::Invalid,
			Ok(Response::Refused {
				reason: Refusal::WrongToolchain, ..
			}) => Stop::ToolchainChanged,
			Ok(Response::Refused { reason, .. }) => Stop::Refused(reason),
			Err(err @ AskError::Dropped(_)) => {
				let restarted = dropped(db, &toolchain, &program, now).await?;
				pass.dropped += 1;
				pass.restarted += usize::from(restarted);
				stop_of(&err)
			}
			Err(err) => stop_of(&err),
		};
		pass.stopped = Some(stop);
		break;
	}
	Ok(pass)
}

/// Keep how `program` ended, under the key of `toolchain` and its source,
/// computed here (RX4). `false` when it was no longer the next waiting.
async fn keep(db: &SqlitePool, toolchain: &str, program: &Waiting, ended: &Ended, now: &(dyn Fn() -> String + Sync)) -> Result<bool, sqlx::Error> {
	let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
	let kept = rust_round_repo::keep_run(&mut tx, toolchain, program.seq, &key(toolchain, &program.source), ended, &now()).await?;
	tx.commit().await?;
	Ok(matches!(kept, Kept::Added(_)))
}

/// Keep a drop of `program`, and at the [`DROPS`]th, the program as
/// `runner-restarted`, in one transaction. `true` when it was kept as that.
async fn dropped(db: &SqlitePool, toolchain: &str, program: &Waiting, now: &(dyn Fn() -> String + Sync)) -> Result<bool, sqlx::Error> {
	let now = now();
	let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
	let restarted = match rust_round_repo::keep_drop(&mut tx, toolchain, program.seq, &now).await? {
		Kept::Added(attempt) if attempt >= DROPS => {
			let rustc = RustcRun::Stopped {
				toolchain: release(toolchain).to_owned(),
				compile_stderr: String::new(),
				why: Why::Exit,
				stdout: String::new(),
				stderr: String::new(),
			};
			// Disallowed for tracing; this is the record itself, stored.
			#[allow(clippy::disallowed_methods)]
			let rustc = serde_json::to_string(&rustc).map_err(|err| sqlx::Error::Encode(Box::new(err)))?;
			let ended = Ended::RunnerRestarted { rustc };
			let kept = rust_round_repo::keep_run(&mut tx, toolchain, program.seq, &key(toolchain, &program.source), &ended, &now).await?;
			matches!(kept, Kept::Added(_))
		}
		Kept::Added(_) | Kept::OutOfOrder => false,
	};
	tx.commit().await?;
	Ok(restarted)
}

#[cfg(test)]
mod tests {
	use super::{run_once, Pass, Runner, Stop, BATCH, DROPS};
	use rust_round_repo::round::read;
	use rust_rounds_wire::protocol::{AskError, Refusal, Request, Response};
	use rust_rounds_wire::recording::{key, CompileRecord, Outcome, Recording, RunRecord, RustcRun};
	use serde_json::json;
	use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};
	use std::sync::{Arc, Mutex};
	use std::time::{Duration, Instant};
	use tokio_util::sync::CancellationToken;

	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");
	const T: &str = "rustc 1.97.0 (2d8144b78 2026-07-07); --edition 2024 -C opt-level=0";

	/// What the fake runner does with a program, by its source.
	#[derive(Clone, Copy)]
	enum Act {
		/// Records it: it prints its own source.
		Record,
		/// Goes away holding it.
		Drop,
		/// Cannot be reached at all.
		Down,
		/// Answers past any deadline.
		Hang,
	}

	/// The runner, in memory: it records what it is asked, and does with
	/// each program what `act` says, by the program's source.
	struct Fake {
		toolchain: Mutex<String>,
		act: fn(&str) -> Act,
		asked: Mutex<Vec<String>>,
	}

	impl Fake {
		fn new(act: fn(&str) -> Act) -> Arc<Self> {
			Arc::new(Self {
				toolchain: Mutex::new(String::from(T)),
				act,
				asked: Mutex::new(Vec::new()),
			})
		}

		fn asked(&self) -> Vec<String> {
			self.asked.lock().unwrap().clone()
		}
	}

	fn recording(toolchain: &str, source: &str) -> Recording {
		Recording {
			key: String::from("the runner's word, never taken"),
			toolchain: toolchain.to_owned(),
			outcome: Outcome::Ok,
			note: None,
			compile: CompileRecord {
				diagnostics: Vec::new(),
				rendered: String::new(),
				rendered_bytes: 0,
				timed_out: false,
				exit_code: Some(0),
				signal: None,
			},
			run: Some(RunRecord {
				stdout: source.to_owned(),
				stdout_bytes: 0,
				stderr: String::new(),
				stderr_bytes: 0,
				cut: false,
				exit_code: Some(0),
				signal: None,
				milliseconds: 1,
			}),
		}
	}

	impl Runner for Fake {
		fn exchange(&self, request: &Request, timeout: Duration) -> Result<Response, AskError> {
			let toolchain = self.toolchain.lock().unwrap().clone();
			let Request::Record { toolchain: asked_for, source } = request else {
				return if matches!((self.act)(""), Act::Down) {
					Err(AskError::Unreachable(std::io::Error::from(std::io::ErrorKind::NotFound)))
				} else {
					Ok(Response::Toolchain { toolchain })
				};
			};
			self.asked.lock().unwrap().push(source.clone());
			if *asked_for != toolchain {
				return Ok(Response::Refused {
					reason: Refusal::WrongToolchain,
					toolchain,
				});
			}
			match (self.act)(source) {
				Act::Record => {
					let recording = recording(&toolchain, source);
					Ok(Response::Recorded {
						rustc: recording.rustc_run(),
						recording: Box::new(recording),
					})
				}
				Act::Drop => Err(AskError::Dropped(std::io::Error::from(std::io::ErrorKind::UnexpectedEof))),
				Act::Down => Err(AskError::Unreachable(std::io::Error::from(std::io::ErrorKind::ConnectionRefused))),
				Act::Hang => {
					std::thread::sleep(timeout);
					Err(AskError::TimedOut)
				}
			}
		}
	}

	async fn pool() -> SqlitePool {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();
		pool
	}

	async fn round(pool: &SqlitePool, id: &str, programs: &[&str]) {
		let cards: Vec<_> = programs
			.iter()
			.enumerate()
			.map(|(n, program)| json!({ "kind": "read", "id": String::from("c") + &n.to_string(), "program": program }))
			.collect();
		let body = json!({ "schema": "some-ui.rust-round/1", "id": id, "cards": cards }).to_string();
		let mut tx = pool.begin().await.unwrap();
		rust_round_repo::put_round(&mut tx, &read(body.as_bytes()).unwrap(), "2026-10-31T00:00:00.000Z")
			.await
			.unwrap();
		tx.commit().await.unwrap();
	}

	async fn pass<R: Runner>(pool: &SqlitePool, runner: &Arc<R>, deadline: Duration) -> Pass {
		run_once(pool, runner, deadline, &CancellationToken::new(), &|| String::from("2026-10-31T01:00:00.000Z"))
			.await
			.unwrap()
	}

	/// `(source, ended, key, rustc)` of every run, in arrival order.
	async fn runs(pool: &SqlitePool) -> Vec<(String, String, String, String)> {
		sqlx::query_as(
			"SELECT source.source, run.ended, run.key, run.rustc FROM rust_round_run AS run
			 JOIN rust_round_source AS source ON source.seq = run.source_seq ORDER BY source.seq",
		)
		.fetch_all(pool)
		.await
		.unwrap()
	}

	const MINUTE: Duration = Duration::from_secs(60);

	/// Every program of a stored round is recorded in one pass, each distinct
	/// program once, under the key the pass computed; a second pass asks
	/// nothing, and a new toolchain records every program again.
	#[tokio::test]
	async fn a_stored_rounds_programs_are_recorded_by_the_runner_in_one_pass() {
		let pool = pool().await;
		round(&pool, "r-01", &["one", "two"]).await;
		round(&pool, "r-02", &["two", "three"]).await;
		let fake = Fake::new(|_| Act::Record);

		assert_eq!(pass(&pool, &fake, MINUTE).await, Pass { recorded: 3, ..Pass::default() });
		assert_eq!(fake.asked(), ["one", "two", "three"]);
		let kept = runs(&pool).await;
		for (source, ended, kept_key, rustc) in &kept {
			assert_eq!(ended, "recorded");
			assert_eq!(*kept_key, key(T, source), "RX4: the pass's own key, not the runner's word");
			let rustc: RustcRun = serde_json::from_str(rustc).unwrap();
			assert!(matches!(rustc, RustcRun::Prints { stdout, toolchain, .. } if stdout == *source && toolchain == "1.97.0"));
		}
		assert_eq!(kept.len(), 3);

		assert_eq!(pass(&pool, &fake, MINUTE).await, Pass::default());
		assert_eq!(fake.asked().len(), 3, "nothing recorded is asked again");

		*fake.toolchain.lock().unwrap() = String::from("rustc 1.98.0 (0 2026-08-21); --edition 2024 -C opt-level=0");
		assert_eq!(pass(&pool, &fake, MINUTE).await.recorded, 3, "a toolchain bump is a new key for every program");
		assert_eq!(runs(&pool).await.len(), 6);
	}

	/// A runner that cannot be reached records nothing and marks nothing:
	/// every program waits, and the next pass with the runner back records
	/// them all.
	#[tokio::test]
	async fn a_runner_that_is_down_leaves_every_program_waiting() {
		let pool = pool().await;
		round(&pool, "r-01", &["one", "two"]).await;
		let down = Fake::new(|_| Act::Down);
		for _ in 0..5 {
			let done = pass(&pool, &down, MINUTE).await;
			assert_eq!(done.stopped, Some(Stop::Unreachable));
			assert_eq!((done.recorded, done.dropped, done.restarted), (0, 0, 0));
		}
		let refused_mid_pass = Fake::new(|source| if source == "two" { Act::Down } else { Act::Record });
		let done = pass(&pool, &refused_mid_pass, MINUTE).await;
		assert_eq!((done.recorded, done.stopped), (1, Some(Stop::Unreachable)));
		let drops: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM rust_round_drop").fetch_one(&pool).await.unwrap();
		assert_eq!(drops, 0, "an outage is not a drop");

		let up = Fake::new(|_| Act::Record);
		assert_eq!(pass(&pool, &up, MINUTE).await.recorded, 1);
		assert!(runs(&pool).await.iter().all(|(_, ended, _, _)| ended == "recorded"));
	}

	/// A program the runner goes away holding is tried again a pass later,
	/// and at its `DROPS`th drop kept as `runner-restarted`; the programs
	/// behind it are then recorded.
	#[tokio::test]
	async fn a_program_that_takes_the_runner_down_is_kept_as_that() {
		let pool = pool().await;
		round(&pool, "r-01", &["fn main() { boom }", "after"]).await;
		let fake = Fake::new(|source| if source == "after" { Act::Record } else { Act::Drop });
		for attempt in 1..DROPS {
			let done = pass(&pool, &fake, MINUTE).await;
			assert_eq!((done.dropped, done.restarted, done.stopped), (1, 0, Some(Stop::Dropped)), "drop {attempt}");
			assert!(runs(&pool).await.is_empty());
		}
		let done = pass(&pool, &fake, MINUTE).await;
		assert_eq!((done.dropped, done.restarted, done.stopped), (1, 1, Some(Stop::Dropped)));
		let kept = runs(&pool).await;
		assert_eq!(kept[0].1, "runner-restarted");
		let rustc: serde_json::Value = serde_json::from_str(&kept[0].3).unwrap();
		assert_eq!(
			rustc,
			json!({ "outcome": "stopped", "toolchain": "1.97.0", "compileStderr": "", "why": "exit", "stdout": "", "stderr": "" })
		);

		assert_eq!(pass(&pool, &fake, MINUTE).await.recorded, 1, "the program behind it is recorded");
		assert_eq!(
			fake.asked().iter().filter(|source| *source == "fn main() { boom }").count(),
			usize::try_from(DROPS).unwrap()
		);
	}

	/// A pass records at most `BATCH` programs, and asks nothing past its
	/// deadline: a runner that does not answer ends the pass at it, and the
	/// program is asked again first next pass.
	#[tokio::test]
	async fn a_pass_is_bounded_by_its_batch_and_its_deadline() {
		let pool = pool().await;
		let sources: Vec<String> = (0..=BATCH).map(|n| String::from("p") + &n.to_string()).collect();
		round(&pool, "r-01", &sources.iter().map(String::as_str).collect::<Vec<_>>()).await;
		let fake = Fake::new(|_| Act::Record);
		assert_eq!(i64::try_from(pass(&pool, &fake, MINUTE).await.recorded).unwrap(), BATCH);
		assert_eq!(pass(&pool, &fake, MINUTE).await.recorded, 1);

		round(&pool, "r-02", &["slow", "behind"]).await;
		let hangs = Fake::new(|source| if source == "slow" { Act::Hang } else { Act::Record });
		let started = Instant::now();
		let done = pass(&pool, &hangs, Duration::from_millis(100)).await;
		assert!(started.elapsed() < Duration::from_secs(5));
		assert_eq!((done.recorded, done.stopped), (0, Some(Stop::TimedOut)));
		assert_eq!(hangs.asked(), ["slow"], "nothing is asked past the deadline");
		let drops: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM rust_round_drop").fetch_one(&pool).await.unwrap();
		assert_eq!(drops, 0, "our own deadline is not the runner's drop");
	}

	/// A runner whose toolchain changed mid-pass refuses the old one; the
	/// pass stops and keeps nothing under either.
	#[tokio::test]
	async fn a_toolchain_changed_mid_pass_keeps_nothing() {
		struct Swapped;
		impl Runner for Swapped {
			fn exchange(&self, request: &Request, _: Duration) -> Result<Response, AskError> {
				Ok(match request {
					Request::Toolchain {} => Response::Toolchain { toolchain: String::from(T) },
					Request::Record { .. } => Response::Refused {
						reason: Refusal::WrongToolchain,
						toolchain: String::from("rustc 1.98.0"),
					},
				})
			}
		}
		let pool = pool().await;
		round(&pool, "r-01", &["one"]).await;
		let done = pass(&pool, &Arc::new(Swapped), MINUTE).await;
		assert_eq!(done.stopped, Some(Stop::ToolchainChanged));
		assert!(runs(&pool).await.is_empty());
	}

	/// No program reaches a log line: a pass and its summary log counts and
	/// the kind of stop only.
	#[tokio::test]
	async fn no_program_reaches_a_log_line() {
		let (captured, _guard) = crate::privacy::capture();
		let pool = pool().await;
		round(&pool, "r-01", &["fn main() { secret_one }", "fn main() { secret_two }"]).await;
		let fake = Fake::new(|source| if source.contains("secret_two") { Act::Drop } else { Act::Record });
		let done = pass(&pool, &fake, MINUTE).await;
		assert_eq!((done.recorded, done.stopped), (1, Some(Stop::Dropped)));
		crate::rust_rounds::log_pass(&Ok(done));
		let lines = captured.lines();
		assert!(
			lines.iter().any(|line| line.contains("runner_dropped")),
			"sanity check: the summary was captured: {lines:?}"
		);
		assert!(lines.iter().all(|line| !line.contains("secret")), "{lines:?}");
	}
}

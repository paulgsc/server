//! One pass of Rust Rounds' recorder (paulgsc/some-ui#2234, RR3).
//!
//! The programs not yet recorded under the runner's toolchain, oldest first,
//! one at a time, each handed to the rust-runner over its socket and kept.
//!
//! RP1–RP4 in `docs/leetype-execution.md` are this module's invariants. The
//! rule for what is kept: **a program is blamed only for what it did.**
//!
//! - **RP1: never on a request's path, and `file_host` spawns no process.**
//!   The runner compiles and runs; a pass only asks it, on a blocking thread
//!   ([`Runner`]), from the background task `rust_rounds::start` spawns.
//! - **RP2: keyed to bytes and toolchain, read by range.** A pass asks the
//!   runner its toolchain, reads what waits under it
//!   (`rust_round_repo::waiting`, an indexed range read) and keeps each run
//!   under `key` of that toolchain and the source it sent, computed here,
//!   with the `RustcRun` derived from the recording, not the runner's word.
//! - **RP3: each outcome is the program's, or nobody's.** [`run_once`]'s
//!   table says what each answer keeps, and which ones stop the pass.
//! - **RP4: bounded by [`BATCH`] and the pass deadline alone.** A pass takes
//!   at most `BATCH` programs. It asks the toolchain only if that ask and one
//!   program's fit before its deadline, and a program only if its wait fits.

use rust_round_repo::{Ended, Kept, Lost, Waiting};
use rust_rounds_wire::protocol::{exchange, AskError, Refusal, Request, Response, ASK_TIMEOUT, CLIENT_TIMEOUT};
use rust_rounds_wire::recording::{key, release, RustcRun, Why};
use sqlx::SqlitePool;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

/// The most programs one pass records.
pub const BATCH: i64 = 20;

/// How many times the runner may be lost while holding one program (it went
/// away, or did not answer in time) before the program is kept as
/// `runner-lost` under that toolchain.
pub const DROPS: i64 = 2;

/// How long a pass waits on each kind of ask.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Waits {
	/// The toolchain ask: answered at once by a runner that is up.
	pub toolchain: Duration,
	/// A program's ask: past the runner's watchdog deadline.
	pub record: Duration,
}

impl Waits {
	/// The server's: [`CLIENT_TIMEOUT`] for the toolchain, [`ASK_TIMEOUT`]
	/// (past the watchdog's job deadline, RX3) for each program.
	pub const SERVER: Self = Self {
		toolchain: CLIENT_TIMEOUT,
		record: ASK_TIMEOUT,
	};

	/// The shortest deadline in which a pass can ask one program: the
	/// toolchain's wait and one program's.
	#[must_use]
	pub const fn least_deadline(self) -> Duration {
		self.toolchain.saturating_add(self.record)
	}
}

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
	/// An outage: the runner could not be reached, never took the whole
	/// request, or did not answer the toolchain ask in time.
	Unreachable,
	/// An outage: the runner said `unavailable`, a fault of its own machine
	/// before the program ran.
	Unavailable,
	/// An outage: the runner refused this server (`peer`, its uid).
	Refused(Refusal),
	/// An outage: the runner answered something that is not an answer to
	/// what was asked.
	Invalid,
	/// The runner's toolchain is not the one this pass asked for: it was
	/// replaced mid-pass. The next pass asks again.
	ToolchainChanged,
	/// The runner was lost while holding a program: kept against it.
	Lost(Lost),
	/// Less than one program's wait was left before the pass's deadline.
	Deadline,
}

impl Stop {
	/// The stop's kind, as the log and `operation_errors_total` name it.
	#[must_use]
	pub const fn kind(self) -> &'static str {
		match self {
			Self::Unreachable => "runner_unreachable",
			Self::Unavailable => "runner_unavailable",
			Self::Refused(_) => "runner_refused",
			Self::Invalid => "runner_invalid",
			Self::ToolchainChanged => "toolchain_changed",
			Self::Lost(Lost::Dropped) => "runner_dropped",
			Self::Lost(Lost::TimedOut) => "runner_timed_out",
			Self::Deadline => "pass_deadline",
		}
	}
}

/// What a pass did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Pass {
	/// Programs asked about.
	pub asked: usize,
	/// Programs the runner recorded and the pass kept.
	pub recorded: usize,
	/// Programs the runner refused, kept as `refused`.
	pub refused: usize,
	/// Times the runner was lost holding a program, each kept against it.
	pub lost: usize,
	/// Programs kept as `runner-lost`, at their last allowed loss.
	pub gave_up: usize,
	/// Why it ended early, if it did.
	pub stopped: Option<Stop>,
	/// The server began shutting down during the pass.
	pub cancelled: bool,
}

impl Pass {
	/// Whether the pass's stop is an error, logged and counted: every stop
	/// but a deadline reached after at least one program was asked about.
	#[must_use]
	pub const fn failed(&self) -> bool {
		match self.stopped {
			None => false,
			Some(Stop::Deadline) => self.asked == 0,
			Some(_) => true,
		}
	}
}

/// Ask `runner` `request` on a blocking thread, waiting up to `timeout`.
async fn ask<R: Runner>(runner: &Arc<R>, request: Request, timeout: Duration) -> Result<Response, AskError> {
	let runner = Arc::clone(runner);
	tokio::task::spawn_blocking(move || runner.exchange(&request, timeout))
		.await
		.unwrap_or_else(|_| Err(AskError::NotSent(std::io::Error::other("the client's thread ended"))))
}

/// Records up to [`BATCH`] waiting programs with `runner`, within `deadline`,
/// waiting on each ask as `waits` says ([`Waits::SERVER`] in the server).
///
/// The toolchain is asked first, only if its wait and one program's fit
/// before the deadline; then each program in arrival order, only if its wait
/// fits. Each program's answer:
///
/// | The runner | Kept | The pass |
/// | --- | --- | --- |
/// | recorded it (under the toolchain asked) | `recorded` | goes on |
/// | refused it, `malformed` or `too-large` | `refused`, with the reason | goes on |
/// | went away holding it (`Dropped`) | a loss (`dropped`); at the [`DROPS`]th, `runner-lost` | stops |
/// | did not answer within its wait (`TimedOut`, past the watchdog) | a loss (`timed-out`); at the [`DROPS`]th, `runner-lost` | stops |
/// | refused it, `wrong-toolchain` | nothing | stops: [`Stop::ToolchainChanged`] |
/// | refused it, `unavailable` | nothing | stops: [`Stop::Unavailable`] |
/// | refused it, `peer` | nothing | stops: [`Stop::Refused`] |
/// | could not be reached, or did not take the request | nothing | stops: [`Stop::Unreachable`] |
/// | answered something else, or a recording under another toolchain | nothing | stops: [`Stop::Invalid`] |
///
/// The toolchain ask keeps nothing whatever happens: any failure is an
/// outage. A stopped pass leaves the rest waiting, and the next starts from
/// the first program waiting.
///
/// # Errors
/// A storage failure, which ends the pass; what was kept before it stays
/// kept, and the next pass carries on from it.
pub async fn run_once<R: Runner>(
	db: &SqlitePool,
	runner: &Arc<R>,
	deadline: Duration,
	waits: Waits,
	cancel: &CancellationToken,
	now: &(dyn Fn() -> String + Sync),
) -> Result<Pass, sqlx::Error> {
	let until = Instant::now() + deadline;
	let mut pass = Pass::default();
	let left = || until.saturating_duration_since(Instant::now());
	if deadline < waits.least_deadline() {
		pass.stopped = Some(Stop::Deadline);
		return Ok(pass);
	}
	let toolchain = match ask(runner, Request::Toolchain {}, waits.toolchain).await {
		Ok(Response::Toolchain { toolchain }) => toolchain,
		Ok(Response::Refused { reason: Refusal::Unavailable, .. }) => return Ok(stopped(pass, Stop::Unavailable)),
		Ok(Response::Refused { reason, .. }) => return Ok(stopped(pass, Stop::Refused(reason))),
		Ok(Response::Recorded { .. }) | Err(AskError::Invalid(_)) => return Ok(stopped(pass, Stop::Invalid)),
		Err(AskError::Unreachable(_) | AskError::NotSent(_) | AskError::Dropped(_) | AskError::TimedOut) => return Ok(stopped(pass, Stop::Unreachable)),
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
		if left() < waits.record {
			pass.stopped = Some(Stop::Deadline);
			break;
		}
		pass.asked += 1;
		let request = Request::Record {
			toolchain: toolchain.clone(),
			source: program.source.clone(),
		};
		let stop = match ask(runner, request, waits.record).await {
			Ok(Response::Recorded { recording, .. }) if recording.toolchain == toolchain => {
				// What the phone reads is derived here from the recording,
				// not taken from the runner's word (RX4).
				// Disallowed for tracing; this is the record itself, stored.
				#[allow(clippy::disallowed_methods)]
				let ended = Ended::Recorded {
					recording: serde_json::to_string(&recording).map_err(encode)?,
					rustc: serde_json::to_string(&recording.rustc_run()).map_err(encode)?,
				};
				pass.recorded += usize::from(keep(db, &toolchain, &program, &ended, now).await?);
				continue;
			}
			Ok(Response::Refused {
				reason: reason @ (Refusal::Malformed | Refusal::TooLarge),
				..
			}) => {
				let ended = Ended::Refused {
					reason: reason.as_str().to_owned(),
					rustc: stopped_run(&toolchain)?,
				};
				pass.refused += usize::from(keep(db, &toolchain, &program, &ended, now).await?);
				continue;
			}
			Ok(Response::Refused {
				reason: Refusal::WrongToolchain, ..
			}) => Stop::ToolchainChanged,
			Ok(Response::Refused { reason: Refusal::Unavailable, .. }) => Stop::Unavailable,
			Ok(Response::Refused {
				reason: reason @ Refusal::Peer, ..
			}) => Stop::Refused(reason),
			// A recording made under another toolchain is not this key's (RX4).
			Ok(Response::Recorded { .. } | Response::Toolchain { .. }) | Err(AskError::Invalid(_)) => Stop::Invalid,
			Err(AskError::Unreachable(_) | AskError::NotSent(_)) => Stop::Unreachable,
			Err(AskError::Dropped(_)) => lose(db, &toolchain, &program, Lost::Dropped, &mut pass, now).await?,
			Err(AskError::TimedOut) => lose(db, &toolchain, &program, Lost::TimedOut, &mut pass, now).await?,
		};
		pass.stopped = Some(stop);
		break;
	}
	Ok(pass)
}

const fn stopped(mut pass: Pass, stop: Stop) -> Pass {
	pass.stopped = Some(stop);
	pass
}

fn encode(err: serde_json::Error) -> sqlx::Error {
	sqlx::Error::Encode(Box::new(err))
}

/// What the phone reads of a program that never ran to a recording: a
/// `stopped` run (`exit`) that printed nothing.
fn stopped_run(toolchain: &str) -> Result<String, sqlx::Error> {
	let rustc = RustcRun::Stopped {
		toolchain: release(toolchain).to_owned(),
		compile_stderr: String::new(),
		why: Why::Exit,
		stdout: String::new(),
		stderr: String::new(),
	};
	// Disallowed for tracing; this is the record itself, stored.
	#[allow(clippy::disallowed_methods)]
	serde_json::to_string(&rustc).map_err(encode)
}

/// Keep how `program` ended, under the key of `toolchain` and its source,
/// computed here (RX4). `false` when it was no longer the next waiting.
async fn keep(db: &SqlitePool, toolchain: &str, program: &Waiting, ended: &Ended, now: &(dyn Fn() -> String + Sync)) -> Result<bool, sqlx::Error> {
	let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
	let kept = rust_round_repo::keep_run(&mut tx, toolchain, program.seq, &key(toolchain, &program.source), ended, &now()).await?;
	tx.commit().await?;
	Ok(matches!(kept, Kept::Added(_)))
}

/// Keep a loss of `program` (`how`), and at the [`DROPS`]th the program as
/// `runner-lost`, in one transaction; count both on `pass`, and say the
/// stop.
async fn lose(db: &SqlitePool, toolchain: &str, program: &Waiting, how: Lost, pass: &mut Pass, now: &(dyn Fn() -> String + Sync)) -> Result<Stop, sqlx::Error> {
	let now = now();
	let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
	let kept = rust_round_repo::keep_drop(&mut tx, toolchain, program.seq, how, &now).await?;
	if let Kept::Added(attempt) = kept {
		pass.lost += 1;
		if attempt >= DROPS {
			let ended = Ended::RunnerLost { rustc: stopped_run(toolchain)? };
			let gave_up = rust_round_repo::keep_run(&mut tx, toolchain, program.seq, &key(toolchain, &program.source), &ended, &now).await?;
			pass.gave_up += usize::from(matches!(gave_up, Kept::Added(_)));
		}
	}
	tx.commit().await?;
	Ok(Stop::Lost(how))
}

#[cfg(test)]
mod tests {
	use super::{run_once, Pass, Runner, Stop, Waits, BATCH, DROPS};
	use rust_round_repo::round::read;
	use rust_round_repo::Lost;
	use rust_rounds_wire::protocol::{AskError, Refusal, Request, Response};
	use rust_rounds_wire::recording::{key, CompileRecord, Outcome, Recording, RunRecord, RustcRun};
	use serde_json::json;
	use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};
	use std::sync::{Arc, Mutex};
	use std::time::{Duration, Instant};
	use tokio_util::sync::CancellationToken;

	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");
	const T: &str = "rustc 1.97.0 (2d8144b78 2026-07-07); --edition 2024 -C opt-level=0";

	/// What the fake runner does with a program, by its source; `""` is the
	/// toolchain ask.
	#[derive(Clone, Copy)]
	enum Act {
		/// Records it: it prints its own source.
		Record,
		/// Refuses it as too large.
		Refuse,
		/// Goes away holding it.
		Drop,
		/// Answers past any wait.
		Hang,
		/// Cannot be reached at all.
		Down,
		/// Says `unavailable`: its own machine failed.
		Unavailable,
		/// Answers something that is not an answer.
		Garble,
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

	fn act(act: Act, toolchain: &str, timeout: Duration) -> Result<Response, AskError> {
		let refused = |reason| {
			Ok(Response::Refused {
				reason,
				toolchain: toolchain.to_owned(),
			})
		};
		match act {
			Act::Record => Ok(Response::Toolchain { toolchain: toolchain.to_owned() }),
			Act::Refuse => refused(Refusal::TooLarge),
			Act::Drop => Err(AskError::Dropped(std::io::Error::from(std::io::ErrorKind::UnexpectedEof))),
			Act::Hang => {
				std::thread::sleep(timeout);
				Err(AskError::TimedOut)
			}
			Act::Down => Err(AskError::Unreachable(std::io::Error::from(std::io::ErrorKind::ConnectionRefused))),
			Act::Unavailable => refused(Refusal::Unavailable),
			Act::Garble => Err(AskError::Invalid(std::io::Error::from(std::io::ErrorKind::InvalidData))),
		}
	}

	impl Runner for Fake {
		fn exchange(&self, request: &Request, timeout: Duration) -> Result<Response, AskError> {
			let toolchain = self.toolchain.lock().unwrap().clone();
			let Request::Record { toolchain: asked_for, source } = request else {
				return act((self.act)(""), &toolchain, timeout);
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
					// The runner's word for what the phone reads, which the
					// pass must not take (RX4): it derives it.
					Ok(Response::Recorded {
						rustc: RustcRun::Panics {
							toolchain: String::from("a lie"),
							compile_stderr: String::new(),
							stdout: String::new(),
							stderr: String::new(),
						},
						recording: Box::new(recording),
					})
				}
				other => act(other, &toolchain, timeout),
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
		rust_round_repo::put_round(&mut tx, "subject-a", &read(body.as_bytes()).unwrap(), "2026-10-31T00:00:00.000Z")
			.await
			.unwrap();
		tx.commit().await.unwrap();
	}

	const MINUTE: Duration = Duration::from_secs(60);

	/// Short waits, for a runner that hangs.
	const SHORT: Waits = Waits {
		toolchain: Duration::from_millis(50),
		record: Duration::from_millis(100),
	};

	async fn pass_within<R: Runner>(pool: &SqlitePool, runner: &Arc<R>, deadline: Duration, waits: Waits) -> Pass {
		run_once(pool, runner, deadline, waits, &CancellationToken::new(), &|| String::from("2026-10-31T01:00:00.000Z"))
			.await
			.unwrap()
	}

	/// A pass with the server's waits and a deadline of a few minutes.
	async fn pass<R: Runner>(pool: &SqlitePool, runner: &Arc<R>) -> Pass {
		pass_within(pool, runner, Waits::SERVER.least_deadline() + MINUTE, Waits::SERVER).await
	}

	/// `(source, ended, refusal, key, rustc)` of every run, in arrival order.
	async fn runs(pool: &SqlitePool) -> Vec<(String, String, Option<String>, String, String)> {
		sqlx::query_as(
			"SELECT source.source, run.ended, run.refusal, run.key, run.rustc FROM rust_round_run AS run
			 JOIN rust_round_source AS source ON source.seq = run.source_seq ORDER BY source.seq",
		)
		.fetch_all(pool)
		.await
		.unwrap()
	}

	/// `(attempt, kind)` of every loss kept.
	async fn losses(pool: &SqlitePool) -> Vec<(i64, String)> {
		sqlx::query_as("SELECT attempt, kind FROM rust_round_drop ORDER BY source_seq, attempt")
			.fetch_all(pool)
			.await
			.unwrap()
	}

	/// Every program of a stored round is recorded in one pass, each distinct
	/// program once, under the key the pass computed and with what the phone
	/// reads derived from the recording; a second pass asks nothing, and a
	/// new toolchain records every program again.
	#[tokio::test]
	async fn a_stored_rounds_programs_are_recorded_by_the_runner_in_one_pass() {
		let pool = pool().await;
		round(&pool, "r-01", &["one", "two"]).await;
		round(&pool, "r-02", &["two", "three"]).await;
		let fake = Fake::new(|_| Act::Record);

		let done = pass(&pool, &fake).await;
		assert_eq!((done.asked, done.recorded, done.stopped), (3, 3, None));
		assert_eq!(fake.asked(), ["one", "two", "three"]);
		let kept = runs(&pool).await;
		for (source, ended, refusal, kept_key, rustc) in &kept {
			assert_eq!((ended.as_str(), refusal), ("recorded", &None));
			assert_eq!(*kept_key, key(T, source), "RX4: the pass's own key, not the runner's word");
			let rustc: RustcRun = serde_json::from_str(rustc).unwrap();
			assert!(
				matches!(&rustc, RustcRun::Prints { stdout, toolchain, .. } if stdout == source && toolchain == "1.97.0"),
				"derived from the recording, not the runner's claim: {rustc:?}"
			);
		}
		assert_eq!(kept.len(), 3);

		assert_eq!(pass(&pool, &fake).await, Pass::default());
		assert_eq!(fake.asked().len(), 3, "nothing recorded is asked again");

		*fake.toolchain.lock().unwrap() = String::from("rustc 1.98.0 (0 2026-08-21); --edition 2024 -C opt-level=0");
		assert_eq!(pass(&pool, &fake).await.recorded, 3, "a toolchain bump is a new key for every program");
		assert_eq!(runs(&pool).await.len(), 6);
	}

	/// A program the runner refuses is kept as `refused`, with the reason,
	/// and the programs behind it are recorded in the same pass.
	#[tokio::test]
	async fn a_refused_program_is_kept_as_that_and_the_queue_moves_on() {
		let pool = pool().await;
		round(&pool, "r-01", &["huge", "after"]).await;
		let fake = Fake::new(|source| if source == "huge" { Act::Refuse } else { Act::Record });
		let done = pass(&pool, &fake).await;
		assert_eq!((done.refused, done.recorded, done.stopped), (1, 1, None));
		assert!(!done.failed());
		let kept = runs(&pool).await;
		assert_eq!((kept[0].1.as_str(), kept[0].2.as_deref()), ("refused", Some("too-large")));
		let rustc: serde_json::Value = serde_json::from_str(&kept[0].4).unwrap();
		assert_eq!(rustc["outcome"], "stopped");
		assert_eq!(kept[1].1, "recorded");
		assert_eq!(pass(&pool, &fake).await, Pass::default(), "a refused program is not asked again");
	}

	/// An outage keeps nothing and marks nothing: a runner that cannot be
	/// reached, that says `unavailable` (its own machine failed before the
	/// program ran) or that answers nonsense, whether to the toolchain ask
	/// or a program's. Every program waits, each such pass fails, and the
	/// next pass with the runner back records them all.
	#[tokio::test]
	async fn an_outage_keeps_nothing_and_every_program_waits() {
		type Acts = fn(&str) -> Act;
		let pool = pool().await;
		round(&pool, "r-01", &["one", "two"]).await;
		let outages: [(Acts, Stop); 6] = [
			(|_| Act::Down, Stop::Unreachable),
			(|_| Act::Unavailable, Stop::Unavailable),
			(|_| Act::Garble, Stop::Invalid),
			(|source| if source == "two" { Act::Down } else { Act::Record }, Stop::Unreachable),
			(|source| if source == "two" { Act::Unavailable } else { Act::Record }, Stop::Unavailable),
			(|source| if source == "two" { Act::Garble } else { Act::Record }, Stop::Invalid),
		];
		for (act, stop) in outages {
			for _ in 0..3 {
				let done = pass(&pool, &Fake::new(act)).await;
				assert_eq!(done.stopped, Some(stop));
				assert!(done.failed(), "{stop:?} is an error");
				assert_eq!((done.refused, done.lost, done.gave_up), (0, 0, 0));
			}
		}
		assert_eq!(runs(&pool).await.len(), 1, "only `one`, recorded on the way to `two`");
		assert!(losses(&pool).await.is_empty(), "an outage is never held against a program");

		assert_eq!(pass(&pool, &Fake::new(|_| Act::Record)).await.recorded, 1);
		assert!(runs(&pool).await.iter().all(|run| run.1 == "recorded"));
	}

	/// A program the runner is lost holding (it went away, or did not answer
	/// within a wait that outlasts its watchdog) is held against it: tried
	/// again a pass later, and at its `DROPS`th loss, of either kind, kept as
	/// `runner-lost`. The programs behind it are then recorded.
	#[tokio::test]
	async fn a_program_that_loses_the_runner_is_kept_as_that() {
		let pool = pool().await;
		round(&pool, "r-01", &["fn main() { boom }", "fn main() { hang }", "after"]).await;
		let fake = Fake::new(|source| match source {
			"fn main() { boom }" => Act::Drop,
			"fn main() { hang }" => Act::Hang,
			_ => Act::Record,
		});
		let deadline = Duration::from_secs(5);
		for attempt in 1..DROPS {
			let done = pass_within(&pool, &fake, deadline, SHORT).await;
			assert_eq!((done.lost, done.gave_up, done.stopped), (1, 0, Some(Stop::Lost(Lost::Dropped))), "loss {attempt}");
			assert!(done.failed());
			assert!(runs(&pool).await.is_empty());
		}
		let done = pass_within(&pool, &fake, deadline, SHORT).await;
		assert_eq!((done.lost, done.gave_up), (1, 1));
		let kept = runs(&pool).await;
		assert_eq!(kept[0].1, "runner-lost");
		let rustc: serde_json::Value = serde_json::from_str(&kept[0].4).unwrap();
		assert_eq!(
			rustc,
			json!({ "outcome": "stopped", "toolchain": "1.97.0", "compileStderr": "", "why": "exit", "stdout": "", "stderr": "" })
		);

		for _ in 0..DROPS {
			let done = pass_within(&pool, &fake, deadline, SHORT).await;
			assert_eq!((done.lost, done.stopped), (1, Some(Stop::Lost(Lost::TimedOut))), "a hang past the wait is a loss");
			assert!(done.failed());
		}
		assert_eq!(runs(&pool).await[1].1, "runner-lost");
		let kinds: Vec<String> = losses(&pool).await.into_iter().map(|loss| loss.1).collect();
		assert_eq!(kinds, ["dropped", "dropped", "timed-out", "timed-out"]);

		assert_eq!(pass_within(&pool, &fake, deadline, SHORT).await.recorded, 1, "the program behind them is recorded");
		assert_eq!(
			fake.asked().iter().filter(|source| *source == "fn main() { boom }").count(),
			usize::try_from(DROPS).unwrap()
		);
	}

	/// A pass records at most `BATCH` programs, asks a program only while its
	/// wait fits before the deadline, and asks nothing in a deadline too
	/// short for the toolchain's wait and one program's, which is an error.
	/// At exactly the shortest deadline the server allows, a program is
	/// recorded.
	#[tokio::test]
	async fn a_pass_is_bounded_by_its_batch_and_its_deadline() {
		let pool = pool().await;
		let sources: Vec<String> = (0..=BATCH).map(|n| String::from("p") + &n.to_string()).collect();
		round(&pool, "r-01", &sources.iter().map(String::as_str).collect::<Vec<_>>()).await;
		let fake = Fake::new(|_| Act::Record);
		assert_eq!(i64::try_from(pass(&pool, &fake).await.recorded).unwrap(), BATCH);
		assert_eq!(pass(&pool, &fake).await.recorded, 1);

		round(&pool, "r-02", &["at-the-minimum"]).await;
		let at_least = pass_within(&pool, &fake, crate::rust_rounds::MIN_DEADLINE, Waits::SERVER).await;
		assert_eq!((at_least.recorded, at_least.stopped), (1, None), "the shortest deadline allowed records a program");

		round(&pool, "r-03", &["too-late"]).await;
		let short = pass_within(&pool, &fake, crate::rust_rounds::MIN_DEADLINE.saturating_sub(Duration::from_millis(1)), Waits::SERVER).await;
		assert_eq!((short.asked, short.stopped), (0, Some(Stop::Deadline)));
		assert!(short.failed(), "a pass that could ask nothing is an error");

		let hangs = Fake::new(|source| if source == "too-late" { Act::Hang } else { Act::Record });
		let started = Instant::now();
		let done = pass_within(&pool, &hangs, Duration::from_millis(200), SHORT).await;
		assert!(started.elapsed() < Duration::from_secs(5));
		assert_eq!((done.asked, done.stopped), (1, Some(Stop::Lost(Lost::TimedOut))));
		assert_eq!(hangs.asked(), ["too-late"], "nothing is asked past the deadline");
	}

	/// Running out of time after asking about a program is a warning, not an
	/// error: what is left waits for the next pass.
	#[tokio::test]
	async fn a_deadline_reached_after_work_is_no_error() {
		struct Slow;
		impl Runner for Slow {
			fn exchange(&self, request: &Request, _: Duration) -> Result<Response, AskError> {
				let Request::Record { source, .. } = request else {
					return Ok(Response::Toolchain { toolchain: String::from(T) });
				};
				std::thread::sleep(Duration::from_millis(150));
				let recording = recording(T, source);
				Ok(Response::Recorded {
					rustc: recording.rustc_run(),
					recording: Box::new(recording),
				})
			}
		}
		let pool = pool().await;
		round(&pool, "r-01", &["slow", "behind"]).await;
		let done = pass_within(&pool, &Arc::new(Slow), Duration::from_millis(230), SHORT).await;
		assert_eq!((done.recorded, done.stopped), (1, Some(Stop::Deadline)));
		assert!(!done.failed());
	}

	/// A runner whose toolchain changed mid-pass refuses the old one; the
	/// pass stops, keeps nothing under either, and fails.
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
		let done = pass(&pool, &Arc::new(Swapped)).await;
		assert_eq!(done.stopped, Some(Stop::ToolchainChanged));
		assert!(done.failed());
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
		let done = pass(&pool, &fake).await;
		assert_eq!((done.recorded, done.stopped), (1, Some(Stop::Lost(Lost::Dropped))));
		crate::rust_rounds::log_pass(&Ok(done));
		let lines = captured.lines();
		assert!(
			lines.iter().any(|line| line.contains("runner_dropped")),
			"sanity check: the summary was captured: {lines:?}"
		);
		assert!(lines.iter().all(|line| !line.contains("secret")), "{lines:?}");
	}
}

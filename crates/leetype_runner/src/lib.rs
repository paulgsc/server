//! Compiles and runs a `leetype` round's programs, offline, and records one
//! `RunResult` per program and constraint set (#381, LTY-EXEC).
//!
//! For a round body carrying a `harness`, [`Runner::record`] builds `A` and
//! every `A + d` ([`round::programs`]), compiles each with
//! `rustc --edition 2021 -C opt-level=0`, and runs each binary twice: with
//! `dimension=bound` for every constraint of `constraintDiff.before`, then of
//! `constraintDiff.after`. Every run yields the client's `RunResult`
//! ([`leetype_round_repo::RunResult`]) exactly, inside a
//! [`leetype_round_repo::RecordedRun`] that also names the variant, the
//! constraint set and every size.
//!
//! **Who calls it.** Only the `record-leetype-runs` command and its tests —
//! on a developer machine or in CI, where `rustc` is installed. The server
//! never links a call to it into a request path: the route and the static
//! snapshot serve what this recorded. The design, the limits and the options
//! rejected are in `apps/servers/file_host/docs/leetype-execution.md`.
//!
//! **What it never does** (#381's nevers, as they apply to a library):
//! it takes no source but a stored round body's, so there is no caller-supplied
//! code to run (never #1); it never chooses a round (never #2); nothing it
//! returns carries a complexity claim — a result is what one program printed
//! at one set of sizes, or which way it failed (never #3); and it knows
//! nothing of `StudySignal`, the ledger, the sampler or the nudge, and does
//! not depend on a crate that does (never #5).
//!
//! **Limits** ([`Limits`], every one configurable): a compile timeout, a
//! wall-clock ceiling per run (past it the run is `budget-exceeded` and its
//! process group is killed and reaped), a ceiling on stdout and on stderr
//! (kept to the ceiling, never streamed; the truncation is noted), an
//! address-space ceiling applied through `prlimit` when it is installed, a
//! fresh temporary directory per round, and a cleared environment.

mod process;
pub mod recorder;
pub mod round;

use crate::process::{describe, resolve, supervise, Captured, Ending, Finished};
use crate::round::{programs, Constraint, ProgramError, RunnableRound};
use leetype_round_repo::{Bounds, Elapsed, ErrorClass, ExecutionError, Observation, RecordedRun, RunResult};
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

pub use recorder::{record_listed, RecordReport, RecorderError, RoundOutcome, Selection};

/// How long `rustc` may take over one program before the compile counts as
/// failed.
pub const DEFAULT_COMPILE_TIMEOUT: Duration = Duration::from_secs(60);
/// The wall-clock ceiling on one run.
///
/// See the design note for why 2 s at `opt-level=0`: every inadmissible
/// variant in the corpus exceeds it at `C′` and every admissible one finishes
/// in tens of milliseconds.
pub const DEFAULT_RUN_CEILING: Duration = Duration::from_millis(2_000);
/// The most stdout a run keeps.
pub const DEFAULT_OUTPUT_CEILING: usize = 4 * 1024;
/// The most stderr a run (or a failed compile) keeps.
pub const DEFAULT_LOGS_CEILING: usize = 4 * 1024;
/// The address-space ceiling on one run, when `prlimit` is installed.
pub const DEFAULT_MEMORY_CEILING: u64 = 512 * 1024 * 1024;

/// Everything the runner bounds, each with a default.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Limits {
	/// The compiler: a path, or a name looked up on `PATH`.
	pub rustc: PathBuf,
	pub compile_timeout: Duration,
	pub run_ceiling: Duration,
	pub output_ceiling: usize,
	pub logs_ceiling: usize,
	/// `None` for no address-space ceiling.
	pub memory_ceiling: Option<u64>,
}

impl Default for Limits {
	fn default() -> Self {
		Self {
			rustc: PathBuf::from("rustc"),
			compile_timeout: DEFAULT_COMPILE_TIMEOUT,
			run_ceiling: DEFAULT_RUN_CEILING,
			output_ceiling: DEFAULT_OUTPUT_CEILING,
			logs_ceiling: DEFAULT_LOGS_CEILING,
			memory_ceiling: Some(DEFAULT_MEMORY_CEILING),
		}
	}
}

/// Why a round was not recorded. A program that fails to compile, panics or
/// runs out of time is **not** one of these: that is a recorded
/// [`RunResult`]. These are the round or the machine being wrong.
#[derive(Debug)]
pub enum RecordError {
	/// The body is not a round the runner can read.
	NotARound(serde_json::Error),
	/// The round has no harness, so there is nothing to run it with.
	NoHarness,
	/// A constraint set is empty, so a run would have no input size.
	NoConstraints(&'static str),
	/// A hunk does not apply to `A`.
	Program(ProgramError),
	/// The compiler could not be found or started. Not the round's fault:
	/// a run of the recorder stops here.
	Toolchain(PathBuf, std::io::Error),
	/// The temporary directory, or a program's file in it.
	Io(std::io::Error),
}

impl RecordError {
	/// Whether this is the machine's fault rather than the round's, so no
	/// other round would fare better.
	#[must_use]
	pub const fn is_toolchain(&self) -> bool {
		matches!(self, Self::Toolchain(..) | Self::Io(_))
	}
}

impl std::fmt::Display for RecordError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::NotARound(err) => write!(f, "not a runnable round: {err}"),
			Self::NoHarness => f.write_str("the round has no harness"),
			Self::NoConstraints(set) => write!(f, "{set} is empty, so a run has no input size"),
			Self::Program(err) => err.fmt(f),
			Self::Toolchain(rustc, err) => write!(f, "could not run {}: {err}", rustc.display()),
			Self::Io(err) => write!(f, "could not prepare the programs: {err}"),
		}
	}
}

impl std::error::Error for RecordError {}

/// The runner, with its compiler (and `prlimit`, if any) found.
#[derive(Debug, Clone)]
pub struct Runner {
	limits: Limits,
	rustc: PathBuf,
	prlimit: Option<PathBuf>,
}

impl Runner {
	/// Find the compiler, and `prlimit` if a memory ceiling is asked for.
	///
	/// # Errors
	/// [`RecordError::Toolchain`] when `limits.rustc` is not found.
	pub fn new(limits: Limits) -> Result<Self, RecordError> {
		let rustc = resolve(&limits.rustc).ok_or_else(|| RecordError::Toolchain(limits.rustc.clone(), std::io::Error::from(std::io::ErrorKind::NotFound)))?;
		let prlimit = limits.memory_ceiling.and_then(|_| resolve(Path::new("prlimit")));
		Ok(Self { limits, rustc, prlimit })
	}

	#[must_use]
	pub const fn limits(&self) -> &Limits {
		&self.limits
	}

	/// Whether runs are held to [`Limits::memory_ceiling`]: one is set and
	/// `prlimit` is installed. Without it, a run's memory is bounded only by
	/// the machine — a known gap, which the recorder reports.
	#[must_use]
	pub const fn memory_ceiling_applied(&self) -> bool {
		self.prlimit.is_some()
	}

	/// Build, compile and run every program of the round `body`, at both
	/// constraint sets, in a fresh temporary directory removed afterwards.
	///
	/// The transcript is in order: `A` first, then `d0` …, each `before`
	/// then `after`. A program that does not compile records a `compile`
	/// error at both bounds.
	///
	/// # Errors
	/// See [`RecordError`]: the round cannot be run at all.
	pub fn record(&self, body: &[u8]) -> Result<Vec<RecordedRun>, RecordError> {
		let round: RunnableRound = serde_json::from_slice(body).map_err(RecordError::NotARound)?;
		let harness = round.harness.as_ref().ok_or(RecordError::NoHarness)?;
		for bounds in Bounds::ALL {
			if round.constraint_diff.set(bounds).is_empty() {
				return Err(RecordError::NoConstraints(match bounds {
					Bounds::Before => "constraintDiff.before",
					Bounds::After => "constraintDiff.after",
				}));
			}
		}
		let programs = programs(&round, harness).map_err(RecordError::Program)?;
		let dir = tempfile::Builder::new().prefix("leetype-run-").tempdir().map_err(RecordError::Io)?;

		let mut transcript = Vec::with_capacity(programs.len() * Bounds::ALL.len());
		for program in &programs {
			let compiled = self.compile(dir.path(), &program.variant.label(), &program.source)?;
			for bounds in Bounds::ALL {
				let constraints = round.constraint_diff.set(bounds);
				let result = match &compiled {
					Ok(binary) => self.run(dir.path(), binary, constraints)?,
					Err(message) => error(constraints, ErrorClass::Compile, message.clone()),
				};
				transcript.push(RecordedRun {
					variant: program.variant,
					bounds,
					sizes: sizes(constraints),
					result,
				});
			}
		}
		dir.close().map_err(RecordError::Io)?;
		Ok(transcript)
	}

	/// Compile `source` as a whole program and run it once with
	/// `constraints` — [`Self::record`]'s two steps, for one program.
	///
	/// # Errors
	/// As [`Self::record`].
	pub fn run_source(&self, source: &str, constraints: &[Constraint]) -> Result<RunResult, RecordError> {
		if constraints.is_empty() {
			return Err(RecordError::NoConstraints("the constraint set"));
		}
		let dir = tempfile::Builder::new().prefix("leetype-run-").tempdir().map_err(RecordError::Io)?;
		let result = match self.compile(dir.path(), "program", source)? {
			Ok(binary) => self.run(dir.path(), &binary, constraints)?,
			Err(message) => error(constraints, ErrorClass::Compile, message),
		};
		dir.close().map_err(RecordError::Io)?;
		Ok(result)
	}

	/// `Ok(Ok(binary))`, or `Ok(Err(message))` for a program that does not
	/// compile in time.
	fn compile(&self, dir: &Path, name: &str, source: &str) -> Result<Result<PathBuf, String>, RecordError> {
		let file = dir.join(String::from(name) + ".rs");
		let binary = dir.join(name);
		std::fs::write(&file, source).map_err(RecordError::Io)?;

		let mut command = Command::new(&self.rustc);
		command
			.args([
				"--edition",
				"2021",
				"-C",
				"opt-level=0",
				"-A",
				"warnings",
				"--crate-type",
				"bin",
				"--crate-name",
				"round",
				"-o",
			])
			.arg(&binary)
			.arg(&file)
			.current_dir(dir)
			.env_clear()
			.env("TMPDIR", dir);
		// What `rustc` (or rustup's proxy for it) and its linker need to be
		// found, and nothing else.
		for key in ["PATH", "HOME", "RUSTUP_HOME", "RUSTUP_TOOLCHAIN", "CARGO_HOME"] {
			if let Some(value) = std::env::var_os(key) {
				command.env(key, value);
			}
		}
		let finished = supervise(command, self.limits.compile_timeout, 0, self.limits.logs_ceiling).map_err(|err| RecordError::Toolchain(self.rustc.clone(), err))?;
		Ok(match finished.ending {
			Ending::Exited(status) if status.success() => Ok(binary),
			Ending::Exited(status) => {
				let mut message = String::from("rustc ");
				message.push_str(&describe(status));
				message.push('\n');
				message.push_str(&String::from_utf8_lossy(&finished.stderr.head));
				note_truncation(&mut message, "rustc's output", &finished.stderr);
				Err(message)
			}
			Ending::TimedOut => {
				let mut message = String::from("rustc did not finish within ");
				message.push_str(&self.limits.compile_timeout.as_secs().to_string());
				message.push_str(" s, and was killed");
				Err(message)
			}
		})
	}

	fn run(&self, dir: &Path, binary: &Path, constraints: &[Constraint]) -> Result<RunResult, RecordError> {
		let mut command = match (&self.prlimit, self.limits.memory_ceiling) {
			(Some(prlimit), Some(bytes)) => {
				let mut command = Command::new(prlimit);
				let mut limit = OsString::from("--as=");
				limit.push(bytes.to_string());
				command.arg(limit).arg("--").arg(binary);
				command
			}
			_ => Command::new(binary),
		};
		for constraint in constraints {
			let mut argument = constraint.dimension.clone();
			argument.push('=');
			argument.push_str(&constraint.bound.to_string());
			command.arg(argument);
		}
		command.current_dir(dir).env_clear();
		let Finished { ending, elapsed, stdout, stderr } =
			supervise(command, self.limits.run_ceiling, self.limits.output_ceiling, self.limits.logs_ceiling).map_err(RecordError::Io)?;

		Ok(match ending {
			Ending::Exited(status) if status.success() => {
				let mut output = String::from_utf8_lossy(&stdout.head).into_owned();
				if !stdout.truncated() && output.ends_with('\n') {
					output.pop();
				}
				let mut logs: Vec<String> = String::from_utf8_lossy(&stderr.head).lines().map(str::to_owned).collect();
				for (stream, captured) in [("stdout", &stdout), ("stderr", &stderr)] {
					if captured.truncated() {
						let mut note = String::new();
						note_truncation(&mut note, stream, captured);
						logs.push(note.trim_start().to_owned());
					}
				}
				RunResult::Ok {
					input_size: input_size(constraints),
					observation: Observation {
						output,
						logs,
						elapsed: Elapsed {
							milliseconds: u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX),
						},
					},
				}
			}
			Ending::Exited(status) => {
				let mut message = String::from("the program ");
				message.push_str(&describe(status));
				let tail = String::from_utf8_lossy(stderr.end());
				if !tail.trim().is_empty() {
					message.push('\n');
					if stderr.truncated() {
						message.push('…');
					}
					message.push_str(tail.trim_end());
				}
				error(constraints, ErrorClass::Runtime, message)
			}
			Ending::TimedOut => {
				let mut message = String::from("the program did not finish within the ");
				message.push_str(&self.limits.run_ceiling.as_millis().to_string());
				message.push_str(" ms wall-clock ceiling, and was killed");
				error(constraints, ErrorClass::BudgetExceeded, message)
			}
		})
	}
}

/// `\n<stream> truncated: kept the first K of N bytes`, when it was.
fn note_truncation(message: &mut String, stream: &str, captured: &Captured) {
	if captured.truncated() {
		message.push('\n');
		message.push_str(stream);
		message.push_str(" truncated: kept the first ");
		message.push_str(&captured.head.len().to_string());
		message.push_str(" of ");
		message.push_str(&captured.total.to_string());
		message.push_str(" bytes");
	}
}

/// The bound of the constraint set's first dimension: `RunResult.inputSize`.
fn input_size(constraints: &[Constraint]) -> u64 {
	constraints.first().map_or(0, |constraint| constraint.bound)
}

fn sizes(constraints: &[Constraint]) -> BTreeMap<String, u64> {
	constraints.iter().map(|constraint| (constraint.dimension.clone(), constraint.bound)).collect()
}

fn error(constraints: &[Constraint], error_class: ErrorClass, message: String) -> RunResult {
	RunResult::Error {
		input_size: input_size(constraints),
		error: ExecutionError { error_class, message },
	}
}

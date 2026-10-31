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
//!
//! **Where the compile and the run live.** In `rust_runner`, the neutral
//! runner Rust Rounds' `rust-runner` service calls too (paulgsc/some-ui#2234):
//! this crate builds a round's programs and turns what each did into the
//! client's `RunResult`, and nothing else spawns a process here.

pub mod recorder;
pub mod round;

use crate::round::{programs, Constraint, ProgramError, RunnableRound};
use leetype_round_repo::{Bounds, Elapsed, ErrorClass, ExecutionError, Observation, RecordedRun, RunResult};
use rust_runner::{describe, without_thread_ids, Captured, Compiler, Ending, Finished, Profile, SpawnError};
pub use rust_runner::{Limits, DEFAULT_COMPILE_TIMEOUT, DEFAULT_LOGS_CEILING, DEFAULT_MEMORY_CEILING, DEFAULT_OUTPUT_CEILING, DEFAULT_RUN_CEILING};
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

pub use recorder::{record_listed, RecordReport, RecorderError, RoundOutcome, Selection};

/// How a round's programs are compiled: the 2021 edition the corpus was
/// written for, warnings off, as `check-round-programs-compile.ts` compiles
/// them.
const PROFILE: Profile = Profile {
	edition: "2021",
	crate_name: "round",
	warnings: false,
};

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
	/// A compiled program could not be started or supervised, for a reason
	/// that is the program's: an argument the OS refuses (a NUL in a
	/// dimension, say), or a descendant that escaped its process group. The round's fault, not the machine's: the
	/// recorder marks this round failed and goes on (review, #399).
	Run(std::io::Error),
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
			Self::Run(err) => write!(f, "could not run a program: {err}"),
		}
	}
}

impl std::error::Error for RecordError {}

impl From<SpawnError> for RecordError {
	fn from(err: SpawnError) -> Self {
		match err {
			SpawnError::Toolchain(rustc, err) => Self::Toolchain(rustc, err),
			SpawnError::Io(err) => Self::Io(err),
			SpawnError::Program(err) => Self::Run(err),
		}
	}
}

/// The runner: `rust_runner`'s compiler, with its `prlimit` (if any) found.
#[derive(Debug, Clone)]
pub struct Runner {
	compiler: Compiler,
}

impl Runner {
	/// Find the compiler, and `prlimit` if a memory ceiling is asked for.
	///
	/// # Errors
	/// [`RecordError::Toolchain`] when `limits.rustc` is not found.
	pub fn new(limits: Limits) -> Result<Self, RecordError> {
		Ok(Self {
			compiler: Compiler::new(limits).map_err(RecordError::from)?,
		})
	}

	#[must_use]
	pub const fn limits(&self) -> &Limits {
		self.compiler.limits()
	}

	/// Whether runs are held to [`Limits::memory_ceiling`]: one is set and
	/// `prlimit` is installed. Without it, a run's memory is bounded only by
	/// the machine — a known gap, which the recorder reports.
	#[must_use]
	pub const fn memory_ceiling_applied(&self) -> bool {
		self.compiler.memory_ceiling_applied()
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
	/// compile in time. The message is the text rustc prints (each
	/// diagnostic's `rendered`, byte for byte what it prints without
	/// `--error-format=json`), kept to the logs ceiling.
	fn compile(&self, dir: &Path, name: &str, source: &str) -> Result<Result<PathBuf, String>, RecordError> {
		let compiled = self.compiler.compile(dir, name, source, &PROFILE)?;
		let limits = self.compiler.limits();
		Ok(match (compiled.ending, &compiled.binary) {
			(Ending::Exited(_), Some(binary)) => Ok(binary.clone()),
			(Ending::Exited(status), None) => {
				let mut message = String::from("rustc ");
				message.push_str(&describe(status));
				message.push('\n');
				let (head, cut) = compiled.rendered_head(limits.logs_ceiling);
				message.push_str(&head);
				if cut {
					let bytes = compiled.rendered.as_bytes();
					let rendered = Captured {
						head: bytes[..limits.logs_ceiling.min(bytes.len())].to_vec(),
						tail: Vec::new(),
						total: u64::try_from(compiled.rendered.len()).unwrap_or(u64::MAX),
					};
					note_truncation(&mut message, "rustc's output", &rendered);
				}
				Err(message)
			}
			(Ending::TimedOut, _) => {
				let mut message = String::from("rustc did not finish within ");
				message.push_str(&limits.compile_timeout.as_secs().to_string());
				message.push_str(" s, and was killed");
				Err(message)
			}
		})
	}

	fn run(&self, dir: &Path, binary: &Path, constraints: &[Constraint]) -> Result<RunResult, RecordError> {
		let arguments: Vec<OsString> = constraints
			.iter()
			.map(|constraint| {
				let mut argument = constraint.dimension.clone();
				argument.push('=');
				argument.push_str(&constraint.bound.to_string());
				OsString::from(argument)
			})
			.collect();
		let Finished { ending, elapsed, stdout, stderr } = self.compiler.run(dir, binary, &arguments)?;
		let limits = self.compiler.limits();

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
				let tail = without_thread_ids(&String::from_utf8_lossy(stderr.end()));
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
				message.push_str(&limits.run_ceiling.as_millis().to_string());
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

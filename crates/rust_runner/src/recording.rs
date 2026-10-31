//! Rust Rounds' recording: one program, compiled and run once, and what
//! `rustc` and the program printed (paulgsc/some-ui#2234, RR3).
//!
//! A [`Recording`] is keyed by [`key`]: a hash of the toolchain and the
//! source, the bytes that decide what rustc does with a program. The same
//! source under the same toolchain is recorded once, wherever it came from,
//! as the runs `record-leetype-runs` stores are keyed to the round body
//! they ran (EX2).
//!
//! The program is a whole binary crate, `main.rs`, compiled as the owner's
//! own code is (`--edition 2024`, warnings kept) at `opt-level=0`, so debug
//! assertions and overflow checks are on. Its run gets no arguments, no
//! stdin and no environment.

use crate::{describe, without_thread_ids, Compiler, Diagnostic, Ending, Limits, Profile, SpawnError};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

/// How every Rust Rounds program is compiled. Part of [`Recorder::toolchain`],
/// so a change here is a new key for every program.
pub const PROFILE: Profile = Profile {
	edition: "2024",
	crate_name: "main",
	warnings: true,
};

/// The program's file is `main.rs`, the name rustc's messages and a panic's
/// location show.
const NAME: &str = "main";

/// What a program came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Outcome {
	/// It compiled, ran, and exited with status 0.
	Ok,
	/// rustc refused it.
	CompileError,
	/// It ran and failed: a non-zero exit (a panic is 101) or a signal (an
	/// allocation past the memory ceiling aborts).
	Runtime,
	/// rustc or the program was still running at its ceiling, and was
	/// killed.
	BudgetExceeded,
}

impl Outcome {
	#[must_use]
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::Ok => "ok",
			Self::CompileError => "compile-error",
			Self::Runtime => "runtime",
			Self::BudgetExceeded => "budget-exceeded",
		}
	}
}

/// One program's recording.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Recording {
	/// [`key`] of the toolchain and the source.
	pub key: String,
	/// [`Recorder::toolchain`].
	pub toolchain: String,
	pub outcome: Outcome,
	/// What happened, in words, when the outcome is not `ok`: `the program
	/// exited with status 101`, `the program did not finish within the 2000
	/// ms wall-clock ceiling, and was killed`. Never why.
	pub note: Option<String>,
	pub compile: CompileRecord,
	/// `None` when it did not compile.
	pub run: Option<RunRecord>,
}

/// What rustc printed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompileRecord {
	/// Each diagnostic, with its code and spans, for as many as start within
	/// the text kept.
	pub diagnostics: Vec<Diagnostic>,
	/// The text rustc prints, kept to the logs ceiling.
	pub rendered: String,
	/// How many bytes of text rustc printed in all; more than `rendered`
	/// holds when it was cut.
	pub rendered_bytes: u64,
}

/// What the program printed, and how it ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunRecord {
	/// The first bytes of stdout, to the output ceiling.
	pub stdout: String,
	pub stdout_bytes: u64,
	/// The last bytes of stderr, to the logs ceiling (where a panic's
	/// message is), with a panic header's thread id dropped.
	pub stderr: String,
	pub stderr_bytes: u64,
	/// Whether either stream was longer than its ceiling, so what is kept
	/// is not all it printed.
	pub cut: bool,
	/// Its exit status, when it exited.
	pub exit_code: Option<i32>,
	/// The signal that ended it, when one did (`9` for the ceiling's kill).
	pub signal: Option<i32>,
	/// From start to end, on the machine that recorded it. Illustrative.
	pub milliseconds: u64,
}

/// The hex SHA-256 of `toolchain`, a NUL, and `source`. A toolchain never
/// holds a NUL, so no two pairs share the hashed bytes.
#[must_use]
pub fn key(toolchain: &str, source: &str) -> String {
	let mut hasher = Sha256::new();
	hasher.update(toolchain.as_bytes());
	hasher.update([0]);
	hasher.update(source.as_bytes());
	hex::encode(hasher.finalize())
}

/// The runner, with its toolchain named.
#[derive(Debug, Clone)]
pub struct Recorder {
	compiler: Compiler,
	toolchain: String,
	work: PathBuf,
}

impl Recorder {
	/// Find the compiler and learn its version. Each program gets a fresh
	/// directory under `work`.
	///
	/// # Errors
	/// [`SpawnError::Toolchain`] when rustc is missing or will not say its
	/// version.
	pub fn new(limits: Limits, work: PathBuf) -> Result<Self, SpawnError> {
		let compiler = Compiler::new(limits)?;
		let version = compiler.version()?;
		let toolchain = version + "; --edition " + PROFILE.edition + " -C opt-level=0";
		Ok(Self { compiler, toolchain, work })
	}

	/// What every recording is keyed to: `rustc -V` and the profile, as
	/// `rustc 1.97.0 (2d8144b78 2026-07-07); --edition 2024 -C opt-level=0`.
	#[must_use]
	pub fn toolchain(&self) -> &str {
		&self.toolchain
	}

	#[must_use]
	pub const fn compiler(&self) -> &Compiler {
		&self.compiler
	}

	/// Compile `source` and, if it builds, run it once.
	///
	/// # Errors
	/// Only the machine's failures ([`SpawnError::is_toolchain`]); whatever
	/// the program does is in the [`Recording`].
	pub fn record(&self, source: &str) -> Result<Recording, SpawnError> {
		let limits = self.compiler.limits();
		let dir = tempfile::Builder::new().prefix("rust-run-").tempdir_in(&self.work).map_err(SpawnError::Io)?;
		let compiled = self.compiler.compile(dir.path(), NAME, source, &PROFILE)?;

		let (rendered, _) = compiled.rendered_head(limits.logs_ceiling);
		let mut start = 0;
		let diagnostics = compiled
			.diagnostics
			.iter()
			.take_while(|diagnostic| {
				let starts = start;
				start += diagnostic.rendered.as_ref().map_or(0, String::len);
				starts < limits.logs_ceiling
			})
			.cloned()
			.collect();
		let compile = CompileRecord {
			diagnostics,
			rendered,
			rendered_bytes: u64::try_from(compiled.rendered.len()).unwrap_or(u64::MAX),
		};
		let key = key(&self.toolchain, source);

		let Some(binary) = &compiled.binary else {
			let (outcome, note) = match compiled.ending {
				Ending::TimedOut => (
					Outcome::BudgetExceeded,
					String::from("rustc did not finish within ") + &limits.compile_timeout.as_secs().to_string() + " s, and was killed",
				),
				Ending::Exited(status) => (Outcome::CompileError, String::from("rustc ") + &describe(status)),
			};
			return Ok(self.recording(key, outcome, Some(note), compile, None));
		};

		let finished = match self.compiler.run(dir.path(), binary, &[]) {
			Ok(finished) => finished,
			// The program's own doing: recorded as a runtime failure.
			Err(SpawnError::Program(err)) => {
				let note = String::from("the program could not be run to its end: ") + &err.to_string();
				return Ok(self.recording(key, Outcome::Runtime, Some(note), compile, None));
			}
			Err(err) => return Err(err),
		};
		dir.close().map_err(SpawnError::Io)?;

		let stderr = without_thread_ids(&String::from_utf8_lossy(finished.stderr.end()));
		let (outcome, note, exit_code, signal) = match finished.ending {
			Ending::Exited(status) if status.success() => (Outcome::Ok, None, status.code(), None),
			Ending::Exited(status) => (Outcome::Runtime, Some(String::from("the program ") + &describe(status)), status.code(), signal_of(status)),
			Ending::TimedOut => (
				Outcome::BudgetExceeded,
				Some(String::from("the program did not finish within the ") + &limits.run_ceiling.as_millis().to_string() + " ms wall-clock ceiling, and was killed"),
				None,
				Some(9),
			),
		};
		let run = RunRecord {
			stdout: String::from_utf8_lossy(&finished.stdout.head).into_owned(),
			stdout_bytes: finished.stdout.total,
			stderr,
			stderr_bytes: finished.stderr.total,
			cut: finished.stdout.truncated() || finished.stderr.truncated(),
			exit_code,
			signal,
			milliseconds: u64::try_from(finished.elapsed.as_millis()).unwrap_or(u64::MAX),
		};
		Ok(self.recording(key, outcome, note, compile, Some(run)))
	}

	fn recording(&self, key: String, outcome: Outcome, note: Option<String>, compile: CompileRecord, run: Option<RunRecord>) -> Recording {
		Recording {
			key,
			toolchain: self.toolchain.clone(),
			outcome,
			note,
			compile,
			run,
		}
	}
}

/// rustc's primary span, as its JSON diagnostics give it and the phone reads
/// it: 1-based lines and columns, columns counted in characters, the end
/// exclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunSpan {
	pub line_start: u64,
	pub column_start: u64,
	pub line_end: u64,
	pub column_end: u64,
}

/// Why a run that compiled ended other than by exiting 0 or panicking.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Why {
	/// A wall-clock ceiling: the run's, or (with nothing run) the compile's.
	Time,
	/// The memory ceiling: an allocation it refused aborted the program.
	Memory,
	/// More output than the ceiling keeps, so what it printed is not whole.
	Output,
	/// Any other exit: a status but 0 or 101, or a signal.
	Exit,
}

/// What the phone reads of a [`Recording`].
///
/// paulgsc/some-ui's `RustcRunSchema` (`some-ui.rust-round-log/1`'s
/// `rustc`, RR1) exactly, field for field. [`Recording::rustc_run`] derives it; nothing is
/// recorded in this shape alone, so a change to it never needs a
/// re-recording.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "kebab-case")]
pub enum RustcRun {
	CompileError {
		toolchain: String,
		/// The first error's code, when it is an `E` code.
		code: Option<String>,
		/// Every diagnostic rustc printed, as printed (to the logs ceiling).
		stderr: String,
		/// The first error's primary span in `main.rs`.
		span: Option<RunSpan>,
	},
	Prints {
		toolchain: String,
		/// What rustc printed while it built (warnings), possibly empty.
		#[serde(rename = "compileStderr")]
		compile_stderr: String,
		stdout: String,
	},
	Panics {
		toolchain: String,
		#[serde(rename = "compileStderr")]
		compile_stderr: String,
		stdout: String,
		stderr: String,
	},
	Stopped {
		toolchain: String,
		#[serde(rename = "compileStderr")]
		compile_stderr: String,
		why: Why,
		/// The program's; empty when it was rustc that ran out of time.
		stdout: String,
		stderr: String,
	},
}

impl Recording {
	/// The release rustc reported, `1.97.0`, from [`Self::toolchain`].
	#[must_use]
	pub fn release(&self) -> &str {
		self
			.toolchain
			.strip_prefix("rustc ")
			.and_then(|rest| rest.split([' ', ';']).next())
			.unwrap_or(&self.toolchain)
	}

	/// This recording as the phone reads it ([`RustcRun`]).
	#[must_use]
	pub fn rustc_run(&self) -> RustcRun {
		let toolchain = self.release().to_owned();
		let Some(run) = &self.run else {
			if self.outcome == Outcome::BudgetExceeded {
				// rustc ran out of time: nothing was run, so nothing printed.
				return RustcRun::Stopped {
					toolchain,
					compile_stderr: self.compile.rendered.clone(),
					why: Why::Time,
					stdout: String::new(),
					stderr: String::new(),
				};
			}
			let error = self.compile.diagnostics.iter().find(|diagnostic| diagnostic.level == "error");
			let code = error.and_then(|error| error.code.clone()).filter(|code| is_error_code(code));
			let span = error
				.and_then(|error| error.spans.iter().find(|span| span.is_primary && span.file == "main.rs"))
				.map(|span| RunSpan {
					line_start: span.line_start,
					column_start: span.column_start,
					line_end: span.line_end,
					column_end: span.column_end,
				});
			return RustcRun::CompileError {
				toolchain,
				code,
				stderr: self.compile.rendered.clone(),
				span,
			};
		};
		let compile_stderr = self.compile.rendered.clone();
		let stdout = run.stdout.clone();
		let stderr = run.stderr.clone();
		let stopped = |why| RustcRun::Stopped {
			toolchain: toolchain.clone(),
			compile_stderr: compile_stderr.clone(),
			why,
			stdout: stdout.clone(),
			stderr: stderr.clone(),
		};
		match self.outcome {
			Outcome::BudgetExceeded => stopped(Why::Time),
			_ if run.cut => stopped(Why::Output),
			Outcome::Ok => RustcRun::Prints {
				toolchain,
				compile_stderr,
				stdout,
			},
			Outcome::Runtime if run.exit_code == Some(101) => RustcRun::Panics {
				toolchain,
				compile_stderr,
				stdout,
				stderr,
			},
			// An allocation the address-space ceiling refused aborts with
			// this line; anything else is some other exit.
			Outcome::Runtime if run.stderr.contains("memory allocation of ") => stopped(Why::Memory),
			Outcome::Runtime | Outcome::CompileError => stopped(Why::Exit),
		}
	}
}

/// `E` and four digits, the only codes the phone's format takes.
fn is_error_code(code: &str) -> bool {
	code.len() == 5 && code.starts_with('E') && code[1..].bytes().all(|byte| byte.is_ascii_digit())
}

fn signal_of(status: std::process::ExitStatus) -> Option<i32> {
	use std::os::unix::process::ExitStatusExt;
	status.signal()
}

#[cfg(test)]
mod tests {
	use super::key;

	#[test]
	fn the_key_is_the_toolchain_and_the_source_and_nothing_else() {
		let one = key("rustc 1.97.0 (2d8144b78 2026-07-07); --edition 2024 -C opt-level=0", "fn main() {}\n");
		assert_eq!(one, key("rustc 1.97.0 (2d8144b78 2026-07-07); --edition 2024 -C opt-level=0", "fn main() {}\n"));
		assert_ne!(one, key("rustc 1.97.1 (8bab26f4f 2026-07-14); --edition 2024 -C opt-level=0", "fn main() {}\n"));
		assert_ne!(one, key("rustc 1.97.0 (2d8144b78 2026-07-07); --edition 2024 -C opt-level=0", "fn main() { }\n"));
		assert_ne!(key("ab", "c"), key("a", "bc"), "the separator keeps the two apart");
		assert_eq!(one.len(), 64);
	}
}

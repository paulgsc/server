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

use crate::{describe, without_thread_ids, Compiler, Ending, Limits, Profile, SpawnError};
/// What a recording holds, and its key: `rust_rounds_wire`'s, the half a
/// client links.
pub use rust_rounds_wire::recording::{key, CompileRecord, Outcome, Recording, RunRecord, RunSpan, RustcRun, Why};
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
	/// Without a uid of the programs' own, the program's directory is removed
	/// afterwards (an error doing so does not fail the recording). With one,
	/// it is left for the caller's tidy, which removes it as that uid: the
	/// runner never walks a tree the program made.
	///
	/// # Errors
	/// Only the machine's failures: [`SpawnError::Toolchain`] and
	/// [`SpawnError::Io`] before the program ran, and
	/// [`SpawnError::Unsupervised`] once it had; whatever the program does is
	/// in the [`Recording`].
	pub fn record(&self, source: &str) -> Result<Recording, SpawnError> {
		let dir = tempfile::Builder::new().prefix("rust-run-").tempdir_in(&self.work).map_err(SpawnError::Io)?;
		if self.compiler.limits().run_as.is_none() {
			let recording = self.record_in(dir.path(), source);
			let _ = dir.close();
			return recording;
		}
		// The runner's own directory, which rustc (writing the binary) and
		// the program run in as the programs' uid. Kept: removing what they
		// leave is the caller's tidy, as that uid.
		let dir = dir.keep();
		std::fs::set_permissions(&dir, std::os::unix::fs::PermissionsExt::from_mode(0o777)).map_err(SpawnError::Io)?;
		self.record_in(&dir, source)
	}

	fn record_in(&self, dir: &std::path::Path, source: &str) -> Result<Recording, SpawnError> {
		let limits = self.compiler.limits();
		let compiled = self.compiler.compile(dir, NAME, source, &PROFILE)?;

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
		let (timed_out, exit_code, signal) = match compiled.ending {
			Ending::TimedOut => (true, None, Some(9)),
			Ending::Exited(status) => (false, status.code(), signal_of(status)),
		};
		let compile = CompileRecord {
			diagnostics,
			rendered: rendered.to_owned(),
			rendered_bytes: u64::try_from(compiled.rendered.len()).unwrap_or(u64::MAX),
			timed_out,
			exit_code,
			signal,
		};
		let key = key(&self.toolchain, source);

		let Some(binary) = &compiled.binary else {
			let (outcome, note) = match compiled.ending {
				Ending::TimedOut => (
					Outcome::BudgetExceeded,
					String::from("rustc did not finish within ") + &limits.compile_timeout.as_secs().to_string() + " s, and was killed",
				),
				Ending::Exited(status) if status.code().is_some() => (Outcome::CompileError, String::from("rustc ") + &describe(status)),
				Ending::Exited(status) => (Outcome::BudgetExceeded, String::from("rustc ") + &describe(status)),
			};
			return Ok(self.recording(key, outcome, Some(note), compile, None));
		};

		let finished = match self.compiler.run(dir, binary, &[]) {
			Ok(finished) => finished,
			// The program's own doing: recorded as a runtime failure.
			Err(SpawnError::Program(err)) => {
				let note = String::from("the program could not be run to its end: ") + &err.to_string();
				return Ok(self.recording(key, Outcome::Runtime, Some(note), compile, None));
			}
			// The machine's, but past the program's start, so possibly the
			// program's doing (its survivors against the sweep). A failure
			// to start it is not: nothing of it ran, and the compile's sweep
			// left nothing of its uid, so it stays `Io` (`unavailable`).
			Err(SpawnError::Io(err)) if !crate::process::not_started(&err) => return Err(SpawnError::Unsupervised(err)),
			Err(err) => return Err(err),
		};
		let stderr = without_thread_ids(&String::from_utf8_lossy(&finished.stderr.tail));
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

fn signal_of(status: std::process::ExitStatus) -> Option<i32> {
	use std::os::unix::process::ExitStatusExt;
	status.signal()
}

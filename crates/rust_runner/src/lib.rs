//! Compiles a Rust program with `rustc` and runs it, bounded, for whoever
//! records what it did: `record-leetype-runs` (through
//! `leetype_runner`) and Rust Rounds' `rust-runner` service
//! ([`recording`], [`service`]). Moved here out of `leetype_runner`
//! (paulgsc/some-ui#2234, RR3), so both recorders call one runner and
//! neither copies it.
//!
//! **Every program is hostile.** No caller is trusted more than another and
//! no source is exempt from a limit: the limits below hold for every compile
//! and every run, and the `rust-runner` container adds the rest (no network,
//! no IPC namespace shared with anything, a read-only root, process, memory
//! and CPU caps). There the runner is root with three capabilities, and every
//! compile and run is a uid of its own ([`RunAs`]), so a program can neither
//! signal the runner nor touch what it owns.
//! The design and its invariants (EX1, RX1–RX4) are in
//! `apps/servers/file_host/docs/leetype-execution.md`.
//!
//! **Limits** ([`Limits`], every one configurable): a compile timeout; a
//! wall-clock ceiling per run, past which the program's whole process group
//! is killed and reaped; a ceiling on stdout and on stderr (kept to the
//! ceiling, never streamed); address-space ceilings (rustc's and the run's),
//! a file-size ceiling and a process ceiling, applied to rustc and to the
//! run through `prlimit` when it is installed; no core dump and no POSIX
//! message queue; a fresh directory per program; and an empty environment
//! for the run. The compile
//! is `rustc -C opt-level=0 --error-format=json`, so a failed compile keeps
//! each diagnostic's code and spans ([`diagnostics`]) as well as the text
//! rustc prints.

pub mod diagnostics;
mod process;
pub mod recording;
pub mod service;

pub use crate::diagnostics::Diagnostic;
pub use crate::process::{describe, resolve, Captured, Ending, Finished};
use std::ffi::OsString;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

/// How long `rustc` may take over one program before the compile counts as
/// failed.
pub const DEFAULT_COMPILE_TIMEOUT: Duration = Duration::from_secs(60);
/// The wall-clock ceiling on one run.
///
/// See `leetype-execution.md` for why 2 s at `opt-level=0`: every
/// inadmissible variant in the round corpus exceeds it at `C′` and every
/// admissible one finishes in tens of milliseconds.
pub const DEFAULT_RUN_CEILING: Duration = Duration::from_millis(2_000);
/// The most stdout a run keeps.
pub const DEFAULT_OUTPUT_CEILING: usize = 4 * 1024;
/// The most stderr a run keeps, and the most of rustc's text a failed
/// compile keeps.
pub const DEFAULT_LOGS_CEILING: usize = 4 * 1024;
/// The address-space ceiling on one run, when `prlimit` is installed.
pub const DEFAULT_MEMORY_CEILING: u64 = 512 * 1024 * 1024;
/// The address-space ceiling on one compile, when `prlimit` is installed.
///
/// rustc reserves more than a program does; a hello world needs about a
/// third of this, and a program that makes rustc need more (a macro that
/// expands without end, a static of hundreds of MiB) is stopped for memory.
pub const DEFAULT_COMPILE_MEMORY_CEILING: u64 = 768 * 1024 * 1024;
/// The largest file a compile or a run may write (`RLIMIT_FSIZE`), when
/// `prlimit` is installed. A program's binary at `opt-level=0` is a few MiB.
pub const DEFAULT_FILE_SIZE_CEILING: u64 = 64 * 1024 * 1024;
/// The most of rustc's JSON stderr read back. Larger than the text a
/// compile keeps, because each diagnostic's JSON carries its long-form
/// explanation; past it, the diagnostics after the cut are not read.
pub const DIAGNOSTICS_CEILING: usize = 1024 * 1024;

/// Everything the runner bounds, each with a default.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Limits {
	/// The compiler: a path, or a name looked up on `PATH`.
	pub rustc: PathBuf,
	pub compile_timeout: Duration,
	pub run_ceiling: Duration,
	pub output_ceiling: usize,
	pub logs_ceiling: usize,
	/// The run's address-space ceiling; `None` for none.
	pub memory_ceiling: Option<u64>,
	/// rustc's address-space ceiling; `None` for none.
	pub compile_memory_ceiling: Option<u64>,
	/// `RLIMIT_FSIZE` for a compile and a run; `None` for none.
	pub file_size_ceiling: Option<u64>,
	/// `RLIMIT_NPROC` for a compile and a run: the most processes and
	/// threads their uid may hold while they fork. Counted per uid, and not
	/// enforced for root, so it bounds a program only under a uid of its own
	/// ([`Self::run_as`]); `None` elsewhere.
	pub process_ceiling: Option<u64>,
	/// The uid every compile and run is started as, and swept after; `None`
	/// to run them as this process's own uid and kill only their process
	/// group.
	pub run_as: Option<RunAs>,
}

/// A uid and gid of the programs' own, which nothing else in the machine (the
/// container) runs as.
///
/// Setting one needs the runner to be root (with `CAP_SETUID` and
/// `CAP_SETGID`), and it must not be the runner's own: a program under the
/// runner's uid could stop it (`SIGSTOP`), swap its socket or read its
/// memory. Children get no supplementary groups (`std` clears them when root
/// sets a uid). After every compile and run, every process of the uid is
/// killed: nothing else can be of it, so whatever is left is the program's,
/// including a descendant that left its process group (`setsid`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunAs {
	pub uid: u32,
	pub gid: u32,
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
			compile_memory_ceiling: Some(DEFAULT_COMPILE_MEMORY_CEILING),
			file_size_ceiling: Some(DEFAULT_FILE_SIZE_CEILING),
			process_ceiling: None,
			run_as: None,
		}
	}
}

/// How a program is compiled, besides the limits: what changes rustc's
/// output for the same source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Profile {
	pub edition: &'static str,
	/// `--crate-name`, which some messages name.
	pub crate_name: &'static str,
	/// Whether warnings are kept (`false` passes `-A warnings`).
	pub warnings: bool,
}

/// Why a program was not compiled or run. A program that fails to compile,
/// panics or runs out of time is **not** one of these: that is a result.
#[derive(Debug)]
pub enum SpawnError {
	/// The compiler could not be found or started. Not the program's fault.
	Toolchain(PathBuf, std::io::Error),
	/// The program's directory or file, or the machine (no processes or
	/// descriptors left, a process that survived the sweep).
	Io(std::io::Error),
	/// A compiled program could not be started or supervised for a reason
	/// that is the program's: an argument the OS refuses (a NUL in it), or a
	/// descendant that escaped its process group and held its output open.
	Program(std::io::Error),
}

impl std::fmt::Display for SpawnError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::Toolchain(rustc, err) => write!(f, "could not run {}: {err}", rustc.display()),
			Self::Io(err) => write!(f, "could not prepare or supervise the program: {err}"),
			Self::Program(err) => write!(f, "could not run a program: {err}"),
		}
	}
}

impl std::error::Error for SpawnError {}

/// What a compile did.
#[derive(Debug, Clone)]
pub struct Compiled {
	/// The binary, when it built.
	pub binary: Option<PathBuf>,
	pub ending: Ending,
	pub diagnostics: Vec<Diagnostic>,
	/// The text rustc prints without `--error-format=json`, whole (up to
	/// [`DIAGNOSTICS_CEILING`] of JSON).
	pub rendered: String,
}

impl Compiled {
	/// At most the first `cap` bytes of [`Self::rendered`], ending on a
	/// character boundary, and whether it was cut.
	#[must_use]
	pub fn rendered_head(&self, cap: usize) -> (&str, bool) {
		let mut end = cap.min(self.rendered.len());
		while !self.rendered.is_char_boundary(end) {
			end -= 1;
		}
		(&self.rendered[..end], end < self.rendered.len())
	}
}

/// The runner, with its compiler (and `prlimit`, if any) found.
#[derive(Debug, Clone)]
pub struct Compiler {
	limits: Limits,
	rustc: PathBuf,
	prlimit: Option<PathBuf>,
}

impl Compiler {
	/// Find the compiler, and `prlimit` if a ceiling needs it.
	///
	/// # Errors
	/// [`SpawnError::Toolchain`] when `limits.rustc` is not found.
	pub fn new(limits: Limits) -> Result<Self, SpawnError> {
		let rustc = resolve(&limits.rustc).ok_or_else(|| SpawnError::Toolchain(limits.rustc.clone(), std::io::Error::from(std::io::ErrorKind::NotFound)))?;
		if limits.run_as.is_some_and(|run_as| run_as.uid == 0 || run_as.uid == rustix::process::getuid().as_raw()) {
			return Err(SpawnError::Io(std::io::Error::new(
				std::io::ErrorKind::InvalidInput,
				"programs must run as a uid of their own, neither root nor the runner's",
			)));
		}
		let ceilings = [limits.memory_ceiling, limits.compile_memory_ceiling, limits.file_size_ceiling, limits.process_ceiling];
		let prlimit = ceilings.iter().any(Option::is_some).then(|| resolve(Path::new("prlimit"))).flatten();
		Ok(Self { limits, rustc, prlimit })
	}

	#[must_use]
	pub const fn limits(&self) -> &Limits {
		&self.limits
	}

	/// Whether runs are held to [`Limits::memory_ceiling`]: one is set and
	/// `prlimit` is installed. Without it, a run's memory is bounded only by
	/// the machine (or the container).
	#[must_use]
	pub const fn memory_ceiling_applied(&self) -> bool {
		self.prlimit.is_some() && self.limits.memory_ceiling.is_some()
	}

	/// `rustc -V`, trimmed: `rustc 1.97.0 (2d8144b78 2026-07-07)`.
	///
	/// # Errors
	/// [`SpawnError::Toolchain`] when it cannot be run or says nothing.
	pub fn version(&self) -> Result<String, SpawnError> {
		let mut command = Command::new(&self.rustc);
		command.arg("-V").env_clear();
		forward_toolchain_env(&mut command);
		let failed = |err| SpawnError::Toolchain(self.rustc.clone(), err);
		let finished = process::supervise(command, self.limits.compile_timeout, 1024, self.limits.logs_ceiling, None).map_err(failed)?;
		let version = String::from_utf8_lossy(&finished.stdout.head).trim().to_owned();
		match finished.ending {
			Ending::Exited(status) if status.success() && !version.is_empty() => Ok(version),
			_ => Err(failed(std::io::Error::other("rustc -V did not print a version"))),
		}
	}

	/// Compile `source` as `<dir>/<name>.rs` into `<dir>/<name>`, with
	/// `--error-format=json` and `opt-level=0`.
	///
	/// # Errors
	/// [`SpawnError::Io`] when the file cannot be written, and
	/// [`SpawnError::Toolchain`] when rustc cannot be run at all. A program
	/// that does not compile is `Ok`, with no binary.
	pub fn compile(&self, dir: &Path, name: &str, source: &str, profile: &Profile) -> Result<Compiled, SpawnError> {
		let file = String::from(name) + ".rs";
		std::fs::write(dir.join(&file), source).map_err(SpawnError::Io)?;

		let mut command = self.limited(&self.rustc, self.limits.compile_memory_ceiling);
		command.args(["--edition", profile.edition, "-C", "opt-level=0"]);
		if !profile.warnings {
			command.args(["-A", "warnings"]);
		}
		command
			.args(["--error-format=json", "--crate-type", "bin", "--crate-name", profile.crate_name, "-o"])
			// Relative to `dir`, the child's working directory, so no message
			// rustc or the program prints names the random temporary
			// directory: a re-recording of an unchanged failure is then the
			// same bytes, and the host's paths reach no reader (review, #399).
			.arg(name)
			.arg(&file)
			.current_dir(dir)
			.env_clear()
			.env("TMPDIR", dir);
		forward_toolchain_env(&mut command);
		let finished = self
			.supervise(command, self.limits.compile_timeout, 0, DIAGNOSTICS_CEILING)
			.map_err(|err| SpawnError::Toolchain(self.rustc.clone(), err))?;
		let read = diagnostics::read(&finished.stderr.head, finished.stderr.truncated());
		let built = matches!(finished.ending, Ending::Exited(status) if status.success());
		Ok(Compiled {
			binary: built.then(|| dir.join(name)),
			ending: finished.ending,
			diagnostics: read.diagnostics,
			rendered: read.rendered,
		})
	}

	/// Run `binary` with `args`, in `dir`, with an empty environment, under
	/// every limit.
	///
	/// # Errors
	/// [`SpawnError::Program`] for what the program caused (an argument the
	/// OS refuses, a descendant holding its output open), and
	/// [`SpawnError::Io`] for the machine's failures.
	pub fn run(&self, dir: &Path, binary: &Path, args: &[OsString]) -> Result<Finished, SpawnError> {
		let mut command = self.limited(binary, self.limits.memory_ceiling);
		command.args(args).current_dir(dir).env_clear();
		self
			.supervise(command, self.limits.run_ceiling, self.limits.output_ceiling, self.limits.logs_ceiling)
			.map_err(|err| match err.kind() {
				// What the program asked for: an argument the OS refuses, or a
				// descendant that escaped its group. Anything else (no
				// processes or descriptors left, a noexec TMPDIR, a sweep that
				// could not finish) is the machine's (review, #399).
				std::io::ErrorKind::InvalidInput | std::io::ErrorKind::TimedOut => SpawnError::Program(err),
				_ => SpawnError::Io(err),
			})
	}

	/// Run `command` as the programs' uid ([`Limits::run_as`], when set), to
	/// completion or `ceiling`, then kill its group and sweep that uid: for
	/// the runner's own work in what a program left (removing it), which must
	/// be done with no more right than the program had.
	///
	/// # Errors
	/// As [`Self::run`]'s machine failures.
	pub fn supervise_as_programs(&self, command: Command, ceiling: Duration) -> std::io::Result<Finished> {
		self.supervise(command, ceiling, self.limits.output_ceiling, self.limits.logs_ceiling)
	}

	/// `program` under `prlimit` (when installed): `memory` as its address
	/// space, the file-size and process ceilings, no core dump and no POSIX
	/// message queue.
	fn limited(&self, program: &Path, memory: Option<u64>) -> Command {
		let Some(prlimit) = &self.prlimit else {
			return Command::new(program);
		};
		let mut command = Command::new(prlimit);
		command.args(["--core=0", "--msgqueue=0"]);
		for (flag, ceiling) in [("--as=", memory), ("--fsize=", self.limits.file_size_ceiling), ("--nproc=", self.limits.process_ceiling)] {
			if let Some(ceiling) = ceiling {
				command.arg(OsString::from(String::from(flag) + &ceiling.to_string()));
			}
		}
		command.arg("--").arg(program);
		command
	}

	/// Start `command` as [`Limits::run_as`], when set, and supervise it,
	/// sweeping that uid afterwards.
	fn supervise(&self, mut command: Command, ceiling: Duration, stdout_cap: usize, stderr_cap: usize) -> std::io::Result<Finished> {
		if let Some(run_as) = self.limits.run_as {
			command.uid(run_as.uid).gid(run_as.gid);
		}
		process::supervise(command, ceiling, stdout_cap, stderr_cap, self.limits.run_as.map(|run_as| run_as.uid))
	}
}

/// What `rustc` (or rustup's proxy for it) and its linker need to be found,
/// and nothing else. Forwarded to the child as they are, not configuration
/// read around a binary's clap `Config` (#484).
fn forward_toolchain_env(command: &mut Command) {
	#[allow(clippy::disallowed_methods)]
	for key in ["PATH", "HOME", "RUSTUP_HOME", "RUSTUP_TOOLCHAIN", "CARGO_HOME"] {
		if let Some(value) = std::env::var_os(key) {
			command.env(key, value);
		}
	}
}

/// `text` with each panic header's thread id dropped: `thread 'main'
/// (3072) panicked` reads `thread 'main' panicked`, as it did before
/// toolchains added the id, so the same failure records the same bytes.
#[must_use]
pub fn without_thread_ids(text: &str) -> String {
	let mut out = String::with_capacity(text.len());
	let mut rest = text;
	while let Some(at) = rest.find("thread '") {
		let (before, from) = rest.split_at(at);
		out.push_str(before);
		let name_start = "thread '".len();
		let Some(name_len) = from[name_start..].find('\'') else {
			out.push_str(from);
			return out;
		};
		let header_end = name_start + name_len + 1;
		out.push_str(&from[..header_end]);
		let after = &from[header_end..];
		let id = after.strip_prefix(" (").and_then(|inner| {
			let digits = inner.find(|c: char| !c.is_ascii_digit())?;
			(digits > 0 && inner[digits..].starts_with(')')).then_some(2 + digits + 1)
		});
		rest = id.map_or(after, |skip| &after[skip..]);
	}
	out.push_str(rest);
	out
}

#[cfg(test)]
mod tests {
	use super::{without_thread_ids, Compiled, Ending};
	use std::os::unix::process::ExitStatusExt;

	#[test]
	fn the_rendered_head_never_splits_a_character() {
		let compiled = Compiled {
			binary: None,
			ending: Ending::Exited(std::process::ExitStatus::from_raw(256)),
			diagnostics: Vec::new(),
			rendered: String::from("aé"),
		};
		assert_eq!(compiled.rendered_head(2), ("a", true), "`é` is bytes 1..3");
		assert_eq!(compiled.rendered_head(3), ("aé", false));
		assert_eq!(compiled.rendered_head(9), ("aé", false));
		assert_eq!(compiled.rendered_head(0), ("", true));
	}

	#[test]
	fn a_panic_headers_thread_id_is_dropped_and_nothing_else() {
		assert_eq!(
			without_thread_ids("\nthread 'main' (3072) panicked at program.rs:1:13:\nboom"),
			"\nthread 'main' panicked at program.rs:1:13:\nboom"
		);
		assert_eq!(without_thread_ids("thread 'main' panicked"), "thread 'main' panicked");
		assert_eq!(without_thread_ids("thread 'w' (12x) and thread 'unterminated"), "thread 'w' (12x) and thread 'unterminated");
		assert_eq!(without_thread_ids("thread 'a' (1) then thread 'b' (22)"), "thread 'a' then thread 'b'");
	}
}

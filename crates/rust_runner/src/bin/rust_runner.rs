//! `rust-runner`: Rust Rounds' sandboxed recorder (paulgsc/some-ui#2234,
//! RR3).
//!
//! ```sh
//! rust-runner serve --socket /run/rust-runner/runner.sock --work /work --program-uid 10001 --program-gid 10001
//! echo '{"ask":"toolchain"}' | rust-runner ask --socket /run/rust-runner/runner.sock
//! ```
//!
//! `serve` answers one request at a time on a Unix socket
//! (`rust_runner::service`), recording each program it is asked to with the
//! pinned toolchain. It runs only in the `rust-runner` container
//! (`infra/docker/Dockerfile.rust-runner`, `infra/compose/rust_runner.yml`):
//! no network, no shared IPC, a read-only root, process, memory and CPU
//! caps. It treats every program as hostile.
//!
//! **Two uids.** `serve` is root, with only `CAP_SETUID`, `CAP_SETGID` and
//! `CAP_KILL`, and not dumpable. Every compile and run is started as
//! `--program-uid`, with no supplementary groups, so a program can neither
//! signal the runner (stop it, say, so its ceilings never fire) nor touch
//! its memory, its socket or its directories: the socket's directory and
//! `--work` are root's and writable by nobody else, and each program gets a
//! directory of its own inside `--work`. A connection from the programs' uid
//! is refused (`rust_runner::service`).
//!
//! **After every job** (RX2) the runner kills every process of the programs'
//! uid, has `clean` (this binary, as that uid, so with no more right than
//! the program had) remove everything in each program's directory, removes
//! the directories, and checks that no System V IPC object or POSIX message
//! queue is left and that the socket's directory holds only the socket it
//! bound. If any of that fails it exits, and the container's restart starts
//! it clean: fresh tmpfs, fresh IPC namespace.
//!
//! `ask` sends one request read from stdin and prints the answer: the
//! client the recording pass in `file_host` is, for an operator and the
//! sandbox check (`scripts/check_rust_runner_sandbox.py`).
//!
//! Exits 2 when it cannot start (no rustc or prlimit, not root, a directory
//! that is not its own, a socket it cannot bind) or has to stop.

use clap::{Parser, Subcommand};
use rust_runner::recording::Recorder;
use rust_runner::service::{answer, ask, Request, Response};
use rust_runner::{Ending, Limits, RunAs};
use rustix::process::getuid;
use std::io::{self, Read, Write};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::time::Duration;

/// How long `clean` may take over what one job left.
const CLEAN_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Parser, Debug)]
#[command(author, version, about = "Records what rustc and a Rust program print, one sandboxed program at a time")]
struct Args {
	#[command(subcommand)]
	command: Mode,
}

#[derive(Subcommand, Debug)]
enum Mode {
	/// Answer requests on a Unix socket, one at a time.
	Serve(Serve),
	/// Send one request, read from stdin, and print the answer.
	Ask {
		#[arg(long, env = "RUST_RUNNER_SOCKET")]
		socket: PathBuf,
		/// How long to wait for the answer, in seconds.
		#[arg(long, default_value_t = 120)]
		timeout_secs: u64,
	},
	/// Remove everything in each program's directory under `work`. Run by
	/// `serve`, as the programs' uid, after every job; refused as root.
	#[command(hide = true)]
	Clean { work: PathBuf },
}

#[derive(Parser, Debug)]
struct Serve {
	/// Where to listen. Its directory must be root's, writable by nobody
	/// else, and hold nothing but this socket, which is bound afresh.
	#[arg(long, env = "RUST_RUNNER_SOCKET")]
	socket: PathBuf,
	/// Root's directory, writable by nobody else; every program gets a fresh
	/// directory in it, removed after every request.
	#[arg(long, env = "RUST_RUNNER_WORK")]
	work: PathBuf,
	/// The compiler: a path, or a name looked up on PATH.
	#[arg(long, env = "RUST_RUNNER_RUSTC", default_value = "rustc")]
	rustc: PathBuf,
	/// Wall-clock ceiling on one run, in milliseconds.
	#[arg(long, env = "RUST_RUNNER_RUN_CEILING_MS", default_value_t = 2_000)]
	run_ceiling_ms: u64,
	/// How long one compile may take, in seconds.
	#[arg(long, env = "RUST_RUNNER_COMPILE_TIMEOUT_SECS", default_value_t = 60)]
	compile_timeout_secs: u64,
	/// Address-space ceiling on one run, in bytes (needs prlimit).
	#[arg(long, env = "RUST_RUNNER_MEMORY_CEILING", default_value_t = rust_runner::DEFAULT_MEMORY_CEILING)]
	memory_ceiling: u64,
	/// Address-space ceiling on one compile, in bytes (needs prlimit).
	#[arg(long, env = "RUST_RUNNER_COMPILE_MEMORY_CEILING", default_value_t = rust_runner::DEFAULT_COMPILE_MEMORY_CEILING)]
	compile_memory_ceiling: u64,
	/// The largest file one compile or run may write, in bytes (needs
	/// prlimit).
	#[arg(long, env = "RUST_RUNNER_FILE_SIZE_CEILING", default_value_t = rust_runner::DEFAULT_FILE_SIZE_CEILING)]
	file_size_ceiling: u64,
	/// `RLIMIT_NPROC` for one compile or run: processes and threads of the
	/// programs' uid (needs prlimit).
	#[arg(long, env = "RUST_RUNNER_PROCESS_CEILING", default_value_t = 64)]
	process_ceiling: u64,
	/// The uid every compile and run is started as, and every process of
	/// which is killed after each: the programs' own, which nothing else in
	/// the container runs as. Neither root nor the runner's.
	#[arg(long, env = "RUST_RUNNER_PROGRAM_UID")]
	program_uid: u32,
	/// The programs' gid; they get no supplementary groups.
	#[arg(long, env = "RUST_RUNNER_PROGRAM_GID")]
	program_gid: u32,
}

fn main() -> ExitCode {
	match Args::parse().command {
		Mode::Clean { work } => match clean(&work) {
			Ok(()) => ExitCode::SUCCESS,
			Err(error) => {
				let _ = writeln!(io::stderr(), "rust-runner clean: {error}");
				ExitCode::from(2)
			}
		},
		Mode::Serve(serve) => match run(&serve) {
			Ok(()) => ExitCode::SUCCESS,
			Err(error) => {
				let _ = writeln!(io::stderr(), "rust-runner: {error}");
				ExitCode::from(2)
			}
		},
		Mode::Ask { socket, timeout_secs } => {
			let mut input = Vec::new();
			let sent = io::stdin()
				.read_to_end(&mut input)
				.and_then(|_| serde_json::from_slice::<Request>(&input).map_err(io::Error::other))
				.and_then(|request| ask(&socket, &request, Duration::from_secs(timeout_secs)));
			let mut stdout = io::stdout().lock();
			match sent.and_then(|response| serde_json::to_writer(&mut stdout, &response).map_err(io::Error::other)) {
				Ok(()) => {
					let _ = writeln!(stdout);
					ExitCode::SUCCESS
				}
				Err(error) => {
					let _ = writeln!(io::stderr(), "rust-runner ask: {error}");
					ExitCode::from(2)
				}
			}
		}
	}
}

fn run(serve: &Serve) -> Result<(), String> {
	if !getuid().is_root() {
		return Err(String::from(
			"serve starts every compile and run as --program-uid, which needs root (with CAP_SETUID and CAP_SETGID)",
		));
	}
	if serve.program_uid == 0 || serve.program_gid == 0 {
		return Err(String::from("--program-uid and --program-gid must not be root's"));
	}
	// No core of the runner, and its /proc files stay root's alone (RX1).
	rustix::process::set_dumpable_behavior(rustix::process::DumpableBehavior::NotDumpable)
		.map_err(|err| String::from("could not make the runner not dumpable: ") + &err.to_string())?;
	let socket_dir = serve
		.socket
		.parent()
		.filter(|dir| !dir.as_os_str().is_empty())
		.ok_or_else(|| String::from("--socket names no directory"))?;
	own(&serve.work)?;
	own(socket_dir)?;
	if same_file(socket_dir, &serve.work) {
		return Err(String::from("--socket's directory must not be --work"));
	}

	let limits = Limits {
		rustc: serve.rustc.clone(),
		compile_timeout: Duration::from_secs(serve.compile_timeout_secs),
		run_ceiling: Duration::from_millis(serve.run_ceiling_ms),
		memory_ceiling: (serve.memory_ceiling > 0).then_some(serve.memory_ceiling),
		compile_memory_ceiling: (serve.compile_memory_ceiling > 0).then_some(serve.compile_memory_ceiling),
		file_size_ceiling: (serve.file_size_ceiling > 0).then_some(serve.file_size_ceiling),
		process_ceiling: (serve.process_ceiling > 0).then_some(serve.process_ceiling),
		run_as: Some(RunAs {
			uid: serve.program_uid,
			gid: serve.program_gid,
		}),
		..Limits::default()
	};
	let recorder = Recorder::new(limits, serve.work.clone()).map_err(|err| err.to_string())?;
	if !recorder.compiler().memory_ceiling_applied() {
		return Err(String::from("prlimit is not installed, so runs would have no memory or process ceiling"));
	}
	tidy_work(&recorder, &serve.work)?;

	// The socket's directory holds the socket and nothing else: an old one is
	// removed, and anything more means the directory is not the runner's own.
	for entry in std::fs::read_dir(socket_dir).map_err(|err| failed("could not read", socket_dir, &err))? {
		let path = entry.map_err(|err| err.to_string())?.path();
		if path != serve.socket {
			return Err(failed("--socket's directory must hold nothing but the socket, and holds", &path, &"an entry"));
		}
		std::fs::remove_file(&path).map_err(|err| failed("could not remove the old", &path, &err))?;
	}
	let listener = UnixListener::bind(&serve.socket).map_err(|err| failed("could not bind", &serve.socket, &err))?;
	// Whoever shares the socket's volume may ask (file_host, as its own
	// non-root uid); a program, which can reach it too, is refused by uid.
	std::fs::set_permissions(&serve.socket, std::fs::Permissions::from_mode(0o666)).map_err(|err| failed("could not open to its clients", &serve.socket, &err))?;
	let bound = std::fs::metadata(&serve.socket).map_err(|err| failed("could not read", &serve.socket, &err))?.ino();
	let _ = writeln!(io::stderr(), "rust-runner: {} listening on {}", recorder.toolchain(), serve.socket.display());

	for stream in listener.incoming() {
		let Ok(stream) = stream else { continue };
		let response = answer(&recorder, &stream).map_err(|err| err.to_string())?;
		drop(stream);
		// What was answered, never the program or what it printed.
		let line = match &response {
			Response::Toolchain { .. } => String::from("toolchain"),
			Response::Recorded { recording, .. } => {
				String::from("recorded ") + recording.outcome.as_str() + " in " + &recording.run.as_ref().map_or(0, |run| run.milliseconds).to_string() + " ms"
			}
			Response::Refused { reason, .. } => String::from("refused ") + reason.as_str(),
		};
		let _ = writeln!(io::stderr(), "rust-runner: {line}");
		tidy(serve, &recorder, socket_dir, bound)?;
	}
	Ok(())
}

/// Undo whatever a program did (RX2): remove every program's directory, check
/// no IPC object outlived it, and check the socket's directory still holds
/// only the socket bound. Anything that cannot be undone stops the runner,
/// and the container's restart starts it clean.
fn tidy(serve: &Serve, recorder: &Recorder, socket_dir: &Path, bound: u64) -> Result<(), String> {
	tidy_work(recorder, &serve.work)?;
	for table in ["shm", "sem", "msg"] {
		let path = Path::new("/proc/sysvipc").join(table);
		let listed = std::fs::read_to_string(&path).map_err(|err| failed("could not read", &path, &err))?;
		if listed.lines().count() > 1 {
			return Err(failed("a program left a System V IPC object, in", &path, &"restarting into a fresh IPC namespace"));
		}
	}
	for dir in ["/dev/shm", "/dev/mqueue"] {
		let dir = Path::new(dir);
		if let Ok(mut entries) = std::fs::read_dir(dir) {
			if entries.next().is_some() {
				return Err(failed("a program left something in", dir, &"restarting into a fresh IPC namespace"));
			}
		}
	}
	own(socket_dir)?;
	for entry in std::fs::read_dir(socket_dir).map_err(|err| failed("could not read", socket_dir, &err))? {
		let path = entry.map_err(|err| err.to_string())?.path();
		if path != serve.socket {
			return Err(failed("something other than the socket appeared:", &path, &"the directory is root's"));
		}
	}
	match std::fs::symlink_metadata(&serve.socket) {
		Ok(metadata) if metadata.ino() == bound => Ok(()),
		_ => Err(failed("the socket is no longer the one the runner bound:", &serve.socket, &"replaced or removed")),
	}
}

/// Remove every program's directory in `work`. Each is the runner's own, open
/// to the programs' uid; what a program made inside it is removed by `clean`
/// as that uid, so the runner never needs a right over a program's files
/// (a directory it made mode 000, say) that the program did not have.
fn tidy_work(recorder: &Recorder, work: &Path) -> Result<(), String> {
	own(work)?;
	let mut jobs = Vec::new();
	for entry in std::fs::read_dir(work).map_err(|err| failed("could not read", work, &err))? {
		let path = entry.map_err(|err| err.to_string())?.path();
		let metadata = std::fs::symlink_metadata(&path).map_err(|err| failed("could not read", &path, &err))?;
		if !metadata.is_dir() || metadata.uid() != getuid().as_raw() {
			return Err(failed("--work holds something the runner did not make:", &path, &"--work is root's"));
		}
		jobs.push(path);
	}
	if jobs.is_empty() {
		return Ok(());
	}
	let exe = std::env::current_exe().map_err(|err| String::from("could not find the runner's own binary: ") + &err.to_string())?;
	let mut command = Command::new(exe);
	command.arg("clean").arg(work).env_clear();
	let cleaned = recorder
		.compiler()
		.supervise_as_programs(command, CLEAN_TIMEOUT)
		.map_err(|err| String::from("could not run clean: ") + &err.to_string())?;
	if !matches!(cleaned.ending, Ending::Exited(status) if status.success()) {
		return Err(String::from("clean could not remove what a program left: ") + String::from_utf8_lossy(&cleaned.stderr.head).trim());
	}
	for job in jobs {
		std::fs::remove_dir(&job).map_err(|err| failed("could not remove", &job, &err))?;
	}
	Ok(())
}

/// `clean`: as the programs' uid, remove everything in each of `work`'s
/// directories (each the runner's, open to that uid). A directory of the
/// uid's is opened up (`u+rwx`) before it is read, so one a program made
/// unreadable goes too. Run only once every process of the uid is gone, so
/// nothing can swap a path between its check and its removal.
fn clean(work: &Path) -> Result<(), String> {
	if getuid().is_root() {
		return Err(String::from("clean runs as the programs' uid, with no more right than theirs, never as root"));
	}
	for job in std::fs::read_dir(work).map_err(|err| failed("could not read", work, &err))? {
		let job = job.map_err(|err| err.to_string())?.path();
		for entry in std::fs::read_dir(&job).map_err(|err| failed("could not read", &job, &err))? {
			let path = entry.map_err(|err| err.to_string())?.path();
			remove_tree(&path).map_err(|err| failed("could not remove", &path, &err))?;
		}
	}
	Ok(())
}

fn remove_tree(path: &Path) -> io::Result<()> {
	let metadata = std::fs::symlink_metadata(path)?;
	if !metadata.is_dir() {
		return std::fs::remove_file(path);
	}
	if metadata.uid() == getuid().as_raw() {
		std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
	}
	for entry in std::fs::read_dir(path)? {
		remove_tree(&entry?.path())?;
	}
	std::fs::remove_dir(path)
}

/// Whether `dir` is a directory of the runner's own: not a link, its uid's,
/// and writable by nobody else, so only the runner (and what it opens up)
/// can put anything in it.
fn own(dir: &Path) -> Result<(), String> {
	let metadata = std::fs::symlink_metadata(dir).map_err(|err| failed("could not read", dir, &err))?;
	if metadata.is_dir() && metadata.uid() == getuid().as_raw() && metadata.mode() & 0o022 == 0 {
		Ok(())
	} else {
		Err(failed("is not a directory of the runner's own:", dir, &"it must be its uid's and writable by nobody else"))
	}
}

fn same_file(one: &Path, other: &Path) -> bool {
	match (std::fs::metadata(one), std::fs::metadata(other)) {
		(Ok(one), Ok(other)) => one.dev() == other.dev() && one.ino() == other.ino(),
		_ => false,
	}
}

/// `<what> <path>: <err>`, a line for the log.
fn failed(what: &str, path: &Path, err: &dyn std::fmt::Display) -> String {
	String::from(what) + " " + &path.display().to_string() + ": " + &err.to_string()
}

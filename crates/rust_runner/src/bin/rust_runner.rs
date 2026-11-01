//! `rust-runner`: Rust Rounds' sandboxed recorder (paulgsc/some-ui#2234,
//! RR3).
//!
//! ```sh
//! rust-runner serve --socket /run/rust-runner/runner.sock --work /work \
//!   --program-uid 10101 --program-gid 10101 --client-uid 10000
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
//! **Two uids.** `serve` is root with only `CAP_SETUID` and `CAP_SETGID`,
//! not dumpable and with no new privileges. Every compile and run is started
//! as `--program-uid`, with no supplementary groups, so a program can
//! neither signal the runner (stop it, say, so its ceilings never fire) nor
//! touch its memory, its socket or its directories: the socket's directory
//! and `--work` are root's and writable by nobody else, and each program gets
//! a directory of its own inside `--work`. Only `--client-uid` and root are
//! served on the socket (`rust_runner::service`).
//!
//! **After every job** (RX2) every process of the programs' uid is killed
//! (by a helper running as that uid: the runner signals nothing), `clean`
//! (this binary, as that uid, so with no more right than the program had)
//! removes everything in each program's directory, the runner removes the
//! now empty directories, and checks that no System V IPC object or POSIX message
//! queue is left and that the socket's directory holds only the socket it
//! bound. If any of that fails it exits, and the container's restart starts
//! it clean: fresh tmpfs, fresh IPC namespace, and (the runner is the
//! container's PID 1) no process left. A watchdog thread bounds every job's
//! whole life the same way, for the one wait nothing else can bound: a
//! helper of the programs' uid that a survivor stopped before its `exec`.
//!
//! `ask` sends one request read from stdin and prints the answer: the
//! client the recording pass in `file_host` is, for an operator and the
//! sandbox check (`scripts/check_rust_runner_sandbox.py`).
//!
//! **No environment of its own** (#616, option B): every setting is a flag,
//! which the image passes in its `CMD` (`infra/docker/Dockerfile.rust-runner`),
//! so a deployment states nothing for it and no variable can change how it
//! runs. A test below holds every flag to that.
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
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

// `clean`'s timeout, the watchdog's sweep allowance and the job deadline
// they sum to are `rust_rounds_wire`'s, so the client's wait (`ASK_TIMEOUT`)
// is derived from the same numbers and outlasts the watchdog.
use rust_runner::service::{job_deadline, ASK_TIMEOUT, CLEAN_TIMEOUT, SWEEP_ALLOWANCE};

/// Where the image's `serve` listens: the volume `file_host` mounts
/// (`infra/compose/rust_runner.yml`), and `file_host`'s own default.
const SOCKET: &str = "/run/rust-runner/runner.sock";

/// How often the watchdog looks at the clock.
const WATCH_POLL: Duration = Duration::from_millis(100);

/// The backstop that bounds every job, whatever its processes do (RX3).
///
/// A thread of the runner's own, which no process of the programs' uid can
/// signal or stop. A job's whole life, from accepting its connection to the
/// end of its tidy, is armed with a deadline; if the job is still running
/// past it (a helper stopped before its `exec` leaves `spawn` blocked, which
/// nothing on the job's own thread can bound), the watchdog exits the
/// runner. In the container the runner is PID 1, so its exit kills every
/// process in the container's PID namespace, and `restart: unless-stopped`
/// brings it back clean. The client sees its connection dropped.
struct Watchdog {
	origin: Instant,
	/// Milliseconds since `origin` at which the armed job is overdue; 0 when
	/// no job is armed.
	until: AtomicU64,
}

impl Watchdog {
	fn start() -> Arc<Self> {
		let dog = Arc::new(Self {
			origin: Instant::now(),
			until: AtomicU64::new(0),
		});
		let watching = Arc::clone(&dog);
		std::thread::spawn(move || loop {
			std::thread::sleep(WATCH_POLL);
			let until = watching.until.load(Ordering::SeqCst);
			if until != 0 && watching.now() >= until {
				let _ = writeln!(io::stderr(), "rust-runner: a job outlived its deadline; exiting, so the restart clears it");
				std::process::exit(3);
			}
		});
		dog
	}

	fn now(&self) -> u64 {
		u64::try_from(self.origin.elapsed().as_millis()).unwrap_or(u64::MAX)
	}

	fn arm(&self, budget: Duration) {
		let budget = u64::try_from(budget.as_millis()).unwrap_or(u64::MAX);
		self.until.store(self.now().saturating_add(budget).max(1), Ordering::SeqCst);
	}

	fn disarm(&self) {
		self.until.store(0, Ordering::SeqCst);
	}
}

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
		/// The socket `serve` listens on; the image's, by default.
		#[arg(long, default_value = SOCKET)]
		socket: PathBuf,
		/// How long to wait for the answer, in seconds. The default outlasts
		/// the watchdog's job deadline (`ASK_TIMEOUT`).
		#[arg(long, default_value_t = ASK_TIMEOUT.as_secs())]
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
	#[arg(long)]
	socket: PathBuf,
	/// Root's directory, writable by nobody else; every program gets a fresh
	/// directory in it, removed after every request.
	#[arg(long)]
	work: PathBuf,
	/// The compiler: a path, or a name looked up on PATH.
	#[arg(long, default_value = "rustc")]
	rustc: PathBuf,
	/// Wall-clock ceiling on one run, in milliseconds.
	#[arg(long, default_value_t = 2_000)]
	run_ceiling_ms: u64,
	/// How long one compile may take, in seconds.
	#[arg(long, default_value_t = 60)]
	compile_timeout_secs: u64,
	/// Address-space ceiling on one run, in bytes (needs prlimit).
	#[arg(long, default_value_t = rust_runner::DEFAULT_MEMORY_CEILING)]
	memory_ceiling: u64,
	/// Address-space ceiling on one compile, in bytes (needs prlimit).
	#[arg(long, default_value_t = rust_runner::DEFAULT_COMPILE_MEMORY_CEILING)]
	compile_memory_ceiling: u64,
	/// The largest file one compile or run may write, in bytes (needs
	/// prlimit).
	#[arg(long, default_value_t = rust_runner::DEFAULT_FILE_SIZE_CEILING)]
	file_size_ceiling: u64,
	/// `RLIMIT_NPROC` for one compile or run: processes and threads of the
	/// programs' uid (needs prlimit).
	#[arg(long, default_value_t = 64)]
	process_ceiling: u64,
	/// The uid every compile and run is started as, and every process of
	/// which is killed after each: the programs' own, which nothing else in
	/// the container runs as. Neither root nor the runner's.
	#[arg(long)]
	program_uid: u32,
	/// The programs' gid; they get no supplementary groups. Like the uid,
	/// one no service in the stack runs as.
	#[arg(long)]
	program_gid: u32,
	/// The uid whose connections are served (`file_host`'s), besides root's;
	/// every other connection is refused unread.
	#[arg(long)]
	client_uid: u32,
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
	if serve.client_uid == serve.program_uid {
		return Err(String::from("--client-uid must not be the programs' uid"));
	}
	// No core of the runner, its /proc files root's alone, and nothing it
	// execs gains a privilege (RX1).
	rustix::process::set_dumpable_behavior(rustix::process::DumpableBehavior::NotDumpable)
		.map_err(|err| String::from("could not make the runner not dumpable: ") + &err.to_string())?;
	rustix::thread::set_no_new_privs(true).map_err(|err| String::from("could not set no-new-privileges: ") + &err.to_string())?;
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
	let watchdog = Watchdog::start();
	let job_deadline = serve_job_deadline(recorder.compiler().limits());
	watchdog.arm(CLEAN_TIMEOUT + SWEEP_ALLOWANCE);
	tidy_work(&recorder, &serve.work)?;
	watchdog.disarm();

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
	// Anything that shares the socket's volume can connect, a program
	// included; only the clients' uids are served (the primary defence).
	std::fs::set_permissions(&serve.socket, std::fs::Permissions::from_mode(0o666)).map_err(|err| failed("could not open to its clients", &serve.socket, &err))?;
	let bound = std::fs::metadata(&serve.socket).map_err(|err| failed("could not read", &serve.socket, &err))?.ino();
	let clients = [0, serve.client_uid];
	let _ = writeln!(io::stderr(), "rust-runner: {} listening on {}", recorder.toolchain(), serve.socket.display());

	for stream in listener.incoming() {
		let Ok(stream) = stream else { continue };
		watchdog.arm(job_deadline);
		let response = answer(&recorder, &stream, Some(&clients)).map_err(|err| err.to_string())?;
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
		watchdog.disarm();
	}
	Ok(())
}

/// The watchdog's deadline for one job: every ceiling it can spend, plus
/// [`SWEEP_ALLOWANCE`]. 132 s with the defaults.
const fn serve_job_deadline(limits: &Limits) -> Duration {
	job_deadline(limits.compile_timeout, limits.run_ceiling)
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
/// to the programs' uid. The runner reads only `work` itself, never inside a
/// program's directory: everything in one is removed by `clean` as the
/// programs' uid, so the runner never walks a tree a program made, nor needs
/// a right over it (a directory made mode 000, say) the program lacked. Then
/// the runner removes the directories, empty.
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
	let jobs: Vec<PathBuf> = std::fs::read_dir(work)
		.map_err(|err| failed("could not read", work, &err))?
		.map(|entry| entry.map(|entry| entry.path()))
		.collect::<io::Result<_>>()
		.map_err(|err| failed("could not read", work, &err))?;
	for job in jobs {
		empty_tree(&job).map_err(|err| failed("could not empty", &job, &err))?;
	}
	Ok(())
}

/// Remove everything under `dir`, however deep: by name relative to the
/// working directory, moving down and back up with `chdir`, so no path
/// grows past `PATH_MAX`, nothing recurses, and at most one listing per
/// level is held. Only real directories (by `lstat`) are entered.
fn empty_tree(dir: &Path) -> io::Result<()> {
	std::env::set_current_dir(dir)?;
	// For each level entered: its name, and the directories in it still to
	// empty.
	let mut levels: Vec<(Option<std::ffi::OsString>, Vec<std::ffi::OsString>)> = vec![(None, unlink_here()?)];
	while let Some((_, pending)) = levels.last_mut() {
		if let Some(next) = pending.pop() {
			if let Err(err) = std::env::set_current_dir(&next) {
				// A host that mediates by full path (AppArmor) refuses to
				// enter a directory past its buffer, so it goes as it is: if
				// it is not empty (a program can rename a filled subtree
				// down there), removal fails and the runner restarts (RX2).
				if too_long(&err) && std::fs::remove_dir(&next).is_ok() {
					continue;
				}
				return Err(err);
			}
			levels.push((Some(next), unlink_here()?));
		} else if let Some((Some(name), _)) = levels.pop() {
			std::env::set_current_dir("..")?;
			std::fs::remove_dir(&name)?;
		}
	}
	std::env::set_current_dir("/")
}

/// Unlink everything in the working directory that is not a directory, and
/// return the directories, each opened to its owner (when it is this uid)
/// so it can be entered and listed.
fn unlink_here() -> io::Result<Vec<std::ffi::OsString>> {
	let mut dirs = Vec::new();
	for entry in std::fs::read_dir(".")? {
		let name = entry?.file_name();
		let metadata = std::fs::symlink_metadata(&name)?;
		if metadata.is_dir() {
			if metadata.uid() == getuid().as_raw() {
				// Past a full-path host's buffer the chmod is refused too;
				// entering it then decides (above).
				match std::fs::set_permissions(&name, std::fs::Permissions::from_mode(0o700)) {
					Err(err) if !too_long(&err) => return Err(err),
					_ => {}
				}
			}
			dirs.push(name);
		} else {
			std::fs::remove_file(&name)?;
		}
	}
	Ok(dirs)
}

/// `ENAMETOOLONG`, which a host whose LSM mediates by full path returns for
/// a relative name once the whole path is past its buffer.
fn too_long(err: &io::Error) -> bool {
	err.raw_os_error() == Some(36)
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

#[cfg(test)]
mod tests {
	use super::Args;
	use clap::CommandFactory;

	/// Every flag of every mode, with its mode's name.
	fn flags(command: &clap::Command, path: &str, out: &mut Vec<(String, Option<String>)>) {
		for arg in command.get_arguments() {
			out.push((
				path.to_owned() + " --" + arg.get_long().unwrap_or_else(|| arg.get_id().as_str()),
				arg.get_env().map(|env| env.to_string_lossy().into_owned()),
			));
		}
		for sub in command.get_subcommands() {
			flags(sub, &(path.to_owned() + " " + sub.get_name()), out);
		}
	}

	#[test]
	fn no_flag_reads_the_environment() {
		let mut all = Vec::new();
		flags(&Args::command(), "rust-runner", &mut all);
		assert!(all.iter().any(|(flag, _)| flag == "rust-runner serve --program-uid"), "{all:?}");
		let read: Vec<_> = all.iter().filter(|(_, env)| env.is_some()).collect();
		assert!(read.is_empty(), "the runner takes flags only (#616, option B): {read:?}");
	}
}

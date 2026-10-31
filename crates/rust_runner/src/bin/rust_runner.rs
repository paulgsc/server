//! `rust-runner`: Rust Rounds' sandboxed recorder (paulgsc/some-ui#2234,
//! RR3).
//!
//! ```sh
//! rust-runner serve --socket /run/rust-runner/runner.sock --work /work --sweep-uid
//! echo '{"ask":"toolchain"}' | rust-runner ask --socket /run/rust-runner/runner.sock
//! ```
//!
//! `serve` answers one request at a time on a Unix socket
//! (`rust_runner::service`), recording each program it is asked to with the
//! pinned toolchain. It runs only in the `rust-runner` container
//! (`infra/docker/Dockerfile.rust-runner`, `infra/compose/rust_runner.yml`):
//! no network, a read-only root, a uid of its own, process, memory and CPU
//! caps. It treats every program as hostile. After every request it empties
//! `--work`; if anything there cannot be removed, or a process of its uid
//! survives the sweep, it exits, and the container's restart starts it clean.
//!
//! `ask` sends one request read from stdin and prints the answer: the
//! client the recording pass in `file_host` is, for an operator and the sandbox check
//! (`scripts/check_rust_runner_sandbox.py`).
//!
//! Exits 2 when it cannot start (no rustc, a socket it cannot bind, a sweep
//! asked for as root) or has to stop.

use clap::{Parser, Subcommand};
use rust_runner::recording::Recorder;
use rust_runner::service::{answer, ask, Request, Response};
use rust_runner::{Limits, Reap};
use std::io::{self, Read, Write};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

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
}

#[derive(Parser, Debug)]
struct Serve {
	/// Where to listen. Removed and bound afresh at start.
	#[arg(long, env = "RUST_RUNNER_SOCKET")]
	socket: PathBuf,
	/// A directory the runner owns; every program gets a fresh directory in
	/// it, and it is emptied after every request.
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
	/// `RLIMIT_NPROC` for one run: processes and threads of the runner's uid
	/// (needs prlimit; not enforced for root).
	#[arg(long, env = "RUST_RUNNER_PROCESS_CEILING", default_value_t = 64)]
	process_ceiling: u64,
	/// After every compile and run, kill every other process of the
	/// runner's uid. Only under a uid the runner has to itself; refused as
	/// root.
	#[arg(long, env = "RUST_RUNNER_SWEEP_UID")]
	sweep_uid: bool,
}

fn main() -> ExitCode {
	match Args::parse().command {
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
	if serve.sweep_uid && rustix::process::getuid().is_root() {
		return Err(String::from("--sweep-uid as root would kill every process on the machine; run as a uid of its own"));
	}
	let limits = Limits {
		rustc: serve.rustc.clone(),
		compile_timeout: Duration::from_secs(serve.compile_timeout_secs),
		run_ceiling: Duration::from_millis(serve.run_ceiling_ms),
		memory_ceiling: (serve.memory_ceiling > 0).then_some(serve.memory_ceiling),
		process_ceiling: (serve.process_ceiling > 0).then_some(serve.process_ceiling),
		reap: if serve.sweep_uid { Reap::Uid } else { Reap::Group },
		..Limits::default()
	};
	empty(&serve.work).map_err(|err| failed("could not empty", &serve.work, &err))?;
	let recorder = Recorder::new(limits, serve.work.clone()).map_err(|err| err.to_string())?;
	if !recorder.compiler().memory_ceiling_applied() {
		return Err(String::from("prlimit is not installed, so runs would have no memory or process ceiling"));
	}

	match std::fs::remove_file(&serve.socket) {
		Ok(()) => {}
		Err(err) if err.kind() == io::ErrorKind::NotFound => {}
		Err(err) => return Err(failed("could not remove the old", &serve.socket, &err)),
	}
	let listener = UnixListener::bind(&serve.socket).map_err(|err| failed("could not bind", &serve.socket, &err))?;
	// Whoever shares the socket's volume may ask; the volume is the boundary.
	std::fs::set_permissions(&serve.socket, std::fs::Permissions::from_mode(0o666)).map_err(|err| failed("could not open to its clients", &serve.socket, &err))?;
	let bound = std::fs::metadata(&serve.socket).map_err(|err| failed("could not read", &serve.socket, &err))?.ino();
	let _ = writeln!(io::stderr(), "rust-runner: {} listening on {}", recorder.toolchain(), serve.socket.display());

	for stream in listener.incoming() {
		let Ok(stream) = stream else { continue };
		let response = answer(&recorder, &stream).map_err(|err| err.to_string())?;
		// What was answered, never the program or what it printed.
		let line = match &response {
			Response::Toolchain { .. } => String::from("toolchain"),
			Response::Recorded { recording, .. } => {
				String::from("recorded ") + recording.outcome.as_str() + " in " + &recording.run.as_ref().map_or(0, |run| run.milliseconds).to_string() + " ms"
			}
			Response::Refused { reason, .. } => String::from("refused ") + reason.as_str(),
		};
		let _ = writeln!(io::stderr(), "rust-runner: {line}");
		tidy(serve, bound)?;
	}
	Ok(())
}

/// Undo whatever a program did to the two places it can write (RX2): empty
/// `--work`, remove everything beside the socket in its directory, and check
/// the socket is still the one bound. Anything that cannot be undone stops
/// the runner, and the container's restart starts it clean.
fn tidy(serve: &Serve, bound: u64) -> Result<(), String> {
	empty(&serve.work).map_err(|err| failed("a program left something the runner cannot remove in", &serve.work, &err))?;
	if let Some(dir) = serve.socket.parent() {
		let entries = std::fs::read_dir(dir).map_err(|err| failed("could not read", dir, &err))?;
		for entry in entries {
			let path = entry.map_err(|err| err.to_string())?.path();
			if path != serve.socket {
				remove(&path).map_err(|err| failed("a program left something that cannot be removed:", &path, &err))?;
			}
		}
	}
	match std::fs::symlink_metadata(&serve.socket) {
		Ok(metadata) if metadata.ino() == bound => Ok(()),
		_ => Err(failed("the socket is no longer the one the runner bound:", &serve.socket, &"replaced or removed")),
	}
}

/// `<what> <path>: <err>`, a line for the log.
fn failed(what: &str, path: &Path, err: &dyn std::fmt::Display) -> String {
	String::from(what) + " " + &path.display().to_string() + ": " + &err.to_string()
}

/// Remove everything in `dir`, which a program may have written to.
fn empty(dir: &Path) -> io::Result<()> {
	for entry in std::fs::read_dir(dir)? {
		remove(&entry?.path())?;
	}
	Ok(())
}

fn remove(path: &Path) -> io::Result<()> {
	if path.is_dir() && !path.is_symlink() {
		std::fs::remove_dir_all(path)
	} else {
		std::fs::remove_file(path)
	}
}

//! Running one child process under a wall-clock ceiling, with its output
//! captured to a ceiling and never streamed anywhere.
//!
//! `std::process` and `rustix` only, no `unsafe`: the child is put in a
//! process group of its own (`process_group(0)`), polled with `try_wait`, and
//! on the ceiling it is killed before it is reaped, and killed again once it
//! has ended, however it ended, so nothing it spawned outlives the run.
//!
//! **Who kills.** Without a uid of the programs' own, the whole group is sent
//! `SIGKILL` (`kill(-pgid)`, through `rustix`, so killing needs no new
//! process: a program that used up its process ceiling cannot stop the kill).
//! With one, the runner (root) signals nothing itself and needs no
//! `CAP_KILL`: it starts a helper *as that uid*, `sh -c 'kill -KILL -1'`,
//! which kills every process the uid may signal, which is every process of
//! it and nothing else, a descendant that left the group (`setsid`)
//! included. Nothing is signalled by pid, so there is no window in which a
//! pid read from `/proc` is reused by another process. `/proc` is only read,
//! to see whether a pass left anything alive.
//!
//! **The helper can be signalled, so the sweep is not the last resort.**
//! From its fork on, the helper is a process of the programs' uid, and a
//! survivor can stop it (`SIGSTOP`) or kill it. Each helper is waited for
//! [`HELPER_TIMEOUT`] at most, and a sweep that cannot finish is the
//! machine's failure: the runner stops, and the container's restart clears
//! everything. A helper stopped before its `exec` blocks `spawn` itself,
//! which no deadline here can bound; the runner's watchdog (`rust-runner
//! serve`) does, by exiting.
//!
//! stdout and stderr are drained by a thread each, so a chatty child never
//! blocks on a full pipe, but only the first `cap` bytes (and, for the error
//! tail, the last `cap`) are kept.

use rustix::process::{getuid, kill_process_group, wait, Pid, Signal, WaitOptions};
use std::io::Read;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// How often a running child is checked on. The elapsed time a run reports
/// is therefore good to about a millisecond: timings are illustrative, never
/// graded.
const POLL: Duration = Duration::from_millis(1);

/// How many times the sweep of a uid kills before giving up. `kill(-1)` walks
/// the process list with forks held off, and a process with `SIGKILL`
/// pending cannot fork, so one pass is enough but for a process caught
/// mid-exec.
const SWEEP_PASSES: usize = 200;

/// How long one helper may take, from its `exec` to its exit. It takes a
/// few milliseconds unless something of the uid has stopped it.
pub(crate) const HELPER_TIMEOUT: Duration = Duration::from_secs(5);

/// The helper that kills every process of the uid it runs as. `dash`'s
/// builtin, so the kill needs no binary of its own.
const SWEEPER: [&str; 3] = ["/bin/sh", "-c", "kill -KILL -1"];

/// A uid of the programs' own, and its gid: run as by the sweep's helper.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Sweep {
	pub uid: u32,
	pub gid: u32,
}

/// One stream's bytes, kept to a ceiling.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Captured {
	/// The first `cap` bytes.
	pub head: Vec<u8>,
	/// The last `cap` bytes (the whole stream, when it fit).
	pub tail: Vec<u8>,
	/// How many bytes the stream carried in all.
	pub total: u64,
}

impl Captured {
	/// Whether anything was dropped.
	#[must_use]
	pub fn truncated(&self) -> bool {
		u64::try_from(self.head.len()).map_or(true, |kept| kept < self.total)
	}
}

/// How a supervised child ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ending {
	/// It exited (or was killed by a signal it raised itself) within the
	/// ceiling.
	Exited(ExitStatus),
	/// It was still running at the ceiling and was killed.
	TimedOut,
}

/// What a supervised child did.
#[derive(Debug)]
pub struct Finished {
	pub ending: Ending,
	/// From spawn to the exit being seen.
	pub elapsed: Duration,
	pub stdout: Captured,
	pub stderr: Captured,
}

/// Run `command` to completion or to `ceiling`, whichever comes first, with
/// stdin closed and stdout and stderr kept to `stdout_cap` and `stderr_cap`
/// bytes. On the ceiling, and once it has ended, its process group is
/// killed, or with `sweep` (the child runs as that uid), every process of
/// the uid.
///
/// # Errors
/// If the child cannot be started or waited on, or a process of `sweep`'s
/// uid survives every pass of the sweep; everything the child itself does is
/// in [`Finished`]. An error starting the child (no pipes or descriptors
/// left, no memory, a fork or exec the kernel refuses) keeps its kind and is
/// marked as such ([`not_started`]): nothing of the child ran.
pub(crate) fn supervise(mut command: Command, ceiling: Duration, stdout_cap: usize, stderr_cap: usize, sweep: Option<Sweep>) -> std::io::Result<Finished> {
	command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).process_group(0);
	let started = Instant::now();
	let mut child = command.spawn().map_err(|err| std::io::Error::new(err.kind(), NotStarted(err)))?;
	let stdout = child.stdout.take().map(|stream| drain(stream, stdout_cap));
	let stderr = child.stderr.take().map(|stream| drain(stream, stderr_cap));
	let ending = loop {
		match child.try_wait() {
			Ok(Some(status)) => break Ending::Exited(status),
			Ok(None) => {}
			Err(err) => {
				let _ = stop(&mut child, sweep);
				let _ = child.wait();
				return Err(err);
			}
		}
		if started.elapsed() >= ceiling {
			stop(&mut child, sweep)?;
			child.wait()?;
			break Ending::TimedOut;
		}
		std::thread::sleep(POLL);
	};
	let elapsed = started.elapsed();
	// Whatever the child left behind in its group goes too, however it
	// ended: a descendant still holding stdout would otherwise outlive the
	// ceiling, and keep the readers below waiting for it (review, #399).
	stop(&mut child, sweep)?;
	if sweep.is_some() {
		reap_orphans();
	}
	let deadline = Instant::now() + READER_GRACE;
	Ok(Finished {
		ending,
		elapsed,
		stdout: joined(stdout, deadline)?,
		stderr: joined(stderr, deadline)?,
	})
}

/// [`supervise`]'s error when the child could not be started: the spawn's
/// own error, kept whole.
#[derive(Debug)]
struct NotStarted(std::io::Error);

impl std::fmt::Display for NotStarted {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "could not be started: {}", self.0)
	}
}

impl std::error::Error for NotStarted {
	fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
		Some(&self.0)
	}
}

/// Whether `err` is [`supervise`]'s failure to start its child, before
/// anything of it ran: the machine's alone, whatever the child would have
/// been.
pub(crate) fn not_started(err: &std::io::Error) -> bool {
	err.get_ref().is_some_and(<dyn std::error::Error + Send + Sync>::is::<NotStarted>)
}

/// How long the output readers get to see end-of-file once the child's
/// group is gone. Only a descendant that left the group (`setsid`) can still
/// hold a pipe then; past this the run is an error rather than a hang.
const READER_GRACE: Duration = Duration::from_secs(1);

/// Kill the child and everything it started: its group, or with `sweep`,
/// every process of the uid.
fn stop(child: &mut Child, sweep: Option<Sweep>) -> std::io::Result<()> {
	if let Some(sweep) = sweep {
		return sweep_uid(sweep);
	}
	kill_group(child);
	Ok(())
}

/// `SIGKILL` the child's whole process group (a compile's linker, say, as
/// well as `rustc`), and the child itself in case the group is already
/// gone. A system call, not a `kill` process: under a process ceiling a
/// program can leave no room for one.
fn kill_group(child: &mut Child) {
	let _ = kill_process_group(Pid::from_child(child), Signal::KILL);
	let _ = child.kill();
}

/// Reap every child that has exited: in the container the runner is PID 1,
/// so a program's orphans become its children, and a zombie of the
/// programs' uid would count against their process ceiling. Only once the
/// supervised child has been waited for, so its status is never taken here.
fn reap_orphans() {
	while let Ok(Some(_)) = wait(WaitOptions::NOHANG) {}
}

/// Kill every process of `sweep.uid` with the helper, as that uid, until a
/// read of `/proc` finds none alive.
///
/// # Errors
/// Never `TimedOut` or `InvalidInput`, which a run reads as the program's
/// doing: a sweep that cannot finish (a helper stopped past
/// [`HELPER_TIMEOUT`], or killed every pass) is the machine's, and stops the
/// runner.
fn sweep_uid(sweep: Sweep) -> std::io::Result<()> {
	if sweep.uid == 0 || sweep.uid == getuid().as_raw() {
		return Err(std::io::Error::other("the sweep is only for the programs' own uid"));
	}
	for _ in 0..SWEEP_PASSES {
		if !any_alive(sweep.uid)? {
			return Ok(());
		}
		// Its exit status is `kill`'s (1 when it found nothing to signal),
		// or a signal a survivor sent it: the next look at /proc settles
		// either way.
		let mut helper = Command::new(SWEEPER[0])
			.args(&SWEEPER[1..])
			.uid(sweep.uid)
			.gid(sweep.gid)
			.env_clear()
			.current_dir("/")
			.stdin(Stdio::null())
			.stdout(Stdio::null())
			.stderr(Stdio::null())
			.spawn()
			.map_err(|err| std::io::Error::other(String::from("could not start the sweep's helper: ") + &err.to_string()))?;
		let started = Instant::now();
		loop {
			match helper.try_wait() {
				Ok(Some(_)) => break,
				Ok(None) if started.elapsed() < HELPER_TIMEOUT => std::thread::sleep(POLL),
				// Stopped by a survivor, or lost: the restart clears it.
				Ok(None) => return Err(std::io::Error::other("the sweep's helper did not finish; something of the programs' uid stopped it")),
				Err(err) => return Err(std::io::Error::other(String::from("could not wait for the sweep's helper: ") + &err.to_string())),
			}
		}
	}
	Err(std::io::Error::other("a process of the programs' uid survived every pass of the sweep"))
}

/// Whether `/proc` lists a live process (not a zombie) of `uid`.
fn any_alive(uid: u32) -> std::io::Result<bool> {
	for entry in std::fs::read_dir("/proc")? {
		let Ok(entry) = entry else { continue };
		if !entry.file_name().to_str().is_some_and(|name| name.bytes().all(|byte| byte.is_ascii_digit())) {
			continue;
		}
		// Gone between the listing and the read: not alive.
		if std::fs::read_to_string(entry.path().join("status")).is_ok_and(|status| alive_and_owned_by(&status, uid)) {
			return Ok(true);
		}
	}
	Ok(false)
}

/// Whether a `/proc/<pid>/status` names a process whose real uid is `uid`
/// and that is not already dead (a zombie waits only on its reaper).
fn alive_and_owned_by(status: &str, uid: u32) -> bool {
	let mut owned = false;
	let mut dead = false;
	for line in status.lines() {
		if let Some(state) = line.strip_prefix("State:") {
			let state = state.trim_start();
			dead = state.starts_with('Z') || state.starts_with('X');
		} else if let Some(ids) = line.strip_prefix("Uid:") {
			owned = ids.split_whitespace().next().and_then(|real| real.parse::<u32>().ok()) == Some(uid);
		}
	}
	owned && !dead
}

fn drain<R: Read + Send + 'static>(mut stream: R, cap: usize) -> JoinHandle<std::io::Result<Captured>> {
	std::thread::spawn(move || {
		let mut captured = Captured::default();
		let mut buffer = [0_u8; 8192];
		loop {
			let read = match stream.read(&mut buffer) {
				Ok(0) => break,
				Ok(read) => read,
				Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
				Err(err) => return Err(err),
			};
			let chunk = &buffer[..read];
			captured.total += u64::try_from(read).unwrap_or(u64::MAX);
			let room = cap.saturating_sub(captured.head.len());
			captured.head.extend_from_slice(&chunk[..room.min(read)]);
			// Every byte passes through the tail, which is cut back to
			// `cap` whenever it doubles: at most about `2 * cap` held.
			captured.tail.extend_from_slice(chunk);
			if captured.tail.len() > cap.saturating_mul(2) {
				let excess = captured.tail.len() - cap;
				captured.tail.drain(..excess);
			}
		}
		if captured.tail.len() > cap {
			let excess = captured.tail.len() - cap;
			captured.tail.drain(..excess);
		}
		Ok(captured)
	})
}

fn joined(handle: Option<JoinHandle<std::io::Result<Captured>>>, deadline: Instant) -> std::io::Result<Captured> {
	let Some(handle) = handle else {
		return Ok(Captured::default());
	};
	while !handle.is_finished() {
		if Instant::now() >= deadline {
			// The reader thread is left blocked on the pipe; it holds only
			// its buffer, and ends when the stray descendant does.
			// `TimedOut`, which the runner reads as the program's fault.
			return Err(std::io::Error::new(
				std::io::ErrorKind::TimedOut,
				"a process outside the program's process group kept its output open after it ended",
			));
		}
		std::thread::sleep(POLL);
	}
	handle.join().map_err(|_| std::io::Error::other("an output reader panicked"))?
}

/// `program` as an absolute path.
///
/// Itself if it names a directory, else the first match on `PATH`. The
/// runner clears every child's environment, so it looks programs up itself
/// rather than rely on how `Command` searches a cleared one.
#[must_use]
pub fn resolve(program: &Path) -> Option<PathBuf> {
	if program.components().count() > 1 {
		return program.is_file().then(|| program.to_path_buf());
	}
	// The lookup `Command` would do, not configuration (#484).
	#[allow(clippy::disallowed_methods)]
	let path = std::env::var_os("PATH")?;
	std::env::split_paths(&path).map(|dir| dir.join(program)).find(|candidate| candidate.is_file())
}

/// A child's exit, in words: its code, or the signal that ended it.
#[must_use]
pub fn describe(status: ExitStatus) -> String {
	let mut text = String::new();
	if let Some(code) = status.code() {
		text.push_str("exited with status ");
		text.push_str(&code.to_string());
	} else if let Some(signal) = status.signal() {
		text.push_str("was killed by signal ");
		text.push_str(&signal.to_string());
	} else {
		text.push_str("ended abnormally");
	}
	text
}

#[cfg(test)]
mod tests {
	use super::{alive_and_owned_by, not_started, supervise, Sweep};
	use std::process::Command;
	use std::time::Duration;

	/// A child that cannot be started is marked as such, keeping its kind; a
	/// failure once it ran (here, a sweep refused for root's uid) is not.
	#[test]
	fn a_spawn_failure_is_told_from_a_later_one() {
		let missing = supervise(Command::new("/nonexistent/rust-runner-test"), Duration::from_secs(5), 64, 64, None).unwrap_err();
		assert!(not_started(&missing), "{missing}");
		assert_eq!(missing.kind(), std::io::ErrorKind::NotFound);

		let sweep = Some(Sweep { uid: 0, gid: 0 });
		let later = supervise(Command::new("true"), Duration::from_secs(5), 64, 64, sweep).unwrap_err();
		assert!(!not_started(&later), "{later}");
	}

	#[test]
	fn the_sweep_reads_the_real_uid_and_passes_over_the_dead() {
		let status = |state: &str, uid: &str| String::from("Name:\tx\nState:\t") + state + "\nUid:\t" + uid + "\t0\t0\t0\n";
		assert!(alive_and_owned_by(&status("R (running)", "10001"), 10001));
		assert!(alive_and_owned_by(&status("S (sleeping)", "10001"), 10001));
		assert!(!alive_and_owned_by(&status("Z (zombie)", "10001"), 10001));
		assert!(!alive_and_owned_by(&status("X (dead)", "10001"), 10001));
		assert!(!alive_and_owned_by(&status("R (running)", "0"), 10001), "another uid's");
		assert!(!alive_and_owned_by("Name:\tx\n", 10001), "no Uid line");
	}
}

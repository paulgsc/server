//! Running one child process under a wall-clock ceiling, with its output
//! captured to a ceiling and never streamed anywhere.
//!
//! `std::process` and `rustix` only, no `unsafe`: the child is put in a
//! process group of its own (`process_group(0)`), polled with `try_wait`, and
//! on the ceiling the whole group is sent `SIGKILL` (`kill(-pgid)`, through
//! `rustix`, so killing needs no new process: a program that used up its
//! process ceiling cannot stop the kill) before the child is reaped. The group
//! is killed again once the child has ended, however it ended, so nothing it
//! spawned outlives the run. Given the programs' own uid, every process of it
//! goes too, which also takes a descendant that left the group (`setsid`).
//! stdout and stderr are drained by a thread each, so a
//! chatty child never blocks on a full pipe, but only the first `cap` bytes
//! (and, for the error tail, the last `cap`) are kept.

use rustix::process::{getuid, kill_process, kill_process_group, Pid, Signal};
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

/// How many times the sweep of a uid looks for survivors before giving up. Each
/// pass kills every process it finds, so only one forking faster than a pass
/// can list `/proc` survives one, and nothing survives `SIGKILL`.
const SWEEP_PASSES: usize = 200;

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
/// bytes. Once it has ended, its process group is killed, and with `sweep`,
/// every process whose real uid is `sweep` (the programs' own, never this
/// process's).
///
/// # Errors
/// If the child cannot be started or waited on, or a process of `sweep`'s
/// uid survives every pass of the sweep; everything the child itself does is
/// in [`Finished`].
pub(crate) fn supervise(mut command: Command, ceiling: Duration, stdout_cap: usize, stderr_cap: usize, sweep: Option<u32>) -> std::io::Result<Finished> {
	command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).process_group(0);
	let started = Instant::now();
	let mut child = command.spawn()?;
	let stdout = child.stdout.take().map(|stream| drain(stream, stdout_cap));
	let stderr = child.stderr.take().map(|stream| drain(stream, stderr_cap));
	let ending = loop {
		match child.try_wait() {
			Ok(Some(status)) => break Ending::Exited(status),
			Ok(None) => {}
			Err(err) => {
				kill_group(&mut child);
				let _ = child.wait();
				return Err(err);
			}
		}
		if started.elapsed() >= ceiling {
			kill_group(&mut child);
			child.wait()?;
			break Ending::TimedOut;
		}
		std::thread::sleep(POLL);
	};
	let elapsed = started.elapsed();
	// Whatever the child left behind in its group goes too, however it
	// ended: a descendant still holding stdout would otherwise outlive the
	// ceiling, and keep the readers below waiting for it (review, #399).
	kill_group(&mut child);
	if let Some(uid) = sweep {
		sweep_uid(uid)?;
	}
	let deadline = Instant::now() + READER_GRACE;
	Ok(Finished {
		ending,
		elapsed,
		stdout: joined(stdout, deadline)?,
		stderr: joined(stderr, deadline)?,
	})
}

/// How long the output readers get to see end-of-file once the child's
/// group is gone. Only a descendant that left the group (`setsid`) can still
/// hold a pipe then; past this the run is an error rather than a hang.
const READER_GRACE: Duration = Duration::from_secs(1);

/// `SIGKILL` the child's whole process group (a compile's linker, say, as
/// well as `rustc`), and the child itself in case the group is already
/// gone. A system call, not a `kill` process: under a process ceiling a
/// program can leave no room for one.
fn kill_group(child: &mut Child) {
	let _ = kill_process_group(Pid::from_child(child), Signal::KILL);
	let _ = child.kill();
}

/// `SIGKILL` every process whose real uid is `uid` (but this one and pid 1,
/// which never are), until a pass of `/proc` finds none left alive.
fn sweep_uid(uid: u32) -> std::io::Result<()> {
	let me = std::process::id();
	if uid == 0 || uid == getuid().as_raw() {
		return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "the sweep is only for the programs' own uid"));
	}
	for _ in 0..SWEEP_PASSES {
		let mut found = false;
		for entry in std::fs::read_dir("/proc")? {
			let Ok(entry) = entry else { continue };
			let Some(pid) = entry.file_name().to_str().and_then(|name| name.parse::<u32>().ok()) else {
				continue;
			};
			if pid == me || pid == 1 {
				continue;
			}
			// Gone between the listing and the read: nothing to kill.
			let Ok(status) = std::fs::read_to_string(entry.path().join("status")) else {
				continue;
			};
			if !alive_and_owned_by(&status, uid) {
				continue;
			}
			found = true;
			if let Some(pid) = i32::try_from(pid).ok().and_then(Pid::from_raw) {
				let _ = kill_process(pid, Signal::KILL);
			}
		}
		if !found {
			return Ok(());
		}
		std::thread::sleep(POLL);
	}
	Err(std::io::Error::other("a process of the programs' uid survived every pass of the sweep"))
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
	use super::alive_and_owned_by;

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

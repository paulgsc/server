//! Running one child process under a wall-clock ceiling, with its output
//! captured to a ceiling and never streamed anywhere.
//!
//! `std::process` only, no `unsafe`: the child is put in a process group of
//! its own (`process_group(0)`), polled with `try_wait`, and on the ceiling
//! the whole group is sent `SIGKILL` (through `kill`, which is how a safe
//! program signals a group) before the child is reaped. stdout and stderr are
//! drained by a thread each, so a chatty child never blocks on a full pipe,
//! but only the first `cap` bytes (and, for the error tail, the last `cap`)
//! are kept.

use std::ffi::OsString;
use std::io::Read;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// How often a running child is checked on. The elapsed time a run reports
/// is therefore good to about a millisecond, which is all Ax. 3.1 asks of
/// it: timings are illustrative, never graded.
const POLL: Duration = Duration::from_millis(1);

/// One stream's bytes, kept to a ceiling.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct Captured {
	/// The first `cap` bytes.
	pub(crate) head: Vec<u8>,
	/// The last `cap` bytes (the whole stream, when it fit).
	pub(crate) tail: Vec<u8>,
	/// How many bytes the stream carried in all.
	pub(crate) total: u64,
}

impl Captured {
	/// Whether anything was dropped.
	pub(crate) fn truncated(&self) -> bool {
		u64::try_from(self.head.len()).map_or(true, |kept| kept < self.total)
	}

	/// The end of the stream: all of it when it fit, else its last `cap`
	/// bytes.
	pub(crate) fn end(&self) -> &[u8] {
		&self.tail
	}
}

/// How a supervised child ended.
#[derive(Debug)]
pub(crate) enum Ending {
	/// It exited (or was killed by a signal it raised itself) within the
	/// ceiling.
	Exited(ExitStatus),
	/// It was still running at the ceiling and was killed.
	TimedOut,
}

#[derive(Debug)]
pub(crate) struct Finished {
	pub(crate) ending: Ending,
	/// From spawn to the exit being seen.
	pub(crate) elapsed: Duration,
	pub(crate) stdout: Captured,
	pub(crate) stderr: Captured,
}

/// Run `command` to completion or to `ceiling`, whichever comes first, with
/// stdin closed and stdout and stderr kept to `stdout_cap` and `stderr_cap`
/// bytes.
///
/// # Errors
/// Only if the child cannot be started or waited on; everything the child
/// itself does is in [`Finished`].
pub(crate) fn supervise(mut command: Command, ceiling: Duration, stdout_cap: usize, stderr_cap: usize) -> std::io::Result<Finished> {
	command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).process_group(0);
	let started = Instant::now();
	let mut child = command.spawn()?;
	let stdout = child.stdout.take().map(|stream| drain(stream, stdout_cap));
	let stderr = child.stderr.take().map(|stream| drain(stream, stderr_cap));
	let ending = loop {
		if let Some(status) = child.try_wait()? {
			break Ending::Exited(status);
		}
		if started.elapsed() >= ceiling {
			kill_group(&mut child);
			child.wait()?;
			break Ending::TimedOut;
		}
		std::thread::sleep(POLL);
	};
	let elapsed = started.elapsed();
	Ok(Finished {
		ending,
		elapsed,
		stdout: joined(stdout)?,
		stderr: joined(stderr)?,
	})
}

/// `SIGKILL` the child's whole process group — a compile's linker, say, as
/// well as `rustc` — and the child itself in case `kill` is not to be had.
fn kill_group(child: &mut Child) {
	let mut group = OsString::from("-");
	group.push(child.id().to_string());
	if let Some(kill) = resolve(Path::new("kill")) {
		let _ = Command::new(kill)
			.arg("-KILL")
			.arg("--")
			.arg(group)
			.env_clear()
			.stdin(Stdio::null())
			.stdout(Stdio::null())
			.stderr(Stdio::null())
			.status();
	}
	let _ = child.kill();
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

fn joined(handle: Option<JoinHandle<std::io::Result<Captured>>>) -> std::io::Result<Captured> {
	match handle {
		None => Ok(Captured::default()),
		Some(handle) => handle.join().map_err(|_| std::io::Error::other("an output reader panicked"))?,
	}
}

/// `program` as an absolute path: itself if it names a directory, else the
/// first match on `PATH`. The runner clears every child's environment, so it
/// looks programs up itself rather than rely on how `Command` searches a
/// cleared one.
pub(crate) fn resolve(program: &Path) -> Option<PathBuf> {
	if program.components().count() > 1 {
		return program.is_file().then(|| program.to_path_buf());
	}
	std::env::split_paths(&std::env::var_os("PATH")?)
		.map(|dir| dir.join(program))
		.find(|candidate| candidate.is_file())
}

/// A child's exit, in words: its code, or the signal that ended it.
pub(crate) fn describe(status: ExitStatus) -> String {
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

//! The `rust-runner` service: one request, one answer, one connection, over
//! a Unix socket.
//!
//! The protocol's types and its client ([`ask`],
//! [`exchange`]) are `rust_rounds_wire`'s, which a client links without
//! this crate; this module answers.
//!
//! The container has no network at all (`network_mode: none`), so the only
//! way in is the socket, on a volume shared with whoever asks (the recording
//! pass in `file_host`, `rust_rounds::pass`). A request is one JSON line:
//!
//! ```json
//! {"ask":"toolchain"}
//! {"ask":"record","toolchain":"rustc 1.97.0 (…); --edition 2024 -C opt-level=0","source":"fn main() {…}"}
//! ```
//!
//! and the answer is one JSON line: `{"answer":"toolchain","toolchain":…}`,
//! `{"answer":"recorded","recording":{…},"rustc":{…}}`, or `{"answer":"refused",
//! "reason":…,"toolchain":…}`. A `record` naming a toolchain other than the
//! runner's is refused. One job at a time: the service answers a connection
//! before it accepts the next.
//!
//! **Who may ask is the peer's uid, and that check is the primary defence.**
//! The socket is mode 0666 in a 0755 directory, on a volume shared with the
//! client's container, so anything in either container can connect to it,
//! a program the runner is recording included. Each connection's uid
//! (`SO_PEERCRED`, which the kernel records at `connect` and the peer cannot
//! change) is checked before anything is read, and only the configured
//! clients are served: the uid of `file_host`, and root. Any other connection is
//! refused unread. A program's would otherwise be recorded after the program
//! was killed, and could queue the next, without end.
//!
//! The answer is not to be trusted for its key: the client (the recording
//! pass in `file_host`) checks the answer's toolchain is the one it asked for and
//! computes [`crate::recording::key`] of that toolchain and the source it
//! sent itself, and stores the recording under that (RX4).

use crate::recording::Recorder;
use crate::SpawnError;
/// The protocol itself, and its client: `rust_rounds_wire`'s, the half a
/// client links.
pub use rust_rounds_wire::protocol::{
	ask, exchange, job_deadline, AskError, Refusal, Request, Response, ANSWER_CEILING, ASK_TIMEOUT, CLEAN_TIMEOUT, CLIENT_TIMEOUT, DEFAULT_JOB_DEADLINE, REQUEST_CEILING,
	SWEEP_ALLOWANCE,
};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

/// Answer the one request on `stream`.
///
/// Returns the answer sent (or that would have been, when the client went
/// away first).
///
/// With `clients`, only a peer whose uid is among them is served; any other
/// is refused before anything is read. `None` serves any peer, for a
/// service whose socket only its own uid can reach (the tests').
///
/// # Errors
/// The machine's failures from [`Recorder::record`], which the service
/// stops on. One before the program ran is first answered with
/// [`Refusal::Unavailable`]; one after ([`SpawnError::Unsupervised`]) is
/// answered with nothing. A client that sends nothing, too much, garbage or
/// too slowly is answered with a refusal, and one that goes away is not an
/// error.
pub fn answer(recorder: &Recorder, stream: &UnixStream, clients: Option<&[u32]>) -> Result<Response, SpawnError> {
	let refused = |reason| Response::Refused {
		reason,
		toolchain: recorder.toolchain().to_owned(),
	};
	if let Some(clients) = clients {
		if !peer_uid(stream).is_ok_and(|uid| clients.contains(&uid)) {
			let response = refused(Refusal::Peer);
			send(stream, &response);
			return Ok(response);
		}
	}
	let mut line = Vec::new();
	let deadline = Deadline::new(stream);
	let read = BufReader::new(deadline.take(REQUEST_CEILING + 1)).read_until(b'\n', &mut line);
	let response = if u64::try_from(line.len()).unwrap_or(u64::MAX) > REQUEST_CEILING {
		refused(Refusal::TooLarge)
	} else {
		match read.ok().and_then(|_| serde_json::from_slice::<Request>(&line).ok()) {
			None => refused(Refusal::Malformed),
			Some(Request::Toolchain {}) => Response::Toolchain {
				toolchain: recorder.toolchain().to_owned(),
			},
			Some(Request::Record { toolchain, .. }) if toolchain != recorder.toolchain() => refused(Refusal::WrongToolchain),
			Some(Request::Record { source, .. }) => match recorder.record(&source) {
				Ok(recording) => Response::Recorded {
					rustc: recording.rustc_run(),
					recording: Box::new(recording),
				},
				// The program ran, and could not be supervised to its end:
				// no answer, so the client counts a drop against it (RP3).
				Err(err @ SpawnError::Unsupervised(_)) => return Err(err),
				// Before the program ran: the machine's alone. Said, so the
				// client keeps nothing against the program, and then the
				// service stops as for any machine failure.
				Err(err) => {
					send(stream, &refused(Refusal::Unavailable));
					return Err(err);
				}
			},
		}
	};
	send(stream, &response);
	Ok(response)
}

/// The uid the kernel recorded for `stream`'s peer at `connect`
/// (`SO_PEERCRED`), as this process's user namespace maps it.
///
/// Read as plain integers, not with rustix's `socket_peercred` (1.1.5),
/// whose `UCred` holds the pid as a `NonZeroI32`: a peer in a PID namespace
/// this process cannot see, as every client in another container is, is
/// reported with pid 0, and that read as an error, so each was refused
/// (#766). Only the uid is checked; the pid is ignored, whatever it is.
fn peer_uid(stream: &UnixStream) -> std::io::Result<u32> {
	let mut cred = libc::ucred {
		pid: 0,
		uid: u32::MAX,
		gid: u32::MAX,
	};
	let size = std::mem::size_of::<libc::ucred>();
	let mut len = libc::socklen_t::try_from(size).map_err(std::io::Error::other)?;
	// SAFETY: `cred` is a live, writable `ucred` and `len` its size, as
	// SO_PEERCRED writes them; every bit pattern is a valid `ucred` (three
	// plain integers); and the descriptor is `stream`'s, open for the call.
	let read = unsafe { libc::getsockopt(stream.as_raw_fd(), libc::SOL_SOCKET, libc::SO_PEERCRED, (&raw mut cred).cast(), &raw mut len) };
	if read != 0 {
		return Err(std::io::Error::last_os_error());
	}
	if usize::try_from(len).ok() != Some(size) {
		return Err(std::io::Error::other("SO_PEERCRED wrote a ucred of another size"));
	}
	Ok(cred.uid)
}

/// Write `response` as one line, within one [`CLIENT_TIMEOUT`] for the
/// whole of it; a client that went away or reads too slowly is not an
/// error.
fn send(stream: &UnixStream, response: &Response) {
	let mut body = Vec::new();
	if serde_json::to_writer(&mut body, response).is_ok() {
		body.push(b'\n');
		let _ = Deadline::new(stream).write_all(&body);
	}
}

/// `stream`, read or written against one deadline for the whole request or
/// answer rather than a timeout per call: each waits only what is left, so a
/// client that trickles a byte at a time, or reads the answer that way,
/// still loses the slot at [`CLIENT_TIMEOUT`].
struct Deadline<'a> {
	stream: &'a UnixStream,
	until: Instant,
}

impl<'a> Deadline<'a> {
	fn new(stream: &'a UnixStream) -> Self {
		Self {
			stream,
			until: Instant::now() + CLIENT_TIMEOUT,
		}
	}

	/// What is left, or `TimedOut` when nothing is.
	fn left(&self) -> std::io::Result<Duration> {
		let left = self.until.saturating_duration_since(Instant::now());
		if left.is_zero() {
			return Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "the client took longer than the client timeout"));
		}
		Ok(left)
	}
}

impl Read for Deadline<'_> {
	fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
		self.stream.set_read_timeout(Some(self.left()?))?;
		(&*self.stream).read(buffer)
	}
}

impl Write for Deadline<'_> {
	fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
		self.stream.set_write_timeout(Some(self.left()?))?;
		(&*self.stream).write(buffer)
	}

	fn flush(&mut self) -> std::io::Result<()> {
		Ok(())
	}
}

//! The `rust-runner` service's protocol: one request, one answer, one
//! connection, over a Unix socket.
//!
//! The container has no network at all (`network_mode: none`), so the only
//! way in is the socket, on a volume shared with whoever asks (the recording
//! pass in `file_host`, RR3's next slice). A request is one JSON line:
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

use crate::recording::{Recorder, Recording, RustcRun};
use crate::SpawnError;
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::{Duration, Instant};

/// The largest request read, in bytes. A round's program is about 25 lines.
pub const REQUEST_CEILING: u64 = 64 * 1024;

/// How long a client may take to send its whole request, and to read the
/// whole answer, before the connection is dropped, so a stalled or trickling
/// client cannot hold the one job slot.
pub const CLIENT_TIMEOUT: Duration = Duration::from_secs(5);

/// The largest answer [`ask`] reads, in bytes.
///
/// An answer holds at most the
/// diagnostics read from 1 MiB of rustc's JSON ([`crate::DIAGNOSTICS_CEILING`])
/// and a few kept streams of 4 KiB; past this the service is not answering
/// as documented.
pub const ANSWER_CEILING: u64 = 4 * 1024 * 1024;

/// What a client asks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "ask", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Request {
	/// The toolchain every recording is keyed to.
	Toolchain {},
	/// Record `source`, if `toolchain` is the runner's.
	Record { toolchain: String, source: String },
}

/// Why a request was not answered with a recording.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Refusal {
	/// Not one JSON request line.
	Malformed,
	/// Past [`REQUEST_CEILING`].
	TooLarge,
	/// The request's toolchain is not the runner's.
	WrongToolchain,
	/// The connection came from a uid that is not a client's (a program's,
	/// say), or its peer could not be read.
	Peer,
}

impl Refusal {
	#[must_use]
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::Malformed => "malformed",
			Self::TooLarge => "too-large",
			Self::WrongToolchain => "wrong-toolchain",
			Self::Peer => "peer",
		}
	}
}

/// What the service answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "answer", rename_all = "kebab-case")]
pub enum Response {
	Toolchain {
		toolchain: String,
	},
	/// The recording, and what the phone reads of it
	/// ([`Recording::rustc_run`]).
	Recorded {
		recording: Box<Recording>,
		rustc: RustcRun,
	},
	Refused {
		reason: Refusal,
		toolchain: String,
	},
}

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
/// stops on. A client that sends nothing, too much, garbage or too slowly is
/// answered with a refusal, and one that goes away is not an error.
pub fn answer(recorder: &Recorder, stream: &UnixStream, clients: Option<&[u32]>) -> Result<Response, SpawnError> {
	let refused = |reason| Response::Refused {
		reason,
		toolchain: recorder.toolchain().to_owned(),
	};
	if let Some(clients) = clients {
		let peer = rustix::net::sockopt::socket_peercred(stream);
		if !peer.is_ok_and(|peer| clients.contains(&peer.uid.as_raw())) {
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
			Some(Request::Record { source, .. }) => {
				let recording = recorder.record(&source)?;
				Response::Recorded {
					rustc: recording.rustc_run(),
					recording: Box::new(recording),
				}
			}
		}
	};
	send(stream, &response);
	Ok(response)
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

/// Ask the service on `socket` one thing, and wait up to `timeout` for the
/// answer.
///
/// # Errors
/// When the socket cannot be reached, or the answer is not one (or is longer
/// than [`ANSWER_CEILING`]).
pub fn ask(socket: &Path, request: &Request, timeout: Duration) -> std::io::Result<Response> {
	let stream = UnixStream::connect(socket)?;
	stream.set_read_timeout(Some(timeout))?;
	stream.set_write_timeout(Some(CLIENT_TIMEOUT))?;
	let mut body = Vec::new();
	serde_json::to_writer(&mut body, request).map_err(std::io::Error::other)?;
	body.push(b'\n');
	(&stream).write_all(&body)?;
	let mut line = Vec::new();
	BufReader::new((&stream).take(ANSWER_CEILING + 1)).read_until(b'\n', &mut line)?;
	if u64::try_from(line.len()).unwrap_or(u64::MAX) > ANSWER_CEILING {
		return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "the answer is longer than the ceiling"));
	}
	serde_json::from_slice(&line).map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))
}

#[cfg(test)]
mod tests {
	use super::{Refusal, Request, Response};

	#[test]
	fn the_wire_reads_as_documented() {
		assert_eq!(serde_json::from_str::<Request>(r#"{"ask":"toolchain"}"#).unwrap(), Request::Toolchain {});
		assert_eq!(
			serde_json::from_str::<Request>(r#"{"ask":"record","toolchain":"t","source":"fn main() {}"}"#).unwrap(),
			Request::Record {
				toolchain: String::from("t"),
				source: String::from("fn main() {}"),
			}
		);
		assert!(
			serde_json::from_str::<Request>(r#"{"ask":"record","source":"fn main() {}"}"#).is_err(),
			"a record names its toolchain"
		);
		assert!(serde_json::from_str::<Request>(r#"{"ask":"run","source":"x"}"#).is_err());
		assert!(
			serde_json::from_str::<Request>(r#"{"ask":"toolchain","trusted":true}"#).is_err(),
			"no field changes how a program is treated"
		);
		let mut refused = Vec::new();
		serde_json::to_writer(
			&mut refused,
			&Response::Refused {
				reason: Refusal::WrongToolchain,
				toolchain: String::from("t"),
			},
		)
		.unwrap();
		assert_eq!(String::from_utf8(refused).unwrap(), r#"{"answer":"refused","reason":"wrong-toolchain","toolchain":"t"}"#);
		for reason in [Refusal::Malformed, Refusal::TooLarge, Refusal::WrongToolchain, Refusal::Peer] {
			assert_eq!(serde_json::to_value(reason).unwrap(), reason.as_str());
		}
	}
}

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
//! runner's is refused, so nothing is ever stored under a key the runner did
//! not produce. One job at a time: the service answers a connection before
//! it accepts the next.

use crate::recording::{Recorder, Recording, RustcRun};
use crate::SpawnError;
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

/// The largest request read, in bytes. A round's program is about 25 lines.
pub const REQUEST_CEILING: u64 = 64 * 1024;

/// How long a client may take to send its request or read its answer
/// before the connection is dropped, so a stalled client cannot hold the
/// one job slot.
pub const CLIENT_TIMEOUT: Duration = Duration::from_secs(5);

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
}

impl Refusal {
	#[must_use]
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::Malformed => "malformed",
			Self::TooLarge => "too-large",
			Self::WrongToolchain => "wrong-toolchain",
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
/// # Errors
/// The machine's failures from [`Recorder::record`], which the service
/// stops on. A client that sends nothing, too much or garbage is answered
/// with a refusal, and one that goes away is not an error.
pub fn answer(recorder: &Recorder, stream: &UnixStream) -> Result<Response, SpawnError> {
	let _ = stream.set_read_timeout(Some(CLIENT_TIMEOUT));
	let _ = stream.set_write_timeout(Some(CLIENT_TIMEOUT));
	let mut line = Vec::new();
	let read = BufReader::new(stream.take(REQUEST_CEILING + 1)).read_until(b'\n', &mut line);
	let refused = |reason| Response::Refused {
		reason,
		toolchain: recorder.toolchain().to_owned(),
	};
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
	let mut body = Vec::new();
	if serde_json::to_writer(&mut body, &response).is_ok() {
		body.push(b'\n');
		let _ = (&*stream).write_all(&body);
	}
	Ok(response)
}

/// Ask the service on `socket` one thing, and wait up to `timeout` for the
/// answer.
///
/// # Errors
/// When the socket cannot be reached, or the answer is not one.
pub fn ask(socket: &Path, request: &Request, timeout: Duration) -> std::io::Result<Response> {
	let stream = UnixStream::connect(socket)?;
	stream.set_read_timeout(Some(timeout))?;
	stream.set_write_timeout(Some(CLIENT_TIMEOUT))?;
	let mut body = Vec::new();
	serde_json::to_writer(&mut body, request).map_err(std::io::Error::other)?;
	body.push(b'\n');
	(&stream).write_all(&body)?;
	let mut line = Vec::new();
	BufReader::new(&stream).read_until(b'\n', &mut line)?;
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
		for reason in [Refusal::Malformed, Refusal::TooLarge, Refusal::WrongToolchain] {
			assert_eq!(serde_json::to_value(reason).unwrap(), reason.as_str());
		}
	}
}

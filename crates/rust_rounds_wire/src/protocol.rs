//! The `rust-runner` service's protocol: one request, one answer, one
//! connection, over a Unix socket (`rust_runner::service` answers it).
//!
//! A request is one JSON line:
//!
//! ```json
//! {"ask":"toolchain"}
//! {"ask":"record","toolchain":"rustc 1.97.0 (…); --edition 2024 -C opt-level=0","source":"fn main() {…}"}
//! ```
//!
//! and the answer is one JSON line: `{"answer":"toolchain","toolchain":…}`,
//! `{"answer":"recorded","recording":{…},"rustc":{…}}`, or `{"answer":"refused",
//! "reason":…,"toolchain":…}`.
//!
//! [`exchange`] tells a client how an exchange failed by when it failed, so
//! a service that is down ([`AskError::Unreachable`]) is told apart from one
//! that went away while it held the job ([`AskError::Dropped`]), which may be
//! the program's own doing.

use crate::recording::{Recording, RustcRun};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, ErrorKind, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

/// The largest request read, in bytes. A round's program is about 25 lines.
pub const REQUEST_CEILING: u64 = 64 * 1024;

/// How long a client may take to send its whole request, and to read the
/// whole answer, before the connection is dropped, so a stalled or trickling
/// client cannot hold the one job slot.
pub const CLIENT_TIMEOUT: Duration = Duration::from_secs(5);

/// How long `rustc` may take over one program before the compile counts as
/// failed.
pub const DEFAULT_COMPILE_TIMEOUT: Duration = Duration::from_secs(60);

/// The wall-clock ceiling on one run.
///
/// See `leetype-execution.md` for why 2 s at `opt-level=0`: every
/// inadmissible variant in the round corpus exceeds it at `C′` and every
/// admissible one finishes in tens of milliseconds.
pub const DEFAULT_RUN_CEILING: Duration = Duration::from_millis(2_000);

/// How long the runner's `clean` may take over what one job left.
pub const CLEAN_TIMEOUT: Duration = Duration::from_secs(30);

/// What a job may take beyond its own ceilings (the compile timeout, the run
/// ceiling, [`CLEAN_TIMEOUT`] and the client's two deadlines) before the
/// runner's watchdog ends the runner: its sweeps, each a few milliseconds and
/// each helper bounded by its own timeout, and the tidy's checks.
pub const SWEEP_ALLOWANCE: Duration = Duration::from_secs(30);

/// The watchdog's deadline for one job under these ceilings: every ceiling
/// it can spend, plus [`SWEEP_ALLOWANCE`] (RX3). Past it, the runner exits
/// and the client sees its connection dropped.
#[must_use]
pub const fn job_deadline(compile_timeout: Duration, run_ceiling: Duration) -> Duration {
	compile_timeout
		.saturating_add(run_ceiling)
		.saturating_add(CLEAN_TIMEOUT)
		.saturating_add(CLIENT_TIMEOUT.saturating_mul(2))
		.saturating_add(SWEEP_ALLOWANCE)
}

/// [`job_deadline`] with the default ceilings, which the runner's image
/// runs with (it sets none, and compose passes no environment, RX1): 132 s.
pub const DEFAULT_JOB_DEADLINE: Duration = job_deadline(DEFAULT_COMPILE_TIMEOUT, DEFAULT_RUN_CEILING);

/// How long a client waits for an answer: past [`DEFAULT_JOB_DEADLINE`] by
/// one [`CLIENT_TIMEOUT`] (137 s), so a job the runner's watchdog ends is
/// seen as a dropped connection, never as the client's own timeout. A client
/// that gave up first would ask again while the runner was still wedged and
/// queue behind it, and one hostile program would cost two attempts.
pub const ASK_TIMEOUT: Duration = DEFAULT_JOB_DEADLINE.saturating_add(CLIENT_TIMEOUT);

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

/// Why [`exchange`] has no answer, by how far it got.
#[derive(Debug)]
pub enum AskError {
	/// No connection: nothing listens on the socket (the service is down or
	/// restarting), or there is no socket. Nothing was asked.
	Unreachable(std::io::Error),
	/// Connected, but the request could not be written whole, so the service
	/// never held the job.
	NotSent(std::io::Error),
	/// The whole request was sent, and the connection ended before a whole
	/// answer came: the service went away while it held the job.
	Dropped(std::io::Error),
	/// The whole request was sent, and no answer came within the timeout.
	TimedOut,
	/// What came back is not an answer: longer than [`ANSWER_CEILING`], or
	/// not one of this protocol's.
	Invalid(std::io::Error),
}

impl std::fmt::Display for AskError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::Unreachable(err) => write!(f, "the runner could not be reached: {err}"),
			Self::NotSent(err) => write!(f, "the request could not be sent: {err}"),
			Self::Dropped(err) => write!(f, "the runner went away before it answered: {err}"),
			Self::TimedOut => f.write_str("the runner did not answer in time"),
			Self::Invalid(err) => write!(f, "the runner's answer is not one: {err}"),
		}
	}
}

impl std::error::Error for AskError {}

impl From<AskError> for std::io::Error {
	fn from(err: AskError) -> Self {
		match err {
			AskError::Unreachable(err) | AskError::NotSent(err) | AskError::Dropped(err) | AskError::Invalid(err) => err,
			AskError::TimedOut => Self::new(ErrorKind::TimedOut, "the runner did not answer in time"),
		}
	}
}

/// Ask the service on `socket` one thing, and wait up to `timeout` for the
/// answer.
///
/// # Errors
/// As [`exchange`], as an I/O error.
pub fn ask(socket: &Path, request: &Request, timeout: Duration) -> std::io::Result<Response> {
	exchange(socket, request, timeout).map_err(std::io::Error::from)
}

/// Ask the service on `socket` one thing, and wait up to `timeout` for the
/// answer, which is one whole line.
///
/// # Errors
/// [`AskError`], by how far the exchange got.
pub fn exchange(socket: &Path, request: &Request, timeout: Duration) -> Result<Response, AskError> {
	let stream = UnixStream::connect(socket).map_err(AskError::Unreachable)?;
	let mut body = Vec::new();
	serde_json::to_writer(&mut body, request).map_err(|err| AskError::NotSent(std::io::Error::other(err)))?;
	body.push(b'\n');
	stream.set_write_timeout(Some(CLIENT_TIMEOUT)).map_err(AskError::NotSent)?;
	(&stream).write_all(&body).map_err(AskError::NotSent)?;
	stream.set_read_timeout(Some(timeout)).map_err(AskError::Dropped)?;
	let mut line = Vec::new();
	match BufReader::new((&stream).take(ANSWER_CEILING + 1)).read_until(b'\n', &mut line) {
		Ok(_) => {}
		Err(err) if matches!(err.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => return Err(AskError::TimedOut),
		Err(err) => return Err(AskError::Dropped(err)),
	}
	if u64::try_from(line.len()).unwrap_or(u64::MAX) > ANSWER_CEILING {
		return Err(AskError::Invalid(std::io::Error::new(ErrorKind::InvalidData, "the answer is longer than the ceiling")));
	}
	if line.last() != Some(&b'\n') {
		return Err(AskError::Dropped(std::io::Error::new(
			ErrorKind::UnexpectedEof,
			"the connection ended before a whole answer",
		)));
	}
	serde_json::from_slice(&line).map_err(|err| AskError::Invalid(std::io::Error::new(ErrorKind::InvalidData, err)))
}

#[cfg(test)]
mod tests {
	use super::{exchange, AskError, Refusal, Request, Response, ASK_TIMEOUT, DEFAULT_JOB_DEADLINE};
	use std::io::{BufRead, BufReader, Write};
	use std::os::unix::net::UnixListener;
	use std::time::Duration;

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

	#[test]
	fn a_client_waits_past_the_watchdog() {
		assert_eq!(DEFAULT_JOB_DEADLINE, Duration::from_secs(132));
		assert!(ASK_TIMEOUT > DEFAULT_JOB_DEADLINE, "a wedged job is seen dropped, not timed out");
	}

	/// A service that reads the request whole and then does `then` with the
	/// connection, on a thread of its own.
	fn serve(dir: &std::path::Path, then: fn(std::os::unix::net::UnixStream)) -> std::path::PathBuf {
		let socket = dir.join("runner.sock");
		let listener = UnixListener::bind(&socket).unwrap();
		std::thread::spawn(move || {
			let (stream, _) = listener.accept().unwrap();
			let mut line = Vec::new();
			BufReader::new(&stream).read_until(b'\n', &mut line).unwrap();
			then(stream);
		});
		socket
	}

	/// A client tells a runner that is not there from one that went away
	/// holding the job, and both from one that is slow or says nonsense.
	#[test]
	fn an_exchange_says_how_far_it_got() {
		let request = Request::Toolchain {};
		let dir = tempfile::tempdir().unwrap();
		let missing = exchange(&dir.path().join("none.sock"), &request, Duration::from_secs(1));
		assert!(matches!(missing, Err(AskError::Unreachable(_))), "{missing:?}");

		let answered = tempfile::tempdir().unwrap();
		let socket = serve(answered.path(), |mut stream| {
			stream.write_all(b"{\"answer\":\"toolchain\",\"toolchain\":\"t\"}\n").unwrap();
		});
		assert_eq!(
			exchange(&socket, &request, Duration::from_secs(5)).unwrap(),
			Response::Toolchain { toolchain: String::from("t") }
		);

		let dropped = tempfile::tempdir().unwrap();
		let socket = serve(dropped.path(), drop);
		let gone = exchange(&socket, &request, Duration::from_secs(5));
		assert!(matches!(gone, Err(AskError::Dropped(_))), "{gone:?}");

		let cut = tempfile::tempdir().unwrap();
		let socket = serve(cut.path(), |mut stream| stream.write_all(b"{\"answer\":\"tool").unwrap());
		let half = exchange(&socket, &request, Duration::from_secs(5));
		assert!(matches!(half, Err(AskError::Dropped(_))), "half an answer is no answer: {half:?}");

		let slow = tempfile::tempdir().unwrap();
		let socket = serve(slow.path(), |stream| {
			std::thread::sleep(Duration::from_millis(500));
			drop(stream);
		});
		let late = exchange(&socket, &request, Duration::from_millis(50));
		assert!(matches!(late, Err(AskError::TimedOut)), "{late:?}");

		let garbled = tempfile::tempdir().unwrap();
		let socket = serve(garbled.path(), |mut stream| stream.write_all(b"not json\n").unwrap());
		let nonsense = exchange(&socket, &request, Duration::from_secs(5));
		assert!(matches!(nonsense, Err(AskError::Invalid(_))), "{nonsense:?}");
	}
}

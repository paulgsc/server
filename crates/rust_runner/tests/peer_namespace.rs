//! The service serves its clients from any PID namespace (#766).
//!
//! On home, `file_host` asks the runner from its own container, so the
//! runner reads `SO_PEERCRED` of a peer in a PID namespace it cannot see,
//! which the kernel reports with pid 0. rustix 1.1.5's `socket_peercred`
//! read that into a `NonZeroI32` and returned an error, and every
//! cross-container client was refused as [`Refusal::Peer`]. Only the uid is
//! the check; the pid must not matter.
//!
//! The service runs as this binary's ignored test, [`the_service_side`],
//! which the test proper runs by name under `unshare --pid --fork` (and
//! `--user --map-root-user` when not root): the service is then PID 1 of a
//! namespace this process is not in, and sees this process, its client,
//! with pid 0 and uid 0. Inside, it checks the pid really is 0 before it
//! answers, so the case it proves is the one home has. A sandbox that cannot
//! make the namespace **fails** the test: one that passes having run nothing
//! proves nothing. It needs `rustc` on `PATH`, as the runner's other tests
//! do.

use rust_runner::recording::Recorder;
use rust_runner::service::{answer, exchange, AskError, Refusal, Request, Response, CLIENT_TIMEOUT};
use rust_runner::Limits;
use std::os::fd::AsRawFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Set in the service's environment: the socket to serve one connection on.
const SOCKET: &str = "RUST_RUNNER_PEER_NAMESPACE_SOCKET";
/// Set in the service's environment: the one client uid it serves.
const CLIENT: &str = "RUST_RUNNER_PEER_NAMESPACE_CLIENT";

#[test]
fn a_client_in_another_pid_namespace_is_served_by_its_uid_alone() {
	let work = tempfile::tempdir().unwrap();
	let toolchain = Recorder::new(Limits::default(), work.path().to_path_buf())
		.unwrap_or_else(|err| panic!("this test needs rustc on PATH: {err}"))
		.toolchain()
		.to_owned();

	// The service sees this process as uid 0: root's own, or this uid mapped
	// to root in the service's user namespace.
	assert_eq!(
		ask_across(&work.path().join("client"), 0),
		Response::Toolchain { toolchain: toolchain.clone() },
		"a client's uid from another PID namespace is served"
	);
	assert_eq!(
		ask_across(&work.path().join("other"), 10_000),
		Response::Refused { reason: Refusal::Peer, toolchain },
		"any other uid from another PID namespace is still refused"
	);
}

/// The service's side: answer one connection on the socket in `SOCKET`,
/// serving the uid in `CLIENT`. Run only by [`ask_across`].
#[test]
#[ignore = "the service's side, run by a_client_in_another_pid_namespace_is_served_by_its_uid_alone in a PID namespace of its own"]
fn the_service_side() {
	// The test's own markers, set by `ask_across` for this run alone.
	#[allow(clippy::disallowed_methods)]
	let (socket, client) = (std::env::var_os(SOCKET).unwrap(), std::env::var_os(CLIENT).unwrap());
	let client = client.to_str().and_then(|client| client.parse::<u32>().ok()).unwrap();
	assert_eq!(std::process::id(), 1, "the service is not in a PID namespace of its own");
	let work = tempfile::tempdir().unwrap();
	let recorder = Recorder::new(Limits::default(), work.path().to_path_buf()).unwrap();
	let listener = UnixListener::bind(Path::new(&socket)).unwrap();
	let (stream, _) = listener.accept().unwrap();
	assert_eq!(peer_pid(&stream), 0, "the client's pid is visible here, so this is not the case home has");
	answer(&recorder, &stream, Some(&[client])).unwrap();
}

/// Start the service serving `client` in a PID namespace of its own, ask it
/// for its toolchain from here, and return its answer.
fn ask_across(socket: &Path, client: u32) -> Response {
	let mut namespaces = vec!["--pid", "--fork", "--kill-child"];
	if !rustix::process::geteuid().is_root() {
		namespaces.splice(0..0, ["--user", "--map-root-user"]);
	}
	let mut service = Command::new("unshare")
		.args(namespaces)
		.arg(std::env::current_exe().unwrap_or_else(|err| panic!("no path to this test binary: {err}")))
		.args(["--exact", "the_service_side", "--ignored", "--nocapture"])
		.env(SOCKET, socket)
		.env(CLIENT, client.to_string())
		.stdout(Stdio::piped())
		.stderr(Stdio::piped())
		.spawn()
		.unwrap_or_else(|err| panic!("this test needs util-linux's unshare on PATH: {err}"));
	let started = Instant::now();
	let answered = loop {
		match exchange(socket, &Request::Toolchain {}, CLIENT_TIMEOUT) {
			Err(AskError::Unreachable(_)) if started.elapsed() < Duration::from_secs(60) && service.try_wait().is_ok_and(|status| status.is_none()) => {
				std::thread::sleep(Duration::from_millis(50));
			}
			answered => break answered,
		}
	};
	let output = service.wait_with_output().unwrap_or_else(|err| panic!("the service was lost: {err}"));
	assert!(
		output.status.success(),
		"the service in its own PID namespace failed ({}); can this sandbox make one?\n{}{}",
		output.status,
		String::from_utf8_lossy(&output.stdout),
		String::from_utf8_lossy(&output.stderr)
	);
	answered.unwrap_or_else(|err| panic!("no answer from the service in its own PID namespace: {err:?}"))
}

/// The peer's pid as the kernel reports it here, read as a plain integer.
fn peer_pid(stream: &UnixStream) -> libc::pid_t {
	let mut cred = libc::ucred { pid: -1, uid: 0, gid: 0 };
	let mut len = libc::socklen_t::try_from(std::mem::size_of::<libc::ucred>()).unwrap_or_else(|err| panic!("{err}"));
	// SAFETY: `cred` and `len` are live, writable and sized as SO_PEERCRED
	// writes them; the descriptor is `stream`'s, open for this call.
	let read = unsafe { libc::getsockopt(stream.as_raw_fd(), libc::SOL_SOCKET, libc::SO_PEERCRED, (&raw mut cred).cast(), &raw mut len) };
	assert_eq!(read, 0, "{}", std::io::Error::last_os_error());
	cred.pid
}

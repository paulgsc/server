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
//! which the test proper runs by name under `unshare --pid --fork`: the
//! service is then PID 1 of a namespace this process is not in, and sees
//! this process, its client, with pid 0. Inside, it checks the pid really is
//! 0 before it answers, so the case it proves is the one home has.
//!
//! The namespace is made the first way that works ([`Namespace`]): as root
//! directly; otherwise in a user namespace of its own (`--user
//! --map-root-user`); and where unprivileged user namespaces are refused, as
//! on GitHub's hosted Ubuntu runner, whose `AppArmor` restricts them, under
//! passwordless `sudo -n`. A sandbox that allows none of them **fails** the
//! test, saying what each refused: one that passes having run nothing proves
//! nothing. It needs `rustc` on `PATH`, as the runner's other tests do.

use rust_runner::recording::Recorder;
use rust_runner::service::{answer, exchange, AskError, Refusal, Request, Response, CLIENT_TIMEOUT};
use rust_runner::Limits;
use std::os::fd::AsRawFd;
use std::os::unix::fs::PermissionsExt;
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
	let namespace = Namespace::find();
	let other = if namespace.seen == 10_000 { 10_001 } else { 10_000 };

	assert_eq!(
		namespace.ask_across(&work.path().join("client"), namespace.seen),
		Response::Toolchain { toolchain: toolchain.clone() },
		"a client's uid from another PID namespace is served ({namespace:?})"
	);
	assert_eq!(
		namespace.ask_across(&work.path().join("other"), other),
		Response::Refused { reason: Refusal::Peer, toolchain },
		"any other uid from another PID namespace is still refused ({namespace:?})"
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
	// Open to the client whoever made it, as the runner opens its own: under
	// `sudo` the service is root and the client is not.
	std::fs::set_permissions(Path::new(&socket), std::fs::Permissions::from_mode(0o666)).unwrap();
	let (stream, _) = listener.accept().unwrap();
	assert_eq!(peer_pid(&stream), 0, "the client's pid is visible here, so this is not the case home has");
	answer(&recorder, &stream, Some(&[client])).unwrap();
}

/// How this process can start the service in a PID namespace it is not in,
/// and the uid the service then sees this process as.
#[derive(Debug)]
struct Namespace {
	/// The command up to and including `unshare`'s flags.
	command: Vec<String>,
	/// Whether the variables the service needs must be passed through
	/// `sudo`, which drops them.
	through_sudo: bool,
	/// This process's uid as the service sees it.
	seen: u32,
}

impl Namespace {
	/// The first way this sandbox allows: as root, in a user namespace of
	/// its own, or under passwordless `sudo`. None is a failure that says
	/// what each refused.
	fn find() -> Self {
		let pid = ["--pid", "--fork", "--kill-child"].map(String::from);
		let euid = rustix::process::geteuid();
		if euid.is_root() {
			return Self {
				command: [vec!["unshare".into()], pid.to_vec()].concat(),
				through_sudo: false,
				seen: 0,
			};
		}
		// This uid is root in the service's user namespace.
		let user = [vec!["unshare".into(), "--user".into(), "--map-root-user".into()], pid.to_vec()].concat();
		let Some(user_refused) = refusal(&user) else {
			return Self {
				command: user,
				through_sudo: false,
				seen: 0,
			};
		};
		// The service is root, and sees this process as its own uid.
		let sudo = [vec!["sudo".into(), "-n".into(), "unshare".into()], pid.to_vec()].concat();
		let Some(sudo_refused) = refusal(&sudo) else {
			return Self {
				command: sudo,
				through_sudo: true,
				seen: euid.as_raw(),
			};
		};
		panic!(
			"this test needs a PID namespace this process is not in, and this sandbox makes none: a user namespace was refused \
			 ({user_refused}) and so was passwordless sudo ({sudo_refused})"
		);
	}

	/// Start the service serving `client` in its own PID namespace, ask it
	/// for its toolchain from here, and return its answer.
	fn ask_across(&self, socket: &Path, client: u32) -> Response {
		let mut variables = vec![(SOCKET.to_owned(), socket.as_os_str().to_owned()), (CLIENT.to_owned(), client.to_string().into())];
		if self.through_sudo {
			variables.extend(toolchain_variables());
		}
		let mut service = Command::new(&self.command[0])
			.args(&self.command[1..])
			.arg("env")
			.args(variables.into_iter().map(|(name, value)| {
				let mut variable = std::ffi::OsString::from(name + "=");
				variable.push(value);
				variable
			}))
			.arg(std::env::current_exe().unwrap_or_else(|err| panic!("no path to this test binary: {err}")))
			.args(["--exact", "the_service_side", "--ignored", "--nocapture"])
			.stdout(Stdio::piped())
			.stderr(Stdio::piped())
			.spawn()
			.unwrap_or_else(|err| panic!("could not start {:?}: {err}", self.command));
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
			"the service in its own PID namespace failed ({}, via {:?})\n{}{}",
			output.status,
			self.command,
			String::from_utf8_lossy(&output.stdout),
			String::from_utf8_lossy(&output.stderr)
		);
		answered.unwrap_or_else(|err| panic!("no answer from the service in its own PID namespace: {err:?}"))
	}
}

/// Why `command` cannot run `true` in a new PID namespace, or `None` if it
/// can.
fn refusal(command: &[String]) -> Option<String> {
	let tried = Command::new(&command[0]).args(&command[1..]).arg("true").stdin(Stdio::null()).output();
	match tried {
		Ok(output) if output.status.success() => None,
		Ok(output) => Some(output.status.to_string() + ": " + String::from_utf8_lossy(&output.stderr).trim()),
		Err(err) => Some(err.to_string()),
	}
}

/// What the service needs to find and run `rustc` as this process does,
/// which `sudo` would otherwise reset.
fn toolchain_variables() -> Vec<(String, std::ffi::OsString)> {
	// Not configuration: forwarded unchanged to the test's own child, as
	// `leetype_runner` forwards them to its.
	#[allow(clippy::disallowed_methods)]
	let read = |name: &str| std::env::var_os(name).map(|value| (name.to_owned(), value));
	["PATH", "HOME", "RUSTUP_HOME", "CARGO_HOME", "RUSTUP_TOOLCHAIN"]
		.iter()
		.filter_map(|name| read(name))
		.collect()
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

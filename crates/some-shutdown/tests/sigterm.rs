//! SIGTERM, as `docker stop` sends it, resolves `shutdown_signal` and the
//! process exits cleanly (#482).
//!
//! A signal's disposition is process-wide, so this does not raise SIGTERM in
//! the test process itself. It re-runs this test binary as a child that waits
//! on `shutdown_signal`, sends the child SIGTERM, and requires a zero exit
//! within a deadline. A child with no SIGTERM handler dies by the signal.

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Set only in the child, so the child half is a no-op in a normal run.
const CHILD: &str = "SOME_SHUTDOWN_CHILD";
const READY: &str = "some-shutdown: handlers installed";
const DEADLINE: Duration = Duration::from_secs(5);

#[tokio::test]
async fn child_waits_for_shutdown_signal() {
	if std::env::var_os(CHILD).is_none() {
		return;
	}
	let shutdown = some_shutdown::shutdown_signal();
	writeln!(std::io::stdout(), "{READY}").unwrap();
	shutdown.await;
}

#[test]
fn sigterm_stops_the_process_cleanly() {
	let mut child = Command::new(std::env::current_exe().unwrap())
		.args(["--exact", "child_waits_for_shutdown_signal", "--nocapture"])
		.env(CHILD, "1")
		.stdout(Stdio::piped())
		.spawn()
		.unwrap();

	// Held to the end: closing the pipe early would fail the child's own
	// writes after the signal, and with them its exit status.
	let mut stdout = BufReader::new(child.stdout.take().unwrap()).lines();
	assert!(stdout.any(|line| line.unwrap().contains(READY)), "the child exited before installing its handlers");

	let sent = Command::new("kill").args(["-TERM", &child.id().to_string()]).status().unwrap();
	assert!(sent.success(), "kill -TERM failed: {sent}");

	let sent_at = Instant::now();
	let status = loop {
		if let Some(status) = child.try_wait().unwrap() {
			break status;
		}
		if sent_at.elapsed() > DEADLINE {
			child.kill().unwrap();
			panic!("the child was still running {DEADLINE:?} after SIGTERM");
		}
		std::thread::sleep(Duration::from_millis(20));
	};
	drop(stdout);
	assert!(status.success(), "the child did not exit cleanly on SIGTERM: {status}");
}

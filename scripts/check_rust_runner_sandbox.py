#!/usr/bin/env python3
"""Run hostile programs against the rust-runner container, as compose starts it.

paulgsc/some-ui#2234 (RR3): every program the runner records is treated as
hostile, so the hostile cases are tests, not review notes. This brings up
`infra/compose/rust_runner.yml` exactly as written, under its own project
name, and checks:

  * the container is the sandbox the compose file declares (RX1): no
    network, a read-only root, uid 10001, no capabilities, no new
    privileges, an init, and its pid, memory and CPU caps;
  * the toolchain is the pinned rustc 1.97.0, and the worked example
    (paulgsc/some-ui#2231) records the exact text it prints;
  * a program that loops, allocates without bound, forks without bound,
    escapes its process group, opens a socket, writes where it can, or
    kills the runner itself ends as a recorded time, memory or runtime
    outcome (or, for the last, a restart), and leaves no process and no
    file behind (RX2, RX3).

Requests go through `rust-runner ask` exec'd as uid 10002, so the runner's
sweep of its own uid does not take the client with it.

Usage:
  python3 scripts/check_rust_runner_sandbox.py           # build, check, tear down
  python3 scripts/check_rust_runner_sandbox.py --no-build  # use the image already tagged

Needs Docker with compose. Stdlib only. Exits 1 on the first failed check.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
import time
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
COMPOSE_FILE = REPO_ROOT / "infra" / "compose" / "rust_runner.yml"
PROJECT = "rust-runner-check"
CONTAINER = "rust-runner"
CLIENT_UID = "10002"
TOOLCHAIN = "rustc 1.97.0 (2d8144b78 2026-07-07); --edition 2024 -C opt-level=0"
# The processes the container holds between jobs: tini and the runner.
RESIDENTS = ("/sbin/docker-init -- /usr/local/bin/rust-runner serve", "/usr/local/bin/rust-runner serve")


class Failed(Exception):
	pass


def compose(*args: str, stdin: str | None = None, check: bool = True, timeout: float = 600) -> subprocess.CompletedProcess[str]:
	command = ["docker", "compose", "-p", PROJECT, "-f", str(COMPOSE_FILE), *args]
	result = subprocess.run(command, input=stdin, capture_output=True, text=True, timeout=timeout, check=False)
	if check and result.returncode != 0:
		raise Failed(f"`{' '.join(command)}` failed ({result.returncode}):\n{result.stdout}{result.stderr}")
	return result


def ask(request: dict, timeout: float = 180) -> dict:
	result = compose("exec", "-T", "--user", CLIENT_UID, CONTAINER, "rust-runner", "ask", stdin=json.dumps(request), timeout=timeout)
	return json.loads(result.stdout)


def record(source: str) -> dict:
	answer = ask({"ask": "record", "toolchain": TOOLCHAIN, "source": source})
	if answer.get("answer") != "recorded":
		raise Failed(f"not recorded: {answer}")
	return answer


def expect(condition: bool, message: str) -> None:
	if not condition:
		raise Failed(message)


def wait_ready(deadline_seconds: float = 120) -> None:
	deadline = time.monotonic() + deadline_seconds
	last = ""
	while time.monotonic() < deadline:
		result = compose("exec", "-T", "--user", CLIENT_UID, CONTAINER, "rust-runner", "ask", stdin='{"ask":"toolchain"}', check=False, timeout=30)
		if result.returncode == 0:
			return
		last = result.stdout + result.stderr
		time.sleep(1)
	raise Failed(f"the runner did not answer within {deadline_seconds} s: {last}")


def leftovers() -> list[str]:
	"""Every process in the container but tini and the runner."""
	result = subprocess.run(["docker", "top", CONTAINER, "-eo", "pid,args"], capture_output=True, text=True, check=True)
	processes = [line.split(None, 1)[1] for line in result.stdout.splitlines()[1:] if len(line.split(None, 1)) == 2]
	return [args for args in processes if args not in RESIDENTS]


def files_left() -> list[str]:
	"""Anything in the two writable places but the socket."""
	# /work is the runner's own (0700), so list it as the runner's uid, which
	# is safe between jobs: the sweep runs only during one.
	work = compose("exec", "-T", CONTAINER, "ls", "-A", "/work").stdout.split()
	run = compose("exec", "-T", CONTAINER, "ls", "-A", "/run/rust-runner").stdout.split()
	return [f"/work/{name}" for name in work] + [f"/run/rust-runner/{name}" for name in run if name != "runner.sock"]


def clean(after: str) -> None:
	# A killed process can take a moment to leave the table.
	deadline = time.monotonic() + 5
	while leftovers() and time.monotonic() < deadline:
		time.sleep(0.2)
	expect(leftovers() == [], f"{after} left processes behind: {leftovers()}")
	expect(files_left() == [], f"{after} left files behind: {files_left()}")


def check_sandbox() -> None:
	inspected = json.loads(subprocess.run(["docker", "inspect", CONTAINER], capture_output=True, text=True, check=True).stdout)[0]
	host = inspected["HostConfig"]
	expect(host["NetworkMode"] == "none", f"network: {host['NetworkMode']}")
	expect(host["ReadonlyRootfs"] is True, "the root is writable")
	expect(inspected["Config"]["User"] == "10001:10001", f"user: {inspected['Config']['User']}")
	expect(host["CapDrop"] == ["ALL"] and not host.get("CapAdd"), f"capabilities: drop {host['CapDrop']} add {host.get('CapAdd')}")
	expect("no-new-privileges:true" in (host.get("SecurityOpt") or []), f"security: {host.get('SecurityOpt')}")
	expect(host["Init"] is True, "no init")
	expect(host["PidsLimit"] == 128, f"pids: {host['PidsLimit']}")
	expect(host["Memory"] == 1024**3, f"memory: {host['Memory']}")
	expect(host["NanoCpus"] == 10**9, f"cpus: {host['NanoCpus']}")
	expect(not inspected["Config"].get("ExposedPorts") and not host.get("PortBindings"), "a port is published")
	print("ok: the container is the declared sandbox")


def utf8_round(line: str) -> str:
	return 'fn main() {\n    let s = String::from("aéz");\n    let r: usize = s.len() - 1;\n    ' + line + "\n}\n"


def check_worked_example() -> None:
	toolchain = ask({"ask": "toolchain"})
	expect(toolchain == {"answer": "toolchain", "toolchain": TOOLCHAIN}, f"toolchain: {toolchain}")

	indexed = record(utf8_round('let c = s[r]; println!("{c}");'))["rustc"]
	expect(indexed["outcome"] == "compile-error" and indexed["code"] == "E0277", f"s[r]: {indexed}")
	expect(indexed["span"] == {"lineStart": 4, "columnStart": 15, "lineEnd": 4, "columnEnd": 16}, f"s[r]'s span: {indexed['span']}")
	expect(indexed["toolchain"] == "1.97.0", f"the phone's toolchain: {indexed['toolchain']}")
	expect(
		indexed["stderr"].startswith(
			"error[E0277]: the type `str` cannot be indexed by `usize`\n"
			" --> main.rs:4:15\n"
			"  |\n"
			'4 |     let c = s[r]; println!("{c}");\n'
			"  |               ^ string indices are ranges of `usize`\n"
		),
		f"s[r]'s text: {indexed['stderr']!r}",
	)
	expect(indexed["stderr"].endswith("For more information about this error, try `rustc --explain E0277`.\n"), "s[r]'s text ends as rustc's does")

	byte = record(utf8_round('let c = s.as_bytes()[r]; println!("{c}");'))["rustc"]
	expect(byte == {"outcome": "prints", "toolchain": "1.97.0", "compileStderr": "", "stdout": "122\n"}, f"s.as_bytes()[r]: {byte}")

	# `r` goes unused in the last two, and rustc says so while it builds:
	# kept apart from what the program printed.
	unused_r = "warning: unused variable: `r`\n --> main.rs:3:9\n"
	split = record(utf8_round('let c = &s[1..2]; println!("{c}");'))["rustc"]
	expect(split.get("compileStderr", "").startswith(unused_r), f"&s[1..2]'s compile text: {split}")
	split.pop("compileStderr", None)
	expect(
		split
		== {
			"outcome": "panics",
			"toolchain": "1.97.0",
			"stdout": "",
			"stderr": "\nthread 'main' panicked at main.rs:4:15:\n"
			"end byte index 2 is not a char boundary; it is inside 'é' (bytes 1..3 of string)\n"
			"note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace\n",
		},
		f"&s[1..2]: {split}",
	)

	whole = record(utf8_round('let c = &s[1..3]; println!("{c}");'))["rustc"]
	expect(whole.pop("compileStderr", "").startswith(unused_r), f"&s[1..3]'s compile text: {whole}")
	expect(whole == {"outcome": "prints", "toolchain": "1.97.0", "stdout": "é\n"}, f"&s[1..3]: {whole}")
	clean("the worked example")
	print("ok: the worked example records what rustc 1.97.0 prints")


LOOP = "fn main() { let mut i: u64 = 0; loop { i = i.wrapping_add(1); std::hint::black_box(i); } }"
HOARD = "fn main() { let mut hoard: Vec<Vec<u8>> = Vec::new(); loop { hoard.push(vec![1_u8; 64 << 20]); std::hint::black_box(&hoard); } }"
FORK_BOMB = """
fn main() {
    let me = std::env::current_exe().unwrap();
    loop {
        let _ = std::process::Command::new(&me).spawn();
    }
}
"""
THREAD_BOMB = """
fn main() {
    let mut started = 0;
    loop {
        match std::thread::Builder::new().spawn(|| loop { std::thread::park(); }) {
            Ok(_) => started += 1,
            Err(err) => { println!("{started} threads, then: {err}"); loop { std::hint::black_box(()); } }
        }
    }
}
"""
ESCAPE = """
fn main() {
    // Out of the process group, holding stdout: only the sweep ends it.
    std::process::Command::new("setsid").args(["sleep", "300"]).spawn().unwrap();
    std::process::Command::new("setsid").args(["sh", "-c", "while :; do :; done"]).spawn().unwrap();
    println!("left them running");
}
"""
SOCKETS = """
use std::net::{TcpListener, TcpStream, ToSocketAddrs};
fn main() {
    // Loopback exists, and nothing else does.
    println!("listen on loopback: {}", TcpListener::bind("127.0.0.1:0").is_ok());
    println!("resolve file-host-server: {}", "file-host-server:3000".to_socket_addrs().is_ok());
    TcpStream::connect("1.1.1.1:80").unwrap();
}
"""
WRITES = """
fn main() {
    std::fs::write("/work/left-behind", vec![0_u8; 1 << 20]).unwrap();
    std::fs::create_dir_all("/work/deep/er").unwrap();
    std::fs::write("/run/rust-runner/left-behind", b"x").unwrap();
    println!("etc: {}", std::fs::write("/etc/left-behind", b"x").is_ok());
    println!("tmp: {}", std::fs::write("/tmp/left-behind", b"x").is_ok());
}
"""
KILL_RUNNER = """
fn main() {
    for entry in std::fs::read_dir("/proc").unwrap() {
        let path = entry.unwrap().path();
        let cmdline = std::fs::read(path.join("cmdline")).unwrap_or_default();
        if cmdline == b"/usr/local/bin/rust-runner\\0serve\\0" {
            let pid = path.file_name().unwrap().to_string_lossy().into_owned();
            std::process::Command::new("sh").args(["-c", &format!("kill -9 {pid}")]).status().unwrap();
        }
    }
}
"""


def check_hostile() -> None:
	looped = record(LOOP)["rustc"]
	expect(looped["outcome"] == "stopped" and looped["why"] == "time", f"a loop: {looped}")
	clean("a loop")
	print("ok: a loop is stopped for time and leaves nothing")

	hoarded = record(HOARD)["rustc"]
	expect(hoarded["outcome"] == "stopped" and hoarded["why"] == "memory", f"unbounded allocation: {hoarded}")
	clean("unbounded allocation")
	print("ok: unbounded allocation is stopped for memory and leaves nothing")

	bombed = record(FORK_BOMB)["rustc"]
	expect(bombed["outcome"] == "stopped" and bombed["why"] == "time", f"a fork bomb: {bombed}")
	clean("a fork bomb")
	print("ok: a fork bomb is held to the process ceiling, stopped for time, and leaves nothing")

	threads = record(THREAD_BOMB)
	expect(threads["rustc"]["outcome"] == "stopped" and "threads, then:" in threads["rustc"]["stdout"], f"a thread bomb: {threads['rustc']}")
	started = int(threads["rustc"]["stdout"].split()[0])
	expect(started < 64, f"a thread bomb started {started} threads, past the process ceiling")
	clean("a thread bomb")
	print(f"ok: a thread bomb is held to the process ceiling ({started} threads) and leaves nothing")

	escaped = record(ESCAPE)["rustc"]
	expect(escaped == {"outcome": "prints", "toolchain": "1.97.0", "compileStderr": "", "stdout": "left them running\n"}, f"processes out of the group: {escaped}")
	clean("processes out of the group")
	print("ok: processes that leave the group are swept")

	sockets = record(SOCKETS)["rustc"]
	expect(sockets["outcome"] == "panics", f"a socket: {sockets}")
	expect("listen on loopback: true\nresolve file-host-server: false\n" == sockets["stdout"], f"a socket's stdout: {sockets['stdout']!r}")
	expect("Network is unreachable" in sockets["stderr"], f"a socket's stderr: {sockets['stderr']!r}")
	clean("a socket")
	print("ok: a socket reaches nothing past loopback, and the connect fails as a panic")

	wrote = record(WRITES)["rustc"]
	expect(wrote == {"outcome": "prints", "toolchain": "1.97.0", "compileStderr": "", "stdout": "etc: false\ntmp: false\n"}, f"writes: {wrote}")
	clean("writes")
	print("ok: only /work and the socket's directory are writable, and both are emptied")

	restarts = json.loads(subprocess.run(["docker", "inspect", CONTAINER], capture_output=True, text=True, check=True).stdout)[0]["RestartCount"]
	killed = compose("exec", "-T", "--user", CLIENT_UID, CONTAINER, "rust-runner", "ask", stdin=json.dumps({"ask": "record", "toolchain": TOOLCHAIN, "source": KILL_RUNNER}), check=False, timeout=180)
	expect(killed.returncode != 0, f"the runner answered after being killed: {killed.stdout}")
	wait_ready()
	after = json.loads(subprocess.run(["docker", "inspect", CONTAINER], capture_output=True, text=True, check=True).stdout)[0]["RestartCount"]
	expect(after == restarts + 1, f"restart count {restarts} -> {after}")
	clean("killing the runner")
	print("ok: a program that kills the runner gets no answer, and the restart leaves nothing")


def main() -> int:
	parser = argparse.ArgumentParser(description=__doc__.split("\n", 1)[0])
	parser.add_argument("--no-build", action="store_true", help="use the image already tagged pgathondu/rust-runner:latest")
	parser.add_argument("--keep", action="store_true", help="leave the container running afterwards")
	args = parser.parse_args()

	try:
		if not args.no_build:
			compose("build", timeout=3600)
		compose("up", "-d", "--no-build", "--pull", "never", "--force-recreate")
		wait_ready()
		check_sandbox()
		check_worked_example()
		check_hostile()
	except (Failed, subprocess.TimeoutExpired, json.JSONDecodeError) as err:
		print(f"FAILED: {err}", file=sys.stderr)
		print(compose("logs", "--no-color", check=False).stdout[-4000:], file=sys.stderr)
		return 1
	finally:
		if not args.keep:
			compose("down", "-v", "--remove-orphans", check=False)
	print("ok: every hostile program was recorded and left nothing behind")
	return 0


if __name__ == "__main__":
	sys.exit(main())

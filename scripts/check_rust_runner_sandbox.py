#!/usr/bin/env python3
"""Run hostile programs against the rust-runner container, as compose starts it.

paulgsc/some-ui#2234 (RR3): every program the runner records is treated as
hostile, so the hostile cases are tests, not review notes. This brings up
`infra/compose/rust_runner.yml` exactly as written, under its own project
name, and checks:

  * the programs' id pair, 10101:10101, is named by no other Dockerfile or
    compose file in the stack;
  * the container is the sandbox the compose file declares (RX1): not
    privileged, no network, no shared IPC, pid or user namespace, no device,
    no unconfined seccomp or AppArmor, the declared mounts only, a read-only
    root, root with only SETUID and SETGID and no new privileges, an init,
    its pid, memory and CPU caps, an environment of known names only, no
    setuid binary and a toolchain only root can write; every program runs
    as 10101 with no group and no capability; and only the client uid
    (10000) and root are served on the socket;
  * the toolchain is the pinned rustc 1.97.0, and the worked example
    (paulgsc/some-ui#2231) records the exact text it prints;
  * a program that loops, allocates without bound, forks or spawns threads
    without bound, escapes its process group, opens a socket, writes where
    it can (a directory it makes unreadable included, and one 20000 levels
    deep), stops or kills the
    runner, queues requests on the runner's socket, or leaves System V IPC
    behind; and a source that makes rustc read /dev/zero, expand a macro
    without end, build a huge static or print a flood of errors; and a
    program that floods stdout: each ends as a recorded outcome, and leaves
    no process, no file and no IPC object behind (RX2, RX3). Only IPC a
    program leaves restarts the runner, into a fresh namespace.

Requests go through `rust-runner ask` exec'd as uid 10000, file_host's, the
one client uid the runner serves besides root.

Usage:
  python3 scripts/check_rust_runner_sandbox.py           # build, check, tear down
  python3 scripts/check_rust_runner_sandbox.py --no-build  # use the image already tagged

Needs Docker with compose. Stdlib only. Exits 1 on the first failed check.
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
import time
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
COMPOSE_FILE = REPO_ROOT / "infra" / "compose" / "rust_runner.yml"
PROJECT = "rust-runner-check"
CONTAINER = "rust-runner"
CLIENT_UID = "10000"
OTHER_UID = "10002"
PROGRAM_ID = "10101"
PROGRAM_IDS = "\t".join([PROGRAM_ID] * 4)
# The image's own variables and the runner's settings; nothing else, and so
# nothing secret, may be in its environment.
ENV_ALLOWED = {
	"PATH",
	"RUSTUP_HOME",
	"CARGO_HOME",
	"RUST_VERSION",
	"RUST_RUNNER_RUSTC",
	"RUST_RUNNER_SOCKET",
	"RUST_RUNNER_WORK",
	"RUST_RUNNER_PROGRAM_UID",
	"RUST_RUNNER_PROGRAM_GID",
	"RUST_RUNNER_CLIENT_UID",
}
# CAP_SETGID (6) and CAP_SETUID (7).
RUNNER_CAPS = 0xC0
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
	"""Anything in the writable places but the socket, and any IPC object."""
	work = compose("exec", "-T", CONTAINER, "ls", "-A", "/work").stdout.split()
	run = compose("exec", "-T", CONTAINER, "ls", "-A", "/run/rust-runner").stdout.split()
	mqueue = compose("exec", "-T", CONTAINER, "ls", "-A", "/dev/mqueue").stdout.split()
	shm = compose("exec", "-T", CONTAINER, "sh", "-c", "ls -A /dev/shm 2>/dev/null || true").stdout.split()
	left = [f"/work/{name}" for name in work] + [f"/run/rust-runner/{name}" for name in run if name != "runner.sock"]
	left += [f"/dev/mqueue/{name}" for name in mqueue] + [f"/dev/shm/{name}" for name in shm]
	for table in ("shm", "sem", "msg"):
		rows = compose("exec", "-T", CONTAINER, "cat", f"/proc/sysvipc/{table}").stdout.splitlines()[1:]
		left += [f"/proc/sysvipc/{table}: {row.split()[1] if row.split() else row}" for row in rows]
	return left


def clean(after: str) -> None:
	# A killed process can take a moment to leave the table.
	deadline = time.monotonic() + 5
	while leftovers() and time.monotonic() < deadline:
		time.sleep(0.2)
	expect(leftovers() == [], f"{after} left processes behind: {leftovers()}")
	expect(files_left() == [], f"{after} left files behind: {files_left()}")


def restart_count() -> int:
	return json.loads(subprocess.run(["docker", "inspect", CONTAINER], capture_output=True, text=True, check=True).stdout)[0]["RestartCount"]


def runner_status() -> str:
	"""The runner's /proc status, read as root from outside the job."""
	pid = compose("exec", "-T", CONTAINER, "sh", "-c", "for p in /proc/[0-9]*; do [ \"$(tr '\\0' ' ' < $p/cmdline)\" = '/usr/local/bin/rust-runner serve ' ] && echo ${p#/proc/}; done").stdout.split()
	expect(len(pid) == 1, f"the runner's pid: {pid}")
	return compose("exec", "-T", CONTAINER, "cat", f"/proc/{pid[0]}/status").stdout


def check_ids() -> None:
	"""No other image or compose file names the programs' uid or gid."""
	others = [path for path in (REPO_ROOT / "infra" / "docker").glob("Dockerfile.*") if path.name != "Dockerfile.rust-runner"]
	others += [path for path in (REPO_ROOT / "infra" / "compose").glob("*.yml") if path.name != "rust_runner.yml"]
	others += [REPO_ROOT / "docker-compose.yml"]
	named = [str(path.relative_to(REPO_ROOT)) for path in others if path.exists() and re.search(rf"\b{PROGRAM_ID}\b", path.read_text())]
	expect(len(others) > 3, f"the stack's files were read: {others}")
	expect(named == [], f"the programs' id {PROGRAM_ID} is named by {named}")
	print(f"ok: no other image or compose file names {PROGRAM_ID}")


def check_sandbox() -> None:
	inspected = json.loads(subprocess.run(["docker", "inspect", CONTAINER], capture_output=True, text=True, check=True).stdout)[0]
	host = inspected["HostConfig"]
	expect(host["NetworkMode"] == "none", f"network: {host['NetworkMode']}")
	expect(host["IpcMode"] == "none", f"ipc: {host['IpcMode']}")
	expect(host.get("Sysctls") == {"kernel.shm_rmid_forced": "1"}, f"sysctls: {host.get('Sysctls')}")
	expect(host["ReadonlyRootfs"] is True, "the root is writable")
	expect(inspected["Config"]["User"] == "0:0", f"user: {inspected['Config']['User']}")
	added = {cap.removeprefix("CAP_") for cap in host.get("CapAdd") or []}
	expect(host["CapDrop"] == ["ALL"] and added == {"SETUID", "SETGID"}, f"capabilities: drop {host['CapDrop']} add {host.get('CapAdd')}")
	expect(host["Privileged"] is False, "privileged")
	expect(host["PidMode"] in ("", "private"), f"pid namespace: {host['PidMode']}")
	expect(host["UsernsMode"] == "", f"user namespace: {host['UsernsMode']}")
	expect(not host.get("Devices") and not host.get("DeviceRequests"), f"devices: {host.get('Devices')} {host.get('DeviceRequests')}")
	security = host.get("SecurityOpt") or []
	expect(
		"no-new-privileges:true" in security and not any(re.match(r"(seccomp|apparmor)[=:]unconfined", option) for option in security),
		f"security: {security}",
	)
	expect(set(host.get("Tmpfs") or {}) == {"/work"}, f"tmpfs: {host.get('Tmpfs')}")
	# Compose may declare the socket's volume either way; together they hold
	# that one volume and nothing else (no host path).
	declared = [(bind.split(":")[0], bind.split(":")[1]) for bind in host.get("Binds") or []]
	declared += [(mount.get("Source"), mount.get("Target")) for mount in host.get("Mounts") or []]
	expect(declared == [(f"{PROJECT}_rust_runner_socket", "/run/rust-runner")], f"mounts declared: {declared}")
	mounted = [(mount.get("Type"), mount.get("Destination")) for mount in inspected.get("Mounts") or []]
	expect(mounted == [("volume", "/run/rust-runner")], f"mounts: {mounted}")
	names = {entry.split("=", 1)[0] for entry in inspected["Config"].get("Env") or []}
	expect(names <= ENV_ALLOWED, f"variables beyond the allowlist: {sorted(names - ENV_ALLOWED)}")
	status = runner_status()
	fields = dict(line.split(":", 1) for line in status.splitlines() if ":" in line)
	expect(fields["Uid"].split()[0] == "0", f"the runner's uid: {fields['Uid']}")
	expect(int(fields["CapEff"], 16) == RUNNER_CAPS, f"the runner's capabilities: {fields['CapEff']}")
	expect(fields["NoNewPrivs"].strip() == "1", f"the runner's no-new-privileges: {fields['NoNewPrivs']}")
	# Inspected with every capability (`--privileged`, this exec only), so a
	# directory the runner's root cannot read (apt's, 0700) hides nothing.
	setuid = compose("exec", "-T", "--privileged", CONTAINER, "find", "/", "-xdev", "-perm", "/6000", "-type", "f").stdout.split()
	expect(setuid == [], f"setuid or setgid binaries: {setuid}")
	writable = compose("exec", "-T", "--privileged", CONTAINER, "find", "/usr/local/rustup", "/usr/local/cargo", "-perm", "/022", "!", "-type", "l").stdout.split()
	expect(writable == [], f"the toolchain is writable by others: {writable[:5]}")
	other = compose("exec", "-T", "--user", OTHER_UID, CONTAINER, "rust-runner", "ask", stdin='{"ask":"toolchain"}', timeout=30)
	expect(json.loads(other.stdout).get("reason") == "peer", f"uid {OTHER_UID} was served: {other.stdout}")
	expect("no-new-privileges:true" in (host.get("SecurityOpt") or []), f"security: {host.get('SecurityOpt')}")
	expect(host["Init"] is True, "no init")
	expect(host["PidsLimit"] == 128, f"pids: {host['PidsLimit']}")
	expect(host["Memory"] == 1024**3, f"memory: {host['Memory']}")
	expect(host["NanoCpus"] == 10**9, f"cpus: {host['NanoCpus']}")
	expect(not inspected["Config"].get("ExposedPorts") and not host.get("PortBindings"), "a port is published")
	identity = record(IDENTITY)["rustc"]
	expect(
		identity["outcome"] == "prints" and identity["stdout"] == f"Uid:\t{PROGRAM_IDS}\nGid:\t{PROGRAM_IDS}\nGroups:\t\nCapEff:\t0000000000000000\n",
		f"a program's identity: {identity}",
	)
	clean("a program's identity")
	print("ok: the container is the declared sandbox, a program is 10101 with no group or capability, and only the client uid is served")


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


IDENTITY = """
fn main() {
    let status = std::fs::read_to_string("/proc/self/status").unwrap();
    for line in status.lines() {
        if ["Uid:", "Gid:", "Groups:", "CapEff:"].iter().any(|key| line.starts_with(key)) {
            println!("{}", line.trim_end_matches(' '));
        }
    }
}
"""
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
use std::os::unix::fs::PermissionsExt;
fn main() {
    // Its own directory: written, and one directory in it made unreadable,
    // which the runner (root, without FOWNER) could not open; `clean`, as
    // the program's uid, can.
    std::fs::write("left-behind", vec![0_u8; 1 << 20]).unwrap();
    std::fs::create_dir_all("deep/er").unwrap();
    std::fs::write("deep/er/file", b"x").unwrap();
    std::fs::set_permissions("deep/er", std::fs::Permissions::from_mode(0o000)).unwrap();
    std::fs::set_permissions("deep", std::fs::Permissions::from_mode(0o000)).unwrap();
    std::os::unix::fs::symlink("/run/rust-runner", "link").unwrap();
    for (name, path) in [
        ("work", "/work/left-behind"),
        ("socket", "/run/rust-runner/left-behind"),
        ("etc", "/etc/left-behind"),
        ("tmp", "/tmp/left-behind"),
        ("shm", "/dev/shm/left-behind"),
        ("mqueue", "/dev/mqueue/left-behind"),
    ] {
        println!("{name}: {}", std::fs::write(path, b"x").is_ok());
    }
}
"""
DEEP = """
use std::os::unix::fs::PermissionsExt;
fn main() {
    // Far past PATH_MAX, by name relative to where it is; the bottom made
    // unreadable. A host whose LSM mediates by full path (AppArmor on
    // GitHub's runners) refuses a name past its buffer: stop there.
    let mut depth = 0;
    while depth < 20_000 {
        match std::fs::create_dir("d") {
            Ok(()) => {}
            Err(error) if error.raw_os_error() == Some(36) => break,
            Err(error) => panic!("{error}"),
        }
        std::env::set_current_dir("d").unwrap();
        depth += 1;
    }
    std::fs::write("bottom", b"x").unwrap();
    std::fs::set_permissions(".", std::fs::Permissions::from_mode(0o000)).unwrap();
    if depth == 20_000 {
        println!("deep");
    } else {
        println!("deep: the host stopped it at {depth}");
    }
}
"""
STOP_RUNNER = """
fn main() {
    // Its parent is the runner: stop it, so the ceiling never fires, or
    // kill it, or read it. As another uid than root's, none of it works.
    let status = std::fs::read_to_string("/proc/self/status").unwrap();
    let parent = status.lines().find_map(|line| line.strip_prefix("PPid:")).unwrap().trim().to_owned();
    let signal = |signal: &str| {
        std::process::Command::new("sh").args(["-c", &(String::from("kill -") + signal + " " + &parent)]).status().unwrap().success()
    };
    println!("stop: {}", signal("STOP"));
    println!("kill: {}", signal("KILL"));
    println!("environ: {}", std::fs::read(String::from("/proc/") + &parent + "/environ").is_ok());
    loop {
        std::hint::black_box(());
    }
}
"""
QUEUES = """
use std::io::Write;
fn main() {
    // The socket is reachable from here; a request queued on it would be
    // answered after this program is killed, and could queue the next.
    let mut socket = std::os::unix::net::UnixStream::connect("/run/rust-runner/runner.sock").unwrap();
    socket.write_all(b"{\\"ask\\":\\"toolchain\\"}\\n").unwrap();
    println!("queued");
}
"""
SHARED_MEMORY = """
unsafe extern "C" {
    fn shmget(key: i32, size: usize, flags: i32) -> i32;
}
fn main() {
    // 64 MiB parked in System V shared memory, never attached: with
    // kernel.shm_rmid_forced it goes when this process does.
    let id = unsafe { shmget(0, 64 << 20, 0o1000 | 0o600) };
    println!("shm: {}", id >= 0);
}
"""
SEMAPHORES = """
unsafe extern "C" {
    fn semget(key: i32, count: i32, flags: i32) -> i32;
    fn msgget(key: i32, flags: i32) -> i32;
}
fn main() {
    let sem = unsafe { semget(0, 1, 0o1000 | 0o600) };
    let msg = unsafe { msgget(0, 0o1000 | 0o600) };
    println!("sem: {}, msg: {}", sem >= 0, msg >= 0);
}
"""
DEV_ZERO = """
static ZERO: &[u8] = include_bytes!("/dev/zero");
fn main() { println!("{}", ZERO.len()); }
"""
MACRO_BLOWUP = """
#![recursion_limit = "100000"]
macro_rules! blow { () => {}; ($t:tt $($rest:tt)*) => { blow!($($rest)*); blow!($($rest)*); }; }
fn main() { blow!(a a a a a a a a a a a a a a a a a a a a a a a a a a a a a a a a a a a a a a a a); }
"""
HUGE_STATIC = """
static BIG: [u8; 1 << 27] = [1; 1 << 27];
fn main() { println!("{}", BIG[12345]); }
"""
ERROR_FLOOD = "fn main() {\n" + "".join(f'    let x{i}: u8 = "no";\n' for i in range(1500)) + "}\n"
STDOUT_FLOOD = """
use std::io::Write;
fn main() {
    let line = [b'x'; 1 << 16];
    let mut out = std::io::stdout().lock();
    for _ in 0..1024 {
        out.write_all(&line).unwrap();
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
	expect(
		wrote == {"outcome": "prints", "toolchain": "1.97.0", "compileStderr": "", "stdout": "work: false\nsocket: false\netc: false\ntmp: false\nshm: false\nmqueue: false\n"},
		f"writes: {wrote}",
	)
	clean("writes")
	print("ok: a program writes only in its own directory, which is removed, unreadable directories and all")

	restarts = restart_count()
	deep = record(DEEP)["rustc"]
	stdout = deep.get("stdout", "")
	# 20000 levels where nothing mediates by full path; on an AppArmor host
	# (GitHub's runners) as deep as it allows, about PATH_MAX bytes of path.
	host_stopped = re.fullmatch(r"deep: the host stopped it at (\d+)\n", stdout)
	expect(
		{k: v for k, v in deep.items() if k != "stdout"} == {"outcome": "prints", "toolchain": "1.97.0", "compileStderr": ""}
		and (stdout == "deep\n" or (host_stopped is not None and int(host_stopped[1]) > 1000)),
		f"a deep tree: {deep}",
	)
	clean("a deep tree")
	expect(restart_count() == restarts, "the runner restarted for a deep tree")
	depth = "20000" if host_stopped is None else host_stopped[1]
	print(f"ok: a directory {depth} levels deep is removed, with no restart")

	stopped = record(STOP_RUNNER)["rustc"]
	expect(stopped["outcome"] == "stopped" and stopped["why"] == "time", f"stopping the runner: {stopped}")
	expect(stopped["stdout"] == "stop: false\nkill: false\nenviron: false\n", f"stopping the runner's stdout: {stopped['stdout']!r}")
	expect(ask({"ask": "toolchain"})["answer"] == "toolchain", "the runner stopped answering")
	expect(restart_count() == restarts, "the runner restarted")
	state = next(line for line in runner_status().splitlines() if line.startswith("State:"))
	expect(state.split()[1] in {"S", "R"}, f"the runner's {state}")
	clean("stopping the runner")
	print("ok: a program can neither stop, kill nor read the runner; it is stopped for time and the runner serves on")

	queued = record(QUEUES)["rustc"]
	expect(queued["outcome"] == "prints" and queued["stdout"] == "queued\n", f"queueing a request: {queued}")
	expect(ask({"ask": "toolchain"})["answer"] == "toolchain", "the runner stopped answering")
	logged = compose("logs", "--no-color", CONTAINER).stdout.splitlines()
	tail = [line.split("rust-runner: ", 1)[-1] for line in logged[-3:]]
	expect(tail[0].startswith("recorded ok in ") and tail[1:] == ["refused peer", "toolchain"], f"the runner's log: {tail}")
	clean("queueing a request")
	print("ok: a request a program queues on the socket is refused, unread")

	shared = record(SHARED_MEMORY)["rustc"]
	expect(shared["outcome"] == "prints" and shared["stdout"] == "shm: true\n", f"shared memory: {shared}")
	expect(restart_count() == restarts, "the runner restarted for shared memory")
	clean("shared memory")
	print("ok: System V shared memory goes with the program that made it")

	semaphores = record(SEMAPHORES)["rustc"]
	expect(semaphores["outcome"] == "prints" and semaphores["stdout"] == "sem: true, msg: true\n", f"semaphores: {semaphores}")
	wait_ready()
	expect(restart_count() == restarts + 1, f"restart count {restarts} -> {restart_count()}")
	clean("semaphores and queues")
	print("ok: a System V semaphore or queue a program leaves restarts the runner into a fresh IPC namespace")


def check_compiles() -> None:
	zero = record(DEV_ZERO)["rustc"]
	expect(zero["outcome"] == "compile-error", f"include_bytes!(/dev/zero): {zero}")
	clean("include_bytes!(/dev/zero)")
	print("ok: include_bytes!(\"/dev/zero\") is a compile error, and leaves nothing")

	for name, source in (("a macro that expands without end", MACRO_BLOWUP), ("a 128 MiB static", HUGE_STATIC)):
		stopped = record(source)["rustc"]
		expect(stopped["outcome"] == "stopped" and stopped["why"] in {"memory", "time"}, f"{name}: {stopped}")
		clean(name)
		print(f"ok: {name} stops rustc for {stopped['why']}, and leaves nothing")

	flood = record(ERROR_FLOOD)["rustc"]
	expect(flood["outcome"] == "compile-error" and flood["code"] == "E0308" and len(flood["stderr"].encode()) <= 4096, f"an error flood: {flood['outcome']}, {len(flood['stderr'])}")
	clean("an error flood")
	print("ok: 1500 errors are a compile error kept to 4 KiB, and leave nothing")

	output = record(STDOUT_FLOOD)["rustc"]
	expect(output["outcome"] == "stopped" and output["why"] == "output" and output["stdout"] == "x" * 4096, f"a stdout flood: {output['outcome']} {output.get('why')}")
	clean("a stdout flood")
	print("ok: 64 MiB of stdout is stopped for output, kept to 4 KiB, and leaves nothing")


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
		check_ids()
		check_sandbox()
		check_worked_example()
		check_hostile()
		check_compiles()
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

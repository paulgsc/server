//! envctl's own binary against stand-in `docker` and `sops` scripts on
//! `PATH`: no daemon and no key exist here, and the real run is the
//! owner's, on the box. The stand-ins play their parts closely enough to
//! hold the promises that matter (#614, #615): no plain value is written to
//! a disk or printed, a container's file is on tmpfs only while compose
//! runs, and the shell wins over `make dev`'s file.

// A test that writes scripts and compares text: the formatting macros are
// the plain way to build both, and nothing here is a hot path.
#![allow(clippy::disallowed_macros)]

use std::{
	fs,
	os::unix::fs::PermissionsExt as _,
	path::{Path, PathBuf},
	process::{Command, Output},
};

/// Values a deployment holds; none may appear on disk in the clear or in
/// anything envctl prints.
const SECRETS: [&str; 4] = ["s3cret-rp-id.example", "hunter2-vapid-key", "/mnt/secret/storage", "grafana-pw-123"];

const SCHEMA: &str = r#"{
  "binary": "probe",
  "about": "A probe.",
  "variables": [
    {"name": "RP_ID", "group": null, "required": true, "default": null, "kind": "string", "values": [], "delimiter": null, "secret": false, "help": "The relying party id.", "required_when": null, "bounds": null},
    {"name": "KEY", "group": null, "required": true, "default": null, "kind": "string", "values": [], "delimiter": null, "secret": true, "help": "A key.", "required_when": null, "bounds": null},
    {"name": "ON", "group": null, "required": false, "default": "false", "kind": "bool", "values": [], "delimiter": null, "secret": false, "help": "A switch.", "required_when": null, "bounds": null},
    {"name": "WIRE", "group": null, "required": false, "default": null, "kind": "string", "values": [], "delimiter": null, "secret": false, "help": "Compose's own.", "required_when": null, "bounds": null}
  ]
}"#;

/// What `docker compose config --no-interpolate` prints for the checkout.
const CONFIG: &str = r#"{"name": "repo", "services": {
  "probe": {
    "labels": {"env-schema": "probe"},
    "env_file": [{"path": "/repo/${ENV_DIR:?make sets ENV_DIR}/probe.env", "required": true}],
    "environment": ["WIRE=w://wire", "GF_PASSWORD=${GRAFANA_PASSWORD:?set it}"],
    "volumes": [{"source": "${STORAGE:?set STORAGE}", "target": "/app/dbs", "type": "bind"}]
  }
}}"#;

/// What `docker inspect` says of the running container.
const INSPECT: &str = r#"[{"Config": {
  "Labels": {"env-schema": "probe", "com.docker.compose.service": "probe"},
  "Env": ["RP_ID=s3cret-rp-id.example", "KEY=hunter2-vapid-key", "ON=true", "WIRE=w://wire", "GF_PASSWORD=grafana-pw-123", "PATH=/usr/bin", "IMAGE_ONLY=1"]
}, "Mounts": [{"Destination": "/app/dbs", "Source": "/mnt/secret/storage"}]}]"#;

/// A `docker` that answers `ps`, `inspect` and `compose`. On `compose ...
/// up`, it checks the container's file is there, private, and holds what
/// sops gave, and records only whether it was.
const DOCKER: &str = r#"#!/bin/sh
here="$(dirname "$0")"
case "$1" in
  ps) echo probe-1 ;;
  inspect) cat "$here/inspect.json" ;;
  compose)
    shift
    [ "$1" = "--env-file" ] && [ "$2" = "/dev/null" ] || { echo "compose without --env-file /dev/null" >&2; exit 9; }
    shift 2
    if [ "$1" = config ]; then cat "$here/config.json"; exit 0; fi
    if [ "$1" = logs ]; then
      echo "compose logs: started" >> "$here/log"
      trap 'echo "compose logs: TERM" >> "$here/log"; exit 143' TERM
      trap '' INT
      i=0; while [ $i -lt 200 ]; do sleep 0.1; i=$((i+1)); done; exit 1
    fi
    file="$ENV_DIR/probe.env"
    mode=$(stat -c %a "$file" 2>/dev/null)
    if [ "$mode" = 600 ] && grep -qx "RP_ID='s3cret-rp-id.example'" "$file" && [ "$STORAGE" = /mnt/secret/storage ] && [ -z "$STRAY" ]; then
      echo "compose $*: file ok" >> "$here/log"
    else
      echo "compose $*: file wrong (mode $mode)" >> "$here/log"; exit 3
    fi ;;
  *) echo "stand-in docker: $*" >&2; exit 2 ;;
esac
"#;

/// A `sops` whose "encryption" is base64 in `ENC[...]`, with sops' own
/// metadata lines: enough for envctl, which only ever pipes to and from it.
const SOPS: &str = r##"#!/bin/sh
here="$(dirname "$0")"
echo "sops $*" >> "$here/log"
case "$1" in
  encrypt)
    while IFS= read -r line; do
      case "$line" in "#"*) printf '#ENC[%s]\n' "$(printf '%s' "$line" | base64 -w0)"; continue ;; esac
      name="${line%%=*}"; value="${line#*=}"
      if [ -z "$value" ]; then printf '%s=\n' "$name"; continue; fi
      printf '%s=ENC[%s]\n' "$name" "$(printf '%s' "$value" | base64 -w0)"
    done
    echo "sops_mac=ENC[stand-in]"; echo "sops_version=stand-in" ;;
  decrypt)
    for last; do :; done
    while IFS= read -r line; do
      case "$line" in "#"*) continue ;; esac
      name="${line%%=*}"; value="${line#*=}"
      case "$name" in sops_*) continue ;; esac
      if [ -z "$value" ]; then printf '%s=\n' "$name"; continue; fi
      inner="${value#ENC[}"; inner="${inner%]}"
      printf '%s=%s\n' "$name" "$(printf '%s' "$inner" | base64 -d)"
    done < "$last" ;;
  *) exit 2 ;;
esac
"##;

struct Fixture {
	root: PathBuf,
	repo: PathBuf,
	bin: PathBuf,
}

impl Fixture {
	fn new(name: &str) -> std::io::Result<Self> {
		let root = std::env::temp_dir().join(format!("envctl-stand-ins-{name}-{}", std::process::id()));
		let _ = fs::remove_dir_all(&root);
		let repo = root.join("repo");
		let bin = root.join("bin");
		fs::create_dir_all(repo.join("infra/env"))?;
		fs::create_dir_all(&bin)?;
		fs::write(repo.join("infra/env/probe.json"), SCHEMA)?;
		fs::write(
			repo.join("docker-compose.yml"),
			"services:\n  probe:\n    env_file:\n      - path: ${ENV_DIR:?make sets ENV_DIR}/probe.env\n    environment:\n      - GF_PASSWORD=${GRAFANA_PASSWORD:?set it}\n      - FROM_SHELL=${STRAY:-none}\n    volumes:\n      - ${STORAGE:?set STORAGE}:/app/dbs\n",
		)?;
		fs::write(bin.join("config.json"), CONFIG)?;
		fs::write(bin.join("inspect.json"), INSPECT)?;
		for (script, text) in [("docker", DOCKER), ("sops", SOPS)] {
			let path = bin.join(script);
			fs::write(&path, text)?;
			fs::set_permissions(&path, fs::Permissions::from_mode(0o755))?;
		}
		Ok(Self { root, repo, bin })
	}

	fn envctl(&self, args: &[&str]) -> Command {
		let mut command = Command::new(env!("CARGO_BIN_EXE_envctl"));
		command
			.arg("--repo")
			.arg(&self.repo)
			.args(args)
			.env_clear()
			.env("PATH", format!("{}:/usr/bin:/bin", self.bin.display()));
		command
	}

	/// Every file under the checkout, as text.
	fn on_disk(&self) -> std::io::Result<Vec<(PathBuf, String)>> {
		fn walk(dir: &Path, out: &mut Vec<(PathBuf, String)>) -> std::io::Result<()> {
			for entry in fs::read_dir(dir)? {
				let path = entry?.path();
				if path.is_dir() {
					walk(&path, out)?;
				} else {
					let text = String::from_utf8_lossy(&fs::read(&path)?).into_owned();
					out.push((path, text));
				}
			}
			Ok(())
		}
		let mut out = Vec::new();
		walk(&self.repo, &mut out)?;
		Ok(out)
	}

	fn log(&self) -> String {
		fs::read_to_string(self.bin.join("log")).unwrap_or_default()
	}
}

impl Drop for Fixture {
	fn drop(&mut self) {
		let _ = fs::remove_dir_all(&self.root);
	}
}

fn printed(output: &Output) -> String {
	format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr))
}

fn no_secret_in(text: &str, what: &str) {
	for secret in SECRETS {
		assert!(!text.contains(secret), "{what} holds {secret}:\n{text}");
	}
}

/// A tmpfs to decrypt into here, if this machine has one.
fn tmpfs() -> Option<PathBuf> {
	let mounts = fs::read_to_string("/proc/self/mounts").ok()?;
	mounts
		.lines()
		.any(|line| line.split_whitespace().nth(1) == Some("/dev/shm") && line.contains(" tmpfs "))
		.then(|| PathBuf::from("/dev/shm"))
}

fn imported(fixture: &Fixture) -> std::io::Result<()> {
	let output = fixture.envctl(&["import", "--role", "home"]).output()?;
	let shown = printed(&output);
	assert!(output.status.success(), "{shown}");
	no_secret_in(&shown, "import's output");
	for (path, text) in fixture.on_disk()? {
		no_secret_in(&text, &path.display().to_string());
	}
	assert!(
		shown.contains("deploy/home/probe.sops.env: 3 new") && shown.contains("+ RP_ID") && shown.contains("+ KEY") && shown.contains("+ ON"),
		"{shown}"
	);
	assert!(
		!shown.contains("WIRE") && !shown.contains("IMAGE_ONLY"),
		"compose's own wiring and the image's own are left out: {shown}"
	);
	assert!(
		shown.contains("deploy/home/compose.sops.env: 2 new") && shown.contains("+ GRAFANA_PASSWORD") && shown.contains("+ STORAGE"),
		"{shown}"
	);
	for step in ["docker ps", "docker compose config", "reading probe-1"] {
		assert!(shown.contains(step), "import names each step on stderr ({step}): {shown}");
	}
	assert!(fixture
		.log()
		.contains("sops encrypt --input-type dotenv --output-type dotenv --filename-override deploy/home/probe.sops.env"));
	Ok(())
}

#[test]
fn import_writes_through_sops_only_and_prints_names_only() {
	let fixture = Fixture::new("import").unwrap();
	fs::write(fixture.repo.join(".env"), "STORAGE=/old\n").unwrap();
	imported(&fixture).unwrap();
	let again = fixture.envctl(&["import", "--role", "home", "--dry-run"]).output().unwrap();
	let shown = printed(&again);
	assert!(shown.contains("probe.sops.env: already what runs") && shown.contains("Dry run"), "{shown}");
	assert!(shown.contains("delete the box's plain-text .env"), "{shown}");
}

#[test]
fn up_decrypts_to_tmpfs_for_compose_alone_then_removes_it() {
	let Some(shm) = tmpfs() else {
		return;
	};
	let fixture = Fixture::new("up").unwrap();
	imported(&fixture).unwrap();
	let env_dir = shm.join(format!("envctl-up-{}", std::process::id()));
	let preflight = fixture.envctl(&["preflight", "--role", "home"]).output().unwrap();
	assert!(preflight.status.success(), "{}", printed(&preflight));
	no_secret_in(&printed(&preflight), "the preflight");
	let up = fixture
		.envctl(&["up", "--role", "home", "--env-dir", env_dir.to_str().unwrap()])
		.env("STRAY", "from the shell")
		.output()
		.unwrap();
	let shown = printed(&up);
	assert!(up.status.success(), "{shown}\n{}", fixture.log());
	no_secret_in(&shown, "up's output");
	assert!(fixture.log().contains("compose up -d: file ok"), "{}", fixture.log());
	assert!(!env_dir.join("probe.env").exists(), "the decrypted file outlived compose");
	for (path, text) in fixture.on_disk().unwrap() {
		no_secret_in(&text, &path.display().to_string());
	}
	let _ = fs::remove_dir_all(&env_dir);
	let mounts = fs::read_to_string("/proc/self/mounts").unwrap();
	if !mounts.lines().any(|line| line.split_whitespace().nth(1) == Some("/tmp")) {
		let disk = fixture.envctl(&["up", "--role", "home", "--env-dir", fixture.root.to_str().unwrap()]).output().unwrap();
		assert!(
			!disk.status.success() && printed(&disk).contains("not tmpfs"),
			"a directory on a disk is refused: {}",
			printed(&disk)
		);
	}
}

#[test]
fn preflight_names_what_is_missing_and_never_a_value() {
	let fixture = Fixture::new("preflight").unwrap();
	imported(&fixture).unwrap();
	// Drop RP_ID from the "encrypted" file, as a role that never set it.
	let path = fixture.repo.join("deploy/home/probe.sops.env");
	let kept: Vec<String> = fs::read_to_string(&path)
		.unwrap()
		.lines()
		.filter(|line| !line.starts_with("RP_ID="))
		.map(str::to_owned)
		.collect();
	fs::write(&path, kept.join("\n") + "\n").unwrap();
	let output = fixture.envctl(&["preflight", "--role", "home"]).output().unwrap();
	let shown = printed(&output);
	assert!(!output.status.success(), "{shown}");
	assert!(shown.contains("RP_ID  [probe (service probe)]"), "{shown}");
	no_secret_in(&shown, "the preflight");
	let decrypted = fixture.log().matches("sops decrypt").count();
	let list = fixture.envctl(&["list", "--role", "home"]).output().unwrap();
	let listing = printed(&list);
	assert!(
		listing.contains("UNSET RP_ID") && listing.contains("set   KEY") && listing.contains("set   STORAGE"),
		"{listing}"
	);
	assert_eq!(fixture.log().matches("sops decrypt").count(), decrypted, "list reads names, and decrypts nothing");
}

#[test]
fn run_gives_the_file_to_the_command_and_the_shell_wins() {
	let fixture = Fixture::new("run").unwrap();
	imported(&fixture).unwrap();
	let run = |extra: &[(&str, &str)], check: &str| {
		let mut command = fixture.envctl(&["run", "--role", "home", "--app", "probe", "--", "sh", "-c", check]);
		command.envs(extra.iter().copied());
		command.output().unwrap()
	};
	let given = run(&[], r#"[ "$RP_ID" = s3cret-rp-id.example ] && [ "$ON" = true ]"#);
	assert!(given.status.success(), "{}", printed(&given));
	let shell_wins = run(&[("ON", "false")], r#"[ "$ON" = false ] && [ "$KEY" = hunter2-vapid-key ]"#);
	assert!(shell_wins.status.success(), "{}", printed(&shell_wins));
	let missing = fixture.envctl(&["run", "--role", "dev", "--app", "probe", "--", "true"]).output().unwrap();
	assert!(printed(&missing).contains("deploy/dev/probe.sops.env does not exist"), "{}", printed(&missing));
}

#[test]
fn check_holds_deploy_to_encrypted_files_without_a_key() {
	let fixture = Fixture::new("check").unwrap();
	imported(&fixture).unwrap();
	let clean = fixture.envctl(&["check"]).output().unwrap();
	assert!(clean.status.success(), "{}", printed(&clean));
	fs::write(fixture.repo.join("deploy/home/plain.sops.env"), "A=1\nsops_mac=ENC[x]\n").unwrap();
	fs::write(fixture.repo.join("deploy/home/file_host.env"), "A=1\n").unwrap();
	let dirty = fixture.envctl(&["check"]).output().unwrap();
	let shown = printed(&dirty);
	assert!(!dirty.status.success(), "{shown}");
	assert!(shown.contains("deploy/home/plain.sops.env is not encrypted by sops: A in the clear"), "{shown}");
	assert!(shown.contains("deploy/home/file_host.env is not a <app>.sops.env file"), "{shown}");
	assert!(!fixture.log().contains("decrypt"), "check holds no key, so it decrypts nothing");
}

#[test]
fn compose_outlives_ctrl_c_passes_sigterm_on_and_removes_the_files() {
	let Some(shm) = tmpfs() else {
		return;
	};
	let fixture = Fixture::new("signals").unwrap();
	imported(&fixture).unwrap();
	let env_dir = shm.join(format!("envctl-signals-{}", std::process::id()));
	let mut child = fixture
		.envctl(&["compose", "--role", "home", "--env-dir", env_dir.to_str().unwrap(), "--", "logs", "-f"])
		.stdout(std::process::Stdio::null())
		.stderr(std::process::Stdio::null())
		.spawn()
		.unwrap();
	let file = env_dir.join("probe.env");
	let started = std::time::Instant::now();
	while !(file.exists() && fixture.log().contains("compose logs: started")) {
		assert!(started.elapsed().as_secs() < 20, "compose never started: {}", fixture.log());
		std::thread::sleep(std::time::Duration::from_millis(50));
	}
	let pid = child.id().to_string();
	let signal = |name: &str| Command::new("kill").args([name, pid.as_str()]).status().unwrap();
	assert!(signal("-INT").success());
	std::thread::sleep(std::time::Duration::from_millis(300));
	assert!(child.try_wait().unwrap().is_none() && file.exists(), "Ctrl-C ended envctl before compose");
	assert!(signal("-TERM").success());
	let status = child.wait().unwrap();
	assert_eq!(status.code(), Some(143), "compose's own exit, after its SIGTERM");
	assert!(fixture.log().contains("compose logs: TERM"), "{}", fixture.log());
	assert!(!file.exists(), "the decrypted file outlived compose");
	let _ = fs::remove_dir_all(&env_dir);
}

#[test]
fn compose_refuses_a_missing_file_and_a_link_off_tmpfs() {
	let Some(shm) = tmpfs() else {
		return;
	};
	let fixture = Fixture::new("refusals").unwrap();
	imported(&fixture).unwrap();
	let mounts = fs::read_to_string("/proc/self/mounts").unwrap();
	if !mounts.lines().any(|line| line.split_whitespace().nth(1) == Some("/tmp")) {
		let base = shm.join(format!("envctl-link-{}", std::process::id()));
		fs::create_dir_all(&base).unwrap();
		let link = base.join("link");
		let _ = fs::remove_file(&link);
		std::os::unix::fs::symlink(&fixture.root, &link).unwrap();
		let through = base.join("new/../link/d");
		let output = fixture
			.envctl(&["compose", "--role", "home", "--env-dir", through.to_str().unwrap(), "--", "ps"])
			.output()
			.unwrap();
		assert!(printed(&output).contains("not tmpfs"), "a link from tmpfs to a disk is followed: {}", printed(&output));
		assert!(!fixture.root.join("d/probe.env").exists());
		let _ = fs::remove_dir_all(&base);
	}
	let relative = fixture.envctl(&["compose", "--role", "home", "--env-dir", "relative/dir", "--", "ps"]).output().unwrap();
	assert!(printed(&relative).contains("not an absolute path"), "{}", printed(&relative));
	fs::remove_file(fixture.repo.join("deploy/home/probe.sops.env")).unwrap();
	let env_dir = shm.join(format!("envctl-missing-{}", std::process::id()));
	let missing = fixture
		.envctl(&["compose", "--role", "home", "--env-dir", env_dir.to_str().unwrap(), "--", "up", "-d"])
		.output()
		.unwrap();
	assert!(
		!missing.status.success() && printed(&missing).contains("deploy/home/probe.sops.env does not exist"),
		"{}",
		printed(&missing)
	);
	assert!(!fixture.log().contains("compose up"), "compose never ran");
	let _ = fs::remove_dir_all(&env_dir);
}

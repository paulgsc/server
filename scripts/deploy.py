#!/usr/bin/env python3
"""DEAD CODE: nothing runs this. Home's deploy is `envctl deploy`
(apps/envctl/src/deploy.rs), since home has no python3 (server#PRNUM). This
file stays only because some-ui's assurance ledger still cites it, and a
`server:` path the ledger cites may not be removed; it is deleted once the
ledger cites the Rust file instead. Change apps/envctl/src/deploy.rs, never
this.

Home's deploy of file_host, with the previous image put back when the new
one does not come up ready (AS18, #564).

`make home-deploy` runs one pass, in the checkout (README.md, "Deploying to
home"). In order:

1. **Records what serves now**: the image of the running `file-host`
   container, by image id.
2. **Pulls** `pgathondu/server:latest` (`--no-pull` uses the image already
   here). Only that image: `rust-runner` and `orchestrator` are pulled by
   hand, and nothing third-party is pulled at all.
3. **Skips an image that already failed here**: a pulled image whose id is
   in the state file's failed list is tagged back to the serving one, so a
   later pass, a reboot or a `make up` does not put a known-bad image up;
   the next publish (a new id) is tried.
4. **Stops here when nothing changed**: the image is the serving one.
5. **Keeps the old image**: tags the serving one `pgathondu/server:previous`,
   so a prune cannot take it before a rollback.
6. **Brings the stack up**: `make up`, which is envctl's preflight on
   `deploy/<role>/`, then `docker compose up -d`. `file_host` applies its
   migrations as it starts, after a verified backup (AS16, #563,
   `apps/servers/file_host/src/migrate.rs`), and exits when it cannot.
7. **Waits up to `--ready-timeout` seconds** for `file-host`'s healthcheck to
   say `healthy` (`file_host --health-check`: `/ready` ready or degraded).
   A container that exits, restarts or turns `unhealthy` fails at once.
8. **Rolls back** when it does not come up and its image changed: tags the
   recorded image back as `pgathondu/server:latest`, recreates `file-host`
   alone from it (`up -d --no-deps --pull never file-host`), waits the same
   bound, and records the failed image so step 3 skips it.

It never touches the database. A migration that ran stays applied: a
database ahead of its binary is one the old binary serves (`schema.rs`), and
the owner's answer (2026-11-10) is that a restore is never automatic.
README.md, "Deploying to home", is the manual restore. Nor does it update
the checkout: an image that needs a compose or `deploy/<role>/` change the
checkout lacks fails its preflight or its start, and is put back, with that
named as the likely cause.

Exit status:

  0  deployed, or nothing to deploy
  1  not deployed: the pull, the preflight or `up` failed; the old
     container still serves
  2  rolled back: the new image did not come up; the previous one serves
  3  nothing ready: the owner has to look (the message says what is known)

The decisions (`judge`, `wait_ready`, `plan_after_failure` and `deploy`
over a `Host` it is handed) are pure, so `--self-test` runs them against a
simulated Docker and clock (`Lint`'s `Deploy script` job). Stdlib only.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
import time
import unittest
from collections.abc import Callable
from dataclasses import dataclass, field
from pathlib import Path
from typing import Protocol

REPO_ROOT = Path(__file__).resolve().parent.parent

# file-host's compose service, its container_name and its image.
# `Tests.test_the_service_is_the_compose_files` holds these to
# infra/compose/file_host.yml.
FILE_HOST = "file-host"
CONTAINER = "file-host-server"
IMAGE = "pgathondu/server:latest"

# The bound on becoming ready, in seconds. file_host copies the database
# and applies its migrations before it listens, so the healthcheck's
# start_period (infra/compose/file_host.yml, 300 s) covers that work: no
# failed probe counts during it, and the first that passes reads healthy.
# The bound is above start_period, so a slow migrating start is never taken
# for a bad image (which would be rolled back and then skipped). A start
# that exits or restarts fails at once, whatever the bound, so only an image
# that hangs costs all of it. `test_the_bound_outlasts_the_start_period`
# holds the two together.
DEFAULT_READY_TIMEOUT = 360.0
POLL_SECONDS = 2.0

EXIT_DEPLOYED = 0
EXIT_NOT_DEPLOYED = 1
EXIT_ROLLED_BACK = 2
EXIT_NOTHING_READY = 3

# What file_host prints at start when it took a backup
# (apps/servers/file_host/src/main.rs, `migrate_on_start`).
BACKUP_LINE = "migrate: backed up to "

# How many failed image ids the state file remembers; older ones fall off.
FAILED_KEPT = 20


# ── The decisions, pure ─────────────────────────────────────────────────────


@dataclass(frozen=True)
class Reading:
    """One look at a container, as `docker inspect` gives it."""

    container: str | None
    status: str | None = None  # created, running, restarting, exited, dead, ...
    health: str | None = None  # starting, healthy, unhealthy; None without a healthcheck
    restarts: int = 0
    image: str | None = None


@dataclass(frozen=True)
class Verdict:
    kind: str  # "ready", "wait", "failed", "unjudgeable"
    reason: str = ""


READY = Verdict("ready")
WAIT = Verdict("wait")


def judge(reading: Reading, baseline_restarts: int) -> Verdict:
    """What one reading says about whether file-host came up.

    Only `healthy` is ready. A container that is gone, has exited, is being
    restarted or has restarted since the wait began has failed now: a server
    that exits at start (a migration it refused, a config it refuses) will
    not become ready by waiting. `unhealthy` is Docker giving up. No
    healthcheck at all is not the image's fault, so it is unjudgeable.
    """
    if reading.container is None:
        return Verdict("failed", f"no {FILE_HOST} container")
    if reading.status in ("exited", "dead", "restarting", "removing", "paused"):
        return Verdict("failed", f"the container is {reading.status}")
    if reading.restarts > baseline_restarts:
        return Verdict("failed", f"the container restarted {reading.restarts - baseline_restarts} time(s)")
    if reading.status != "running":
        return WAIT
    if reading.health is None:
        return Verdict("unjudgeable", "the container has no healthcheck, so its readiness cannot be read")
    if reading.health == "healthy":
        return READY
    if reading.health == "unhealthy":
        return Verdict("failed", "its healthcheck says unhealthy")
    return WAIT


def wait_ready(read: Callable[[], Reading], now: Callable[[], float], sleep: Callable[[float], None], timeout: float, poll: float = POLL_SECONDS) -> tuple[Verdict, float]:
    """Polls until a reading decides, or `timeout` seconds pass. Returns the
    verdict and the seconds it took."""
    start = now()
    reading = read()
    baseline = reading.restarts
    while True:
        verdict = judge(reading, baseline)
        elapsed = now() - start
        if verdict.kind != "wait":
            return verdict, elapsed
        if elapsed >= timeout:
            state = reading.health or reading.status or "unknown"
            return Verdict("failed", f"not ready within {timeout:.0f} s (last seen: {state})"), elapsed
        sleep(min(poll, timeout - elapsed))
        reading = read()


@dataclass(frozen=True)
class Plan:
    rollback: bool
    reason: str = ""


def plan_after_failure(old_image: str | None, new_image: str | None) -> Plan:
    """Whether putting the previous image back can help.

    Not when nothing served before (a first deploy), and not when the image
    did not change: then the cause is the configuration or a dependency,
    which the same image would meet again.
    """
    if old_image is None:
        return Plan(False, f"no {FILE_HOST} container served before this deploy, so there is no previous image")
    if new_image == old_image:
        return Plan(False, "the image did not change, so the cause is not the image: look at deploy/<role>/, the config it refuses, or a dependency")
    return Plan(True)


def previous_ref(image_ref: str) -> str:
    """`pgathondu/server:latest` → `pgathondu/server:previous`; a registry
    port (`host:5000/x`) is not a tag."""
    name = image_ref.split("@", 1)[0]
    slash = name.rfind("/")
    colon = name.rfind(":")
    if colon > slash:
        name = name[:colon]
    return f"{name}:previous"


def backup_from(lines: list[str]) -> str | None:
    """The backup file_host took at this start, from its log, if it took one."""
    found = None
    for line in lines:
        at = line.find(BACKUP_LINE)
        if at >= 0:
            found = line[at + len(BACKUP_LINE) :].removesuffix(" (verified)").strip()
    return found


DATABASE_NOTE = (
    "The database stays where the migrations put it: the old image serves a database ahead of it "
    "(apps/servers/file_host/src/schema.rs). Nothing was restored or deleted."
)


def stuck_note(backup: str | None) -> str:
    restore = (
        f"This start backed the database up to {backup} (on the host, under ${{STORAGE}}/backups). "
        if backup
        else "This start took no backup that its log names. "
    )
    return (
        restore + "It is never restored automatically: every write since it would leave the live database. "
        "If restoring is the way out, move the live file aside first, never delete it "
        '(README.md, "Deploying to home").'
    )


# ── The deploy, over a host it is handed ────────────────────────────────────


class Host(Protocol):
    def reading(self, container: str) -> Reading: ...
    def image_id(self, ref: str) -> str | None: ...
    def describe(self, image_id: str) -> str: ...
    def tag(self, image_id: str, ref: str) -> bool: ...
    def pull(self, ref: str) -> bool: ...
    def up(self) -> bool: ...
    def recreate_file_host(self) -> bool: ...
    def logs(self, container: str) -> list[str]: ...
    def failed_images(self) -> list[str]: ...
    def remember_failed(self, image_id: str) -> None: ...


@dataclass
class Options:
    pull: bool = True
    # The image compose starts file-host from (infra/compose/file_host.yml);
    # another only for a rehearsal against a local registry.
    image: str = IMAGE
    ready_timeout: float = DEFAULT_READY_TIMEOUT
    poll: float = POLL_SECONDS


# Said with every failure a new image could owe to the checkout rather than
# to itself: this pass does not update the checkout (the owner's call,
# 2026-11-10), so a new variable or compose change the image needs is not
# here yet.
CHECKOUT_HINT = (
    "If the preflight or the container's last log lines name a missing or refused variable, "
    "this image needs a newer checkout on home: `git pull` there, then `make home-deploy` again."
)


def deploy(host: Host, options: Options, now: Callable[[], float], sleep: Callable[[float], None], say: Callable[[str], None]) -> int:
    before = host.reading(CONTAINER)
    old = before.image if before.container else None
    say(f"deploy: {FILE_HOST} serves {host.describe(old)}" if old else f"deploy: no {FILE_HOST} container serves now")

    if options.pull and not host.pull(options.image):
        say(f"deploy: pulling {options.image} failed; nothing was replaced")
        return EXIT_NOT_DEPLOYED

    candidate = host.image_id(options.image)
    if candidate is not None and candidate != old and old is not None and candidate in set(host.failed_images()):
        # Back to what serves, so a reboot or a `make up` by hand does not
        # start the image that failed either.
        host.tag(old, options.image)
        say(f"deploy: {options.image} is {candidate}, which failed here before; staying on {old} until a newer one is published")
        candidate = old
    if candidate is not None and candidate == old:
        # Nothing to deploy only when what has that image is ready: a
        # container that exited, is restarting or reads unhealthy is not
        # serving, and `make up` below is the way to bring it back (with no
        # previous image to roll back to, since it is the same one).
        state = judge(before, before.restarts)
        if state.kind == "ready":
            say("deploy: the image is the one serving; nothing restarted")
            return EXIT_DEPLOYED
        say(f"deploy: the image is the container's, but it is not ready ({state.reason or before.health or before.status}); bringing it up")

    if old and candidate and candidate != old:
        kept = previous_ref(options.image)
        if host.tag(old, kept):
            say(f"deploy: kept the serving {FILE_HOST} image as {kept}")
        else:
            say(f"deploy: could not tag the serving image as {kept}; it stays as {old}")

    up_ok = host.up()
    after = host.reading(CONTAINER)
    new = after.image
    if not up_ok:
        # The preflight refused (it names each gap above), or compose did.
        if old is not None and new == old:
            say(f"deploy: make up failed (its reason is above); {FILE_HOST} was not replaced and still serves")
            say(f"deploy: {CHECKOUT_HINT}")
            return EXIT_NOT_DEPLOYED
        say("deploy: make up failed (its reason is above); judging what it left")
    say(f"deploy: started {host.describe(new)}; waiting up to {options.ready_timeout:.0f} s for it to be ready" if new else f"deploy: no {FILE_HOST} container after up")

    def read() -> Reading:
        return host.reading(CONTAINER)

    verdict, took = wait_ready(read, now, sleep, options.ready_timeout, options.poll)
    if verdict.kind == "ready":
        say(f"deploy: ready after {took:.0f} s")
        return EXIT_DEPLOYED
    say(f"deploy: the new {FILE_HOST} did not come up: {verdict.reason}")
    lines = host.logs(CONTAINER)
    backup = backup_from(lines)
    for line in lines[-5:]:
        say(f"deploy: its log: {line}")
    if verdict.kind == "unjudgeable":
        say(DATABASE_NOTE)
        return EXIT_NOTHING_READY

    plan = plan_after_failure(old, new)
    if not plan.rollback:
        say(f"deploy: not rolling back: {plan.reason}")
        say(DATABASE_NOTE)
        say(stuck_note(backup))
        return EXIT_NOTHING_READY

    assert old is not None
    # The image that failed: the container's, or, when `up` left no
    # container at all, the one it was asked to start. Recorded only when
    # known, and never the serving one.
    failed_image = new or candidate
    if failed_image is not None and failed_image != old:
        host.remember_failed(failed_image)
    say(f"deploy: ROLLING BACK to {host.describe(old)}")
    if not host.tag(old, options.image) or not host.recreate_file_host():
        say(f"deploy: the rollback could not restart {old}")
        say(DATABASE_NOTE)
        say(stuck_note(backup))
        return EXIT_NOTHING_READY
    verdict, took_back = wait_ready(read, now, sleep, options.ready_timeout, options.poll)
    if verdict.kind == "ready":
        say(f"deploy: ROLLED BACK: the previous image is ready after {took_back:.0f} s. The image that failed is {failed_image or 'unknown'}, and is skipped until a newer one is published.")
        say(f"deploy: {CHECKOUT_HINT}")
        say(DATABASE_NOTE)
        return EXIT_ROLLED_BACK
    say(f"deploy: the previous image did not come up either: {verdict.reason}")
    say(DATABASE_NOTE)
    say(stuck_note(backup))
    return EXIT_NOTHING_READY


# ── On demand: a client needed a newer server ─────────────────────────────
#
# file_host writes ${STORAGE}/deploy/wanted when a client asks `/compat` for a
# contract epoch above its own (apps/servers/file_host/src/deploy.rs, which
# holds the bounds: one marker at a time, a gap between pulls, a ceiling, a
# need no published image met waits six hours). A systemd path unit
# (nix/modules/home-deploy.nix) runs `deploy.py --on-demand` when it appears.
# This pass pulls, reads the pulled image's epoch, deploys it only when that
# is above the running one, records what it did in state.json beside the
# marker (which file_host reads for its answers and metrics), and removes the
# marker. Nothing else ever pulls: with no client newer than home, home sends
# no request anywhere.

# The shortest time between two pulls: deploy.rs's PULL_GAP, checked here
# again so a marker written by hand cannot make home pull twice in a row.
PULL_GAP_SECONDS = 600


class Demand(Protocol):
    def read_marker(self) -> dict[str, object] | None: ...
    def clear_marker(self) -> None: ...
    def read_state(self) -> dict[str, object]: ...
    def write_state(self, state: dict[str, object]) -> None: ...
    def image_epoch(self, image_id: str) -> int | None: ...
    def image_bytes(self, image_id: str) -> int: ...


# What each exit of `deploy` is counted as in state.json's `counts`.
OUTCOME_OF_EXIT = {EXIT_DEPLOYED: "deployed", EXIT_ROLLED_BACK: "rolled_back", EXIT_NOT_DEPLOYED: "failed", EXIT_NOTHING_READY: "failed"}


def as_int(value: object, default: int = 0) -> int:
    return value if isinstance(value, int) and not isinstance(value, bool) else default


def on_demand(host: Host, demand: Demand, options: Options, wall: Callable[[], float], now: Callable[[], float], sleep: Callable[[float], None], say: Callable[[str], None]) -> int:
    marker = demand.read_marker()
    if marker is None:
        say("deploy: no deploy is wanted; nothing to do")
        return EXIT_DEPLOYED
    needs = as_int(marker.get("needs"))
    running = as_int(marker.get("running"), -1)
    if running < 0:
        # A marker without what home ran (unreadable, or written by hand):
        # ask the serving image itself, never assume 0, which would make any
        # pulled image look newer.
        serving_now = host.reading(CONTAINER)
        running = (demand.image_epoch(serving_now.image) if serving_now.container and serving_now.image else None) or -1
    if running < 0:
        say("deploy: the marker does not say which epoch home runs, and the serving image cannot either; not deploying")
        demand.clear_marker()
        return EXIT_NOT_DEPLOYED
    say(f"deploy: a client needs contract epoch {needs}; home runs {running}")

    state = demand.read_state()
    raw_counts = state.get("counts")
    counts: dict[str, int] = {key: as_int(value) for key, value in raw_counts.items()} if isinstance(raw_counts, dict) else {}

    def finish(code: int, outcome: str, **fields: object) -> int:
        counts[outcome] = counts.get(outcome, 0) + 1
        state.update(fields)
        state["counts"] = counts
        state["last_run"] = {"at": int(wall()), "exit": code, "outcome": outcome, "needs": needs}
        demand.write_state(state)
        demand.clear_marker()
        return code

    last_pull = state.get("last_pull_at")
    if isinstance(last_pull, int) and wall() - last_pull < PULL_GAP_SECONDS:
        say(f"deploy: the last pull was {int(wall() - last_pull)} s ago, under the {PULL_GAP_SECONDS} s gap; not pulling")
        return finish(EXIT_DEPLOYED, "deferred")

    before = host.reading(CONTAINER)
    serving = before.image if before.container else None
    previous_id = host.image_id(options.image)
    pulled = host.pull(options.image)
    counts["pulls"] = counts.get("pulls", 0) + 1
    state["last_pull_at"] = int(wall())
    if not pulled:
        say(f"deploy: pulling {options.image} failed; nothing was replaced")
        return finish(EXIT_NOT_DEPLOYED, "failed")
    candidate = host.image_id(options.image)
    if candidate is not None and candidate != previous_id:
        counts["pulled_bytes"] = counts.get("pulled_bytes", 0) + demand.image_bytes(candidate)
    epoch = demand.image_epoch(candidate) if candidate else None
    state["latest_seen_epoch"] = epoch
    if epoch is None or epoch <= running:
        # Lazily on the previous image (the owner, 2026-11-10): a newer build
        # of the same contract is not a reason to restart. :latest goes back
        # to what serves, so a reboot does not start it either.
        if serving is not None and candidate is not None and candidate != serving:
            host.tag(serving, options.image)
        seen = "an epoch it could not read" if epoch is None else f"epoch {epoch}"
        say(f"deploy: the newest image has {seen}, not above home's {running}; staying on the running one")
        if epoch is not None and epoch < needs:
            say(f"deploy: nothing published serves epoch {needs} yet; home answers `unavailable` to it for six hours")
        return finish(EXIT_DEPLOYED, "no_newer_image")

    say(f"deploy: the newest image has epoch {epoch}; deploying it")
    code = deploy(host, Options(pull=False, image=options.image, ready_timeout=options.ready_timeout, poll=options.poll), now, sleep, say)
    return finish(code, OUTCOME_OF_EXIT.get(code, "failed"))


# ── The real host ───────────────────────────────────────────────────────────


def default_state_dir() -> Path:
    """The failed images are remembered under the user's own state directory."""
    return Path(os.environ.get("XDG_STATE_HOME") or Path.home() / ".local" / "state") / "server-deploy"


class RealHost:
    def __init__(self, repo: Path, state_dir: Path, as_user: list[str] | None = None) -> None:
        self.repo = repo
        self.state = state_dir / "failed-images.json"
        # `make` (envctl, sops, cargo) runs as the checkout's owner when the
        # on-demand unit runs this as root: `runuser -u <user> -- env HOME=…`.
        self.as_user = as_user or []

    def _run(self, *args: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(list(args), cwd=self.repo, capture_output=True, text=True, check=False)

    def _stream(self, *args: str) -> int:
        """Runs a command with its output shown as it comes."""
        sys.stdout.flush()
        return subprocess.run(list(args), cwd=self.repo, check=False).returncode

    def reading(self, container: str) -> Reading:
        done = self._run("docker", "inspect", "--format", "{{json .}}", container)
        if done.returncode != 0:
            return Reading(None)
        info = json.loads(done.stdout)
        state = info.get("State") or {}
        health = (state.get("Health") or {}).get("Status")
        return Reading(container, state.get("Status"), health, int(info.get("RestartCount") or 0), info.get("Image"))

    def image_id(self, ref: str) -> str | None:
        done = self._run("docker", "image", "inspect", "--format", "{{.Id}}", ref)
        return (done.stdout.strip() or None) if done.returncode == 0 else None

    def describe(self, image_id: str) -> str:
        done = self._run("docker", "image", "inspect", "--format", '{{join .RepoDigests ", "}}', image_id)
        digests = done.stdout.strip() if done.returncode == 0 else ""
        return f"{image_id} ({digests})" if digests else image_id

    def tag(self, image_id: str, ref: str) -> bool:
        return self._run("docker", "tag", image_id, ref).returncode == 0

    def pull(self, ref: str) -> bool:
        return self._run("docker", "pull", "-q", ref).returncode == 0

    def up(self) -> bool:
        return self._stream(*self.as_user, "make", "-s", "up") == 0

    def recreate_file_host(self) -> bool:
        # --no-deps: nothing else is the image that failed. --pull never: the
        # tag just moved back to the old image; a pull would undo that.
        return self._stream(*self.as_user, "make", "-s", "compose", f"ARGS=up -d --no-deps --pull never {FILE_HOST}") == 0

    def logs(self, container: str) -> list[str]:
        done = self._run("docker", "logs", "--tail", "200", container)
        return (done.stdout + done.stderr).splitlines()

    def failed_images(self) -> list[str]:
        try:
            value = json.loads(self.state.read_text())
        except (OSError, ValueError):
            return []
        return [str(item) for item in value] if isinstance(value, list) else []

    def remember_failed(self, image_id: str) -> None:
        kept = [item for item in self.failed_images() if item != image_id][-(FAILED_KEPT - 1) :] + [image_id]
        try:
            self.state.parent.mkdir(parents=True, exist_ok=True)
            self.state.write_text(json.dumps(kept))
        except OSError as err:
            print(f"deploy: could not record {image_id} as failed in {self.state}: {err}", flush=True)


class RealDemand:
    """${STORAGE}/deploy on the host: the marker and state.json."""

    def __init__(self, folder: Path) -> None:
        self.folder = folder
        self.marker = folder / "wanted"
        self.state = folder / "state.json"

    def read_marker(self) -> dict[str, object] | None:
        try:
            value = json.loads(self.marker.read_text())
        except FileNotFoundError:
            return None
        except (OSError, ValueError):
            return {}
        return value if isinstance(value, dict) else {}

    def clear_marker(self) -> None:
        self.marker.unlink(missing_ok=True)

    def read_state(self) -> dict[str, object]:
        try:
            value = json.loads(self.state.read_text())
        except (OSError, ValueError):
            return {}
        return value if isinstance(value, dict) else {}

    def write_state(self, state: dict[str, object]) -> None:
        # Whole or not at all: file_host reads it every 10 s.
        temp = self.folder / ".state.json.tmp"
        temp.write_text(json.dumps(state, sort_keys=True) + "\n")
        temp.chmod(0o644)
        temp.replace(self.state)

    def image_epoch(self, image_id: str) -> int | None:
        # An image from before the epoch existed refuses the flag: None.
        done = subprocess.run(
            ["docker", "run", "--rm", "--network", "none", "--entrypoint", "/usr/local/bin/file_host", image_id, "--contract-epoch"],
            capture_output=True,
            text=True,
            check=False,
        )
        text = done.stdout.strip()
        return int(text) if done.returncode == 0 and text.isdigit() else None

    def image_bytes(self, image_id: str) -> int:
        done = subprocess.run(["docker", "image", "inspect", "--format", "{{.Size}}", image_id], capture_output=True, text=True, check=False)
        text = done.stdout.strip()
        return int(text) if done.returncode == 0 and text.isdigit() else 0


def runuser_prefix(user: str, runtime_dir: Path | None) -> list[str]:
    """`make` as `user`, with its home (sops' key) and, when given, a tmpfs
    runtime directory it owns (envctl's decrypted files)."""
    import pwd

    entry = pwd.getpwnam(user)
    env = [f"HOME={entry.pw_dir}"]
    if runtime_dir is not None:
        runtime_dir.mkdir(mode=0o700, exist_ok=True)
        os.chown(runtime_dir, entry.pw_uid, entry.pw_gid)
        env.append(f"XDG_RUNTIME_DIR={runtime_dir}")
    return ["runuser", "-u", user, "--", "env", *env]


# ── Self-test: a simulated host and clock ───────────────────────────────────


class Clock:
    def __init__(self) -> None:
        self.t = 0.0

    def now(self) -> float:
        return self.t

    def sleep(self, seconds: float) -> None:
        self.t += seconds


# How a file-host image behaves once started: seconds since start -> (status, health, restarts).
Behaviour = Callable[[float], tuple[str, str | None, int]]


def ready_after(seconds: float) -> Behaviour:
    return lambda t: ("running", "healthy" if t >= seconds else "starting", 0)


def never_ready() -> Behaviour:
    # start_period 300 s, then 5 failed probes 30 s apart.
    return lambda t: ("running", "unhealthy" if t >= 450 else "starting", 0)


def crashes_after(seconds: float) -> Behaviour:
    # A migration file_host refuses: it exits, and `restart: unless-stopped`
    # brings it back to exit again.
    return lambda t: ("running", "starting", 0) if t < seconds else ("restarting", "starting", 1)


def no_healthcheck() -> Behaviour:
    return lambda t: ("running", None, 0)


FH_REF = IMAGE


@dataclass
class FakeHost:
    clock: Clock
    behaviours: dict[str, Behaviour]
    tags: dict[str, str]
    serving: str | None
    registry: dict[str, str] = field(default_factory=dict)  # what a pull fetches
    pull_ok: bool = True
    up_ok: bool = True
    up_removes: bool = False  # `up` removed the old container and created none
    log: list[str] = field(default_factory=list)
    failed: list[str] = field(default_factory=list)
    calls: list[str] = field(default_factory=list)
    started_at: float = 0.0

    def reading(self, container: str) -> Reading:
        if self.serving is None:
            return Reading(None)
        status, health, restarts = self.behaviours[self.serving](self.clock.now() - self.started_at)
        return Reading(container, status, health, restarts, self.serving)

    def image_id(self, ref: str) -> str | None:
        return self.tags.get(ref)

    def describe(self, image_id: str) -> str:
        return image_id

    def tag(self, image_id: str, ref: str) -> bool:
        self.calls.append(f"tag {image_id} {ref}")
        self.tags[ref] = image_id
        return True

    def pull(self, ref: str) -> bool:
        self.calls.append(f"pull {ref}")
        if self.pull_ok and ref in self.registry:
            self.tags[ref] = self.registry[ref]
        return self.pull_ok

    def _start(self) -> None:
        image = self.tags[FH_REF]
        if image != self.serving:
            self.serving = image
            self.started_at = self.clock.now()

    def up(self) -> bool:
        self.calls.append("up")
        if self.up_removes:
            self.serving = None
            return False
        if self.up_ok:
            self._start()
        return self.up_ok

    def recreate_file_host(self) -> bool:
        self.calls.append("recreate")
        image = self.tags[FH_REF]
        if image != self.serving:
            self.serving = image
            self.started_at = self.clock.now()
        return True

    def logs(self, container: str) -> list[str]:
        return self.log

    def failed_images(self) -> list[str]:
        return list(self.failed)

    def remember_failed(self, image_id: str) -> None:
        self.failed.append(image_id)


@dataclass
class FakeDemand:
    marker: dict[str, object] | None
    state: dict[str, object] = field(default_factory=dict)
    epochs: dict[str, int | None] = field(default_factory=dict)
    cleared: bool = False

    def read_marker(self) -> dict[str, object] | None:
        return self.marker

    def clear_marker(self) -> None:
        self.marker = None
        self.cleared = True

    def read_state(self) -> dict[str, object]:
        return json.loads(json.dumps(self.state))

    def write_state(self, state: dict[str, object]) -> None:
        self.state = json.loads(json.dumps(state))

    def image_epoch(self, image_id: str) -> int | None:
        return self.epochs.get(image_id)

    def image_bytes(self, image_id: str) -> int:
        return 1000


class Tests(unittest.TestCase):
    def run_deploy(self, host: FakeHost, **options: object) -> tuple[int, list[str]]:
        said: list[str] = []
        code = deploy(host, Options(**options), host.clock.now, host.clock.sleep, said.append)  # type: ignore[arg-type]
        return code, said

    def world(self, old: Behaviour | None, new: Behaviour, publish: bool = True, **kw: object) -> FakeHost:
        clock = Clock()
        behaviours = {"sha256:new": new}
        tags = {}
        if old is not None:
            behaviours["sha256:old"] = old
            tags[FH_REF] = "sha256:old"
        registry = {FH_REF: "sha256:new" if publish else tags.get(FH_REF, "sha256:new")}
        return FakeHost(clock, behaviours, tags, "sha256:old" if old else None, registry=registry, **kw)  # type: ignore[arg-type]

    # judge, wait_ready
    def test_only_healthy_is_ready(self) -> None:
        self.assertEqual(judge(Reading("c", "running", "healthy"), 0), READY)
        self.assertEqual(judge(Reading("c", "running", "starting"), 0), WAIT)
        self.assertEqual(judge(Reading("c", "created"), 0), WAIT)

    def test_a_container_that_exits_or_restarts_has_failed_now(self) -> None:
        for status in ("exited", "dead", "restarting"):
            self.assertEqual(judge(Reading("c", status, "starting"), 0).kind, "failed", status)
        self.assertEqual(judge(Reading("c", "running", "starting", restarts=3), 2).kind, "failed")
        self.assertEqual(judge(Reading("c", "running", "starting", restarts=3), 3), WAIT)
        self.assertEqual(judge(Reading("c", "running", "unhealthy"), 0).kind, "failed")
        self.assertEqual(judge(Reading(None), 0).kind, "failed")
        self.assertEqual(judge(Reading("c", "running", None), 0).kind, "unjudgeable")

    def test_waiting_is_bounded(self) -> None:
        clock = Clock()
        verdict, took = wait_ready(lambda: Reading("c", "running", "starting"), clock.now, clock.sleep, 90, 2)
        self.assertEqual(verdict.kind, "failed")
        self.assertIn("within 90 s", verdict.reason)
        self.assertEqual(took, 90)

    def test_rollback_only_to_a_different_previous_image(self) -> None:
        self.assertTrue(plan_after_failure("a", "b").rollback)
        self.assertFalse(plan_after_failure("a", "a").rollback)
        self.assertFalse(plan_after_failure(None, "b").rollback)

    def test_previous_ref(self) -> None:
        self.assertEqual(previous_ref("pgathondu/server:latest"), "pgathondu/server:previous")
        self.assertEqual(previous_ref("host:5000/x/server:latest"), "host:5000/x/server:previous")
        self.assertEqual(previous_ref("host:5000/server"), "host:5000/server:previous")

    def test_backup_from_reads_file_hosts_start(self) -> None:
        lines = ["migrate: backed up to /app/dbs/backups/hopium-20261110T120000Z-before-20261110000100.db (verified)", "migrate: applied 2 (…)"]
        self.assertEqual(backup_from(lines), "/app/dbs/backups/hopium-20261110T120000Z-before-20261110000100.db")
        self.assertIsNone(backup_from(["migrate: current, nothing to apply"]))

    # One pass, end to end against the simulation
    def test_nothing_published_restarts_nothing(self) -> None:
        host = self.world(ready_after(0), ready_after(0), publish=False)
        code, said = self.run_deploy(host)
        self.assertEqual(code, EXIT_DEPLOYED)
        self.assertNotIn("up", host.calls)
        self.assertIn("nothing restarted", said[-1])

    def test_the_same_image_not_serving_is_brought_up_not_called_deployed(self) -> None:
        host = self.world(ready_after(0), ready_after(0), publish=False)
        host.behaviours["sha256:old"] = lambda t: ("exited", None, 0)
        code, said = self.run_deploy(host)
        self.assertIn("up", host.calls)
        self.assertNotEqual(code, EXIT_DEPLOYED)
        self.assertFalse(any("nothing restarted" in line for line in said))
        self.assertNotIn("recreate", host.calls, "no previous image to roll back to")

    def test_skipping_a_failed_image_still_brings_a_stopped_container_up(self) -> None:
        host = self.world(ready_after(0), ready_after(0), failed=["sha256:new"])
        host.behaviours["sha256:old"] = lambda t: ("exited", None, 0)
        code, said = self.run_deploy(host)
        self.assertTrue(any("failed here before" in line for line in said))
        self.assertEqual(host.tags[FH_REF], "sha256:old")
        self.assertIn("up", host.calls)
        self.assertNotEqual(code, EXIT_DEPLOYED)

    def test_only_file_hosts_image_is_pulled(self) -> None:
        host = self.world(ready_after(0), ready_after(0), publish=False)
        self.run_deploy(host)
        self.assertEqual([call for call in host.calls if call.startswith("pull")], [f"pull {IMAGE}"])

    def test_a_ready_image_is_deployed_and_the_old_one_kept(self) -> None:
        host = self.world(ready_after(10), ready_after(12))
        code, _ = self.run_deploy(host)
        self.assertEqual(code, EXIT_DEPLOYED)
        self.assertEqual(host.serving, "sha256:new")
        self.assertEqual(host.tags["pgathondu/server:previous"], "sha256:old")
        self.assertEqual(host.calls[-2:], ["tag sha256:old pgathondu/server:previous", "up"])

    def test_an_image_that_never_becomes_ready_ends_with_the_previous_one_serving(self) -> None:
        timeout = DEFAULT_READY_TIMEOUT
        host = self.world(ready_after(10), never_ready())
        code, said = self.run_deploy(host, ready_timeout=timeout)
        self.assertEqual(code, EXIT_ROLLED_BACK)
        self.assertEqual(host.serving, "sha256:old")
        self.assertEqual(host.tags[FH_REF], "sha256:old")
        self.assertEqual(host.calls[-2:], ["tag sha256:old pgathondu/server:latest", "recreate"])
        self.assertLessEqual(host.clock.now(), 2 * timeout + POLL_SECONDS)
        self.assertEqual(host.failed, ["sha256:new"])
        self.assertTrue(any("ROLLED BACK" in line for line in said))
        self.assertTrue(any("needs a newer checkout on home" in line for line in said))
        self.assertTrue(any("Nothing was restored or deleted" in line for line in said))

    def test_a_failed_image_is_not_tried_again(self) -> None:
        host = self.world(ready_after(0), crashes_after(3))
        self.assertEqual(self.run_deploy(host)[0], EXIT_ROLLED_BACK)
        host.calls.clear()
        code, said = self.run_deploy(host)  # the registry still has the bad one
        self.assertEqual(code, EXIT_DEPLOYED)
        self.assertNotIn("up", host.calls)
        self.assertEqual(host.tags[FH_REF], "sha256:old", "a reboot starts the old image, not the failed one")
        self.assertTrue(any("failed here before" in line for line in said))
        # A newer publish is tried.
        host.behaviours["sha256:newer"] = ready_after(5)
        host.registry[FH_REF] = "sha256:newer"
        self.assertEqual(self.run_deploy(host)[0], EXIT_DEPLOYED)
        self.assertEqual(host.serving, "sha256:newer")

    def test_a_refused_migration_rolls_back_without_waiting_out_the_bound(self) -> None:
        host = self.world(ready_after(4), crashes_after(3), log=["Error: migrate: refusing to migrate: failed: [20261110000100]"])
        code, said = self.run_deploy(host)
        self.assertEqual(code, EXIT_ROLLED_BACK)
        self.assertEqual(host.serving, "sha256:old")
        self.assertLess(host.clock.now(), 15)
        self.assertTrue(any("its log: Error: migrate: refusing" in line for line in said))

    def test_when_the_previous_image_fails_too_it_says_so_and_names_the_backup(self) -> None:
        backup = "/app/dbs/backups/hopium-20261110T120000Z-before-20261110000100.db"
        host = self.world(never_ready(), never_ready(), log=[f"{BACKUP_LINE}{backup} (verified)"])
        code, said = self.run_deploy(host)
        self.assertEqual(code, EXIT_NOTHING_READY)
        text = "\n".join(said)
        self.assertIn(backup, text)
        self.assertIn("never restored automatically", text)
        self.assertFalse(any("restore" in call for call in host.calls))

    def test_a_first_deploy_has_nothing_to_roll_back_to(self) -> None:
        host = self.world(None, never_ready())
        code, said = self.run_deploy(host)
        self.assertEqual(code, EXIT_NOTHING_READY)
        self.assertTrue(any("no previous image" in line for line in said))

    def test_a_failed_pull_replaces_nothing(self) -> None:
        host = self.world(ready_after(0), ready_after(0), pull_ok=False)
        code, _ = self.run_deploy(host)
        self.assertEqual(code, EXIT_NOT_DEPLOYED)
        self.assertEqual(host.serving, "sha256:old")
        self.assertEqual(host.calls, [f"pull {FH_REF}"])

    def test_a_refused_preflight_leaves_the_old_one_serving_and_names_the_checkout(self) -> None:
        host = self.world(ready_after(0), ready_after(0), up_ok=False)
        code, said = self.run_deploy(host)
        self.assertEqual(code, EXIT_NOT_DEPLOYED)
        self.assertEqual(host.serving, "sha256:old")
        self.assertEqual(host.failed, [])
        self.assertTrue(any("still serves" in line for line in said))
        self.assertTrue(any("needs a newer checkout on home" in line for line in said))

    def test_an_up_that_leaves_no_container_still_rolls_back(self) -> None:
        host = self.world(ready_after(0), ready_after(0), up_removes=True)
        code, said = self.run_deploy(host)
        self.assertEqual(code, EXIT_ROLLED_BACK)
        self.assertEqual(host.serving, "sha256:old")
        self.assertEqual(host.tags[FH_REF], "sha256:old")
        self.assertEqual(host.failed, ["sha256:new"], "the image up was asked to start")
        self.assertTrue(any("ROLLED BACK" in line for line in said))

    # On demand
    WALL = 1_800_000_000.0

    def run_on_demand(self, host: FakeHost, demand: FakeDemand) -> tuple[int, list[str]]:
        said: list[str] = []
        code = on_demand(host, demand, Options(), lambda: self.WALL, host.clock.now, host.clock.sleep, said.append)
        return code, said

    def test_no_marker_no_pull(self) -> None:
        host = self.world(ready_after(0), ready_after(0))
        demand = FakeDemand(None)
        self.assertEqual(self.run_on_demand(host, demand)[0], EXIT_DEPLOYED)
        self.assertEqual(host.calls, [])
        self.assertEqual(demand.state, {})

    def test_a_newer_epoch_is_deployed_and_recorded(self) -> None:
        host = self.world(ready_after(0), ready_after(5))
        demand = FakeDemand({"needs": 2, "running": 1}, epochs={"sha256:new": 2})
        code, _ = self.run_on_demand(host, demand)
        self.assertEqual(code, EXIT_DEPLOYED)
        self.assertEqual(host.serving, "sha256:new")
        self.assertTrue(demand.cleared)
        self.assertEqual(demand.state["latest_seen_epoch"], 2)
        self.assertEqual(demand.state["counts"], {"pulls": 1, "pulled_bytes": 1000, "deployed": 1})
        self.assertEqual(demand.state["last_run"], {"at": int(self.WALL), "exit": 0, "outcome": "deployed", "needs": 2})
        self.assertNotIn(f"pull {IMAGE}", host.calls[1:], "pulled once, not again by deploy")

    def test_the_same_epoch_republished_is_not_deployed(self) -> None:
        # Lazily on the previous image: a newer build of the same contract.
        host = self.world(ready_after(0), ready_after(0))
        demand = FakeDemand({"needs": 2, "running": 1}, epochs={"sha256:new": 1})
        code, said = self.run_on_demand(host, demand)
        self.assertEqual(code, EXIT_DEPLOYED)
        self.assertNotIn("up", host.calls)
        self.assertEqual(host.tags[FH_REF], "sha256:old", "a reboot does not start it")
        self.assertEqual(demand.state["counts"]["no_newer_image"], 1)
        self.assertEqual(demand.state["latest_seen_epoch"], 1)
        self.assertTrue(any("unavailable" in line for line in said))

    def test_an_image_whose_epoch_cannot_be_read_is_not_deployed(self) -> None:
        host = self.world(ready_after(0), ready_after(0))
        demand = FakeDemand({"needs": 2, "running": 1}, epochs={"sha256:new": None})
        self.assertEqual(self.run_on_demand(host, demand)[0], EXIT_DEPLOYED)
        self.assertNotIn("up", host.calls)
        self.assertIsNone(demand.state["latest_seen_epoch"])

    def test_a_malformed_marker_never_reads_as_epoch_zero(self) -> None:
        host = self.world(ready_after(0), ready_after(0))
        # The serving image says 2; the published one is 2 as well: nothing newer.
        demand = FakeDemand({}, epochs={"sha256:old": 2, "sha256:new": 2})
        self.assertEqual(self.run_on_demand(host, demand)[0], EXIT_DEPLOYED)
        self.assertNotIn("up", host.calls)
        # And when nothing can say what runs, nothing is deployed.
        host = self.world(ready_after(0), ready_after(0))
        demand = FakeDemand({}, epochs={"sha256:new": 9})
        self.assertEqual(self.run_on_demand(host, demand)[0], EXIT_NOT_DEPLOYED)
        self.assertNotIn("up", host.calls)
        self.assertTrue(demand.cleared)

    def test_a_pull_within_the_gap_is_deferred(self) -> None:
        host = self.world(ready_after(0), ready_after(0))
        demand = FakeDemand({"needs": 2, "running": 1}, state={"last_pull_at": int(self.WALL) - 60})
        self.assertEqual(self.run_on_demand(host, demand)[0], EXIT_DEPLOYED)
        self.assertEqual(host.calls, [])
        self.assertEqual(demand.state["counts"], {"deferred": 1})
        self.assertTrue(demand.cleared)

    def test_a_newer_epoch_that_fails_is_rolled_back_and_counted(self) -> None:
        host = self.world(ready_after(0), crashes_after(3))
        demand = FakeDemand({"needs": 2, "running": 1}, state={"counts": {"pulls": 4, "deployed": 3}}, epochs={"sha256:new": 2})
        code, _ = self.run_on_demand(host, demand)
        self.assertEqual(code, EXIT_ROLLED_BACK)
        self.assertEqual(host.serving, "sha256:old")
        self.assertEqual(demand.state["counts"], {"pulls": 5, "pulled_bytes": 1000, "deployed": 3, "rolled_back": 1})
        self.assertEqual(demand.state["last_run"]["exit"], EXIT_ROLLED_BACK)

    def test_a_failed_pull_is_counted_and_clears_the_marker(self) -> None:
        host = self.world(ready_after(0), ready_after(0), pull_ok=False)
        demand = FakeDemand({"needs": 2, "running": 1})
        self.assertEqual(self.run_on_demand(host, demand)[0], EXIT_NOT_DEPLOYED)
        self.assertEqual(demand.state["counts"], {"pulls": 1, "failed": 1})
        self.assertTrue(demand.cleared)
        self.assertEqual(demand.state["last_pull_at"], int(self.WALL), "a failed pull keeps the gap too")

    def test_the_bound_outlasts_the_start_period(self) -> None:
        text = (REPO_ROOT / "infra" / "compose" / "file_host.yml").read_text()
        found = re.search(r"^      start_period: (\d+)s$", text, re.MULTILINE)
        self.assertIsNotNone(found)
        assert found is not None
        self.assertGreater(DEFAULT_READY_TIMEOUT, float(found.group(1)))

    def test_the_service_is_the_compose_files(self) -> None:
        text = (REPO_ROOT / "infra" / "compose" / "file_host.yml").read_text()
        block = re.search(rf"^  {re.escape(FILE_HOST)}:\n((?:    .*\n|\s*\n)+)", text, re.MULTILINE)
        self.assertIsNotNone(block)
        assert block is not None
        self.assertIn(f"image: {IMAGE}\n", block.group(1))
        self.assertIn(f"container_name: {CONTAINER}\n", block.group(1))


def self_test() -> int:
    result = unittest.TextTestRunner(verbosity=1).run(unittest.defaultTestLoader.loadTestsFromTestCase(Tests))
    return 0 if result.wasSuccessful() else 1


def main() -> int:
    parser = argparse.ArgumentParser(description="Home's deploy of file_host: pull, and if the image changed, up, health-wait and roll back (AS18, #564).")
    parser.add_argument("--self-test", action="store_true", help="run the decisions against a simulated host, then exit")
    parser.add_argument("--no-pull", dest="pull", action="store_false", help="use the images already here")
    parser.add_argument("--ready-timeout", type=float, default=DEFAULT_READY_TIMEOUT, help="seconds to wait for each start to be ready (default %(default)s)")
    parser.add_argument("--state-dir", type=Path, default=None, help="where the failed images are remembered (default $XDG_STATE_HOME/server-deploy)")
    parser.add_argument("--image", default=IMAGE, help="the image file-host runs (default %(default)s, as infra/compose/file_host.yml names it)")
    parser.add_argument("--on-demand", type=Path, metavar="DEPLOY_DIR", help="run only if DEPLOY_DIR/wanted asks (${STORAGE}/deploy; nix/modules/home-deploy.nix)")
    parser.add_argument("--as-user", help="run make as this user (the on-demand unit runs as root)")
    parser.add_argument("--runtime-dir", type=Path, help="with --as-user: a tmpfs directory for envctl, chowned to the user")
    args = parser.parse_args()
    if args.self_test:
        return self_test()
    if args.ready_timeout <= 0:
        parser.error("--ready-timeout must be positive")

    def say(line: str) -> None:
        print(line, flush=True)

    as_user = runuser_prefix(args.as_user, args.runtime_dir) if args.as_user else None
    host = RealHost(REPO_ROOT, args.state_dir or default_state_dir(), as_user)
    options = Options(pull=args.pull, image=args.image, ready_timeout=args.ready_timeout)
    if args.on_demand is not None:
        demand = RealDemand(args.on_demand)
        try:
            return on_demand(host, demand, options, time.time, time.monotonic, time.sleep, say)
        except Exception:
            # Never leave the marker behind a crash: the path unit would start
            # this again at once. The failure is recorded for the alert, and
            # the traceback goes to the journal.
            state = demand.read_state()
            counts = state.get("counts") if isinstance(state.get("counts"), dict) else {}
            state["counts"] = {**counts, "failed": as_int(counts.get("failed")) + 1}
            state["last_run"] = {"at": int(time.time()), "exit": 1, "outcome": "failed"}
            demand.write_state(state)
            demand.clear_marker()
            raise
    return deploy(host, options, time.monotonic, time.sleep, say)


if __name__ == "__main__":
    sys.exit(main())

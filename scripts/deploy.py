#!/usr/bin/env python3
"""Deploy file_host on home, and put the previous image back when the new one
does not come up ready (AS18, #564).

`make up` runs this after its preflight. In order:

1. **Records what serves now**: the image of the running `file-host`
   container, by its image id, with its registry digest in the log.
2. **Pulls** `file-host`'s image, with `--pull` (`make up PULL=1`). Only that
   image: the rest of the stack is not this script's to roll back.
3. **Keeps the old image**: when the image about to start differs from the
   serving one, tags the serving one `<repository>:previous`, so a prune
   cannot take it and a rollback by hand can name it.
4. **Migrates**: `docker compose run --rm migrate` (AS16, #563), while the
   old file_host still serves. A failure stops here, with nothing replaced.
5. **Brings the stack up**: `docker compose up -d`.
6. **Waits up to `--ready-timeout` seconds** for the container's healthcheck
   to say `healthy`. That check is `file_host --health-check`, which passes
   when `/ready` says ready or degraded and fails when it says unavailable
   (AS15, #560). A container that exits, restarts or turns `unhealthy`
   fails at once, without waiting out the bound.
7. **Rolls back** when it does not come up: tags the recorded image back as
   the image compose names and recreates `file-host` alone from it
   (`up -d --no-deps --pull never`), then waits the same bound for it.

It never touches the database. A migration that ran stays applied: a
database ahead of its binary is one the old binary serves (`schema.rs`), and
#564 records that the database stays where the migration put it. Restoring
the pre-migration backup would take every write since the backup out of the
live database. Why that is not done here, and when it would be safe, is
README.md, "Deploying to home".

Exit status, for whatever alerts on it once AS17's channel exists:

  0  deployed: file-host is up and ready (the new image, or the same one)
  1  not deployed: the pull or the migration failed; nothing was replaced
  2  rolled back: the new image did not come up; the previous one serves
  3  nothing ready: the owner has to look (the message says what is known)

The decision logic (`judge`, `wait_ready`, `plan_after_failure` and `deploy`
over a `Docker` it is handed) is pure, so `--self-test` runs it against a
simulated Docker and clock: an image that never becomes ready ends with the
previous one serving, inside the bound plus a restart. Stdlib only, like the
repo's other scripts; home already runs `scripts/env_schema.py` under the
same `make up`.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
import time
import unittest
from collections.abc import Callable
from dataclasses import dataclass, field
from pathlib import Path
from typing import Protocol

REPO_ROOT = Path(__file__).resolve().parent.parent

SERVICE = "file-host"
MIGRATE_SERVICE = "migrate"

# The bound on becoming ready, in seconds. The healthcheck
# (infra/compose/file_host.yml) probes every 5 s during its 40 s
# start_period, then every 30 s with a 10 s timeout: 90 s covers every
# start-period probe, one full-interval probe after it, and 10 s of slack.
# A rollback waits the same bound again, so a bad image costs at most about
# 3 minutes of the month's 43 (AV1), plus the pull and the migration.
DEFAULT_READY_TIMEOUT = 90.0
POLL_SECONDS = 2.0

EXIT_DEPLOYED = 0
EXIT_NOT_DEPLOYED = 1
EXIT_ROLLED_BACK = 2
EXIT_NOTHING_READY = 3

# What `migrate` prints when it took a backup (apps/servers/file_host/src/bin/migrate.rs).
BACKUP_LINE = "migrate: backed up to "


# ── The decisions, pure ─────────────────────────────────────────────────────


@dataclass(frozen=True)
class Reading:
    """One look at the service's container, as `docker inspect` gives it."""

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
    """What one reading says about whether the container came up.

    Only `healthy` is ready. A container that is gone, has exited, is being
    restarted or has restarted since the wait began has failed now: a server
    that exits at boot (a config it refuses, say) will not become ready by
    waiting. `unhealthy` is Docker giving up. No healthcheck at all is not
    the image's fault, so it is unjudgeable rather than a reason to roll back.
    """
    if reading.container is None:
        return Verdict("failed", f"no {SERVICE} container")
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
    first = read()
    baseline = first.restarts
    reading = first
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
    did not change: then the cause is the configuration, `.env` or a
    dependency, which the same image would meet again.
    """
    if old_image is None:
        return Plan(False, f"no {SERVICE} container served before this deploy, so there is no previous image")
    if new_image == old_image:
        return Plan(False, "the image did not change, so the cause is not the image: look at .env, the config it refuses, or a dependency")
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
    """The backup `migrate` took this run, from its output, if it took one."""
    for line in lines:
        if line.startswith(BACKUP_LINE):
            return line[len(BACKUP_LINE) :].removesuffix(" (verified)").strip()
    return None


DATABASE_NOTE = (
    "The database stays where the migration put it (#564): the old image serves a database ahead of it "
    "(apps/servers/file_host/src/schema.rs). Nothing was restored or deleted."
)


def stuck_note(backup: str | None) -> str:
    restore = (
        f"This deploy's migration backed the database up to {backup} (on the host, under ${{STORAGE}}/backups). "
        if backup
        else "This deploy's migration took no backup: nothing was pending. "
    )
    return (
        restore + "It is not restored automatically: every write since it would leave the live database. "
        "If restoring is the way out, move the live file aside first, never delete it "
        "(README.md, \"Deploying to home\")."
    )


# ── The deploy, over a Docker it is handed ──────────────────────────────────


class Docker(Protocol):
    def image_ref(self) -> str: ...
    def reading(self) -> Reading: ...
    def image_id(self, ref: str) -> str | None: ...
    def describe(self, image_id: str) -> str: ...
    def tag(self, image_id: str, ref: str) -> bool: ...
    def pull(self) -> bool: ...
    def migrate(self) -> tuple[bool, list[str]]: ...
    def up(self) -> bool: ...
    def recreate_service(self) -> bool: ...


@dataclass
class Options:
    pull: bool = False
    ready_timeout: float = DEFAULT_READY_TIMEOUT
    poll: float = POLL_SECONDS


def deploy(docker: Docker, options: Options, now: Callable[[], float], sleep: Callable[[float], None], say: Callable[[str], None]) -> int:
    ref = docker.image_ref()
    before = docker.reading()
    old = before.image if before.container else None
    say(f"deploy: serving now: {docker.describe(old)}" if old else f"deploy: no {SERVICE} container serves now")

    if options.pull:
        say(f"deploy: pulling {ref}")
        if not docker.pull():
            say(f"deploy: the pull failed; nothing was replaced{', the old file_host still serves' if old else ''}")
            return EXIT_NOT_DEPLOYED

    candidate = docker.image_id(ref)
    if old and candidate and candidate != old:
        kept = previous_ref(ref)
        if docker.tag(old, kept):
            say(f"deploy: kept the serving image as {kept}")
        else:
            say(f"deploy: could not tag the serving image as {kept}; it stays as {old}")

    ok, lines = docker.migrate()
    if not ok:
        say(f"deploy: migrate failed (its reason is above); nothing was replaced{', the old file_host still serves' if old else ''}")
        return EXIT_NOT_DEPLOYED
    backup = backup_from(lines)

    if not docker.up():
        say("deploy: docker compose up -d failed; reading what it left")
    after = docker.reading()
    new = after.image
    say(f"deploy: started {docker.describe(new)}; waiting up to {options.ready_timeout:.0f} s for it to be ready" if new else f"deploy: no {SERVICE} container after up")

    verdict, took = wait_ready(docker.reading, now, sleep, options.ready_timeout, options.poll)
    if verdict.kind == "ready":
        say(f"deploy: ready after {took:.0f} s")
        return EXIT_DEPLOYED
    say(f"deploy: the new {SERVICE} did not come up: {verdict.reason}")
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
    say(f"deploy: ROLLING BACK to {docker.describe(old)}")
    if not docker.tag(old, ref) or not docker.recreate_service():
        say(f"deploy: the rollback could not restart {old}")
        say(DATABASE_NOTE)
        say(stuck_note(backup))
        return EXIT_NOTHING_READY
    verdict, took_back = wait_ready(docker.reading, now, sleep, options.ready_timeout, options.poll)
    if verdict.kind == "ready":
        say(f"deploy: ROLLED BACK: the previous image is ready after {took_back:.0f} s. The image that failed is {new}.")
        say(f"deploy: {ref} now names the previous image here; the next `make up PULL=1` pulls the new one again.")
        say(DATABASE_NOTE)
        return EXIT_ROLLED_BACK
    say(f"deploy: the previous image did not come up either: {verdict.reason}")
    say(DATABASE_NOTE)
    say(stuck_note(backup))
    return EXIT_NOTHING_READY


# ── The real Docker ─────────────────────────────────────────────────────────


class ComposeDocker:
    def __init__(self, cwd: Path = REPO_ROOT) -> None:
        self.cwd = cwd

    def _run(self, *args: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(list(args), cwd=self.cwd, capture_output=True, text=True, check=False)

    def _stream(self, *args: str) -> tuple[int, list[str]]:
        """Runs a command with its output shown as it comes, and kept."""
        lines: list[str] = []
        with subprocess.Popen(list(args), cwd=self.cwd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True) as proc:
            assert proc.stdout is not None
            for line in proc.stdout:
                sys.stdout.write(line)
                lines.append(line.rstrip("\n"))
        return proc.returncode, lines

    def image_ref(self) -> str:
        done = self._run("docker", "compose", "config", "--format", "json")
        if done.returncode != 0:
            raise SystemExit(f"deploy: docker compose config failed:\n{done.stderr}")
        return str(json.loads(done.stdout)["services"][SERVICE]["image"])

    def reading(self) -> Reading:
        ids = self._run("docker", "compose", "ps", "-a", "-q", SERVICE).stdout.split()
        if not ids:
            return Reading(None)
        done = self._run("docker", "inspect", "--format", "{{json .}}", ids[0])
        if done.returncode != 0:
            return Reading(None)
        info = json.loads(done.stdout)
        state = info.get("State") or {}
        health = (state.get("Health") or {}).get("Status")
        return Reading(ids[0], state.get("Status"), health, int(info.get("RestartCount") or 0), info.get("Image"))

    def image_id(self, ref: str) -> str | None:
        done = self._run("docker", "image", "inspect", "--format", "{{.Id}}", ref)
        return (done.stdout.strip() or None) if done.returncode == 0 else None

    def describe(self, image_id: str) -> str:
        done = self._run("docker", "image", "inspect", "--format", "{{join .RepoDigests \", \"}}", image_id)
        digests = done.stdout.strip() if done.returncode == 0 else ""
        return f"{image_id} ({digests})" if digests else image_id

    def tag(self, image_id: str, ref: str) -> bool:
        return self._run("docker", "tag", image_id, ref).returncode == 0

    def pull(self) -> bool:
        return self._stream("docker", "compose", "pull", MIGRATE_SERVICE, SERVICE)[0] == 0

    def migrate(self) -> tuple[bool, list[str]]:
        code, lines = self._stream("docker", "compose", "run", "--rm", MIGRATE_SERVICE)
        return code == 0, lines

    def up(self) -> bool:
        return self._stream("docker", "compose", "up", "-d")[0] == 0

    def recreate_service(self) -> bool:
        # --no-deps: not the migrate one-shot, which is the same image and has
        # nothing to do on a database ahead of it. --pull never: the tag just
        # moved back to the old image; a pull would undo that.
        return self._stream("docker", "compose", "up", "-d", "--no-deps", "--pull", "never", SERVICE)[0] == 0


# ── Self-test: a simulated Docker and clock ─────────────────────────────────


class Clock:
    def __init__(self) -> None:
        self.t = 0.0

    def now(self) -> float:
        return self.t

    def sleep(self, seconds: float) -> None:
        self.t += seconds


# How an image behaves once started: seconds since start -> (status, health, restarts).
Behaviour = Callable[[float], tuple[str, str | None, int]]


def ready_after(seconds: float) -> Behaviour:
    return lambda t: ("running", "healthy" if t >= seconds else "starting", 0)


def never_ready() -> Behaviour:
    # /ready unavailable from the start: starting through the 40 s
    # start_period, unhealthy after 5 failed 30 s probes.
    return lambda t: ("running", "unhealthy" if t >= 190 else "starting", 0)


def crashes_after(seconds: float) -> Behaviour:
    return lambda t: ("running", "starting", 0) if t < seconds else ("restarting", "starting", 1)


def no_healthcheck() -> Behaviour:
    return lambda t: ("running", None, 0)


@dataclass
class FakeDocker:
    clock: Clock
    behaviours: dict[str, Behaviour]
    tags: dict[str, str]
    serving: str | None
    registry: dict[str, str] = field(default_factory=dict)  # what a pull fetches
    ref: str = "pgathondu/server:latest"
    migrate_ok: bool = True
    migrate_lines: list[str] = field(default_factory=list)
    pull_ok: bool = True
    calls: list[str] = field(default_factory=list)
    started_at: float = 0.0

    def image_ref(self) -> str:
        return self.ref

    def reading(self) -> Reading:
        if self.serving is None:
            return Reading(None)
        status, health, restarts = self.behaviours[self.serving](self.clock.now() - self.started_at)
        return Reading("c-" + self.serving, status, health, restarts, self.serving)

    def image_id(self, ref: str) -> str | None:
        return self.tags.get(ref)

    def describe(self, image_id: str) -> str:
        return image_id

    def tag(self, image_id: str, ref: str) -> bool:
        self.calls.append(f"tag {image_id} {ref}")
        self.tags[ref] = image_id
        return True

    def pull(self) -> bool:
        self.calls.append("pull")
        if self.pull_ok:
            self.tags.update(self.registry)
        return self.pull_ok

    def migrate(self) -> tuple[bool, list[str]]:
        self.calls.append("migrate")
        self.clock.sleep(1)
        return self.migrate_ok, self.migrate_lines

    def _start(self) -> None:
        # Compose recreates the container when the image its tag names moved.
        image = self.tags[self.ref]
        if image != self.serving:
            self.serving = image
            self.started_at = self.clock.now()

    def up(self) -> bool:
        self.calls.append("up")
        self._start()
        return True

    def recreate_service(self) -> bool:
        self.calls.append("recreate")
        self._start()
        return True


class Tests(unittest.TestCase):
    def run_deploy(self, docker: FakeDocker, **options: object) -> tuple[int, list[str]]:
        said: list[str] = []
        code = deploy(docker, Options(**options), docker.clock.now, docker.clock.sleep, said.append)  # type: ignore[arg-type]
        return code, said

    def world(self, old: Behaviour | None, new: Behaviour, **kw: object) -> FakeDocker:
        clock = Clock()
        behaviours = {"sha256:new": new}
        tags = {}
        if old is not None:
            behaviours["sha256:old"] = old
            tags["pgathondu/server:latest"] = "sha256:old"
        return FakeDocker(clock, behaviours, tags, "sha256:old" if old else None, registry={"pgathondu/server:latest": "sha256:new"}, **kw)  # type: ignore[arg-type]

    # judge
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

    def test_no_healthcheck_is_unjudgeable(self) -> None:
        self.assertEqual(judge(Reading("c", "running", None), 0).kind, "unjudgeable")

    # wait_ready
    def test_waiting_is_bounded(self) -> None:
        clock = Clock()
        verdict, took = wait_ready(lambda: Reading("c", "running", "starting"), clock.now, clock.sleep, 90, 2)
        self.assertEqual(verdict.kind, "failed")
        self.assertIn("within 90 s", verdict.reason)
        self.assertEqual(took, 90)

    def test_a_restart_before_the_wait_does_not_count(self) -> None:
        clock = Clock()
        verdict, _ = wait_ready(lambda: Reading("c", "running", "healthy", restarts=4), clock.now, clock.sleep, 90)
        self.assertEqual(verdict, READY)

    # plan_after_failure, previous_ref, backup_from
    def test_rollback_only_to_a_different_previous_image(self) -> None:
        self.assertTrue(plan_after_failure("a", "b").rollback)
        self.assertFalse(plan_after_failure("a", "a").rollback)
        self.assertFalse(plan_after_failure(None, "b").rollback)

    def test_previous_ref(self) -> None:
        self.assertEqual(previous_ref("pgathondu/server:latest"), "pgathondu/server:previous")
        self.assertEqual(previous_ref("pgathondu/server"), "pgathondu/server:previous")
        self.assertEqual(previous_ref("host:5000/x/server:latest"), "host:5000/x/server:previous")
        self.assertEqual(previous_ref("host:5000/server"), "host:5000/server:previous")

    def test_backup_from_reads_migrates_line(self) -> None:
        lines = ["migrate: backed up to /app/dbs/backups/hopium-20261031T120000Z-before-2.db (verified)", "migrate: applied 1 (2 x)"]
        self.assertEqual(backup_from(lines), "/app/dbs/backups/hopium-20261031T120000Z-before-2.db")
        self.assertIsNone(backup_from(["migrate: current, nothing to apply"]))

    # deploy, end to end against the simulation
    def test_a_ready_image_is_deployed_and_the_old_one_kept(self) -> None:
        docker = self.world(ready_after(10), ready_after(12))
        code, _ = self.run_deploy(docker, pull=True)
        self.assertEqual(code, EXIT_DEPLOYED)
        self.assertEqual(docker.serving, "sha256:new")
        self.assertEqual(docker.tags["pgathondu/server:previous"], "sha256:old")
        self.assertEqual(docker.calls, ["pull", "tag sha256:old pgathondu/server:previous", "migrate", "up"])

    def test_an_image_that_never_becomes_ready_ends_with_the_previous_one_serving(self) -> None:
        # Issue #564's "Done means": inside T plus a restart.
        timeout = 90.0
        docker = self.world(ready_after(10), never_ready())
        code, said = self.run_deploy(docker, pull=True, ready_timeout=timeout)
        self.assertEqual(code, EXIT_ROLLED_BACK)
        self.assertEqual(docker.serving, "sha256:old")
        self.assertEqual(docker.tags["pgathondu/server:latest"], "sha256:old")
        self.assertEqual(docker.calls[-2:], ["tag sha256:old pgathondu/server:latest", "recreate"])
        started_new = 1.0  # after the one-second migration
        self.assertLessEqual(docker.clock.now() - started_new, timeout + 10 + POLL_SECONDS)
        self.assertTrue(any("ROLLED BACK" in line for line in said))
        self.assertTrue(any("Nothing was restored or deleted" in line for line in said))

    def test_a_crash_at_boot_rolls_back_without_waiting_out_the_bound(self) -> None:
        docker = self.world(ready_after(4), crashes_after(3))
        code, _ = self.run_deploy(docker, pull=True)
        self.assertEqual(code, EXIT_ROLLED_BACK)
        self.assertEqual(docker.serving, "sha256:old")
        self.assertLess(docker.clock.now(), 15)

    def test_when_the_previous_image_fails_too_it_says_so_and_names_the_backup(self) -> None:
        backup = "/app/dbs/backups/hopium-20261031T120000Z-before-2.db"
        docker = self.world(never_ready(), never_ready(), migrate_lines=[f"{BACKUP_LINE}{backup} (verified)"])
        code, said = self.run_deploy(docker, pull=True)
        self.assertEqual(code, EXIT_NOTHING_READY)
        text = "\n".join(said)
        self.assertIn(backup, text)
        self.assertIn("not restored automatically", text)
        self.assertNotIn("restore", " ".join(docker.calls))

    def test_the_same_image_failing_is_not_rolled_back(self) -> None:
        docker = self.world(ready_after(0), ready_after(0))
        docker.behaviours["sha256:old"] = never_ready()
        docker.started_at = 0
        docker.clock.t = 0
        # no pull: the image does not change; the old container itself is not ready
        code, said = self.run_deploy(docker)
        self.assertEqual(code, EXIT_NOTHING_READY)
        self.assertNotIn("recreate", docker.calls)
        self.assertTrue(any("did not change" in line for line in said))

    def test_a_first_deploy_has_nothing_to_roll_back_to(self) -> None:
        docker = self.world(None, never_ready())
        docker.tags["pgathondu/server:latest"] = "sha256:new"
        code, said = self.run_deploy(docker)
        self.assertEqual(code, EXIT_NOTHING_READY)
        self.assertTrue(any("no previous image" in line for line in said))

    def test_a_failed_migration_or_pull_replaces_nothing(self) -> None:
        docker = self.world(ready_after(0), ready_after(0), migrate_ok=False)
        code, _ = self.run_deploy(docker, pull=True)
        self.assertEqual(code, EXIT_NOT_DEPLOYED)
        self.assertEqual(docker.serving, "sha256:old")
        self.assertNotIn("up", docker.calls)

        docker = self.world(ready_after(0), ready_after(0), pull_ok=False)
        code, _ = self.run_deploy(docker, pull=True)
        self.assertEqual(code, EXIT_NOT_DEPLOYED)
        self.assertEqual(docker.calls, ["pull"])

    def test_no_healthcheck_is_reported_not_rolled_back(self) -> None:
        docker = self.world(ready_after(0), no_healthcheck())
        code, _ = self.run_deploy(docker, pull=True)
        self.assertEqual(code, EXIT_NOTHING_READY)
        self.assertEqual(docker.serving, "sha256:new")
        self.assertNotIn("recreate", docker.calls)


def self_test() -> int:
    result = unittest.TextTestRunner(verbosity=1).run(unittest.defaultTestLoader.loadTestsFromTestCase(Tests))
    return 0 if result.wasSuccessful() else 1


def main() -> int:
    parser = argparse.ArgumentParser(description="Deploy file_host on home; roll back an image that does not come up ready (AS18, #564).")
    parser.add_argument("--self-test", action="store_true", help="run the decision logic against a simulated Docker, then exit")
    parser.add_argument("--pull", action="store_true", help=f"pull {SERVICE}'s image first")
    parser.add_argument("--ready-timeout", type=float, default=DEFAULT_READY_TIMEOUT, help="seconds to wait for each start to be ready (default %(default)s)")
    args = parser.parse_args()
    if args.self_test:
        return self_test()
    if args.ready_timeout <= 0:
        parser.error("--ready-timeout must be positive")

    def say(line: str) -> None:
        print(line, flush=True)

    return deploy(ComposeDocker(), Options(pull=args.pull, ready_timeout=args.ready_timeout), time.monotonic, time.sleep, say)


if __name__ == "__main__":
    sys.exit(main())

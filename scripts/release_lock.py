#!/usr/bin/env python3
"""Reconcile home's release with what has been published (RELEASE R2, #789).

`deploy/home/release.json` names every image compose starts on home
(`apps/envctl/src/release.rs`, R1 #788). This works out, from the sources of
truth alone, the newest published reference of every image the release
follows, and writes the release so it names those. A run carries no images
of its own, so runs are idempotent and order-free: any run computes the
whole set, a dropped or cancelled run loses nothing, and the last one wins
with the right answer (docs/release.md, "Invariants").

    python3 scripts/release_lock.py              # reconcile deploy/home/release.json
    python3 scripts/release_lock.py --self-test

Where "newest published" comes from, per followed service (`SOURCES`):

- **A server image** (`changesets`): the newest image changeset on main
  (`.github/docker-changesets/*.json`, ordered by its `source_sha`'s place in
  main's history, as publish-image-changesets.yml orders them) that includes
  the image and whose `pgathondu/<suffix>:<source_sha>` tag Docker Hub has.
  A changeset whose publish failed has no tag, so the one before it is used.
- **A some-ui image** (`hub`): the most recently pushed tag of the shape its
  release publishes (a whole commit for render, a version for www), never
  `latest`. None is followed yet: they enter with U1 (#791) once R3's set
  check (#790) can refuse an inconsistent set, so a www release can never
  again move without the render that records it (#756's break).

Docker Hub's tag API gives each tag's digest, so every reference is
`repo:tag@sha256:…`. What it never does:

- **Moves a held entry.** `file-host` stays `pgathondu/server:latest` until R4
  (#793), because `envctl deploy` pulls, tags and rolls back that tag.
- **Moves an entry it does not follow** (third-party images are R7's, #796;
  locally built ones are not published), or adds or removes a service.
- **Moves an entry backwards in serial**: `serial` rises once, from main's,
  when anything changes; `server_commit` is main's commit it formed on.

It prints one line per change for the release pull request. Exit status: 0,
whether or not anything changed (the workflow reads the file); 2 when a
source cannot be read, so the run fails loudly and the next one tries again.
Stdlib only.
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
import unittest
import urllib.error
import urllib.request
from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
RELEASE = REPO_ROOT / "deploy" / "home" / "release.json"
CHANGESETS = REPO_ROOT / ".github" / "docker-changesets"
HUB = "https://hub.docker.com/v2/repositories"

# Entries no source moves, and why. Each names the story that frees it.
HELD = {
    "file-host": "held on pgathondu/server:latest until R4 (#793): envctl deploy pulls, tags and rolls back that tag",
}


@dataclass(frozen=True)
class Changesets:
    """A server image: the newest changeset on main that pushed it."""

    suffix: str


@dataclass(frozen=True)
class Hub:
    """A some-ui image: the most recently pushed tag of its release's shape."""

    repository: str
    shape: str  # a regex the whole tag matches


# Every service the release follows, and where its newest reference comes
# from. A service not here keeps whatever main's release says.
SOURCES: dict[str, Changesets | Hub] = {
    "kokoro": Changesets("kokoro"),
    "orchestrator": Changesets("orchestrator"),
    "rust-runner": Changesets("rust-runner"),
}

COMMIT = r"[0-9a-f]{40}"
VERSION = r"[0-9]+\.[0-9]+\.[0-9]+"


class Unreadable(Exception):
    pass


# ── Reading the sources ─────────────────────────────────────────────────────


def hub_get(path: str) -> dict | None:
    """One Docker Hub API document, None when it does not exist."""
    request = urllib.request.Request(f"{HUB}/{path}", headers={"User-Agent": "release-lock"})
    try:
        with urllib.request.urlopen(request, timeout=30) as response:
            return json.load(response)
    except urllib.error.HTTPError as err:
        if err.code == 404:
            return None
        raise Unreadable(f"Docker Hub answered {err.code} for {path}") from err
    except (urllib.error.URLError, TimeoutError, ValueError) as err:
        raise Unreadable(f"Docker Hub could not be read for {path}: {err}") from err


def main_order(repo: Path) -> list[str]:
    """main's commits, oldest first."""
    done = subprocess.run(["git", "rev-list", "--reverse", "--topo-order", "HEAD"], cwd=repo, capture_output=True, text=True, check=False)
    if done.returncode != 0:
        raise Unreadable(f"git rev-list failed: {done.stderr.strip()}")
    return done.stdout.split()


def changesets(folder: Path) -> list[dict]:
    found = []
    for path in sorted(folder.glob("*.json")):
        try:
            found.append(json.loads(path.read_text()))
        except ValueError as err:
            raise Unreadable(f"{path.name} is not JSON: {err}") from err
    return found


@dataclass(frozen=True)
class World:
    """What the sources say, read once: injected whole by the tests."""

    order: list[str]
    changesets: list[dict]
    tag: Callable[[str, str], str | None]  # (repository, tag) → digest, None if absent
    tags: Callable[[str], list[tuple[str, str]]]  # repository → [(tag, digest)], newest push first


def real_world(repo: Path) -> World:
    def tag(repository: str, name: str) -> str | None:
        found = hub_get(f"{repository}/tags/{name}")
        return None if found is None else found.get("digest")

    def tags(repository: str) -> list[tuple[str, str]]:
        found = hub_get(f"{repository}/tags?page_size=100&ordering=last_updated") or {}
        return [(entry["name"], entry["digest"]) for entry in found.get("results", []) if entry.get("digest")]

    return World(main_order(repo), changesets(repo / ".github" / "docker-changesets"), tag, tags)


# ── The decision, pure ──────────────────────────────────────────────────────


def newest(source: Changesets | Hub, world: World) -> str | None:
    """The newest published reference of one followed image, or None."""
    if isinstance(source, Changesets):
        place = {sha: index for index, sha in enumerate(world.order)}
        shas = {
            changeset["source_sha"]
            for changeset in world.changesets
            if changeset.get("source_sha") in place and any(image.get("repo_suffix") == source.suffix for image in changeset.get("include", []))
        }
        repository = f"pgathondu/{source.suffix}"
        for sha in sorted(shas, key=place.__getitem__, reverse=True):
            digest = world.tag(repository, sha)
            if digest:
                return f"{repository}:{sha}@{digest}"
        return None
    for name, digest in world.tags(source.repository):
        if name != "latest" and re.fullmatch(source.shape, name):
            return f"{source.repository}:{name}@{digest}"
    return None


def reconcile(release: dict, world: World, server_commit: str, sources: dict | None = None) -> tuple[dict, list[str]]:
    """main's release with every followed entry at its newest, and what moved."""
    sources = SOURCES if sources is None else sources
    services = {name: dict(entry) for name, entry in release["services"].items()}
    said: list[str] = []
    for name, source in sorted(sources.items()):
        if name not in services:
            raise Unreadable(f"the release follows {name}, which it does not name")
        if name in HELD:
            said.append(f"- {name}: {HELD[name]}")
            continue
        wanted = newest(source, world)
        if wanted is None or wanted == services[name]["image"]:
            continue
        said.append(f"- {name}: {services[name]['image']} → {wanted}")
        services[name]["image"] = wanted
    if services == release["services"]:
        return release, said
    return {"serial": int(release["serial"]) + 1, "server_commit": server_commit, "services": services}, said


def render(release: dict) -> str:
    """The release as R1 wrote it: one service per line, in name order."""
    services = release["services"]
    lines = [f'    {json.dumps(name)}: {{"image": {json.dumps(services[name]["image"])}}}' for name in sorted(services)]
    return (
        "{\n"
        f'  "serial": {json.dumps(release["serial"])},\n'
        f'  "server_commit": {json.dumps(release.get("server_commit", ""))},\n'
        '  "services": {\n' + ",\n".join(lines) + "\n  }\n}\n"
    )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--release", type=Path, default=RELEASE)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        result = unittest.main(argv=[sys.argv[0]], exit=False, verbosity=1).result
        return 0 if result.wasSuccessful() else 1
    try:
        world = real_world(REPO_ROOT)
        release = json.loads(args.release.read_text())
        updated, said = reconcile(release, world, world.order[-1] if world.order else "")
    except (Unreadable, KeyError, ValueError) as err:
        print(f"release_lock: {err}", file=sys.stderr)
        return 2
    if updated is not release:
        args.release.write_text(render(updated))
    print("\n".join(said))
    return 0


# ── Tests ───────────────────────────────────────────────────────────────────

A, B, C = ("a" * 40, "b" * 40, "c" * 40)
DA, DB, DC = ("sha256:" + "1" * 64, "sha256:" + "2" * 64, "sha256:" + "3" * 64)


def sample() -> dict:
    return {
        "serial": 4,
        "server_commit": "old",
        "services": {
            "file-host": {"image": "pgathondu/server:latest"},
            "rust-runner": {"image": "pgathondu/rust-runner:latest"},
            "render": {"image": f"pgathondu/render:{A}@{DA}"},
            "redis": {"image": "redis:latest"},
        },
    }


def world(published: dict[tuple[str, str], str], changesets_: list[dict], order: list[str], hub_tags: dict[str, list[tuple[str, str]]] | None = None) -> World:
    return World(order, changesets_, lambda repository, tag: published.get((repository, tag)), lambda repository: (hub_tags or {}).get(repository, []))


def changeset(sha: str, *suffixes: str) -> dict:
    return {"source_sha": sha, "include": [{"repo_suffix": suffix} for suffix in suffixes]}


class Tests(unittest.TestCase):
    def test_the_newest_published_changeset_wins_whatever_order_they_were_read(self) -> None:
        sets = [changeset(C, "rust-runner"), changeset(A, "rust-runner"), changeset(B, "server")]
        seen = world({("pgathondu/rust-runner", A): DA, ("pgathondu/rust-runner", C): DC}, sets, [A, B, C])
        updated, said = reconcile(sample(), seen, "main", {"rust-runner": Changesets("rust-runner")})
        self.assertEqual(updated["services"]["rust-runner"]["image"], f"pgathondu/rust-runner:{C}@{DC}")
        self.assertEqual((updated["serial"], updated["server_commit"]), (5, "main"))
        self.assertEqual(len(said), 1)

    def test_a_changeset_whose_publish_failed_falls_back_to_the_one_before(self) -> None:
        sets = [changeset(A, "rust-runner"), changeset(C, "rust-runner")]
        seen = world({("pgathondu/rust-runner", A): DA}, sets, [A, B, C])
        updated, _ = reconcile(sample(), seen, "main", {"rust-runner": Changesets("rust-runner")})
        self.assertEqual(updated["services"]["rust-runner"]["image"], f"pgathondu/rust-runner:{A}@{DA}")

    def test_a_changeset_not_on_main_is_ignored(self) -> None:
        seen = world({("pgathondu/rust-runner", C): DC}, [changeset(C, "rust-runner")], [A, B])
        before = sample()
        updated, said = reconcile(before, seen, "main", {"rust-runner": Changesets("rust-runner")})
        self.assertIs(updated, before)
        self.assertEqual(said, [])

    def test_reconciling_twice_changes_nothing_the_second_time(self) -> None:
        seen = world({("pgathondu/rust-runner", A): DA}, [changeset(A, "rust-runner")], [A])
        once, _ = reconcile(sample(), seen, "m1", {"rust-runner": Changesets("rust-runner")})
        twice, said = reconcile(once, seen, "m2", {"rust-runner": Changesets("rust-runner")})
        self.assertIs(twice, once, "idempotent: a second run on the same sources writes nothing")
        self.assertEqual((twice["serial"], said), (5, []))

    def test_file_host_is_held_until_r4(self) -> None:
        seen = world({("pgathondu/server", A): DA}, [changeset(A, "server")], [A])
        before = sample()
        updated, said = reconcile(before, seen, "main", {"file-host": Changesets("server")})
        self.assertIs(updated, before)
        self.assertTrue(any("file-host: held" in line and "#793" in line for line in said), said)

    def test_a_hub_source_takes_the_newest_tag_of_its_shape_never_latest(self) -> None:
        tags = {"pgathondu/render": [("latest", DC), ("v-test", DC), (B, DB), (A, DA)]}
        updated, said = reconcile(sample(), world({}, [], [A], tags), "main", {"render": Hub("pgathondu/render", COMMIT)})
        self.assertEqual(updated["services"]["render"]["image"], f"pgathondu/render:{B}@{DB}")
        self.assertEqual(len(said), 1)

    def test_an_unfollowed_entry_is_never_moved(self) -> None:
        seen = world({("pgathondu/rust-runner", A): DA}, [changeset(A, "rust-runner")], [A], {"redis": [("7.4", DA)]})
        updated, _ = reconcile(sample(), seen, "main", {"rust-runner": Changesets("rust-runner")})
        self.assertEqual(updated["services"]["redis"]["image"], "redis:latest")
        self.assertEqual(updated["services"]["render"]["image"], f"pgathondu/render:{A}@{DA}")

    def test_following_a_service_the_release_lacks_is_unreadable(self) -> None:
        with self.assertRaises(Unreadable):
            reconcile(sample(), world({}, [], []), "main", {"ghost": Changesets("ghost")})

    def test_the_checked_in_sources_name_real_services(self) -> None:
        release = json.loads(RELEASE.read_text())
        for name in list(SOURCES) + list(HELD):
            self.assertIn(name, release["services"])
        self.assertFalse(set(SOURCES) & set(HELD), "a held entry is not also followed")

    def test_the_file_keeps_r1_s_shape_and_round_trips(self) -> None:
        text = RELEASE.read_text()
        self.assertEqual(render(json.loads(text)), text, "release.json is written exactly as render writes it")

    def test_every_server_image_home_runs_is_followed(self) -> None:
        """Each image the detector publishes and home runs is in SOURCES or HELD (#805)."""
        detect = (REPO_ROOT / ".github" / "scripts" / "detect" / "src" / "main.rs").read_text()
        suffixes = set(re.findall(r'repo_suffix: "([^"]+)"', detect))
        release = json.loads(RELEASE.read_text())
        followed = {source.suffix for source in SOURCES.values() if isinstance(source, Changesets)}
        held = {"server"}  # file-host, HELD until R4
        for suffix in suffixes:
            runs = any(entry["image"].startswith(f"pgathondu/{suffix}:") for entry in release["services"].values())
            if runs or suffix in followed:
                self.assertIn(suffix, followed | held, f"pgathondu/{suffix} is published and home runs it, so the release follows it")
        self.assertIn("kokoro", followed)

    def test_the_checked_in_changesets_all_read(self) -> None:
        for found in changesets(CHANGESETS):
            self.assertIn("source_sha", found)


if __name__ == "__main__":
    sys.exit(main())

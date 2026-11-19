#!/usr/bin/env python3
"""Reconcile home's release with what has been published (RELEASE R2, #789).

`deploy/home/release.json` names every image compose starts on home
(`apps/envctl/src/release.rs`, R1 #788). This works out, from the sources of
truth alone, the newest published reference of every image the release
follows, and writes the release so it names those. A run carries no images
of its own, so runs are idempotent and order-free: any run computes the
whole set, a dropped or cancelled run loses nothing, and the last one wins
with the right answer (docs/release.md, "Invariants").

    python3 scripts/release_lock.py                # reconcile deploy/home/release.json
    python3 scripts/release_lock.py --third-party  # refresh the third-party pins only
    python3 scripts/release_lock.py --third-party-due  # `due` or `not due`: git alone
    python3 scripts/release_lock.py --self-test

Where "newest published" comes from, per followed service (`SOURCES`):

- **A server image** (`changesets`): the newest image changeset on main
  (`.github/docker-changesets/*.json`, ordered by its `source_sha`'s place in
  main's history, as publish-image-changesets.yml orders them) that includes
  the image and whose `pgathondu/<suffix>:<source_sha>` tag Docker Hub has.
  A changeset whose publish failed has no tag, so the one before it is used.
- **A some-ui image** (`hub`): the most recently pushed tag of the shape its
  release publishes (a whole commit for render), never `latest`. render is
  followed since R4 (#793), once `file-host` is pinned by digest and so the
  set check (`scripts/check_release.py`, R3 #790) reads both ends of its
  requirement (RL7). www and resume-render are run on home since R8 (#853)
  and pinned by digest, but not followed: neither image writes its labels
  yet, and RL7 follows a some-ui image only once it does.

- **A third-party image** (`THIRD_PARTY`, R7 #796), only with
  `--third-party`: the current digest of the tag its entry already names (a
  real version, never `latest`). Which tag is a reviewed change to the
  release; the refresh moves only the digest under it. It is its own mode,
  so a third-party move is its own release and never rides with an app
  image (C120). It is due (`--third-party-due`) when no commit on main in
  the last REFRESH_DAYS changed a third-party entry: derived from main's
  history, never from what started the run, so a run that is cancelled or
  fails loses nothing (C347).

Docker Hub's tag API gives each tag's digest (another registry's v2 API, for
an image named with its host), so every reference is `repo:tag@sha256:…`.
What it never does:

- **Moves a held entry** (`HELD`: www and resume-render, until their
  images write the labels RL7 needs, #853).
- **Moves an entry it does not follow**, or adds or removes a service. The
  default mode moves only `SOURCES`; `--third-party` only `THIRD_PARTY`.
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
import urllib.parse
import urllib.request
from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
RELEASE = REPO_ROOT / "deploy" / "home" / "release.json"
CHANGESETS = REPO_ROOT / ".github" / "docker-changesets"
HUB = "https://hub.docker.com/v2/repositories"

# Entries no source moves, and why. Each names the story that frees it.
HELD: dict[str, str] = {
    # R8 (#853): home runs both, and a reviewed change moves each pin until
    # its image writes the set check's labels (docs/release.md, RL7).
    "resume-render": "held until pgathondu/resume-render writes io.paulgsc.provides (RL7, #853)",
    "www": "held until pgathondu/www writes io.paulgsc.requires (RL7, #853)",
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


COMMIT = r"[0-9a-f]{40}"

# Every service the release follows, and where its newest reference comes
# from. A service not here keeps whatever main's release says.

SOURCES: dict[str, Changesets | Hub] = {
    # Since R4 (#793): envctl deploy applies whole releases by digest, so
    # file_host is followed like any server image.
    "dashboard-builder": Changesets("dashboard-builder"),
    "file-host": Changesets("server"),
    "kokoro": Changesets("kokoro"),
    "orchestrator": Changesets("orchestrator"),
    # Since R4 (#793): file-host is pinned by digest, so the release pull
    # request's set check reads both ends of render's requirement (RL7).
    "render": Hub("pgathondu/render", COMMIT),
    "rust-runner": Changesets("rust-runner"),
    # Not www or resume-render: HELD (above) until they write their labels.
}

# Every image home runs that nobody here publishes (R7, #796; C120). Each
# entry is `repo:<real tag>@sha256:…`, the tag the current major or minor
# version where the publisher has one, else the exact version, and only
# `--third-party` moves it: to that tag's current digest. caddy runs at NixOS
# level and is not in the release (#787, decision 6).
THIRD_PARTY = frozenset(
    {
        "blackbox-exporter",
        "cadvisor",
        "grafana",
        "metabase",
        "nats",
        "nats-exporter",
        "node_exporter",
        "process-exporter",
        "prometheus",
        "redis",
        "redis-exporter",
        "redisinsight",
    }
)
# `[host[:port]/]path:tag[@digest]`; the tag is what follows the last colon
# after the last slash, so a registry's port is never read as one.
# How long a third-party digest stays before a refresh is due (C120's
# monthly cadence, C347).
REFRESH_DAYS = 30
TAGGED = re.compile(r"(?P<repository>(?:[a-z0-9.-]+(?::[0-9]+)?/)?[a-z0-9][a-z0-9._/-]*):(?P<tag>[A-Za-z0-9_][A-Za-z0-9_.-]*)(@sha256:[0-9a-f]{64})?")
# Docker Hub's own names for itself, which its tag API takes without.
HUB_HOSTS = ("docker.io", "index.docker.io", "registry-1.docker.io")


def located(repository: str) -> tuple[str | None, str]:
    """Where a repository's tags are read: (None, Docker Hub path) or (registry host, name).

    A first component with a dot or a port is a registry host, as Docker reads
    it; Docker Hub's official images are `library/<name>`.
    """
    first, _, rest = repository.partition("/")
    if rest and ("." in first or ":" in first) and first not in HUB_HOSTS:
        return first, rest
    path = rest if rest and first in HUB_HOSTS else repository
    return None, path if "/" in path else "library/" + path



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


# What a registry answers a manifest request with: a multi-platform index
# first, so a digest is the one `docker pull` resolves, as Docker Hub's is.
MANIFESTS = ", ".join(
    (
        "application/vnd.oci.image.index.v1+json",
        "application/vnd.docker.distribution.manifest.list.v2+json",
        "application/vnd.oci.image.manifest.v1+json",
        "application/vnd.docker.distribution.manifest.v2+json",
    )
)


def registry_digest(host: str, name: str, tag: str) -> str | None:
    """A tag's digest from a v2 registry (gcr.io, ghcr.io), anonymously; None when absent."""

    def head(token: str | None) -> urllib.request.Request:
        headers = {"Accept": MANIFESTS, "User-Agent": "release-lock"}
        if token:
            headers["Authorization"] = f"Bearer {token}"
        return urllib.request.Request(f"https://{host}/v2/{name}/manifests/{tag}", headers=headers, method="HEAD")

    try:
        try:
            with urllib.request.urlopen(head(None), timeout=30) as response:
                return response.headers.get("Docker-Content-Digest")
        except urllib.error.HTTPError as err:
            challenge = err.headers.get("WWW-Authenticate", "") if err.code == 401 else ""
            if not challenge.startswith("Bearer "):
                raise
        # An anonymous pull token, as `docker pull` gets one.
        fields = dict(re.findall(r'(\w+)="([^"]*)"', challenge))
        query = urllib.parse.urlencode({key: fields[key] for key in ("service", "scope") if key in fields})
        with urllib.request.urlopen(urllib.request.Request(f"{fields.get('realm', '')}?{query}", headers={"User-Agent": "release-lock"}), timeout=30) as response:
            token = json.load(response).get("token")
        with urllib.request.urlopen(head(token), timeout=30) as response:
            return response.headers.get("Docker-Content-Digest")
    except urllib.error.HTTPError as err:
        if err.code == 404:
            return None
        raise Unreadable(f"{host} answered {err.code} for {name}:{tag}") from err
    except (urllib.error.URLError, TimeoutError, ValueError) as err:
        raise Unreadable(f"{host} could not be read for {name}:{tag}: {err}") from err


def third_party_moved_since(repo: Path, days: int, release: Path = RELEASE) -> bool:
    """Whether a commit on main in the last `days` changed a third-party entry (git alone)."""
    lines = "|".join(re.escape(json.dumps(name)) for name in sorted(THIRD_PARTY))
    done = subprocess.run(
        ["git", "log", f"--since={days} days ago", "--format=%H", "-G", f"^ *({lines}): ", "HEAD", "--", str(release.relative_to(repo))],
        cwd=repo,
        capture_output=True,
        text=True,
        check=False,
    )
    if done.returncode != 0:
        raise Unreadable(f"git log failed: {done.stderr.strip()}")
    return bool(done.stdout.strip())


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
        host, path = located(repository)
        if host is not None:
            return registry_digest(host, path, name)
        found = hub_get(f"{path}/tags/{name}")
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


def refresh_third_party(release: dict, world: World, server_commit: str, names: frozenset[str] | None = None) -> tuple[dict, list[str]]:
    """main's release with each third-party entry at its tag's current digest, and what moved."""
    names = THIRD_PARTY if names is None else names
    services = {name: dict(entry) for name, entry in release["services"].items()}
    said: list[str] = []
    for name in sorted(names):
        if name not in services:
            raise Unreadable(f"{name} is third-party, and the release does not name it")
        pinned = TAGGED.fullmatch(services[name]["image"])
        if pinned is None or pinned["tag"] == "latest":
            raise Unreadable(f"{name} names {services[name]['image']}: a third-party entry is repo:<a real tag>, never latest")
        digest = world.tag(pinned["repository"], pinned["tag"])
        if digest is None:
            raise Unreadable(f"{name}'s tag {pinned['repository']}:{pinned['tag']} is not published")
        wanted = f"{pinned['repository']}:{pinned['tag']}@{digest}"
        if wanted == services[name]["image"]:
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
    parser.add_argument("--third-party", action="store_true", help="refresh the third-party pins, and nothing else")
    parser.add_argument("--third-party-due", action="store_true", help="print `due` or `not due` for the third-party refresh, from git alone")
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        result = unittest.main(argv=[sys.argv[0]], exit=False, verbosity=1).result
        return 0 if result.wasSuccessful() else 1
    if args.third_party_due:
        try:
            print("not due" if third_party_moved_since(REPO_ROOT, REFRESH_DAYS) else "due")
        except Unreadable as err:
            print(f"release_lock: {err}", file=sys.stderr)
            return 2
        return 0
    try:
        world = real_world(REPO_ROOT)
        release = json.loads(args.release.read_text())
        form = refresh_third_party if args.third_party else reconcile
        updated, said = form(release, world, world.order[-1] if world.order else "")
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

    def test_file_host_is_followed_by_digest_since_r4(self) -> None:
        seen = world({("pgathondu/server", A): DA}, [changeset(A, "server")], [A])
        updated, said = reconcile(sample(), seen, "main", {"file-host": SOURCES["file-host"]})
        self.assertEqual(updated["services"]["file-host"]["image"], f"pgathondu/server:{A}@{DA}")
        self.assertEqual(len(said), 1)
        self.assertNotIn("file-host", HELD)

    def test_a_hub_source_takes_the_newest_tag_of_its_shape_never_latest(self) -> None:
        tags = {"pgathondu/render": [("latest", DC), ("v-test", DC), (B, DB), (A, DA)]}
        updated, said = reconcile(sample(), world({}, [], [A], tags), "main", {"render": Hub("pgathondu/render", COMMIT)})
        self.assertEqual(updated["services"]["render"]["image"], f"pgathondu/render:{B}@{DB}")
        self.assertEqual(len(said), 1)

    def test_www_and_resume_render_wait_for_their_labels(self) -> None:
        """RL7 (#853): pinned in the release, not followed until their images write the labels."""
        release = json.loads(RELEASE.read_text())["services"]
        for name in ("www", "resume-render"):
            self.assertIn("@sha256:", release[name]["image"])
            self.assertNotIn(name, SOURCES)
            self.assertIn(name, HELD)

    def test_an_unfollowed_entry_is_never_moved(self) -> None:
        seen = world({("pgathondu/rust-runner", A): DA}, [changeset(A, "rust-runner")], [A], {"redis": [("7.4", DA)]})
        updated, _ = reconcile(sample(), seen, "main", {"rust-runner": Changesets("rust-runner")})
        self.assertEqual(updated["services"]["redis"]["image"], "redis:latest")
        self.assertEqual(updated["services"]["render"]["image"], f"pgathondu/render:{A}@{DA}")

    def test_following_a_service_the_release_lacks_is_unreadable(self) -> None:
        with self.assertRaises(Unreadable):
            reconcile(sample(), world({}, [], []), "main", {"ghost": Changesets("ghost")})

    def test_render_is_followed_once_file_host_is_pinned(self) -> None:
        """RL7: render is followed only while file-host is pinned by digest, so the set check reads both ends."""
        file_host = json.loads(RELEASE.read_text())["services"]["file-host"]["image"]
        self.assertIn("@sha256:", file_host)
        self.assertEqual(SOURCES["render"], Hub("pgathondu/render", COMMIT))
        self.assertEqual(SOURCES["file-host"], Changesets("server"))

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
        # The suffixes of the held entries' repositories (file-host's is server).
        held = {release["services"][name]["image"].split("@")[0].rsplit(":", 1)[0].removeprefix("pgathondu/") for name in HELD}
        for suffix in suffixes:
            runs = any(entry["image"].startswith(f"pgathondu/{suffix}:") for entry in release["services"].values())
            if runs or suffix in followed:
                self.assertIn(suffix, followed | held, f"pgathondu/{suffix} is published and home runs it, so the release follows it")
        self.assertIn("kokoro", followed)

    def test_the_third_party_refresh_moves_only_the_digest_under_each_tag(self) -> None:
        before = sample()
        before["services"]["redis"]["image"] = f"redis:8.10@{DA}"
        seen = world({("redis", "8.10"): DB, ("pgathondu/rust-runner", A): DA}, [changeset(A, "rust-runner")], [A])
        updated, said = refresh_third_party(before, seen, "main", frozenset({"redis"}))
        self.assertEqual(updated["services"]["redis"]["image"], f"redis:8.10@{DB}")
        self.assertEqual(updated["services"]["rust-runner"], before["services"]["rust-runner"], "an app image never moves with a third-party one")
        self.assertEqual((updated["serial"], len(said)), (5, 1))
        again, said = refresh_third_party(updated, seen, "m2", frozenset({"redis"}))
        self.assertIs(again, updated, "idempotent")
        self.assertEqual(said, [])

    def test_an_app_reconcile_never_moves_a_third_party_entry(self) -> None:
        before = sample()
        before["services"]["redis"]["image"] = f"redis:8.10@{DA}"
        seen = world({("redis", "8.10"): DB, ("pgathondu/rust-runner", A): DA}, [changeset(A, "rust-runner")], [A])
        updated, _ = reconcile(before, seen, "main", {"rust-runner": Changesets("rust-runner")})
        self.assertEqual(updated["services"]["redis"]["image"], f"redis:8.10@{DA}")

    def test_a_third_party_entry_on_latest_or_an_unpublished_tag_is_unreadable(self) -> None:
        for image, published in (("redis:latest", {("redis", "latest"): DA}), ("redis", {}), ("redis:9.9", {})):
            before = sample()
            before["services"]["redis"]["image"] = image
            with self.assertRaises(Unreadable, msg=image):
                refresh_third_party(before, world(published, [], []), "main", frozenset({"redis"}))

    def test_a_third_party_refresh_is_due_only_after_a_quiet_month(self) -> None:
        import tempfile
        import time

        with tempfile.TemporaryDirectory() as scratch:
            repo = Path(scratch)
            release = repo / "deploy" / "home" / "release.json"
            release.parent.mkdir(parents=True)

            def commit(services: dict, days_ago: int) -> None:
                release.write_text(render({"serial": 1, "server_commit": "", "services": services}))
                stamp = f"@{int(time.time()) - days_ago * 86400} +0000"
                env = {"GIT_AUTHOR_DATE": stamp, "GIT_COMMITTER_DATE": stamp, "GIT_AUTHOR_NAME": "t", "GIT_AUTHOR_EMAIL": "t@t", "GIT_COMMITTER_NAME": "t", "GIT_COMMITTER_EMAIL": "t@t", "PATH": "/usr/bin:/bin"}
                subprocess.run(["git", "add", "-A"], cwd=repo, check=True, env=env)
                subprocess.run(["git", "commit", "-qm", "r"], cwd=repo, check=True, env=env)

            subprocess.run(["git", "init", "-q"], cwd=repo, check=True)
            commit({"redis": {"image": f"redis:8.10@{DA}"}, "rust-runner": {"image": "pgathondu/rust-runner:a"}}, 400)
            commit({"redis": {"image": f"redis:8.10@{DB}"}, "rust-runner": {"image": "pgathondu/rust-runner:a"}}, 31)
            # An app move inside the window does not count.
            commit({"redis": {"image": f"redis:8.10@{DB}"}, "rust-runner": {"image": "pgathondu/rust-runner:b"}}, 1)
            self.assertFalse(third_party_moved_since(repo, 30, release), "a third-party move 31 days ago, an app move since: due")
            # A serial and server_commit change alone (render writes both) is no move.
            release.write_text(render({"serial": 2, "server_commit": "x", "services": {"redis": {"image": f"redis:8.10@{DB}"}, "rust-runner": {"image": "pgathondu/rust-runner:b"}}}))
            subprocess.run(["git", "commit", "-qam", "s"], cwd=repo, check=True, env={"GIT_AUTHOR_NAME": "t", "GIT_AUTHOR_EMAIL": "t@t", "GIT_COMMITTER_NAME": "t", "GIT_COMMITTER_EMAIL": "t@t", "PATH": "/usr/bin:/bin"})
            self.assertFalse(third_party_moved_since(repo, 30, release), "only serial and server_commit moved: due")
            # A hyphenated name, which the pattern escapes, counts.
            commit({"redis": {"image": f"redis:8.10@{DB}"}, "redis-exporter": {"image": f"oliver006/redis_exporter:v1@{DA}"}, "rust-runner": {"image": "pgathondu/rust-runner:b"}}, 2)
            self.assertTrue(third_party_moved_since(repo, 30, release), "redis-exporter moved 2 days ago: not due")

    def test_a_repository_is_read_where_docker_pulls_it_from(self) -> None:
        self.assertEqual(located("redis"), (None, "library/redis"))
        self.assertEqual(located("prom/prometheus"), (None, "prom/prometheus"))
        self.assertEqual(located("docker.io/redis"), (None, "library/redis"))
        self.assertEqual(located("docker.io/prom/prometheus"), (None, "prom/prometheus"))
        self.assertEqual(located("gcr.io/cadvisor/cadvisor"), ("gcr.io", "cadvisor/cadvisor"))
        self.assertEqual(located("localhost:5000/tools/x"), ("localhost:5000", "tools/x"))

    def test_a_registry_port_is_never_read_as_the_tag(self) -> None:
        pinned = TAGGED.fullmatch(f"localhost:5000/tools/x:1.2@{DA}")
        assert pinned is not None
        self.assertEqual((pinned["repository"], pinned["tag"]), ("localhost:5000/tools/x", "1.2"))
        self.assertIsNone(TAGGED.fullmatch("localhost:5000/tools/x"), "a port is no tag")

    def test_every_image_nobody_here_publishes_is_third_party_and_pinned(self) -> None:
        """No `:latest` and no bare tag among them (R7, #796); file-host is RL5's to free."""
        release = json.loads(RELEASE.read_text())
        ours = set(SOURCES) | set(HELD) | {"render"}
        self.assertEqual(set(release["services"]) - ours, set(THIRD_PARTY))
        self.assertFalse(THIRD_PARTY & ours)
        for name in THIRD_PARTY:
            pinned = TAGGED.fullmatch(release["services"][name]["image"])
            self.assertIsNotNone(pinned, name)
            assert pinned is not None
            self.assertNotEqual(pinned["tag"], "latest", name)
            self.assertTrue(release["services"][name]["image"].split("@")[-1].startswith("sha256:"), name)

    def test_the_checked_in_changesets_all_read(self) -> None:
        for found in changesets(CHANGESETS):
            self.assertIn("source_sha", found)


if __name__ == "__main__":
    sys.exit(main())

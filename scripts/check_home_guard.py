#!/usr/bin/env python3
"""Home starts nothing outside its release (RELEASE R6, #795).

Home runs one release, `deploy/home/release.json`, applied whole by
`make home-deploy` (docs/release.md, RL1-RL8). This refuses every way back to
a combinatorial space, where one image could be pulled, built or floated on
its own:

- a service in the home include set (docker-compose.yml's `include:`) whose
  `image:` is not `${IMAGE_<SERVICE>:?...}`, filled from the release;
- a `build:` in the home include set: local builds live in
  `infra/compose/dev.build.yml`, which the home role never loads;
- a release entry without a digest;
- a `:latest` in the home include set, the release, or the NixOS modules;
- a literal `image = "..."` in the NixOS modules, which read theirs from the
  release (C127);
- a README or docs/ line telling someone to pull, build, or rebuild-and-start
  a service by hand (`make compose ARGS="pull ..."`, `docker compose pull`,
  `up --build`), when `make home-deploy` is the one command.

`envctl compose --role home` refuses the same commands at run time
(`apps/envctl/src/compose.rs`, `home_refusal`).

    python3 scripts/check_home_guard.py
    python3 scripts/check_home_guard.py --self-test

Exit status: 0 when nothing is refused; 1 naming every refusal. Stdlib only,
line-based: the compose files keep one key per line.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
import unittest
from dataclasses import dataclass
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent

# A service whose `build:` stays until its first publish, and the story that
# frees it. Each entry is a debt with an owner, never a way around the rule:
# the self-test holds the list to these names.
BUILD_UNTIL_PUBLISHED = {
    "dashboard-builder": "until pgathondu/dashboard-builder's first publish (R6a, #868): then compose reads ${IMAGE_DASHBOARD_BUILDER}",
}

INCLUDE = re.compile(r"^\s*-\s*path:\s*(\S+)\s*$")
SERVICE = re.compile(r"^  ([A-Za-z0-9_.-]+):\s*$")
IMAGE = re.compile(r"^\s{4}image:\s*(.*?)\s*$")
BUILD = re.compile(r"^\s{4}build:")
FROM_RELEASE = re.compile(r"^\$\{IMAGE_[A-Z0-9_]+:\?[^}]*\}$")
LATEST = re.compile(r":latest\b")
NIX_LITERAL_IMAGE = re.compile(r"""\bimage\s*=\s*["']""")
BY_HAND = re.compile(r"""ARGS="(pull|build|up[^"]*--build)\b|docker compose (pull|build)\b|docker compose up[^\n`]*--build""")


@dataclass(frozen=True)
class Refusal:
    path: str
    line: int
    why: str

    def __str__(self) -> str:
        return f"{self.path}:{self.line}: {self.why}"


def code(line: str) -> str:
    """A line without its `#` comment (YAML and Nix), crudely: enough for keys."""
    return line.split("#", 1)[0]


def included(compose: str) -> list[str]:
    """The files docker-compose.yml includes, uncommented lines only."""
    return [m.group(1) for line in compose.splitlines() if (m := INCLUDE.match(code(line)))]


def compose_refusals(path: str, text: str, allowed_builds: dict[str, str]) -> list[Refusal]:
    found: list[Refusal] = []
    in_services, service = False, None
    for number, raw in enumerate(text.splitlines(), 1):
        line = code(raw)
        if re.match(r"^\S", line):
            in_services, service = line.startswith("services:"), None
            continue
        if not in_services:
            if LATEST.search(line):
                found.append(Refusal(path, number, "a `:latest` floats: name the release's digest"))
            continue
        if m := SERVICE.match(line):
            service = m.group(1)
            continue
        if m := IMAGE.match(line):
            if not FROM_RELEASE.match(m.group(1)):
                found.append(Refusal(path, number, f"service {service}'s image is `{m.group(1)}`, not ${{IMAGE_<SERVICE>:?...}} from deploy/home/release.json"))
        elif BUILD.match(line) and service not in allowed_builds:
            found.append(Refusal(path, number, f"service {service} builds on home: move its `build:` to infra/compose/dev.build.yml and publish it"))
        if LATEST.search(line):
            found.append(Refusal(path, number, "a `:latest` floats: name the release's digest"))
    return found


def release_refusals(path: str, release: dict) -> list[Refusal]:
    found = []
    for name, entry in sorted(release.get("services", {}).items()):
        image = entry.get("image", "")
        if "@sha256:" not in image:
            found.append(Refusal(path, 0, f"{name} is `{image}`, with no digest: a tag can move under a release (RL4)"))
        if LATEST.search(image.split("@", 1)[0]):
            found.append(Refusal(path, 0, f"{name} is on `latest`"))
    return found


def nix_refusals(path: str, text: str) -> list[Refusal]:
    found = []
    for number, raw in enumerate(text.splitlines(), 1):
        line = code(raw)
        if NIX_LITERAL_IMAGE.search(line):
            found.append(Refusal(path, number, "a literal image: a module reads its image from deploy/home/release.json (C127)"))
        elif LATEST.search(line):
            found.append(Refusal(path, number, "a `:latest` floats: name the release's digest"))
    return found


def docs_refusals(path: str, text: str) -> list[Refusal]:
    return [
        Refusal(path, number, "pulls, builds or rebuilds a service by hand: home deploys with `make home-deploy` alone")
        for number, line in enumerate(text.splitlines(), 1)
        if BY_HAND.search(line)
    ]


def check(root: Path) -> list[Refusal]:
    found: list[Refusal] = []
    for name in included((root / "docker-compose.yml").read_text()):
        path = (root / name).resolve().relative_to(root.resolve())
        found += compose_refusals(str(path), (root / path).read_text(), BUILD_UNTIL_PUBLISHED)
    release = root / "deploy" / "home" / "release.json"
    found += release_refusals(str(release.relative_to(root)), json.loads(release.read_text()))
    for module in sorted((root / "nix" / "modules").rglob("*.nix")):
        # The modules' own test sets a literal image on purpose, to prove a
        # host can't (test.nix, "a host cannot set a module's image").
        if module.name == "test.nix":
            continue
        found += nix_refusals(str(module.relative_to(root)), module.read_text())
    for doc in [root / "README.md", *sorted((root / "docs").rglob("*.md"))]:
        found += docs_refusals(str(doc.relative_to(root)), doc.read_text())
    return found


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        result = unittest.main(argv=[sys.argv[0]], exit=False, verbosity=1).result
        return 0 if result.wasSuccessful() else 1
    found = check(REPO_ROOT)
    for refusal in found:
        print(refusal)
    if found:
        print(f"{len(found)} refused: home starts nothing outside deploy/home/release.json (RELEASE R6, #795)", file=sys.stderr)
        return 1
    print("home starts nothing outside its release: no build, no :latest, no literal image, no pull by hand")
    return 0


# ── Tests ───────────────────────────────────────────────────────────────────

D = "sha256:" + "1" * 64
GOOD_COMPOSE = """services:
  redis:
    image: ${IMAGE_REDIS:?deploy/<role>/release.json sets it}
    container_name: redis
"""


class Tests(unittest.TestCase):
    def test_an_image_from_the_release_passes(self) -> None:
        self.assertEqual(compose_refusals("x.yml", GOOD_COMPOSE, {}), [])

    def test_an_image_written_any_other_way_is_refused(self) -> None:
        for image in ("redis:7", "redis:latest", f"redis@{D}", "${REDIS_IMAGE}", "${IMAGE_REDIS}"):
            found = compose_refusals("x.yml", GOOD_COMPOSE.replace("${IMAGE_REDIS:?deploy/<role>/release.json sets it}", image), {})
            self.assertTrue(any("not ${IMAGE_" in str(r) for r in found), image)

    def test_a_build_is_refused_unless_it_waits_on_a_named_publish(self) -> None:
        text = GOOD_COMPOSE + "    build:\n      context: .\n"
        self.assertTrue(any("builds on home" in str(r) for r in compose_refusals("x.yml", text, {})))
        self.assertEqual(compose_refusals("x.yml", text, {"redis": "until its publish"}), [])

    def test_the_build_debt_is_named_and_owned(self) -> None:
        self.assertEqual(set(BUILD_UNTIL_PUBLISHED), {"dashboard-builder"})
        self.assertTrue(all("#" in why for why in BUILD_UNTIL_PUBLISHED.values()), "each names the story that frees it")

    def test_a_latest_is_refused_in_a_comment_never(self) -> None:
        self.assertTrue(any(":latest" in str(r) for r in compose_refusals("x.yml", GOOD_COMPOSE + "    environment:\n      X: some:latest\n", {})))
        self.assertEqual(compose_refusals("x.yml", GOOD_COMPOSE + "    # was redis:latest\n", {}), [])

    def test_a_release_entry_needs_a_digest_and_no_latest(self) -> None:
        self.assertEqual(release_refusals("r.json", {"services": {"redis": {"image": f"redis:8.10@{D}"}}}), [])
        self.assertEqual(len(release_refusals("r.json", {"services": {"redis": {"image": "redis:8.10"}}})), 1)
        self.assertEqual(len(release_refusals("r.json", {"services": {"redis": {"image": "redis:latest"}}})), 2)
        self.assertEqual(len(release_refusals("r.json", {"services": {"redis": {"image": f"redis:latest@{D}"}}})), 1)

    def test_a_nix_module_names_no_image_of_its_own(self) -> None:
        self.assertEqual(nix_refusals("m.nix", 'image = release.services.${name}.image;\n'), [])
        self.assertEqual(len(nix_refusals("m.nix", '  image = "pgathondu/server:latest";\n')), 1)
        self.assertEqual(len(nix_refusals("m.nix", "  image = 'x';\n")), 1)
        self.assertEqual(nix_refusals("m.nix", '  # image = "old";\n'), [])

    def test_docs_never_pull_or_build_by_hand(self) -> None:
        for line in (
            'make compose ARGS="pull rust-runner"',
            'make compose ARGS="build render"',
            'make compose ARGS="up --build -d render"',
            "docker compose pull",
            "docker compose build kokoro",
            "docker compose up -d --build render",
        ):
            self.assertEqual(len(docs_refusals("README.md", line)), 1, line)
        for line in ("make home-deploy", 'make compose ARGS="ps"', 'make compose ARGS="logs render"', "docker compose ps"):
            self.assertEqual(docs_refusals("README.md", line), [], line)

    def test_only_uncommented_includes_count(self) -> None:
        compose = "include:\n  - path: ./a.yml\n  # - path: ./caddy.yml We set caddy at NIXOS sys level\n"
        self.assertEqual(included(compose), ["./a.yml"])


if __name__ == "__main__":
    sys.exit(main())

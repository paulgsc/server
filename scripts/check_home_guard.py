#!/usr/bin/env python3
"""Home starts nothing outside its release (RELEASE R6, #795; docs/release.md, RL12).

Home runs one release, `deploy/home/release.json`, applied whole by
`make home-deploy` (docs/release.md, RL1-RL12). This refuses every way back to
a combinatorial space, where one image could be pulled, built or floated on
its own:

- a service of the home stack whose `image:` is not `${IMAGE_<SERVICE>:?...}`,
  filled from the release, or that names no image at all;
- a `build:` in the home stack: local builds live in
  `infra/compose/dev.build.yml`, which the home role never loads;
- a `:latest` anywhere in the home stack, the release, or the NixOS modules;
- a release entry without a digest;
- an image a NixOS module names itself (C127);
- a README or docs line telling someone to pull, build, watch or
  rebuild-and-start by hand, when `make home-deploy` is the one command.

The home stack is read as compose itself reads it,
`docker compose config --no-interpolate --format json` from the repository
root, the call envctl makes (`apps/envctl/src/compose.rs`, `config`): every
include, nested or short, every anchor, merge and `extends:` resolved, so
nothing is parsed by hand and nothing passes for being written oddly.
`envctl compose --role home` refuses the matching commands at run time
(`home_refusal`).

    python3 scripts/check_home_guard.py
    python3 scripts/check_home_guard.py --self-test

Needs docker compose (no daemon). Exit status: 0 when nothing is refused; 1
naming every refusal; 2 when compose cannot read the stack.
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
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

FROM_RELEASE = re.compile(r"^\$\{IMAGE_[A-Z0-9_]+:\?[^}]*\}$")
LATEST = re.compile(r":latest\b")
# An image a module names itself: `image = "..."`, the string on the next
# line, or through mkDefault/mkForce/mkOverride, with or without `lib.` and
# parentheses.
NIX_IMAGE = re.compile(r"\bimage\s*=\s*(.*)$")
NIX_LITERAL = re.compile(r"""^(?:(?:lib\.)?mk(?:Default|Force|Override\s+\d+)\s+)?\(?\s*["']""")
# Pulling, building, watching or rebuilding by hand, through make or compose,
# whatever options come before the command. A word ends at a space, a quote
# or the line: `pull-worker` and `--build-arg` are other words.
END = r"""(?=["'\s`]|$)"""
OPTION = r"(?:-{1,2}[\w-]+(?:[= ][^\s\"'`-]\S*)?\s+)*"
BY_HAND = re.compile(
    rf"""ARGS=["']?{OPTION}(?:pull|build|watch){END}|ARGS=["'][^"']*\s--(?:build|watch){END}"""
    rf"""|docker[ -]compose\s+{OPTION}(?:pull|build|watch){END}|docker[ -]compose\b[^\n`]*\s--(?:build|watch){END}"""
)


@dataclass(frozen=True)
class Refusal:
    path: str
    line: int
    why: str

    def __str__(self) -> str:
        return f"{self.path}:{self.line}: {self.why}" if self.line else f"{self.path}: {self.why}"


class Unreadable(Exception):
    pass


def compose_config(root: Path) -> dict:
    """The home stack as compose reads it: docker-compose.yml and all it includes."""
    done = subprocess.run(
        ["docker", "compose", "--env-file", "/dev/null", "config", "--no-interpolate", "--format", "json"],
        cwd=root,
        capture_output=True,
        text=True,
        check=False,
    )
    if done.returncode != 0:
        raise Unreadable(f"docker compose config failed: {done.stderr.strip() or done.returncode}")
    return json.loads(done.stdout)


def strings(value: object):
    """Every string in a JSON value."""
    if isinstance(value, str):
        yield value
    elif isinstance(value, dict):
        for item in value.values():
            yield from strings(item)
    elif isinstance(value, list):
        for item in value:
            yield from strings(item)


def config_refusals(config: dict, allowed_builds: dict[str, str]) -> list[Refusal]:
    found: list[Refusal] = []
    where = "docker-compose.yml"
    for name, service in sorted(config.get("services", {}).items()):
        image = service.get("image")
        if image is None and "build" not in service:
            found.append(Refusal(where, 0, f"service {name} names no image: it must read ${{IMAGE_<SERVICE>:?...}} from the release"))
        elif image is not None and not FROM_RELEASE.match(image):
            found.append(Refusal(where, 0, f"service {name}'s image is `{image}`, not ${{IMAGE_<SERVICE>:?...}} from deploy/home/release.json"))
        if "build" in service and name not in allowed_builds:
            found.append(Refusal(where, 0, f"service {name} builds on home: move its `build:` to infra/compose/dev.build.yml and publish it"))
        if any(LATEST.search(text) for text in strings(service)):
            found.append(Refusal(where, 0, f"service {name} names a `:latest`, which floats: name the release's digest"))
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


def code(line: str) -> str:
    """A Nix line without its `#` comment."""
    return line.split("#", 1)[0]


def nix_refusals(path: str, text: str) -> list[Refusal]:
    found = []
    lines = [code(raw) for raw in text.splitlines()]
    for number, line in enumerate(lines, 1):
        if m := NIX_IMAGE.search(line):
            value = m.group(1).strip()
            if not value:
                # The value on the next line that says anything.
                value = next((later.strip() for later in lines[number:] if later.strip()), "")
            if NIX_LITERAL.match(value):
                found.append(Refusal(path, number, "a literal image: a module reads its image from deploy/home/release.json (C127)"))
                continue
        if LATEST.search(line):
            found.append(Refusal(path, number, "a `:latest` floats: name the release's digest"))
    return found


def docs_refusals(path: str, text: str) -> list[Refusal]:
    return [
        Refusal(path, number, "pulls, builds, watches or rebuilds by hand: home deploys with `make home-deploy` alone")
        for number, line in enumerate(text.splitlines(), 1)
        if BY_HAND.search(line)
    ]


def check(root: Path, config: dict) -> list[Refusal]:
    found = config_refusals(config, BUILD_UNTIL_PUBLISHED)
    release = root / "deploy" / "home" / "release.json"
    found += release_refusals(str(release.relative_to(root)), json.loads(release.read_text()))
    for module in sorted((root / "nix" / "modules").rglob("*.nix")):
        # The modules' own test sets a literal image on purpose, to prove a
        # host can't (test.nix, "a host cannot set a module's image").
        if module.name == "test.nix":
            continue
        found += nix_refusals(str(module.relative_to(root)), module.read_text())
    docs = [root / "README.md", *sorted((root / "docs").rglob("*.md")), *sorted((root / "apps").rglob("*.md")), *sorted((root / "infra").rglob("*.md"))]
    for doc in docs:
        found += docs_refusals(str(doc.relative_to(root)), doc.read_text())
    return found


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        result = unittest.main(argv=[sys.argv[0]], exit=False, verbosity=1).result
        return 0 if result.wasSuccessful() else 1
    try:
        config = compose_config(REPO_ROOT)
    except (Unreadable, ValueError) as err:
        print(f"check_home_guard: {err}", file=sys.stderr)
        return 2
    found = check(REPO_ROOT, config)
    for refusal in found:
        print(refusal)
    if found:
        print(f"{len(found)} refused: home starts nothing outside deploy/home/release.json (RELEASE R6, #795)", file=sys.stderr)
        return 1
    print(f"home starts nothing outside its release: {len(config.get('services', {}))} services read, no build, no :latest, no literal image, no pull by hand")
    return 0


# ── Tests ───────────────────────────────────────────────────────────────────

D = "sha256:" + "1" * 64
FROM = "${IMAGE_REDIS:?deploy/<role>/release.json sets it}"


def stack(**services: dict) -> dict:
    return {"services": services}


class Tests(unittest.TestCase):
    def test_an_image_from_the_release_passes(self) -> None:
        self.assertEqual(config_refusals(stack(redis={"image": FROM}), {}), [])

    def test_an_image_written_any_other_way_is_refused(self) -> None:
        for image in ("redis:7", "redis:latest", f"redis@{D}", "${REDIS_IMAGE}", "${IMAGE_REDIS}"):
            found = config_refusals(stack(redis={"image": image}), {})
            self.assertTrue(any("not ${IMAGE_" in str(r) for r in found), image)

    def test_a_service_with_no_image_is_refused(self) -> None:
        self.assertTrue(any("names no image" in str(r) for r in config_refusals(stack(x={"container_name": "x"}), {})))

    def test_a_build_is_refused_unless_it_waits_on_a_named_publish(self) -> None:
        build = {"build": {"context": "."}}
        self.assertTrue(any("builds on home" in str(r) for r in config_refusals(stack(redis={"image": FROM, **build}), {})))
        self.assertEqual(config_refusals(stack(redis=build), {"redis": "until its publish"}), [])

    def test_the_build_debt_is_named_and_owned(self) -> None:
        self.assertEqual(set(BUILD_UNTIL_PUBLISHED), {"dashboard-builder"})
        self.assertTrue(all("#" in why for why in BUILD_UNTIL_PUBLISHED.values()), "each names the story that frees it")

    def test_a_latest_anywhere_in_a_service_is_refused(self) -> None:
        found = config_refusals(stack(x={"image": FROM, "environment": {"SIDECAR": "some:latest"}}), {})
        self.assertTrue(any(":latest" in str(r) for r in found), found)

    def test_a_release_entry_needs_a_digest_and_no_latest(self) -> None:
        self.assertEqual(release_refusals("r.json", {"services": {"redis": {"image": f"redis:8.10@{D}"}}}), [])
        self.assertEqual(len(release_refusals("r.json", {"services": {"redis": {"image": "redis:8.10"}}})), 1)
        self.assertEqual(len(release_refusals("r.json", {"services": {"redis": {"image": "redis:latest"}}})), 2)
        self.assertEqual(len(release_refusals("r.json", {"services": {"redis": {"image": f"redis:latest@{D}"}}})), 1)

    def test_a_nix_module_names_no_image_of_its_own(self) -> None:
        self.assertEqual(nix_refusals("m.nix", "image = release.services.${name}.image;\n"), [])
        self.assertEqual(nix_refusals("m.nix", "  image =\n    release.services.${name}.image\n"), [])
        self.assertEqual(nix_refusals("m.nix", "  image = lib.mkOption {\n"), [])
        self.assertEqual(nix_refusals("m.nix", '  # image = "old";\n'), [])
        for literal in (
            '  image = "pgathondu/server:latest";\n',
            "  image = 'x';\n",
            '  image =\n    "pgathondu/x:1";\n',
            '  image = lib.mkDefault "pgathondu/x:1";\n',
            '  image = mkForce "pgathondu/x:1";\n',
            '  image = lib.mkDefault ("pgathondu/x:1");\n',
        ):
            self.assertEqual(len(nix_refusals("m.nix", literal)), 1, literal)

    def test_docs_never_pull_build_or_watch_by_hand(self) -> None:
        for line in (
            'make compose ARGS="pull rust-runner"',
            'make compose ARGS="build render"',
            'make compose ARGS="up --build -d render"',
            "make compose ARGS='pull'",
            "make compose ARGS=pull",
            'make compose ARGS="-p x pull"',
            'make compose ARGS="watch"',
            "docker compose pull",
            "docker compose build kokoro",
            "docker compose up -d --build render",
            "docker compose -f x.yml pull",
            "docker compose --profile t build",
            "docker-compose up -d --build file-host",
        ):
            self.assertEqual(len(docs_refusals("README.md", line)), 1, line)
        for line in (
            "make home-deploy",
            'make compose ARGS="ps"',
            'make compose ARGS="logs render"',
            'make compose ARGS="logs pull-worker"',
            'make compose ARGS="up -d"',
            "make compose ARGS=pull-thing",
            "docker compose ps",
            "docker compose run --build-arg X=1 tool",
        ):
            self.assertEqual(docs_refusals("README.md", line), [], line)

    def test_the_real_home_stack_is_read_whole_by_compose(self) -> None:
        """Never vacuous: compose reads every home service, each from the release, dashboard-builder the one build owed."""
        config = compose_config(REPO_ROOT)
        services = config.get("services", {})
        self.assertGreaterEqual(len(services), 17)
        found = config_refusals(config, {})
        builds = [r.why for r in found if "builds on home" in r.why]
        self.assertEqual(len(builds), 1, builds)
        self.assertIn("dashboard-builder", builds[0])
        self.assertEqual([r for r in found if "builds on home" not in r.why], [], "every image is the release's")


if __name__ == "__main__":
    sys.exit(main())

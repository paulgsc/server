#!/usr/bin/env python3
"""Home starts nothing outside its release (RELEASE R6, #795).

Home runs one release, `deploy/home/release.json`, applied whole by
`make home-deploy` (docs/release.md, RL1-RL8). This refuses every way back to
a combinatorial space, where one image could be pulled, built or floated on
its own:

- a service in the home include set (docker-compose.yml and every file it
  includes, long or short form, nested, as envctl reads them) whose
  `image:` is not `${IMAGE_<SERVICE>:?...}`, filled from the release, or
  that has no image at all;
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
line-based: the compose files keep one key per line, and a file whose
services it cannot read is refused rather than passed.
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

# An include, long form (`- path: x.yml`) or short (`- x.yml`), as envctl's
# `compose::included` reads it.
INCLUDE = re.compile(r"^\s+-\s+(?:path:\s*)?['\"]?([^\s'\"]+\.ya?ml)['\"]?\s*$")
KEY = re.compile(r"^(\s*)([A-Za-z0-9_.-]+):(.*)$")
FROM_RELEASE = re.compile(r"^\$\{IMAGE_[A-Z0-9_]+:\?[^}]*\}$")
LATEST = re.compile(r":latest\b")
# An image a module names itself: `image = "..."`, the string on the next
# line, or through lib.mkDefault/mkForce/mkOverride.
NIX_IMAGE = re.compile(r"\bimage\s*=\s*(.*)$")
NIX_LITERAL = re.compile(r"""^(?:lib\.mk(?:Default|Force|Override\s+\d+)\s+)?["']""")
# Pulling, building, watching or rebuilding by hand, through make or compose,
# whatever flags come before the command.
FLAGS = r"(?:-{1,2}[\w-]+(?:[= ]\S+)?\s+)*"
BY_HAND = re.compile(
    rf"""ARGS=["']?{FLAGS}(?:pull|build|watch)\b|ARGS=["'][^"']*--(?:build|watch)\b"""
    rf"""|docker[ -]compose\s+{FLAGS}(?:pull|build|watch)\b|docker[ -]compose\b[^\n`]*\s--(?:build|watch)\b"""
)


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
    """The files a compose file's top-level `include:` names, in order."""
    inside, found = False, []
    for raw in compose.splitlines():
        line = code(raw)
        if not line.strip():
            continue
        if not line[0].isspace():
            inside = line.rstrip() == "include:"
            continue
        if inside and (m := INCLUDE.match(line)):
            found.append(m.group(1))
    return found


def home_files(root: Path) -> list[Path]:
    """docker-compose.yml and every file it includes, nested, as envctl's `compose::files`."""
    files: list[Path] = []
    pending = [root / "docker-compose.yml"]
    while pending:
        path = pending.pop(0).resolve()
        if path in files:
            continue
        files.append(path)
        pending += [path.parent / name for name in included(path.read_text())]
    return files


def compose_refusals(path: str, text: str, allowed_builds: dict[str, str]) -> list[Refusal]:
    """Each service's image and build, read by indentation whatever its width."""
    found: list[Refusal] = []
    in_services = False
    service_indent: int | None = None
    service: tuple[str, int] | None = None
    seen: dict[str, dict[str, int]] = {}
    for number, raw in enumerate(text.splitlines(), 1):
        line = code(raw).rstrip()
        if LATEST.search(line):
            found.append(Refusal(path, number, "a `:latest` floats: name the release's digest"))
        if not line.strip():
            continue
        indent = len(line) - len(line.lstrip())
        if indent == 0:
            in_services = line.startswith("services:")
            service_indent, service = None, None
            continue
        m = KEY.match(line)
        if not in_services or not m:
            continue
        if service_indent is None:
            service_indent = indent
        if indent == service_indent:
            service = (m.group(2), number)
            seen[m.group(2)] = {}
        elif service and indent > service_indent and m.group(2) in ("image", "build", "extends") and m.group(2) not in seen[service[0]]:
            # Only the service's own keys: the first deeper level seen.
            depth = seen[service[0]].setdefault("_depth", indent)
            if indent == depth:
                seen[service[0]][m.group(2)] = number
                value = m.group(3).strip().strip("'\"")
                if m.group(2) == "image" and not FROM_RELEASE.match(value):
                    found.append(Refusal(path, number, f"service {service[0]}'s image is `{value}`, not ${{IMAGE_<SERVICE>:?...}} from deploy/home/release.json"))
                elif m.group(2) == "build" and service[0] not in allowed_builds:
                    found.append(Refusal(path, number, f"service {service[0]} builds on home: move its `build:` to infra/compose/dev.build.yml and publish it"))
                elif m.group(2) == "extends":
                    found.append(Refusal(path, number, f"service {service[0]} extends another: its image is not read here, so write it out"))
        elif service and indent > service_indent:
            seen[service[0]].setdefault("_depth", indent)
    if "services:" in [code(l).rstrip() for l in text.splitlines()] and not seen:
        found.append(Refusal(path, 0, "its services could not be read, so none was checked: keep one key per line"))
    for name, keys in seen.items():
        if "image" not in keys and "build" not in keys and "extends" not in keys:
            found.append(Refusal(path, 0, f"service {name} names no image: it must read ${{IMAGE_<SERVICE>:?...}} from the release"))
        elif "image" not in keys and "build" in keys and name in allowed_builds:
            continue
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
        Refusal(path, number, "pulls, builds or rebuilds a service by hand: home deploys with `make home-deploy` alone")
        for number, line in enumerate(text.splitlines(), 1)
        if BY_HAND.search(line)
    ]


def check(root: Path) -> list[Refusal]:
    found: list[Refusal] = []
    for path in home_files(root):
        found += compose_refusals(str(path.relative_to(root.resolve())), path.read_text(), BUILD_UNTIL_PUBLISHED)
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

    def test_includes_are_read_as_envctl_reads_them(self) -> None:
        compose = "include:\n  - ./short.yml\n  - path: ./long.yml\nservices:\n  x:\n    env_file:\n      - path: ./not-an-include.yml\n"
        self.assertEqual(included(compose), ["./short.yml", "./long.yml"])

    def test_every_home_file_is_read_nested_and_the_root_s_own_services(self) -> None:
        import tempfile

        with tempfile.TemporaryDirectory() as scratch:
            root = Path(scratch)
            (root / "infra").mkdir()
            (root / "docker-compose.yml").write_text("include:\n  - ./infra/a.yml\nservices:\n  root:\n    image: root:1\n")
            (root / "infra" / "a.yml").write_text("include:\n  - b.yml\n" + GOOD_COMPOSE)
            (root / "infra" / "b.yml").write_text("services:\n  b:\n    build: .\n")
            self.assertEqual([path.name for path in home_files(root)], ["docker-compose.yml", "a.yml", "b.yml"])
            found = [str(r) for path in home_files(root) for r in compose_refusals(path.name, path.read_text(), {})]
            self.assertTrue(any("service root's image is `root:1`" in r for r in found), found)
            self.assertTrue(any("service b builds on home" in r for r in found), found)

    def test_a_file_indented_otherwise_is_read_not_passed(self) -> None:
        text = "services:\n    redis:\n        image: redis:7\n        build: .\n"
        found = [str(r) for r in compose_refusals("x.yml", text, {})]
        self.assertTrue(any("image is `redis:7`" in r for r in found), found)
        self.assertTrue(any("builds on home" in r for r in found), found)

    def test_a_service_s_nested_keys_are_not_its_own(self) -> None:
        text = GOOD_COMPOSE + "    deploy:\n      resources:\n        image: not-a-key-of-the-service\n"
        self.assertEqual(compose_refusals("x.yml", text, {}), [])

    def test_a_service_with_no_image_or_one_it_inherits_is_refused(self) -> None:
        found = [str(r) for r in compose_refusals("x.yml", "services:\n  x:\n    container_name: x\n", {})]
        self.assertTrue(any("names no image" in r for r in found), found)
        found = [str(r) for r in compose_refusals("x.yml", "services:\n  x:\n    extends:\n      file: y.yml\n", {})]
        self.assertTrue(any("extends another" in r for r in found), found)

    def test_a_quoted_image_from_the_release_still_passes(self) -> None:
        text = GOOD_COMPOSE.replace("image: ${IMAGE_REDIS:?deploy/<role>/release.json sets it}", "image: '${IMAGE_REDIS:?x}'")
        self.assertEqual(compose_refusals("x.yml", text, {}), [])

    def test_a_nix_literal_on_the_next_line_or_through_mk_default_is_refused(self) -> None:
        self.assertEqual(len(nix_refusals("m.nix", '  image =\n    "pgathondu/x:1";\n')), 1)
        self.assertEqual(len(nix_refusals("m.nix", '  image = lib.mkDefault "pgathondu/x:1";\n')), 1)
        self.assertEqual(nix_refusals("m.nix", "  image =\n    release.services.${name}.image\n"), [])

    def test_docs_by_hand_whatever_the_quotes_or_flags(self) -> None:
        for line in (
            "make compose ARGS='pull'",
            "make compose ARGS=pull",
            'make compose ARGS="-p x pull"',
            "docker compose -f x.yml pull",
            "docker compose --profile t build",
            "docker-compose up -d --build file-host",
            'make compose ARGS="watch"',
        ):
            self.assertEqual(len(docs_refusals("README.md", line)), 1, line)

    def test_the_real_home_set_is_read_whole(self) -> None:
        """Never vacuous: every home file is read and its services seen, and dashboard-builder is the one build owed."""
        files = home_files(REPO_ROOT)
        self.assertGreaterEqual(len(files), 10)
        found = [r for path in files for r in compose_refusals(path.name, path.read_text(), {})]
        self.assertFalse([r for r in found if "could not be read" in r.why or "names no image" in r.why], "every service is read")
        builds = [str(r) for r in found if "builds on home" in r.why]
        self.assertEqual(len(builds), 1, builds)
        self.assertIn("dashboard-builder", builds[0])
        # Every image line the files hold is one the parser judged: none refused, so each is the release's.
        images = sum(len(re.findall(r"^\s+image:", path.read_text(), re.M)) for path in files)
        self.assertGreaterEqual(images, 17)
        self.assertFalse([r for r in found if "image is" in r.why], "each image is ${IMAGE_*}")


if __name__ == "__main__":
    sys.exit(main())

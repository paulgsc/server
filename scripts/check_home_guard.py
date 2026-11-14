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
- a README or docs line writing a compose command that `envctl compose
  --role home` refuses (pull, build, watch, `--build`, an `up` of some
  services, another stack), when `make home-deploy` is the one command: the
  doc's command is judged by the same rule, ported word for word
  (`home_refusal`), both sides tested on scripts/home_refusal_cases.json.

The home stack is read as compose itself reads it,
`docker compose config --no-interpolate --format json` from the repository
root, the call envctl makes (`apps/envctl/src/compose.rs`, `config`): every
include, nested or short, every anchor, merge and `extends:` resolved, so
nothing is parsed by hand and nothing passes for being written oddly.
`envctl compose --role home` refuses the same commands at run time.

    python3 scripts/check_home_guard.py
    python3 scripts/check_home_guard.py --self-test

Needs docker compose (no daemon). Exit status: 0 when nothing is refused; 1
naming every refusal; 2 when compose cannot read the stack.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
import unittest
from dataclasses import dataclass
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent


FROM_RELEASE = re.compile(r"^\$\{IMAGE_[A-Z0-9_]+:\?[^}]*\}$")
LATEST = re.compile(r":latest\b")
# An image a module names itself: `image = "..."`, the string on the next
# line, or through mkDefault/mkForce/mkOverride, with or without `lib.` and
# parentheses.
NIX_IMAGE = re.compile(r"\bimage\s*=\s*(.*)$")
NIX_LITERAL = re.compile(r"""^(?:(?:lib\.)?mk(?:Default|Force|Override\s+\d+)\s+)?\(?\s*["']""")
# A command written in a doc: make's `ARGS="..."` (quoted or one word), or
# what follows `docker compose` / `docker-compose` up to the end of a code
# span, a `;`, `&` or `|`, or the line.
DOC_ARGS = re.compile(r"""ARGS=(?:"([^"]*)"|'([^']*)'|(\S+))""")
DOC_COMPOSE = re.compile(r"""docker[ -]compose(?:\s+([^`;&|\n]*))?""")

# The rule `envctl compose --role home` keeps (apps/envctl/src/compose.rs,
# `home_refusal`), word for word: both read scripts/home_refusal_cases.json in
# their tests, so they cannot drift apart.
GLOBAL_LONG = ("--project-name", "--profile", "--env-file", "--ansi", "--progress", "--parallel")
GLOBAL_SHORT = "p"
OTHER_STACK = ("--file", "--project-directory")
UP_LONG = ("--timeout", "--scale", "--pull", "--wait-timeout", "--exit-code-from", "--attach", "--no-attach")
UP_SHORT = "t"
RUN_LONG = ("--env", "--env-from-file", "--volume", "--workdir", "--user", "--publish", "--label", "--name", "--entrypoint", "--cap-add", "--cap-drop", "--pull", "--scale")
RUN_SHORT = "evwupl"
FALSE = ("false", "0", "f", "F", "FALSE", "False")
CASES = REPO_ROOT / "scripts" / "home_refusal_cases.json"


def short(arg: str) -> bool:
    return len(arg) > 1 and arg.startswith("-") and not arg.startswith("--")


def consumes(arg: str, long: tuple[str, ...], shorts: str) -> bool:
    """Whether an option takes the next word as its value (a short cluster by its last letter)."""
    if "=" in arg:
        return False
    return arg[-1] in shorts if short(arg) else arg in long


def home_refusal(args: list[str]) -> str | None:
    """Why `envctl compose --role home` refuses `args` (the words after `docker compose`), if it does."""
    rest = iter(args)
    for arg in rest:
        if not arg.startswith("-"):
            command = arg
            break
        if any(arg == option or arg.startswith(option + "=") for option in OTHER_STACK) or (short(arg) and "f" in arg[1:]):
            return f"another stack (`{arg}`)"
        if consumes(arg, GLOBAL_LONG, GLOBAL_SHORT):
            next(rest, None)
    else:
        return None
    every = list(rest)
    if command == "exec":
        options: list[str] = []
    elif command in ("run", "create"):
        options, words = [], iter(every)
        for arg in words:
            if not arg.startswith("-"):
                break
            options.append(arg)
            if consumes(arg, RUN_LONG, RUN_SHORT):
                next(words, None)
    else:
        options = every

    def set_(flag: str) -> bool:
        return any(arg == flag or (arg.startswith(flag + "=") and arg[len(flag) + 1 :] not in FALSE) for arg in options)

    if command in ("pull", "build", "watch"):
        return f"`compose {command}`"
    if set_("--build"):
        return f"`compose {command} --build`"
    if command != "up":
        return None
    if set_("--watch") or any(short(arg) and "w" in arg[1:] for arg in every):
        return "`compose up --watch`"
    services, words = [], iter(every)
    for arg in words:
        if consumes(arg, UP_LONG, UP_SHORT):
            next(words, None)
        elif not arg.startswith("-"):
            services.append(arg)
    return f"`compose up {' '.join(services)}`" if services else None


def doc_commands(line: str) -> list[list[str]]:
    """Each compose command a doc line writes, as words."""
    found = [(m.group(1) or m.group(2) or m.group(3) or "").split() for m in DOC_ARGS.finditer(line)]
    found += [(m.group(1) or "").split() for m in DOC_COMPOSE.finditer(line)]
    # A sentence's punctuation is not part of the last word.
    return [words[:-1] + [words[-1].rstrip(".,;:)")] if words else words for words in found]

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
    # The repository's own files only: a shell's COMPOSE_FILE would read another stack.
    env = {name: value for name, value in os.environ.items() if name != "COMPOSE_FILE"}
    try:
        done = subprocess.run(
            ["docker", "compose", "--env-file", "/dev/null", "config", "--no-interpolate", "--format", "json"],
            cwd=root,
            capture_output=True,
            text=True,
            check=False,
            env=env,
        )
    except OSError as err:
        raise Unreadable(f"docker compose could not run: {err}") from err
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


def config_refusals(config: dict) -> list[Refusal]:
    found: list[Refusal] = []
    where = "docker-compose.yml"
    for name, service in sorted(config.get("services", {}).items()):
        image = service.get("image")
        if image is None and "build" not in service:
            found.append(Refusal(where, 0, f"service {name} names no image: it must read ${{IMAGE_<SERVICE>:?...}} from the release"))
        elif image is not None and not FROM_RELEASE.match(image):
            found.append(Refusal(where, 0, f"service {name}'s image is `{image}`, not ${{IMAGE_<SERVICE>:?...}} from deploy/home/release.json"))
        if "restart" not in service:
            found.append(Refusal(where, 0, f"service {name} states no `restart:`: one with none reads as `no`, which the deploy judges as a one-shot (RL11)"))
        if "build" in service:
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
    found = []
    for number, line in enumerate(text.splitlines(), 1):
        for words in doc_commands(line):
            if why := home_refusal(words):
                found.append(Refusal(path, number, f"{why} by hand: envctl refuses it on home, which deploys with `make home-deploy` alone"))
                break
    return found


def check(root: Path, config: dict) -> list[Refusal]:
    found = config_refusals(config)
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
    """Services as compose prints them, each with a `restart:` unless it says None."""
    restart = {"restart": "unless-stopped"}
    return {"services": {name: {k: v for k, v in {**restart, **service}.items() if v is not None} for name, service in services.items()}}


class Tests(unittest.TestCase):
    def test_an_image_from_the_release_passes(self) -> None:
        self.assertEqual(config_refusals(stack(redis={"image": FROM})), [])

    def test_a_service_must_state_its_restart(self) -> None:
        found = config_refusals(stack(redis={"image": FROM, "restart": None}))
        self.assertTrue(any("states no `restart:`" in str(r) for r in found), found)
        self.assertEqual(config_refusals(stack(builder={"image": FROM, "restart": "no"})), [])

    def test_an_image_written_any_other_way_is_refused(self) -> None:
        for image in ("redis:7", "redis:latest", f"redis@{D}", "${REDIS_IMAGE}", "${IMAGE_REDIS}"):
            found = config_refusals(stack(redis={"image": image}))
            self.assertTrue(any("not ${IMAGE_" in str(r) for r in found), image)

    def test_a_service_with_no_image_is_refused(self) -> None:
        self.assertTrue(any("names no image" in str(r) for r in config_refusals(stack(x={"container_name": "x"}))))

    def test_a_build_is_refused(self) -> None:
        build = {"build": {"context": "."}}
        self.assertTrue(any("builds on home" in str(r) for r in config_refusals(stack(redis={"image": FROM, **build}))))
        self.assertTrue(any("builds on home" in str(r) for r in config_refusals(stack(redis=build))))

    def test_a_latest_anywhere_in_a_service_is_refused(self) -> None:
        found = config_refusals(stack(x={"image": FROM, "environment": {"SIDECAR": "some:latest"}}))
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

    def test_the_rule_is_envctl_s_word_for_word(self) -> None:
        """The cases apps/envctl/src/compose.rs's test reads too."""
        cases = json.loads(CASES.read_text())
        self.assertGreater(len(cases["refused"]), 20)
        for line in cases["refused"]:
            self.assertIsNotNone(home_refusal(line.split()), line)
        for line in cases["allowed"]:
            self.assertIsNone(home_refusal(line.split()), line)

    def test_docs_are_judged_by_the_same_rule(self) -> None:
        cases = json.loads(CASES.read_text())
        for line in cases["refused"]:
            for written in (f'make compose ARGS="{line}"', f"docker compose {line}", f"`docker compose {line}`."):
                if line.strip():
                    self.assertEqual(len(docs_refusals("README.md", written)), 1, written)
        for line in cases["allowed"]:
            for written in (f'make compose ARGS="{line}"', f"docker compose {line}"):
                self.assertEqual(docs_refusals("README.md", written), [], written)

    def test_a_doc_s_quotes_punctuation_and_second_command(self) -> None:
        for line in (
            "make compose ARGS='pull'",
            "make compose ARGS=pull",
            "docker-compose up -d --build file-host",
            "docker compose pull.",
            "first `docker compose up -d`, then `docker compose pull`",
        ):
            self.assertEqual(len(docs_refusals("README.md", line)), 1, line)
        for line in (
            "make home-deploy",
            "make compose ARGS=pull-thing",
            "docker compose up -d; docker compose exec x tool -w",
            'make compose ARGS="exec backup tool --build"',
            "docker compose run --build-arg X=1 tool",
        ):
            self.assertEqual(docs_refusals("README.md", line), [], line)

    def test_the_real_home_stack_is_read_whole_by_compose(self) -> None:
        """Never vacuous: compose reads every home service, each from the release, none built."""
        config = compose_config(REPO_ROOT)
        services = config.get("services", {})
        self.assertGreaterEqual(len(services), 18)
        self.assertIn("dashboard-builder", services)
        self.assertEqual(config_refusals(config), [], "every image is the release's")


if __name__ == "__main__":
    sys.exit(main())

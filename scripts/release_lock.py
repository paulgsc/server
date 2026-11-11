#!/usr/bin/env python3
"""Move published images into home's release (RELEASE R2, #789; epic #787).

`deploy/home/release.json` names every image compose starts on home
(`apps/envctl/src/release.rs`, R1 #788). When an image is published, CI
(`.github/workflows/release-lock.yml`) runs this with the published reference
by tag and digest. Every release entry whose repository is that image's gets
the new reference, `serial` rises once, and `server_commit` becomes the main
commit the release was formed at. The workflow then opens the release pull
request.

    python3 scripts/release_lock.py --server-commit <sha> \\
        --set pgathondu/rust-runner:<sha>@sha256:<digest> [--set ...]
    python3 scripts/release_lock.py --self-test

What it never does:

- **Moves a held entry.** `file-host` stays `pgathondu/server:latest` until R4
  (#793): `envctl deploy` pulls, tags and rolls back that tag, and a digest
  there would leave it pulling an image compose never starts.
- **Takes a reference without a digest.** A tag alone can move under the
  release; the release names bytes.
- **Adds a service.** A repository no entry starts (an image home does not
  run, such as `pgathondu/obs`) is reported and left out: compose decides
  which services exist, and `envctl check` holds the release to it.

It prints one line per change, which the workflow puts in the pull request.
Exit status: 0 whether or not anything changed (the workflow reads the file),
2 on a malformed reference or release. Stdlib only.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
import tempfile
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
RELEASE = REPO_ROOT / "deploy" / "home" / "release.json"

# Entries no publish moves, and why. Each names the story that frees it.
HELD = {
    "file-host": "held on pgathondu/server:latest until R4 (#793): envctl deploy pulls, tags and rolls back that tag",
}

PUBLISHED = re.compile(r"^(?P<repository>[a-z0-9][a-z0-9._/-]*):(?P<tag>[A-Za-z0-9._-]+)@sha256:[0-9a-f]{64}$")


class Refused(Exception):
    pass


def repository(image: str) -> str:
    """`repo:tag@sha256:…` → `repo`. A registry host's port is not a tag."""
    image = image.split("@", 1)[0]
    slash = image.rfind("/")
    colon = image.rfind(":")
    return image[:colon] if colon > slash else image


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


def apply(release: dict, published: list[str], server_commit: str) -> tuple[dict, list[str]]:
    """The release with each published reference in, and what was said."""
    said: list[str] = []
    changed = False
    services = {name: dict(entry) for name, entry in release["services"].items()}
    for reference in published:
        match = PUBLISHED.match(reference)
        if match is None:
            raise Refused(f"{reference} is not repo:tag@sha256:<64 hex>; a release names bytes, not a tag alone")
        wanted = match.group("repository")
        names = sorted(name for name, entry in services.items() if repository(entry["image"]) == wanted)
        if not names:
            said.append(f"- {wanted}: no service home runs starts it; left out")
            continue
        for name in names:
            if name in HELD:
                said.append(f"- {name}: {HELD[name]}")
            elif services[name]["image"] == reference:
                said.append(f"- {name}: already {reference}")
            else:
                said.append(f"- {name}: {services[name]['image']} → {reference}")
                services[name]["image"] = reference
                changed = True
    if not changed:
        return release, said
    return {"serial": int(release["serial"]) + 1, "server_commit": server_commit, "services": services}, said


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--release", type=Path, default=RELEASE)
    parser.add_argument("--server-commit", default="")
    parser.add_argument("--set", dest="published", action="append", default=[], metavar="REPO:TAG@sha256:DIGEST")
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        result = unittest.main(argv=[sys.argv[0]], exit=False, verbosity=1).result
        return 0 if result.wasSuccessful() else 1
    try:
        release = json.loads(args.release.read_text())
        updated, said = apply(release, args.published, args.server_commit)
    except (Refused, KeyError, ValueError) as err:
        print(f"release_lock: {err}", file=sys.stderr)
        return 2
    if updated is not release:
        args.release.write_text(render(updated))
    print("\n".join(said))
    return 0


# ── Tests ───────────────────────────────────────────────────────────────────

DIGEST = "sha256:" + "a" * 64
OTHER = "sha256:" + "b" * 64


def sample() -> dict:
    return {
        "serial": 4,
        "server_commit": "old",
        "services": {
            "file-host": {"image": "pgathondu/server:latest"},
            "rust-runner": {"image": "pgathondu/rust-runner:latest"},
            "redis": {"image": "redis:latest"},
            "local": {"image": "registry.example:5000/team/app:1"},
        },
    }


class Tests(unittest.TestCase):
    def test_a_published_image_moves_its_entries_and_the_serial_once(self) -> None:
        updated, said = apply(sample(), [f"pgathondu/rust-runner:abc@{DIGEST}", f"redis:7.4@{OTHER}"], "new")
        self.assertEqual(updated["serial"], 5)
        self.assertEqual(updated["server_commit"], "new")
        self.assertEqual(updated["services"]["rust-runner"]["image"], f"pgathondu/rust-runner:abc@{DIGEST}")
        self.assertEqual(updated["services"]["redis"]["image"], f"redis:7.4@{OTHER}")
        self.assertIn(f"- rust-runner: pgathondu/rust-runner:latest → pgathondu/rust-runner:abc@{DIGEST}", said)

    def test_file_host_is_held_until_r4(self) -> None:
        before = sample()
        updated, said = apply(before, [f"pgathondu/server:abc@{DIGEST}"], "new")
        self.assertIs(updated, before, "nothing changed, so the release is not rewritten")
        self.assertEqual(updated["services"]["file-host"]["image"], "pgathondu/server:latest")
        self.assertTrue(any("file-host: held" in line and "#793" in line for line in said), said)

    def test_an_image_no_service_starts_is_left_out(self) -> None:
        before = sample()
        updated, said = apply(before, [f"pgathondu/obs:abc@{DIGEST}"], "new")
        self.assertIs(updated, before)
        self.assertEqual(said, ["- pgathondu/obs: no service home runs starts it; left out"])

    def test_the_same_reference_again_changes_nothing(self) -> None:
        once, _ = apply(sample(), [f"pgathondu/rust-runner:abc@{DIGEST}"], "c1")
        twice, said = apply(once, [f"pgathondu/rust-runner:abc@{DIGEST}"], "c2")
        self.assertIs(twice, once)
        self.assertEqual(twice["serial"], 5)
        self.assertEqual(said, [f"- rust-runner: already pgathondu/rust-runner:abc@{DIGEST}"])

    def test_a_reference_without_a_digest_is_refused(self) -> None:
        for reference in ["pgathondu/rust-runner:abc", "pgathondu/rust-runner@" + DIGEST, "pgathondu/rust-runner:abc@sha256:short", ""]:
            with self.assertRaises(Refused, msg=reference):
                apply(sample(), [reference], "c")

    def test_a_registry_port_is_not_a_tag(self) -> None:
        self.assertEqual(repository("registry.example:5000/team/app:1"), "registry.example:5000/team/app")
        self.assertEqual(repository("redis:7.4@" + DIGEST), "redis")
        self.assertEqual(repository("some-render:local"), "some-render")
        self.assertEqual(repository("nats"), "nats")

    def test_the_file_keeps_r1_s_shape_and_round_trips(self) -> None:
        text = RELEASE.read_text()
        release = json.loads(text)
        self.assertEqual(render(release), text, "release.json is written exactly as render writes it")
        for name in HELD:
            self.assertIn(name, release["services"], "a held entry names a real service")

    def test_main_writes_the_file_and_prints_the_changes(self) -> None:
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "release.json"
            path.write_text(render(sample()))
            updated, said = apply(json.loads(path.read_text()), [f"pgathondu/rust-runner:abc@{DIGEST}"], "c")
            path.write_text(render(updated))
            self.assertEqual(json.loads(path.read_text())["services"]["rust-runner"]["image"], f"pgathondu/rust-runner:abc@{DIGEST}")
            self.assertEqual(len(said), 1)


if __name__ == "__main__":
    sys.exit(main())

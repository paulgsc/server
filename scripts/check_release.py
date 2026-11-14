#!/usr/bin/env python3
"""Refuse a release whose images do not fit together (RELEASE R3, #790).

Every image in `deploy/home/release.json` may say what it provides and what
it requires of the others, as two OCI labels its Dockerfile writes:

    io.paulgsc.provides="contract=6,render-folders=1"
    io.paulgsc.requires="nats-subjects=1,rust-runner-protocol=1"

Each is comma-separated `name=value` pairs, and a value is either an epoch
(an integer: a requirement of n is met by a provision of at least n) or a
40-hex commit (met by the same commit only). A release is consistent when
every image's every requirement is met by another image's provision. This
reads each image's labels from the registry by digest, without pulling:
Docker Hub's registry API, the manifest (the linux/amd64 one of an index),
then the config blob that holds the labels.

    python3 scripts/check_release.py                 # deploy/home/release.json
    python3 scripts/check_release.py --release FILE
    python3 scripts/check_release.py --self-test

What requires nothing: a reference with no digest (`redis:latest`,
`pgathondu/server:latest`: a tag can move, so there is nothing fixed to
read), an image outside Docker Hub, and an image without the labels (one
published before R3, or a third-party one).

`envctl` holds the same rule for deploy time
(`apps/envctl/src/release/constraints.rs`, `unmet`); `test_the_rule_is_envctl_s`
runs this side on the cases that file tests. docs/release.md, "Constraints", says what each edge means.

Exit status: 0 when the release is consistent; 1 when a requirement is
unmet or a label is malformed, naming each; 2 when the registry cannot be
read, so the run fails loudly and can be re-run. Stdlib only.
"""

from __future__ import annotations

import argparse
import base64
import json
import os
import re
import sys
import time
import unittest
import urllib.error
import urllib.parse
import urllib.request
from collections.abc import Callable
from dataclasses import dataclass, field
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
RELEASE = REPO_ROOT / "deploy" / "home" / "release.json"
DETECT = REPO_ROOT / ".github" / "scripts" / "detect" / "src" / "main.rs"

PROVIDES = "io.paulgsc.provides"
REQUIRES = "io.paulgsc.requires"

NAME = re.compile(r"[a-z0-9][a-z0-9-]*")
EPOCH = re.compile(r"[0-9]+")
COMMIT = re.compile(r"[0-9a-f]{40}")
DIGEST = re.compile(r"sha256:[0-9a-f]{64}")

REGISTRY = "https://registry-1.docker.io/v2"
AUTH = "https://auth.docker.io/token"
ACCEPT = ", ".join(
    [
        "application/vnd.oci.image.index.v1+json",
        "application/vnd.docker.distribution.manifest.list.v2+json",
        "application/vnd.oci.image.manifest.v1+json",
        "application/vnd.docker.distribution.manifest.v2+json",
    ]
)
INDEXES = {"application/vnd.oci.image.index.v1+json", "application/vnd.docker.distribution.manifest.list.v2+json"}


class Malformed(Exception):
    pass


class Unreadable(Exception):
    pass


# ── The rule, pure ──────────────────────────────────────────────────────────


def pairs(text: str) -> dict[str, str]:
    """One label's `name=value` pairs. Empty text says nothing."""
    found: dict[str, str] = {}
    for part in (text or "").split(","):
        part = part.strip()
        if not part:
            continue
        name, sep, value = part.partition("=")
        if not sep or not NAME.fullmatch(name) or not (EPOCH.fullmatch(value) or COMMIT.fullmatch(value)):
            raise Malformed(f"`{part}` is not name=epoch or name=<40-hex commit>")
        if name in found:
            raise Malformed(f"`{name}` is given twice")
        found[name] = value
    return found


@dataclass(frozen=True)
class Constraints:
    provides: dict[str, str] = field(default_factory=dict)
    requires: dict[str, str] = field(default_factory=dict)


def constraints(labels: dict[str, str] | None) -> Constraints:
    labels = labels or {}
    return Constraints(pairs(labels.get(PROVIDES, "")), pairs(labels.get(REQUIRES, "")))


def meets(provided: str, required: str) -> bool:
    """An epoch is met by one at least as high; a commit by itself."""
    if EPOCH.fullmatch(required):
        return bool(EPOCH.fullmatch(provided)) and int(provided) >= int(required)
    return provided == required


def unmet(images: dict[str, Constraints]) -> list[str]:
    """Each requirement no other image in the set meets, as one line."""
    problems = []
    for service in sorted(images):
        for name, required in sorted(images[service].requires.items()):
            offered = {other: images[other].provides[name] for other in sorted(images) if other != service and name in images[other].provides}
            if any(meets(value, required) for value in offered.values()):
                continue
            if offered:
                given = ", ".join(f"{other} provides {name}={value}" for other, value in offered.items())
                problems.append(f"{service} requires {name}={required}, but {given}")
            else:
                problems.append(f"{service} requires {name}={required}, which no other image in the release provides")
    return problems


# ── Reading the registry ────────────────────────────────────────────────────


@dataclass(frozen=True)
class Reference:
    repository: str  # Docker Hub's name: `library/redis`, `pgathondu/render`
    digest: str


def hub_reference(image: str) -> Reference | None:
    """The Docker Hub repository and digest of a reference, None when it has
    no digest or names another registry."""
    name, _, digest = image.partition("@")
    if not DIGEST.fullmatch(digest):
        return None
    path = name
    last = path.rsplit("/", 1)[-1]
    if ":" in last:
        path = path[: len(path) - len(last)] + last.split(":", 1)[0]
    first = path.split("/", 1)[0]
    if "/" in path and ("." in first or ":" in first or first == "localhost"):
        if first not in ("docker.io", "index.docker.io", "registry-1.docker.io"):
            return None
        path = path.split("/", 1)[1]
    if "/" not in path:
        path = f"library/{path}"
    return Reference(path, digest)


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):  # noqa: ANN001, ARG002
        return None


def fetch(url: str, headers: dict[str, str], *, follow: bool = True) -> tuple[bytes, str]:
    """A body and its content type. A blob's redirect is followed without
    the registry's token, which its storage does not take."""
    opener = urllib.request.build_opener(NoRedirect)
    for attempt in range(4):
        request = urllib.request.Request(url, headers={"User-Agent": "check-release", **headers})
        try:
            with opener.open(request, timeout=30) as response:
                return response.read(), response.headers.get("Content-Type", "").split(";")[0]
        except urllib.error.HTTPError as err:
            if err.code in (301, 302, 303, 307, 308) and follow and err.headers.get("Location"):
                return fetch(urllib.parse.urljoin(url, err.headers["Location"]), {}, follow=False)
            if err.code in (429, 500, 502, 503, 504) and attempt < 3:
                time.sleep(5 * 2**attempt)
                continue
            raise Unreadable(f"{url} answered {err.code}") from err
        except (urllib.error.URLError, TimeoutError) as err:
            if attempt < 3:
                time.sleep(5 * 2**attempt)
                continue
            raise Unreadable(f"{url} could not be read: {err}") from err
    raise Unreadable(f"{url} could not be read")


def token(repository: str) -> str:
    headers = {}
    user, secret = os.environ.get("DOCKERHUB_USERNAME"), os.environ.get("DOCKERHUB_TOKEN")
    if user and secret:  # a higher pull limit than anonymous; the same answers
        headers["Authorization"] = "Basic " + base64.b64encode(f"{user}:{secret}".encode()).decode()
    body, _ = fetch(f"{AUTH}?service=registry.docker.io&scope=repository:{repository}:pull", headers)
    try:
        return json.loads(body)["token"]
    except (ValueError, KeyError) as err:
        raise Unreadable(f"no registry token for {repository}") from err


def registry_labels(reference: Reference) -> dict[str, str]:
    """An image's labels, read by digest: index → linux/amd64 manifest → config."""
    auth = {"Authorization": f"Bearer {token(reference.repository)}"}
    base = f"{REGISTRY}/{reference.repository}"
    try:
        body, kind = fetch(f"{base}/manifests/{reference.digest}", {**auth, "Accept": ACCEPT})
        manifest = json.loads(body)
        if kind in INDEXES or "manifests" in manifest:
            chosen = [m for m in manifest.get("manifests", []) if m.get("platform", {}).get("os") == "linux" and m.get("platform", {}).get("architecture") == "amd64"]
            if not chosen:
                raise Unreadable(f"{reference.repository}@{reference.digest} has no linux/amd64 image")
            body, _ = fetch(f"{base}/manifests/{chosen[0]['digest']}", {**auth, "Accept": ACCEPT})
            manifest = json.loads(body)
        body, _ = fetch(f"{base}/blobs/{manifest['config']['digest']}", auth)
        config = json.loads(body)
    except (ValueError, KeyError) as err:
        raise Unreadable(f"{reference.repository}@{reference.digest} is not an image the registry describes: {err}") from err
    return (config.get("config") or {}).get("Labels") or {}


def check(release: dict, labels: Callable[[Reference], dict[str, str]]) -> list[str]:
    """Every problem with a release, given a way to read an image's labels."""
    images: dict[str, Constraints] = {}
    problems: list[str] = []
    for service, entry in sorted(release["services"].items()):
        reference = hub_reference(entry["image"])
        try:
            images[service] = constraints(labels(reference) if reference else None)
        except Malformed as err:
            problems.append(f"{service} ({entry['image']}): {err}")
    return problems + unmet(images)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--release", type=Path, default=RELEASE)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        result = unittest.main(argv=[sys.argv[0]], exit=False, verbosity=1).result
        return 0 if result.wasSuccessful() else 1
    try:
        release = json.loads(args.release.read_text())
        problems = check(release, registry_labels)
    except Unreadable as err:
        print(f"check_release: {err}", file=sys.stderr)
        return 2
    if problems:
        print(f"check_release: {args.release} does not fit together:", file=sys.stderr)
        for problem in problems:
            print(f"  - {problem}", file=sys.stderr)
        return 1
    print(f"check_release: every requirement in {args.release.name} (serial {release['serial']}) is met")
    return 0


# ── Tests ───────────────────────────────────────────────────────────────────

DA, DB, DC = ("sha256:" + "1" * 64, "sha256:" + "2" * 64, "sha256:" + "3" * 64)
SHA = "d495258" + "0" * 33


def release(**images: str) -> dict:
    return {"serial": 1, "services": {name: {"image": image} for name, image in images.items()}}


def registry(labels: dict[str, dict[str, str]]) -> Callable[[Reference], dict[str, str]]:
    return lambda reference: labels.get(reference.digest, {})


def label(provides: str = "", requires: str = "") -> dict[str, str]:
    return {PROVIDES: provides, REQUIRES: requires}


def dockerfile_labels(path: Path) -> dict[str, str] | None:
    """The `io.paulgsc.*` labels a Dockerfile writes, None when it writes neither."""
    text = path.read_text()
    found = {name: m.group(1) for name in (PROVIDES, REQUIRES) if (m := re.search(re.escape(name) + r'="([^"]*)"', text))}
    return found or None


class Tests(unittest.TestCase):
    def test_render_reading_a_newer_layout_than_file_host_writes_is_refused(self) -> None:
        """#790's own case: render requires render-folders=4, file_host provides 3."""
        seen = registry({DA: label("contract=6,render-folders=3"), DB: label(requires="render-folders=4")})
        problems = check(release(**{"file-host": f"pgathondu/server:x@{DA}", "render": f"pgathondu/render:{SHA}@{DB}"}), seen)
        self.assertEqual(problems, ["render requires render-folders=4, but file-host provides render-folders=3"])

    def test_an_epoch_at_or_above_the_requirement_meets_it(self) -> None:
        for provided in ("4", "5"):
            seen = registry({DA: label(f"render-folders={provided}"), DB: label(requires="render-folders=4")})
            self.assertEqual(check(release(a=f"r/a:1@{DA}", b=f"r/b:1@{DB}"), seen), [], provided)

    def test_a_commit_is_met_by_itself_only(self) -> None:
        other = "e" * 40
        seen = registry({DA: label(f"ui={SHA}"), DB: label(requires=f"ui={SHA}"), DC: label(requires=f"ui={other}")})
        self.assertEqual(check(release(a=f"r/a:1@{DA}", b=f"r/b:1@{DB}"), seen), [])
        self.assertEqual(check(release(a=f"r/a:1@{DA}", c=f"r/c:1@{DC}"), seen), [f"c requires ui={other}, but a provides ui={SHA}"])

    def test_a_requirement_nothing_provides_is_named(self) -> None:
        seen = registry({DA: label(requires="nats-subjects=1")})
        self.assertEqual(check(release(a=f"r/a:1@{DA}"), seen), ["a requires nats-subjects=1, which no other image in the release provides"])

    def test_an_image_does_not_meet_its_own_requirement(self) -> None:
        seen = registry({DA: label("x=2", "x=1")})
        self.assertEqual(check(release(a=f"r/a:1@{DA}"), seen), ["a requires x=1, which no other image in the release provides"])

    def test_a_reference_without_a_digest_or_off_hub_requires_nothing_and_is_not_read(self) -> None:
        read: list[Reference] = []

        def labels(reference: Reference) -> dict[str, str]:
            read.append(reference)
            return label(requires="never=1")

        refs = release(redis="redis:latest", fh="pgathondu/server:latest", cad=f"gcr.io/cadvisor/cadvisor:v1@{DA}")
        self.assertEqual(check(refs, labels), [])
        self.assertEqual(read, [])

    def test_an_image_without_labels_requires_nothing(self) -> None:
        self.assertEqual(check(release(a=f"r/a:1@{DA}"), registry({DA: {"org.opencontainers.image.title": "a"}})), [])

    def test_a_malformed_label_is_refused_not_skipped(self) -> None:
        for bad in ("render-folders", "render-folders=x", "Render=1", "a=1,a=2", "c=abc123"):
            problems = check(release(a=f"r/a:1@{DA}"), registry({DA: label(requires=bad)}))
            self.assertEqual(len(problems), 1, bad)
            self.assertIn("a (r/a:1@", problems[0])

    def test_hub_references_name_their_repository(self) -> None:
        self.assertEqual(hub_reference(f"redis:7.4@{DA}"), Reference("library/redis", DA))
        self.assertEqual(hub_reference(f"pgathondu/render:{SHA}@{DA}"), Reference("pgathondu/render", DA))
        self.assertEqual(hub_reference(f"docker.io/prom/prometheus:v3@{DA}"), Reference("prom/prometheus", DA))
        self.assertEqual(hub_reference(f"pgathondu/kokoro@{DA}"), Reference("pgathondu/kokoro", DA))
        self.assertIsNone(hub_reference(f"gcr.io/cadvisor/cadvisor:v1@{DA}"))
        self.assertIsNone(hub_reference("pgathondu/server:latest"))

    def test_the_rule_is_envctl_s(self) -> None:
        """The cases apps/envctl/src/release/constraints.rs's `unmet` is tested on, run here."""
        source = (REPO_ROOT / "apps" / "envctl" / "src" / "release" / "constraints.rs").read_text()
        cases = re.findall(r'case\(\s*"([^"]*)",\s*"([^"]*)",\s*"([^"]*)",\s*(true|false),?\s*\)', source)
        self.assertGreaterEqual(len(cases), 5, "constraints.rs's shared cases were read")
        for provides, requires, _, met in cases:
            seen = registry({DA: label(provides), DB: label(requires=requires)})
            self.assertEqual(check(release(p=f"r/p:1@{DA}", r=f"r/r:1@{DB}"), seen) == [], met == "true", (provides, requires))

    def test_every_image_home_runs_from_server_says_what_it_provides_and_requires(self) -> None:
        """Each Dockerfile the detector publishes, for an image the release names, writes both labels, well formed."""
        detect = DETECT.read_text()
        published = re.findall(r'dockerfile: "\./([^"]+)",\s*repo_suffix: "([^"]+)"', detect)
        self.assertGreaterEqual(len(published), 4)
        names = json.loads(RELEASE.read_text())["services"].values()
        checked = 0
        for dockerfile, suffix in published:
            if not any(entry["image"].startswith(f"pgathondu/{suffix}:") for entry in names):
                continue
            found = dockerfile_labels(REPO_ROOT / dockerfile)
            self.assertIsNotNone(found, f"{dockerfile} says nothing of what it provides and requires")
            self.assertEqual(set(found or {}), {PROVIDES, REQUIRES}, dockerfile)
            constraints(found)
            checked += 1
        self.assertGreaterEqual(checked, 3)

    def test_the_server_images_as_built_fit_together(self) -> None:
        """file_host, orchestrator and rust-runner, as their Dockerfiles stand, meet each other."""
        images = {name: constraints(dockerfile_labels(REPO_ROOT / "infra" / "docker" / f"Dockerfile.{name}")) for name in ("server", "orchestrator", "rust-runner")}
        self.assertEqual(unmet(images), [])
        self.assertIn("render-folders", images["server"].provides)


if __name__ == "__main__":
    sys.exit(main())

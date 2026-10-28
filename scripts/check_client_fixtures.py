#!/usr/bin/env python3
"""The fixtures this server and paulgsc/some-ui keep of each other agree.

JSON files cross the seam in both directions. Each side checks its
own code against the copy it holds of the other side's output, so a copy that
falls behind keeps every test green while proving nothing (#490). This
compares each copy with a fresh dump of what it copies:

    here (crates/db/activity/testdata/)       some-ui (`pnpm dump:*`)
      activity_catalog.snapshot.json         <- dump:activity-catalog
      session_duration_policy.snapshot.json  <- dump:session-duration-policy

    some-ui (found by file name)              here (`cargo run --bin ...`)
      provisioned-session.snapshot.json      <- dump-provisioned-session
      proposed-session.snapshot.json         <- dump-proposed-session
      game-scenarios.snapshot.json           <- dump-game-scenarios

`.github/workflows/routes.yml` runs the dumps and passes their outputs in
(`--dumps`), with some-ui's `main` checked out (`--some-ui`). some-ui's copies
are found by file name anywhere in its tree, so moving one is a change to
that repo alone; a name that matches no file, or more than one, fails.

The comparison is of parsed JSON. An array whose items all carry an `id` is
compared by id, so a difference names the item (`[id=interview]`) rather
than an index that shifts.

KNOWN lists differences that are real and wait on a decision outside the PR
that would otherwise fail, each naming the issue that decides it. A known
difference is reported as a warning. A known difference that no longer
occurs fails too, so the list never claims a gap that is gone: delete the
entry in the change that closes it, or in the next one here.

`--self-test` runs the comparison on fixed inputs first, so an edit that
stops it seeing a difference fails instead of passing every run.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Any

REPO_ROOT = Path(__file__).resolve().parent.parent
TESTDATA = REPO_ROOT / "crates" / "db" / "activity" / "testdata"


@dataclass(frozen=True)
class Pair:
	name: str
	# Whose dump is the source of truth ("some-ui's"), for the message.
	source: str
	# The dump's file name inside --dumps.
	dump: str
	# This repo's copy (a path), or some-ui's (a file name to find there).
	copy_here: Path | None = None
	copy_in_some_ui: str | None = None


PAIRS = (
	Pair("activity catalog", "some-ui's", "activity-catalog.json", copy_here=TESTDATA / "activity_catalog.snapshot.json"),
	Pair("session duration policy", "some-ui's", "session-duration-policy.json", copy_here=TESTDATA / "session_duration_policy.snapshot.json"),
	Pair("provisioned session", "the server's", "provisioned-session.json", copy_in_some_ui="provisioned-session.snapshot.json"),
	Pair("proposed session", "the server's", "proposed-session.json", copy_in_some_ui="proposed-session.snapshot.json"),
	Pair("game scenarios", "the server's", "game-scenarios.json", copy_in_some_ui="game-scenarios.snapshot.json"),
)

# Regenerating the catalog fixture makes tests/catalog_parity.rs fail until the
# seed matches it, and what `GET /activities` serves is the owner's call.
SEED_WAITS = "so the fixture and the seed wait on the owner's call on the seed, paulgsc/server#492 item 5"

# (pair name, difference) -> why it is allowed for now, and where that is decided.
KNOWN: dict[tuple[str, str], str] = {
	("activity catalog", "[id=interview]: only in the copy"): f"some-ui retired `interview` (some-ui #1660), {SEED_WAITS}",
	(
		"activity catalog",
		'[id=leetype]/description: "Prove one competency at a time against the smallest code that shows it." in the dump, '
		'"Prove one competency at a time by typing the smallest code that shows it." in the copy',
	): f"some-ui reworded it after the seed was written, {SEED_WAITS}",
	("activity catalog", "[id=leetype]/input: only in the dump"): (
		f"some-ui added `input` (some-ui #1176); the seed has no column for it and the parity test refuses unknown fields, {SEED_WAITS}"
	),
	("provisioned session", "/origin: only in the dump"): (
		"some-ui's copy predates `origin` and its `SessionRecord` has no such field; paulgsc/some-ui#2117 (U9) regenerates it"
	),
}


def by_id(items: list[Any]) -> dict[str, Any] | None:
	if items and all(isinstance(item, dict) and isinstance(item.get("id"), str) for item in items):
		keyed = {item["id"]: item for item in items}
		if len(keyed) == len(items):
			return keyed
	return None


def differences(dump: Any, copy: Any, path: str = "") -> list[str]:
	"""Every place `copy` differs from `dump`, as `<path>: <how>`."""
	if isinstance(dump, dict) and isinstance(copy, dict):
		out: list[str] = []
		for key in sorted(dump.keys() | copy.keys()):
			here = f"{path}/{key}"
			if key not in copy:
				out.append(f"{here}: only in the dump")
			elif key not in dump:
				out.append(f"{here}: only in the copy")
			else:
				out.extend(differences(dump[key], copy[key], here))
		return out
	if isinstance(dump, list) and isinstance(copy, list):
		keyed_dump, keyed_copy = by_id(dump), by_id(copy)
		if keyed_dump is not None and keyed_copy is not None:
			out = []
			for key in sorted(keyed_dump.keys() | keyed_copy.keys()):
				here = f"{path}[id={key}]"
				if key not in keyed_copy:
					out.append(f"{here}: only in the dump")
				elif key not in keyed_dump:
					out.append(f"{here}: only in the copy")
				else:
					out.extend(differences(keyed_dump[key], keyed_copy[key], here))
			if [item["id"] for item in dump if item["id"] in keyed_copy] != [item["id"] for item in copy if item["id"] in keyed_dump]:
				out.append(f"{path}: the shared ids are in a different order")
			return out
		if len(dump) != len(copy):
			return [f"{path}: {len(dump)} items in the dump, {len(copy)} in the copy"]
		out = []
		for index, (left, right) in enumerate(zip(dump, copy)):
			out.extend(differences(left, right, f"{path}[{index}]"))
		return out
	if dump != copy or type(dump) is not type(copy):
		return [f"{path or '/'}: {json.dumps(dump)} in the dump, {json.dumps(copy)} in the copy"]
	return []


def find_in_some_ui(some_ui: Path, name: str) -> Path:
	listed = subprocess.run(["git", "-C", str(some_ui), "ls-files", "--", f"*{name}"], check=True, capture_output=True, text=True).stdout.split()
	matches = [path for path in listed if Path(path).name == name]
	if len(matches) != 1:
		raise LookupError(f"expected one file named {name} in some-ui, found {len(matches)}: {matches}")
	return some_ui / matches[0]


def check(some_ui: Path, dumps: Path) -> int:
	failures = 0
	seen: set[tuple[str, str]] = set()
	for pair in PAIRS:
		try:
			copy_path = pair.copy_here if pair.copy_here is not None else find_in_some_ui(some_ui, pair.copy_in_some_ui or "")
		except LookupError as err:
			print(f"::error title=Fixture not found ({pair.name})::{err}")
			failures += 1
			continue
		dump = json.loads((dumps / pair.dump).read_text())
		copy = json.loads(copy_path.read_text())
		where = copy_path.relative_to(REPO_ROOT) if pair.copy_here is not None else f"some-ui/{copy_path.relative_to(some_ui)}"
		found = differences(dump, copy)
		for difference in found:
			key = (pair.name, difference)
			seen.add(key)
			if key in KNOWN:
				print(f"::warning title=Known fixture difference ({pair.name})::{where} {difference}. {KNOWN[key]}")
			else:
				print(f"::error title=Fixture differs ({pair.name})::{where} {difference}. Regenerate it from {pair.source} dump.")
				failures += 1
		if not found:
			print(f"{pair.name}: {where} matches {pair.source} dump")
	for key, why in KNOWN.items():
		if key not in seen:
			print(f"::error title=Known difference is gone ({key[0]})::{key[1]} no longer occurs ({why}). Delete it from KNOWN in scripts/check_client_fixtures.py.")
			failures += 1
	return 1 if failures else 0


def self_test() -> None:
	catalog_dump = [{"id": "a", "n": 1}, {"id": "b", "n": [1, 2]}]
	assert differences(catalog_dump, [{"id": "a", "n": 1}, {"id": "b", "n": [1, 2]}]) == []
	assert differences(catalog_dump, [{"id": "a", "n": 1}, {"id": "b", "n": [1, 2]}, {"id": "c"}]) == ["[id=c]: only in the copy"]
	assert differences(catalog_dump, [{"id": "a", "n": 2}, {"id": "b", "n": [1, 2]}]) == ["[id=a]/n: 1 in the dump, 2 in the copy"]
	assert differences(catalog_dump, [{"id": "b", "n": [1, 2]}, {"id": "a", "n": 1}]) == [": the shared ids are in a different order"]
	assert differences({"x": 1, "origin": "system"}, {"x": 1}) == ["/origin: only in the dump"]
	assert differences({"x": [1, 2]}, {"x": [1]}) == ["/x: 2 items in the dump, 1 in the copy"]
	assert differences({"x": 1}, {"x": 1.0}) == ["/x: 1 in the dump, 1.0 in the copy"]
	assert differences({"x": None}, {"x": False}) == ["/x: null in the dump, false in the copy"]
	print("self-test passed")


def main() -> int:
	parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
	parser.add_argument("--some-ui", type=Path, help="a checkout of paulgsc/some-ui")
	parser.add_argument("--dumps", type=Path, help="the directory holding the dumps")
	parser.add_argument("--self-test", action="store_true")
	args = parser.parse_args()
	if args.self_test:
		self_test()
		return 0
	if args.some_ui is None or args.dumps is None:
		parser.error("--some-ui and --dumps are required")
	return check(args.some_ui.resolve(), args.dumps.resolve())


if __name__ == "__main__":
	sys.exit(main())

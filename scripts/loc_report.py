#!/usr/bin/env python3
"""How a pull request changes the amount of hand-written code (#379).

Counts code lines (not blanks or comments) with tokei in two trees, the PR's
merge base and its head, and prints the difference as Markdown: per language,
and per workspace member. `.github/workflows/loc.yml` appends it to the job
summary. It reports and never fails, because code that grows is often
a feature; a gate would block the PRs that are supposed to add code.

    python3 scripts/loc_report.py BASE_DIR HEAD_DIR

Both trees should hold tracked files only (loc.yml extracts each with
`git archive`), so build output and anything untracked is never counted.
Generated, vendored and lock files are excluded below, because the number is
meant to reflect code someone wrote and has to maintain.

Code outside every workspace member is grouped by its top-level directory
(`infra/`, `scripts/`, ...), so a change there still shows up somewhere.

`--self-test` runs the diff and bucketing logic against made-up tokei output,
like scripts/check_instrument_skip.py's rule tests, so a change that breaks the
arithmetic fails CI instead of printing a wrong number.
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
import tomllib
from collections import Counter
from pathlib import Path

# gitignore-style patterns, handed to `tokei --exclude`.
EXCLUDED = [
	"Cargo.lock",
	"flake.lock",
	"pnpm-lock.yaml",
	".sqlx",
	"target",
	"node_modules",
	# The clippy ratchet's recorded findings: rewritten by a script.
	"scripts/clippy_baseline.json",
	# Rendered from the Jsonnet next to it by infra/grafana/scripts/build.sh.
	"infra/grafana/generated",
	# Test fixtures written by the tests that read them.
	"*.snapshot.json",
	# `dump-routes` output, also copied into paulgsc/some-ui.
	"apps/servers/file_host/docs/route-inventory.md",
	# Image changesets, written by detect.yml and merged by its bot PRs.
	".github/docker-changesets",
]

OUTSIDE = "(outside the workspace)"


def tokei(tree: Path, binary: str) -> dict:
	"""tokei's JSON report for `tree`, with file paths relative to it."""
	command = [binary, "--output", "json", "--hidden"]
	for pattern in EXCLUDED:
		command += ["--exclude", pattern]
	result = subprocess.run([*command, "."], cwd=tree, capture_output=True, text=True, check=True)
	return json.loads(result.stdout)


def workspace_members(tree: Path) -> list[str]:
	manifest = tree / "Cargo.toml"
	if not manifest.exists():
		return []
	with manifest.open("rb") as file:
		return tomllib.load(file).get("workspace", {}).get("members", [])


def bucket(path: str, members: list[str]) -> str:
	"""The workspace member a file belongs to, or its top-level directory."""
	owners = [member for member in members if path == member or path.startswith(member + "/")]
	if owners:
		return max(owners, key=len)
	top, _, rest = path.partition("/")
	return f"{top}/" if rest else OUTSIDE


def code_lines(report: dict, members: list[str]) -> tuple[Counter, Counter]:
	"""Code lines per language and per bucket.

	Only each language's own lines. tokei also reports embedded languages as
	`children` (Rust inside Markdown, Markdown in doc comments), which would
	count those lines under a second language.
	"""
	languages, buckets = Counter(), Counter()
	for language, stats in report.items():
		if language == "Total":
			continue
		languages[language] += stats["code"]
		for file in stats["reports"]:
			path = file["name"].removeprefix("./")
			buckets[bucket(path, members)] += file["stats"]["code"]
	return languages, buckets


def signed(number: int) -> str:
	if number > 0:
		return f"+{number:,}"
	if number < 0:
		return f"−{-number:,}"
	return "0"


def table(label: str, base: Counter, head: Counter) -> list[str]:
	"""Rows that changed, largest change first, and a total over every row."""
	changed = [key for key in base.keys() | head.keys() if base[key] != head[key]]
	changed.sort(key=lambda key: (-abs(head[key] - base[key]), key))
	lines = [f"| {label} | base | head | Δ |", "|---|--:|--:|--:|"]
	for key in changed:
		lines.append(f"| {key} | {base[key]:,} | {head[key]:,} | {signed(head[key] - base[key])} |")
	total_base, total_head = sum(base.values()), sum(head.values())
	lines.append(f"| **Total** | {total_base:,} | {total_head:,} | **{signed(total_head - total_base)}** |")
	unchanged = len((base.keys() | head.keys()) - set(changed))
	if unchanged:
		lines.append("")
		lines.append(f"{unchanged} unchanged {'row' if unchanged == 1 else 'rows'} not shown.")
	return lines


def render(base: tuple[Counter, Counter], head: tuple[Counter, Counter]) -> str:
	base_languages, base_buckets = base
	head_languages, head_buckets = head
	delta = sum(head_languages.values()) - sum(base_languages.values())
	lines = [
		"## Lines of code",
		"",
		f"Code lines (tokei; blanks, comments and generated files excluded): **{signed(delta)}**.",
		"",
		*table("Language", base_languages, head_languages),
		"",
		*table("Crate or directory", base_buckets, head_buckets),
		"",
	]
	return "\n".join(lines)


def report(base: Path, head: Path, binary: str) -> str:
	return render(
		code_lines(tokei(base, binary), workspace_members(base)),
		code_lines(tokei(head, binary), workspace_members(head)),
	)


def fake(files: dict[str, tuple[str, int]]) -> dict:
	"""tokei's JSON shape, for `{path: (language, code lines)}`."""
	out: dict = {}
	for path, (language, code) in files.items():
		entry = out.setdefault(language, {"code": 0, "reports": [], "children": {}})
		entry["code"] += code
		entry["reports"].append({"name": f"./{path}", "stats": {"code": code}})
	out["Total"] = {"code": sum(code for _, code in files.values())}
	return out


def self_test() -> int:
	members = ["crates/db", "crates/db/outcome", "apps/servers/file_host"]
	failures = []

	for path, expected in [
		("crates/db/outcome/src/lib.rs", "crates/db/outcome"),
		("crates/db/src/lib.rs", "crates/db"),
		("crates/db_extra/src/lib.rs", "crates/"),
		("apps/servers/file_host/src/net.rs", "apps/servers/file_host"),
		("infra/grafana/lib/a.libsonnet", "infra/"),
		("Makefile", OUTSIDE),
	]:
		if (got := bucket(path, members)) != expected:
			failures.append(f"bucket({path!r}) is {got!r}, expected {expected!r}")

	base = code_lines(
		fake({
			"apps/servers/file_host/src/net.rs": ("Rust", 100),
			"crates/db/outcome/src/lib.rs": ("Rust", 50),
			"scripts/a.py": ("Python", 10),
		}),
		members,
	)
	head = code_lines(
		fake({
			"apps/servers/file_host/src/net.rs": ("Rust", 40),
			"crates/db/outcome/src/lib.rs": ("Rust", 50),
			"scripts/a.py": ("Python", 10),
			"scripts/b.py": ("Python", 5),
		}),
		members,
	)
	if base[0]["Rust"] != 150 or head[0]["Rust"] != 90:
		failures.append(f"Rust lines {base[0]['Rust']} -> {head[0]['Rust']}, expected 150 -> 90")
	rendered = render(base, head)
	for needle in [
		"**−55**",
		"| Rust | 150 | 90 | −60 |",
		"| Python | 10 | 15 | +5 |",
		"| apps/servers/file_host | 100 | 40 | −60 |",
		"| scripts/ | 10 | 15 | +5 |",
		"1 unchanged row not shown.",
	]:
		if needle not in rendered:
			failures.append(f"report lacks {needle!r}")
	if "crates/db/outcome |" in rendered:
		failures.append("report lists an unchanged crate")

	if failures:
		print("::error::loc_report.py's own tests failed:")
		for failure in failures:
			print(f"  {failure}")
		return 1
	print("loc_report.py self-test: bucketing, arithmetic and rendering as expected")
	return 0


def main() -> int:
	arguments = sys.argv[1:]
	if "--self-test" in arguments:
		return self_test()
	if len(arguments) != 2:
		print(__doc__, file=sys.stderr)
		return 2
	print(report(Path(arguments[0]), Path(arguments[1]), os.environ.get("TOKEI", "tokei")))
	return 0


if __name__ == "__main__":
	sys.exit(main())

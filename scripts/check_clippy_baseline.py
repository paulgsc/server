#!/usr/bin/env python3
"""Clippy as a ratchet: no new findings, and the debt only goes down.

`.cargo/config.toml` enables `clippy::pedantic`/`nursery` and promotes every
warning to an error with `-D warnings`. The workspace does not meet that bar
today — over a thousand findings, most of them pedantic documentation lints —
so a plain `cargo clippy --workspace -- -D warnings` gate can never pass, and
a gate that can never pass gets paused (which is what happened to lint.yml
from 748df86 until this check replaced it). Lowering the bar to what the code
already meets would throw away the strictness the config chose on purpose.

So the bar stays where it is and existing findings are recorded instead, in
`scripts/clippy_baseline.json`. Three comparisons, each closing a way debt
could grow unnoticed:

  * **findings vs the committed baseline.** A finding the baseline doesn't
    have fails: new code meets the bar. A baseline entry with no finding
    behind it also fails, until the baseline is regenerated with `--update`
    and committed — otherwise a fix leaves headroom that the next change
    could silently spend.
  * **the committed baseline vs the base branch's** (`--base`, which lint.yml
    passes on every PR). Without this, a PR could record its own new findings
    with `--update`, and the first comparison would then pass against the
    baseline that PR itself committed. The baseline may only shrink relative
    to main — except when the PR bumps the pinned clippy toolchain
    (`--allow-growth`, which lint.yml passes only then), since a new clippy
    release brings new lints.
  * **`--update` refuses to record growth** over the baseline it replaces,
    for the same reason, so the local command can't be the way debt sneaks in
    either (again unless `--allow-growth`).

A finding's identity is (file, lint, enclosing item, the source text it
points at), counted. Not the line number, so an unrelated edit that shifts a
finding down the file is not a new finding. Each other part closes a swap that
would otherwise leave the counts unchanged and pass:

  * the source text — fixing one finding and adding a *different* one of the
    same lint in the same file;
  * the enclosing item (the nearest `fn`/`struct`/`enum`/`trait`/`mod`/`impl`
    header at or above the finding) — removing one of several byte-identical
    findings and adding the same line in another item. The workspace has
    exactly this shape: 19 identical `println!` skip guards in one file, one
    per test fn, whose neighbouring lines are identical too.

The floor, stated rather than hidden: two byte-identical findings inside the
*same* item are indistinguishable without line numbers, so removing one and
adding the same line elsewhere in that item passes. The multiset of findings
is unchanged by that edit; closing it would cost the line-shift immunity.

The cost is deliberate: editing a line that carries recorded debt changes its
identity, so a line you touch has to meet the bar — whitespace aside, which is
normalised away — and so does renaming the item that contains it.

Real compiler errors (`E0xxx`) and a build that did not finish always fail,
baseline or not.

Reads the output of:

    cargo clippy --workspace --all-targets --keep-going --message-format=json \\
      -- --cap-lints=warn -A unknown-lints

`--cap-lints=warn` is what makes the set complete: with the config's
`-D warnings` in force, a crate with any finding fails to build, and every
crate that depends on it is then never linted at all. `--all-targets` so test
code is linted too — without it `#[cfg(test)]` code is never compiled, and a
finding there is invisible rather than absent.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from collections import Counter
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
BASELINE = REPO_ROOT / "scripts" / "clippy_baseline.json"
COMPILER_ERROR = re.compile(r"^E\d{4}$")
WHITESPACE = re.compile(r"\s+")
ITEM_HEADER = re.compile(
	r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:(?:async|const|unsafe|default)\s+|extern\s+\"[^\"]*\"\s+)*"
	r"(?:(?P<kind>fn|struct|enum|trait|mod|union)\s+(?P<name>[A-Za-z_][A-Za-z0-9_]*)|(?P<impl>impl\b[^{;]*))"
)

Key = tuple[str, str, str, str]  # (file, lint, enclosing item, source text)

_sources: dict[str, list[str] | None] = {}


def enclosing_item(file: str, line_start: int | None) -> str:
	"""The nearest item header at or above `line_start`, e.g. `fn test_connect`.

	Lexical, like the repo's other check scripts: it reads the file on disk,
	which is the tree clippy just built.
	"""
	if line_start is None:
		return ""
	if file not in _sources:
		path = REPO_ROOT / file
		_sources[file] = path.read_text(encoding="utf-8").splitlines() if path.is_file() else None
	lines = _sources[file]
	if lines is None:
		return ""
	for index in range(min(line_start, len(lines)) - 1, -1, -1):
		match = ITEM_HEADER.match(lines[index])
		if match:
			if match.group("impl"):
				return WHITESPACE.sub(" ", match.group("impl")).strip()
			return match.group("kind") + " " + match.group("name")
	return ""


def primary_span(message: dict) -> dict:
	return next((span for span in message.get("spans", []) if span.get("is_primary")), {})


def relative_file(span: dict) -> str:
	name = span.get("file_name")
	if not name:
		return "<no span>"
	path = Path(name)
	if path.is_absolute():
		try:
			return str(path.resolve().relative_to(REPO_ROOT))
		except ValueError:
			return "<outside the workspace>"
	return name


def source_text(span: dict, message: dict) -> str:
	"""The source the finding points at, whitespace-normalised."""
	lines = [line.get("text", "") for line in span.get("text", [])]
	text = WHITESPACE.sub(" ", " ".join(lines)).strip()
	return text or message.get("message", "")


def read(stream) -> tuple[Counter, list[str], bool]:
	"""Findings per key, hard compiler errors, and whether the build finished."""
	seen: set[tuple] = set()
	findings: Counter = Counter()
	errors: list[str] = []
	succeeded = False

	for line in stream:
		line = line.strip()
		if not line.startswith("{"):
			continue
		record = json.loads(line)
		reason = record.get("reason")
		if reason == "build-finished":
			succeeded = bool(record.get("success"))
			continue
		if reason != "compiler-message":
			continue

		message = record["message"]
		code = (message.get("code") or {}).get("code")
		if code is None:
			# Summaries ("N warnings emitted") and notes carry no code.
			if message.get("level") == "error":
				errors.append(message.get("rendered") or message.get("message", ""))
			continue
		if COMPILER_ERROR.match(code):
			errors.append(message.get("rendered") or message.get("message", ""))
			continue

		span = primary_span(message)
		file = relative_file(span)
		# `--all-targets` compiles a lib's sources twice (lib and lib test),
		# and each reports the same finding; count it once.
		occurrence = (file, span.get("line_start"), span.get("column_start"), code, message.get("message"))
		if occurrence in seen:
			continue
		seen.add(occurrence)
		findings[(file, code, enclosing_item(file, span.get("line_start")), source_text(span, message))] += 1

	return findings, errors, succeeded


def to_json(findings: Counter) -> dict:
	nested: dict[str, dict[str, dict[str, dict[str, int]]]] = {}
	for (file, code, item, text), count in sorted(findings.items()):
		nested.setdefault(file, {}).setdefault(code, {}).setdefault(item or "<file scope>", {})[text] = count
	return nested


def load(path: Path) -> Counter:
	nested = json.loads(path.read_text(encoding="utf-8"))
	counts: Counter = Counter()
	for file, codes in nested.items():
		for code, items in codes.items():
			for item, texts in items.items():
				for text, count in texts.items():
					counts[(file, code, "" if item == "<file scope>" else item, text)] = count
	return counts


def exceeding(current: Counter, allowed: Counter) -> list[tuple[Key, int, int]]:
	return sorted((key, allowed.get(key, 0), count) for key, count in current.items() if count > allowed.get(key, 0))


def report(heading: str, rows: list[tuple[Key, int, int]]) -> None:
	print(heading)
	for (file, code, item, text), before, after in rows:
		print(f"  {file} [{item or 'file scope'}]: {code} {before} -> {after}: {text[:120]}")


def main() -> int:
	parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
	parser.add_argument("messages", type=Path, help="cargo clippy --message-format=json output")
	parser.add_argument("--update", action="store_true", help="rewrite the baseline from these findings (refuses growth)")
	parser.add_argument("--base", type=Path, help="the base branch's baseline; the committed one may not exceed it")
	parser.add_argument("--allow-growth", action="store_true", help="permit new findings: only for a clippy toolchain bump")
	args = parser.parse_args()

	with args.messages.open(encoding="utf-8") as stream:
		findings, errors, succeeded = read(stream)

	if errors or not succeeded:
		print("::error::clippy did not build the workspace — these are compiler errors, not lint findings:")
		for error in errors:
			print(error)
		if not errors:
			print("  (no error diagnostics were reported; see the cargo output above)")
		return 1

	committed = load(BASELINE) if BASELINE.exists() else Counter()

	if args.update:
		grew = exceeding(findings, committed)
		if grew and not args.allow_growth:
			report("::error::refusing to record new findings in the baseline — fix them instead:", grew)
			print("(--allow-growth exists only for a clippy toolchain bump, which brings new lints.)")
			return 1
		BASELINE.write_text(json.dumps(to_json(findings), indent=2, sort_keys=True) + "\n", encoding="utf-8")
		files = len({file for file, *_ in findings})
		print(f"wrote {sum(findings.values())} findings across {files} files to {BASELINE.relative_to(REPO_ROOT)}")
		return 0

	failed = False

	grew = exceeding(findings, committed)
	if grew:
		report("::error::new clippy findings — fix them (the baseline records existing debt, it is not headroom):", grew)
		failed = True

	# (key, recorded, found): the baseline allows more than clippy found.
	stale = [(key, recorded, found) for key, found, recorded in exceeding(committed, findings)]
	if stale:
		print("::error::clippy findings were fixed but the baseline still allows them — run")
		print("  python3 scripts/check_clippy_baseline.py <messages.json> --update")
		report("and commit scripts/clippy_baseline.json so the fix can't be undone silently:", stale)
		failed = True

	if args.base is not None:
		recorded = exceeding(committed, load(args.base))
		if recorded and not args.allow_growth:
			report("::error::the committed baseline records findings the base branch's does not — fix them instead:", recorded)
			failed = True

	if failed:
		return 1
	print(f"clippy: {sum(findings.values())} findings, all recorded in the baseline; nothing new")
	return 0


if __name__ == "__main__":
	sys.exit(main())

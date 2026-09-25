#!/usr/bin/env python3
"""Every `#[instrument]` must say what it skips.

`#[tracing::instrument]` records every argument of the function it wraps as a
span field unless told otherwise. On a handler that is the request body, the
extracted state, the headers — log bloat at best, and at worst a field that
carries something this deployment has promised not to record. So the policy
is that an `#[instrument]` names its arguments explicitly: `skip(...)` for
the ones it drops, or `skip_all` and an explicit `fields(...)` list.

This replaces a grep step in lint.yml that had two blind spots, both silent:

  * it searched `crates/ src/`, and there is no root `src/` — every handler
    lives under `apps/`, so the directory the policy mattered most for was
    never scanned;
  * it read one line at a time, so an attribute written across several
    lines was judged by its first line alone (`#[instrument(` with the
    `skip_all` on the next line was reported, and one with no skip at all
    passed as long as its first line mentioned one elsewhere).

Here an attribute is read to its matching `]`, however many lines that is,
and `skip(`/`skip_all` must appear as an argument outside any string literal
(`name = "skip"` records every argument all the same). `--self-test` runs the
rule against known-compliant and known-noncompliant attributes, and runs in
lint.yml next to the real scan.

Deliberately lexical, like scripts/check_metric_contract.py: a real parse
would need the whole workspace to build, which would make the cheapest check
in the repo the most expensive one to run.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
RUST_ROOTS = [REPO_ROOT / "apps", REPO_ROOT / "crates"]

ATTRIBUTE = re.compile(r"#\[\s*(?:tracing\s*::\s*)?instrument\b")
# `skip(...)` or `skip_all` as an argument of the attribute — matched only
# after string literals are blanked, so `name = "skip"` or
# `fields(note = "skip_all")` is not mistaken for a skip directive.
SKIP = re.compile(r"(?:[(,]\s*)(?:skip_all\b|skip\s*\()")
STRING = re.compile(r'"(?:\\.|[^"\\])*"')


def names_its_skips(attribute: str) -> bool:
	return bool(SKIP.search(STRING.sub('""', attribute)))


def attribute_text(source: str, start: int) -> str:
	"""The attribute beginning at `start` (its `#`), through its matching `]`."""
	depth = 0
	for index in range(start + 1, len(source)):
		char = source[index]
		if char == "[":
			depth += 1
		elif char == "]":
			depth -= 1
			if depth == 0:
				return source[start : index + 1]
	return source[start:]


def offenders() -> list[str]:
	found = []
	for root in RUST_ROOTS:
		for path in sorted(root.rglob("*.rs")):
			if "target" in path.relative_to(REPO_ROOT).parts:
				continue
			source = path.read_text(encoding="utf-8")
			for match in ATTRIBUTE.finditer(source):
				attribute = attribute_text(source, match.start())
				if not names_its_skips(attribute):
					line = source.count("\n", 0, match.start()) + 1
					found.append(f"{path.relative_to(REPO_ROOT)}:{line}: {' '.join(attribute.split())}")
	return found


COMPLIANT = [
	"#[instrument(skip_all)]",
	'#[instrument(name = "health", skip_all)]',
	"#[instrument(skip(state, body))]",
	"#[tracing::instrument(skip (state))]",
	'#[instrument(\n\tname = "x",\n\tskip_all,\n\tfields(otel.kind = "server")\n)]',
]
NONCOMPLIANT = [
	"#[instrument]",
	'#[instrument(name = "health")]',
	'#[instrument(name = "skip")]',
	'#[instrument(fields(note = "skip_all"))]',
	'#[instrument(name = "skip(x)")]',
	"#[instrument(fields(skipped = true))]",
]


def self_test() -> int:
	failures = [f"not compliant: {case!r}" for case in COMPLIANT if not names_its_skips(case)]
	failures += [f"compliant: {case!r}" for case in NONCOMPLIANT if names_its_skips(case)]
	if failures:
		print("::error::check_instrument_skip.py's own rule tests failed:")
		for failure in failures:
			print(f"  {failure}")
		return 1
	print(f"check_instrument_skip.py rule tests: {len(COMPLIANT)} compliant, {len(NONCOMPLIANT)} not, as expected")
	return 0


def main() -> int:
	if "--self-test" in sys.argv[1:]:
		return self_test()
	found = offenders()
	if not found:
		print("every #[instrument] names what it skips")
		return 0
	print("::error::#[instrument] without skip(...) or skip_all — name the arguments it records explicitly:")
	for offender in found:
		print(f"  {offender}")
	return 1


if __name__ == "__main__":
	sys.exit(main())

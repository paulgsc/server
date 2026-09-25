#!/usr/bin/env python3
"""Clippy as a ratchet: no new findings, and the debt only goes down.

`.cargo/config.toml` enables `clippy::pedantic`/`nursery` and promotes every
warning to an error with `-D warnings`. The workspace does not meet that bar
today — several hundred findings, most of them pedantic documentation lints —
so a plain `cargo clippy --workspace -- -D warnings` gate can never pass, and
a gate that can never pass gets paused (which is what happened to lint.yml
from 748df86 until this check replaced it). Lowering the bar to what the code
already meets would throw away the strictness the config chose on purpose.

So the bar stays where it is and existing findings are recorded instead, in
`scripts/clippy_baseline.json`, as a count per (file, lint):

  * a count going *up*, or a (file, lint) pair the baseline has never seen,
    fails — that is a new finding, and new code meets the bar;
  * a count going *down* also fails, until the baseline is regenerated with
    `--update` and committed. Otherwise a fix leaves headroom behind it that
    the next change could silently spend.

Counts rather than line numbers so an unrelated edit that shifts a finding
down the file is not a new finding. Real compiler errors (`E0xxx`) and a
failed build always fail, baseline or not.

Reads the output of:

    cargo clippy --workspace --all-targets --keep-going --message-format=json \\
      -- --cap-lints=warn -A unknown-lints

`--cap-lints=warn` is what makes the count complete: with the config's
`-D warnings` in force, a crate with any finding fails to build, and every
crate that depends on it is then never linted at all. `--all-targets` so test
code is linted too — without it `#[cfg(test)]` code is never compiled, and a
finding there is invisible rather than absent.

The findings depend on the clippy version. lint.yml pins its toolchain; when
bumping it, regenerate the baseline with the new version in the same change.
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


def primary_file(message: dict) -> str | None:
	for span in message.get("spans", []):
		if span.get("is_primary"):
			name = span["file_name"]
			path = Path(name)
			if path.is_absolute():
				try:
					return str(path.resolve().relative_to(REPO_ROOT))
				except ValueError:
					return "<outside the workspace>"
			return name
	return None


def read(stream) -> tuple[Counter, list[str], bool]:
	"""Findings per (file, lint), hard compiler errors, and whether the build finished."""
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

		file = primary_file(message) or "<no span>"
		span = next((s for s in message.get("spans", []) if s.get("is_primary")), {})
		# `--all-targets` compiles a lib's sources twice (lib and lib test),
		# and each reports the same finding; count it once.
		identity = (file, span.get("line_start"), span.get("column_start"), code, message.get("message"))
		if identity in seen:
			continue
		seen.add(identity)
		findings[(file, code)] += 1

	return findings, errors, succeeded


def as_json(findings: Counter) -> dict:
	nested: dict[str, dict[str, int]] = {}
	for (file, code), count in sorted(findings.items()):
		nested.setdefault(file, {})[code] = count
	return nested


def load_baseline() -> Counter:
	if not BASELINE.exists():
		return Counter()
	nested = json.loads(BASELINE.read_text(encoding="utf-8"))
	return Counter({(file, code): count for file, codes in nested.items() for code, count in codes.items()})


def main() -> int:
	parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
	parser.add_argument("messages", type=Path, help="cargo clippy --message-format=json output")
	parser.add_argument("--update", action="store_true", help="rewrite the baseline from these findings")
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

	if args.update:
		BASELINE.write_text(json.dumps(as_json(findings), indent=2, sort_keys=True) + "\n", encoding="utf-8")
		print(f"wrote {sum(findings.values())} findings across {len({f for f, _ in findings})} files to {BASELINE.relative_to(REPO_ROOT)}")
		return 0

	baseline = load_baseline()
	grew = sorted((key, baseline.get(key, 0), count) for key, count in findings.items() if count > baseline.get(key, 0))
	shrank = sorted((key, count, findings.get(key, 0)) for key, count in baseline.items() if findings.get(key, 0) < count)

	if grew:
		print("::error::new clippy findings — fix them (the baseline records existing debt, it is not headroom):")
		for (file, code), before, after in grew:
			print(f"  {file}: {code} {before} -> {after}")
	if shrank:
		print("::error::clippy findings were fixed but the baseline still allows them — run")
		print("  python3 scripts/check_clippy_baseline.py <messages.json> --update")
		print("and commit scripts/clippy_baseline.json so the fix can't be undone silently:")
		for (file, code), before, after in shrank:
			print(f"  {file}: {code} {before} -> {after}")
	if grew or shrank:
		return 1

	print(f"clippy: {sum(findings.values())} findings, all recorded in the baseline; nothing new")
	return 0


if __name__ == "__main__":
	sys.exit(main())

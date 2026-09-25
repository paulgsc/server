#!/usr/bin/env python3
"""Fail on tracked `.rs` files that no build target compiles.

Rust has no "unused file" lint. A file becomes part of a crate only when some
`mod` declaration reaches it, so a file nothing declares is invisible to
rustc: its `dead_code` never fires, its imports are never resolved, and
clippy never reads it. It can sit in the tree indefinitely, reading like live
code. Thirteen files had done exactly that before this check existed.

rustc already knows the answer. Every compilation writes a Makefile-style
dep-info file (`target/<profile>/**/*.d`) listing each source file it read.
The union of those lists is the set of files the build actually uses, and any
tracked `.rs` file outside that set is an orphan.

Run this after a build that covers every target and feature:

    cargo check --workspace --all-targets --all-features
    python3 scripts/check_orphan_sources.py

Without `--all-targets`, tests, examples and benches count as orphans. Without
`--all-features`, feature-gated modules do. In a fresh CI checkout the
target directory holds only that one build. Locally, stale `.d` files from
older builds can hide a newly orphaned file. That is a false negative only, so
the check never fails on a file that is actually compiled.

EXEMPT lists the paths that are not workspace sources, with a reason for each.
"""

from __future__ import annotations

import os
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(os.environ.get("REPO_ROOT", Path(__file__).resolve().parent.parent))
TARGET = Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target"))

# Each entry is a regex over the repo-relative path.
EXEMPT = {
	# Its own standalone crate, excluded from the workspace in the root
	# Cargo.toml, so the workspace build never compiles it.
	r"^\.github/scripts/": "standalone crate outside the workspace",
	# trybuild fixtures, compiled by trybuild in a scratch project whose
	# dep-info lands outside this target directory.
	r"/tests/ui/": "trybuild fixture",
}

# A dep-info line is `output: dep dep ...`, with spaces inside a path escaped
# as `\ `.
_DEP = re.compile(r"(?:\\ |[^\s])+")


def compiled_sources() -> set[str]:
	seen: set[str] = set()
	for dep_file in TARGET.rglob("*.d"):
		try:
			text = dep_file.read_text(errors="replace")
		except OSError:
			continue
		for line in text.splitlines():
			_, sep, deps = line.partition(": ")
			if not sep:
				continue
			for raw in _DEP.findall(deps):
				path = raw.replace("\\ ", " ")
				if not path.endswith(".rs"):
					continue
				p = Path(path)
				if p.is_absolute():
					try:
						p = p.relative_to(ROOT)
					except ValueError:
						continue  # a registry or toolchain source
				seen.add(p.as_posix())
	return seen


def tracked_sources() -> list[str]:
	out = subprocess.run(
		["git", "ls-files", "*.rs"], cwd=ROOT, check=True, capture_output=True, text=True
	).stdout
	return sorted(line for line in out.splitlines() if line)


def main() -> int:
	compiled = compiled_sources()
	if not compiled:
		print(
			f"no dep-info found under {TARGET}; run "
			"`cargo check --workspace --all-targets --all-features` first",
			file=sys.stderr,
		)
		return 2

	orphans = [
		f
		for f in tracked_sources()
		if f not in compiled and not any(re.search(pat, f) for pat in EXEMPT)
	]
	if orphans:
		print(f"{len(orphans)} tracked .rs file(s) are compiled by no target:\n")
		for f in orphans:
			print(f"  {f}")
		print(
			"\nEach one needs a `mod` declaration that reaches it, or should be "
			"deleted. If it is deliberately outside the build, add it to EXEMPT "
			"in scripts/check_orphan_sources.py with the reason."
		)
		return 1
	print(f"ok: every tracked .rs file is compiled ({len(compiled)} sources seen)")
	return 0


if __name__ == "__main__":
	sys.exit(main())

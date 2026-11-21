#!/usr/bin/env python3
"""Every file a workspace manifest names exists (S11, paulgsc/server#491).

`clippy::cargo_common_metadata` (denied through the workspace's `cargo` group)
asks each package for a `readme`, and is satisfied by the key alone: it never
looks for the file. So 36 manifests named a `README.md` that was never
written, and nothing saw it. This reads every tracked `Cargo.toml` and fails
on a `readme`, `license-file` or `build` path that resolves to no file.

A key set to `{ workspace = true }` (or `key.workspace = true`) is resolved
against the root manifest's `[workspace.package]`, relative to the root, as
cargo does. `readme = false` names no file and passes.

Stdlib only (tomllib, Python 3.11+). Run from the repository root:

    python3 scripts/check_manifests.py --self-test
    python3 scripts/check_manifests.py
"""

from __future__ import annotations

import subprocess
import sys
import tempfile
import tomllib
from pathlib import Path

# The package keys that name a file, relative to the manifest's directory.
FILE_KEYS = ("readme", "license-file", "build")


def inherited(value: object) -> bool:
	return isinstance(value, dict) and value.get("workspace") is True


def problems_in(root: Path, manifests: list[Path]) -> list[str]:
	"""Why each of `manifests` (relative to `root`) names a missing file."""
	root_manifest = tomllib.loads((root / "Cargo.toml").read_text())
	shared = root_manifest.get("workspace", {}).get("package", {})
	found: list[str] = []
	for manifest in manifests:
		data = tomllib.loads((root / manifest).read_text())
		package = data.get("package")
		if not isinstance(package, dict):
			continue
		for key in FILE_KEYS:
			value = package.get(key)
			base = (root / manifest).parent
			if inherited(value):
				if key not in shared:
					found.append(f"{manifest}: {key} is inherited but [workspace.package] sets none")
					continue
				value, base = shared[key], root
			if value is None or value is False:
				continue
			if value is True:
				# `build = true` / `readme = true`: cargo's own default name.
				value = {"readme": "README.md", "build": "build.rs"}.get(key)
				if value is None:
					continue
			if not isinstance(value, str):
				found.append(f"{manifest}: {key} is neither a path nor false")
				continue
			if not (base / value).is_file():
				found.append(f"{manifest}: {key} = \"{value}\" names no file ({(base / value).relative_to(root) if (base / value).is_relative_to(root) else base / value})")
	return found


def tracked_manifests(root: Path) -> list[Path]:
	out = subprocess.run(["git", "ls-files", "-z", "--", "Cargo.toml", "*/Cargo.toml"], cwd=root, check=True, capture_output=True).stdout
	return [Path(name) for name in out.decode().split("\0") if name]


def self_test() -> None:
	with tempfile.TemporaryDirectory() as tmp:
		root = Path(tmp)
		(root / "README.md").write_text("root\n")
		(root / "Cargo.toml").write_text('[workspace]\nmembers = ["a", "b", "c", "d", "e", "f"]\n[workspace.package]\nreadme = "README.md"\n')
		for name, body in {
			"a": 'readme = "README.md"\n',  # present
			"b": 'readme = "README.md"\n',  # missing
			"c": "readme.workspace = true\n",  # inherited, present at the root
			"d": "readme = false\n",  # names no file
			"e": 'build = "build.rs"\nlicense-file = "LICENSE"\n',  # both missing
			"f": "license-file = { workspace = true }\n",  # inherited, unset
		}.items():
			(root / name).mkdir()
			(root / name / "Cargo.toml").write_text(f'[package]\nname = "{name}"\n{body}')
		(root / "a" / "README.md").write_text("a\n")
		got = problems_in(root, [Path(n) / "Cargo.toml" for n in "abcdef"])
		want_prefixes = ["b/Cargo.toml: readme", "e/Cargo.toml: license-file", "e/Cargo.toml: build", "f/Cargo.toml: license-file is inherited"]
		assert len(got) == len(want_prefixes), got
		for prefix in want_prefixes:
			assert any(problem.startswith(prefix) for problem in got), (prefix, got)
	print("check_manifests self-test: ok")


def main() -> int:
	if sys.argv[1:] == ["--self-test"]:
		self_test()
		return 0
	root = Path.cwd()
	manifests = tracked_manifests(root)
	found = problems_in(root, manifests)
	for problem in found:
		print(problem)
	if found:
		print(f"\n{len(found)} manifest path(s) name no file: write the file, or inherit the workspace's (`readme.workspace = true`).")
		return 1
	print(f"{len(manifests)} manifests: every file they name exists.")
	return 0


if __name__ == "__main__":
	sys.exit(main())

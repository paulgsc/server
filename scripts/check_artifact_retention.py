#!/usr/bin/env python3
"""Every uploaded artifact lives one day, or says why it lives longer.

An `actions/upload-artifact` or `actions/upload-pages-artifact` step with no
`retention-days` keeps its artifact for the repository default, 90 days unless
Settings says otherwise: a period nobody chose, for a file almost always read
only by later jobs of the same run. And artifacts cannot be overwritten across
runs, so each run's copy stays until it expires. So the rule (CLAUDE.md,
"Workflow storage") is that such a step sets `retention-days: 1`, or sets a
longer period explicitly with a `Retention:` line in the unbroken `#` comment
block directly above the step, saying who reads it after that day. A step with
no `retention-days` fails either way: a stated reason belongs next to the
number it justifies. So does one whose value is not a literal number of days
(`0`, empty, a `${{ }}` expression): each can resolve to the default.

Scans `.github/workflows/*.y(a)ml` and `.github/actions/**/action.y(a)ml`. It
reads `uses:` lines, so an upload wrapped in a local composite action is seen
there, in the action, not at its call site.

Deliberately lexical, like scripts/check_instrument_skip.py: a YAML parser
would be a dependency for the one check in lint.yml that needs none. So it
accepts one closed shape rather than listing the ones it rejects: the value is
read only as a direct child of the step's block-style `with:` map, which is
the only place the action receives it. A `retention-days` anywhere else in the
step (under `env:`, say) does not count, and a flow-style `with: {...}`, or
a whole step written as `- {uses: ...}`, fails with a message asking for block
style, since the check cannot read it.
`--self-test` runs the rule against known-compliant and known-noncompliant
steps first, so a regex edit that stops matching fails instead of passing
every scan silently. paulgsc/some-ui runs the same rule
(packages/eslint/src/workflow-guards.ts).
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent

UPLOAD_STEP = re.compile(r"^(\s*)(-\s+)?uses:\s*[\"']?actions/upload-(?:pages-)?artifact@")
# The same reference anywhere else on a line (`- {uses: actions/upload-artifact@v4, ...}`)
# is a step this check cannot read, so it fails rather than being skipped.
UPLOAD_ANYWHERE = re.compile(r"uses:\s*[\"']?actions/upload-(?:pages-)?artifact@")
SEQUENCE_ITEM = re.compile(r"^(\s*)-(\s+)\S")
RETENTION = re.compile(r"^\s*retention-days:\s*(.*)$")
WITH_KEY = re.compile(r"^\s*with:(.*)$")
RETENTION_TAG = re.compile(r"^#+\s*Retention:\s*\S")
DAYS = re.compile(r"^[1-9][0-9]*$")


def indent_of(line: str) -> int:
	return len(line) - len(line.lstrip())


def is_blank_or_comment(line: str) -> bool:
	stripped = line.strip()
	return stripped == "" or stripped.startswith("#")


def scalar_value(raw: str) -> str:
	"""`"1"`, `'1'` and `1 # note` all read as `1`."""
	value = re.sub(r"\s+#.*$", "", raw).strip()
	quoted = re.fullmatch(r"([\"'])(.*)\1", value)
	return quoted.group(2) if quoted else value


def step_start(lines: list[str], uses_line: int) -> int:
	"""The step's own `- ` item line: walk up to the item whose key column matches."""
	key_column = indent_of(lines[uses_line])
	for index in range(uses_line - 1, -1, -1):
		line = lines[index]
		if is_blank_or_comment(line):
			continue
		item = SEQUENCE_ITEM.match(line)
		if item and len(item.group(1)) + 1 + len(item.group(2)) == key_column:
			return index
		if indent_of(line) < key_column:
			break
	return uses_line


def step_body(lines: list[str], start: int, key_column: int) -> list[str]:
	"""The step's lines, its `- ` marker blanked so every key sits at `key_column`."""
	first = lines[start]
	body = [" " * key_column + first[key_column:]] if SEQUENCE_ITEM.match(first) else [first]
	for following in lines[start + 1 :]:
		if not is_blank_or_comment(following) and indent_of(following) < key_column:
			break
		body.append(following)
	return body


def with_retention(body: list[str], key_column: int) -> tuple[str, str | None]:
	"""("value", v) for `retention-days` directly under a block `with:`, else ("missing"|"flow", None)."""
	for index, line in enumerate(body):
		key = WITH_KEY.match(line)
		if not key or indent_of(line) != key_column:
			continue
		if re.sub(r"(^|\s+)#.*$", "", key.group(1)).strip():
			return ("flow", None)
		child_column = None
		for child in body[index + 1 :]:
			if is_blank_or_comment(child):
				continue
			if indent_of(child) <= key_column:
				break
			if child_column is None:
				child_column = indent_of(child)
			retention = RETENTION.match(child)
			if retention and indent_of(child) == child_column:
				return ("value", scalar_value(retention.group(1)))
		return ("missing", None)
	return ("missing", None)


def violations(text: str) -> list[tuple[int, str]]:
	"""(1-based line of the step, what is wrong) for every noncompliant upload."""
	lines = text.split("\n")
	found = []
	for index, line in enumerate(lines):
		upload = UPLOAD_STEP.match(line)
		if not upload:
			if not line.strip().startswith("#") and UPLOAD_ANYWHERE.search(line):
				found.append((index + 1, "is written in a form this check cannot read (a flow-style step?): write the step as a block mapping, `uses:` and `with:` each on their own line"))
			continue
		key_column = len(upload.group(1)) + len(upload.group(2) or "")
		start = index if upload.group(2) else step_start(lines, index)

		shape, value = with_retention(step_body(lines, start, key_column), key_column)
		if shape == "flow":
			found.append((start + 1, "has a flow-style `with: {...}` this check cannot read: write `with:` as a block mapping, with `retention-days` on its own line"))
			continue
		if value is None:
			found.append((start + 1, "sets no retention-days in its `with:` map: set `retention-days: 1`, or a longer period with a `# Retention: <why>` line directly above the step"))
			continue
		if value == "1":
			continue
		if not DAYS.match(value):
			found.append((start + 1, f"sets retention-days to {value!r}, not a literal number of days: 0, empty or an expression can mean the repository default, so no `Retention:` line can justify it"))
			continue

		tagged = False
		for above in reversed(lines[:start]):
			if not above.strip().startswith("#"):
				break
			if RETENTION_TAG.match(above.strip()):
				tagged = True
		if not tagged:
			found.append((start + 1, f"keeps its artifact for {value} (not 1 day) with no `# Retention: <why>` line in the comment block directly above it"))
	return found


def workflow_files() -> list[Path]:
	github = REPO_ROOT / ".github"
	files = [*github.glob("workflows/*.yml"), *github.glob("workflows/*.yaml")]
	files += [*github.glob("actions/**/action.yml"), *github.glob("actions/**/action.yaml")]
	return sorted(files)


def offenders() -> list[str]:
	return [
		f"{path.relative_to(REPO_ROOT)}:{line}: upload step {problem}"
		for path in workflow_files()
		for line, problem in violations(path.read_text(encoding="utf-8"))
	]


def steps(*body: str) -> str:
	return "\n".join(["jobs:", "  a:", "    steps:", *body])


COMPLIANT = [
	steps("      - name: Upload", "        uses: actions/upload-artifact@v4", "        with:", "          retention-days: 1"),
	steps("      - uses: actions/upload-pages-artifact@v3", "        with:", "          retention-days: '1' # same run"),
	steps(
		"      # Retention: 30 days, read by the release checklist.",
		"      # More words.",
		"      - uses: actions/upload-artifact@v4",
		"        with:",
		"          retention-days: 30",
	),
	steps("      - uses: actions/download-artifact@v4", "      - uses: someone/upload-artifact@v1", "      - run: echo actions/upload-artifact@v4"),
	steps("      - with: # inputs first", "          retention-days: 1", "        uses: actions/upload-artifact@v4"),
	steps("      # e.g. - {uses: actions/upload-artifact@v4}", "      - run: echo"),
]
NONCOMPLIANT = [
	# No retention-days, tagged or not.
	steps("      - uses: actions/upload-artifact@v4", "        with:", "          name: a"),
	steps("      # Retention: tagged, but the period is the default", "      - uses: actions/upload-artifact@v4"),
	# retention-days in the next step does not count for this one.
	steps("      - uses: actions/upload-artifact@v4", "      - run: echo", "        with:", "          retention-days: 1"),
	# Anything but 1, untagged.
	steps("      - uses: actions/upload-artifact@v4", "        with:", "          retention-days: 90"),
	steps("      - uses: actions/upload-artifact@v4", "        with:", "          retention-days: 0"),
	steps("      - uses: actions/upload-artifact@v4", "        with:", "          retention-days: ${{ inputs.days }}"),
	# Not a literal number of days: can mean the default, so a tag does not excuse it.
	steps("      # Retention: tagged.", "      - uses: actions/upload-artifact@v4", "        with:", "          retention-days: 0"),
	steps("      # Retention: tagged.", "      - uses: actions/upload-artifact@v4", "        with:", "          retention-days:"),
	steps("      # Retention: tagged.", "      - uses: actions/upload-artifact@v4", "        with:", "          retention-days: ${{ inputs.days }}"),
	steps("      # Retention: tagged.", "      - uses: actions/upload-artifact@v4", "        with:", "          retention-days: 7d"),
	# Only a direct child of the block `with:` map is the action's input.
	steps("      - uses: actions/upload-artifact@v4", "        env:", "          retention-days: 1"),
	steps("      - uses: actions/upload-artifact@v4", "        with:", "          name: a", "        env:", "          retention-days: 1"),
	steps("      - uses: actions/upload-artifact@v4", "        with:", "          nested:", "            retention-days: 1"),
	steps("      - uses: actions/upload-artifact@v4", "        retention-days: 1"),
	# Flow style is not read, so it fails and says so.
	steps("      - uses: actions/upload-artifact@v4", "        with: {path: dist, retention-days: 1}"),
	steps("      - {uses: actions/upload-artifact@v4, with: {path: dist}}"),
	steps("      - {uses: actions/upload-artifact@v4, with: {path: dist, retention-days: 1}}"),
	# A tag cut off by a blank line, a code line, or with no reason after it.
	steps("      # Retention: 30 days.", "", "      - uses: actions/upload-artifact@v4", "        with:", "          retention-days: 30"),
	steps("      # Retention: 30 days.", "      - run: echo", "      - uses: actions/upload-artifact@v4", "        with:", "          retention-days: 30"),
	steps("      # Retention:", "      - uses: actions/upload-artifact@v4", "        with:", "          retention-days: 30"),
]


def self_test() -> int:
	failures = [f"not compliant: {case!r}" for case in COMPLIANT if violations(case)]
	failures += [f"compliant: {case!r}" for case in NONCOMPLIANT if len(violations(case)) != 1]
	if failures:
		print("::error::check_artifact_retention.py's own rule tests failed:")
		for failure in failures:
			print(f"  {failure}")
		return 1
	print(f"check_artifact_retention.py rule tests: {len(COMPLIANT)} compliant, {len(NONCOMPLIANT)} not, as expected")
	return 0


def main() -> int:
	if "--self-test" in sys.argv[1:]:
		return self_test()
	found = offenders()
	if not found:
		print(f"every artifact upload in {len(workflow_files())} workflow files keeps one day or says why")
		return 0
	print("::error::artifact upload without a chosen retention (CLAUDE.md, \"Workflow storage\"):")
	for offender in found:
		print(f"  {offender}")
	return 1


if __name__ == "__main__":
	sys.exit(main())

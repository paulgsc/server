#!/usr/bin/env python3
"""One check that passes only when every check a pull request runs here passed (GC5, #526).

`main`'s ruleset can require checks only by name, and the checks this
repository runs on a pull request live in several workflows, some of them
filtered by path (`routes.yml`'s `Emit the route snapshot` and `Check some-ui
against it`) and some skipped inside a run when their paths did not change
(`test.yml`'s `rust_ci` and `infra_ci`). A job cannot `needs` a job in
another workflow, so this script, run as `.github/workflows/gate.yml`'s
`PR gate` job, waits on them through the API instead:

1. **What should run.** It reads every `.github/workflows/*.y(a)ml` the pull
   request's merge commit holds (the ones GitHub runs for it) and, for each
   with a `pull_request` or `pull_request_target` trigger, decides from that
   trigger's `branches`/`branches-ignore` and `paths`/`paths-ignore` filters,
   against the base branch and the pull request's changed files, whether
   GitHub starts it. Those runs are **expected**.
2. **What runs.** It lists the workflow runs for the head commit (events
   `pull_request` and `pull_request_target`, and `dynamic`: the runs GitHub
   starts from no file here, as CodeQL's default setup does), the latest per
   workflow file, and the check runs on the head commit that another app
   (code scanning's `CodeQL`, say) reports. Every one of them is awaited,
   expected or not, except this gate's own run.
3. **The verdict.** It passes once every expected workflow has a run, and
   every awaited run and check run has completed with `success`, `skipped`
   or `neutral`. Any other conclusion (`failure`, `cancelled`, `timed_out`,
   `action_required`, `startup_failure`, `stale`) fails it as soon as it is
   seen, naming the run's failed jobs: nothing that failed passes without a
   re-run. A run is judged by its own conclusion, so a job its workflow lets
   fail (`continue-on-error`: deny.yml's advisories) is reported as a
   warning, as its workflow's author chose. Anything still unfinished after
   `--minutes` (60) fails it too.

**Skipped passes only where its paths did not change.** A whole workflow is
skipped by its `paths` filter: step 1 reads the same filter, so a run it
expects that never appears fails the gate, while one whose paths did not
change is not expected and not waited on. Inside a run, a job skipped by its
`if:` passes; every `if:` on a pull-request job here reads either the
event (`sync`, `stage`: not pull-request jobs) or a path diff
(`determine_jobs`), and a job skipped because one it `needs` failed is
reported through that one. A new `if:` that skips for any other reason is a
review question, not one this script can answer.

**A check that never appears.** GitHub creates every workflow run for an
event when it handles the event, before any job starts, so by the time this
job runs the others exist. An expected run still missing after GRACE fails
the gate, naming the filter that expected it; it does not wait out the whole
deadline. One that GitHub started though step 1 did not expect it is awaited
and warned about, since then this reading of the filter is wrong. GitHub
evaluates a path filter on at most the first 300 changed files, so on a
larger pull request a missing path-filtered run is a warning, not a failure.

**Not itself.** Runs of this gate's own workflow file are never awaited, so
the gate does not wait on itself, and that workflow must hold only the gate:
any other job in it would be awaited by nothing, so the gate fails if its
own run has more than one job.

**Re-runs.** After "Re-run failed jobs" on another workflow, re-run this one
too: it reads the latest attempt of each run, but nothing starts it again.

    python3 scripts/pr_gate.py                   # in Actions, from the event
    python3 scripts/pr_gate.py --repo paulgsc/server --pr 555 --once
    python3 scripts/pr_gate.py --self-test

`--once` evaluates once and prints where every check stands, without
waiting. Needs `GITHUB_TOKEN` with `actions: read`, `checks: read` and
`pull-requests: read`. Design and boundaries: docs/assurance.md.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import sys
import time
import urllib.error
import urllib.request
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

REPO_ROOT = Path(__file__).resolve().parent.parent
PR_EVENTS = ("pull_request", "pull_request_target")
# `dynamic`: a run GitHub starts from no file here, as CodeQL's default setup does for a pull request.
AWAITED_EVENTS = (*PR_EVENTS, "dynamic")
PASSING = {"success", "skipped", "neutral"}
GRACE_SECONDS = 10 * 60
POLL_SECONDS = 30
PATH_FILTER_LIMIT = 300


class Unreadable(Exception):
	pass


# --- reading a workflow's pull-request trigger ------------------------------------------


@dataclass(frozen=True)
class Trigger:
	branches: tuple[str, ...] | None = None
	branches_ignore: tuple[str, ...] | None = None
	paths: tuple[str, ...] | None = None
	paths_ignore: tuple[str, ...] | None = None


FILTERS = {"branches": "branches", "branches-ignore": "branches_ignore", "paths": "paths", "paths-ignore": "paths_ignore"}


def indent_of(line: str) -> int:
	return len(line) - len(line.lstrip(" "))


def uncomment(value: str) -> str:
	return re.sub(r"(^|\s)#.*$", "", value).strip()


def scalar(value: str) -> str:
	value = uncomment(value)
	quoted = re.fullmatch(r"([\"'])(.*)\1", value)
	return quoted.group(2) if quoted else value


def flow_list(value: str, where: str) -> list[str]:
	value = uncomment(value)
	if not (value.startswith("[") and value.endswith("]")):
		raise Unreadable(f"{where}: expected a list, found {value!r}")
	inner = value[1:-1].strip()
	return [scalar(item) for item in inner.split(",")] if inner else []


def block(lines: list[str], start: int, parent_indent: int) -> list[tuple[int, str]]:
	"""(index, line) of the lines nested under the key at `start`, comments and blanks dropped."""
	out = []
	for index in range(start + 1, len(lines)):
		line = lines[index]
		if not line.strip() or line.lstrip().startswith("#"):
			continue
		if indent_of(line) <= parent_indent:
			break
		out.append((index, line))
	return out


def read_trigger(text: str) -> Trigger | None:
	"""The workflow's pull-request trigger, or None when it has none. Raises Unreadable."""
	lines = text.replace("\t", "    ").split("\n")
	on_line = next((i for i, line in enumerate(lines) if re.match(r"""^(on|"on"|'on'|true):""", line)), None)
	if on_line is None:
		raise Unreadable("no top-level `on:`")
	rest = uncomment(lines[on_line].split(":", 1)[1])
	if rest:
		events = flow_list(rest, "on") if rest.startswith("[") else [scalar(rest)]
		return Trigger() if any(e in PR_EVENTS for e in events) else None
	children = block(lines, on_line, 0)
	if not children:
		raise Unreadable("`on:` is empty")
	column = indent_of(children[0][1])
	found: list[Trigger] = []
	for index, line in children:
		if indent_of(line) != column:
			continue
		key = re.match(r"^\s*([\w-]+):(.*)$", line)
		if not key:
			if line.strip().startswith("- "):
				events = [scalar(item.strip()[2:]) for _, item in children if indent_of(item) == column and item.strip().startswith("- ")]
				return Trigger() if any(e in PR_EVENTS for e in events) else None
			raise Unreadable(f"line {index + 1}: cannot read {line.strip()!r}")
		if key.group(1) not in PR_EVENTS:
			continue
		value = uncomment(key.group(2))
		if value in ("", "null", "~", "{}"):
			found.append(read_filters(lines, index, column, key.group(1)))
		else:
			raise Unreadable(f"line {index + 1}: `{key.group(1)}:` is written inline; write it as a block mapping")
	if len(found) > 1:
		raise Unreadable("both pull_request and pull_request_target: this gate reads one")
	return found[0] if found else None


def read_filters(lines: list[str], start: int, column: int, event: str) -> Trigger:
	values: dict[str, tuple[str, ...]] = {}
	children = block(lines, start, column)
	if not children:
		return Trigger()
	inner = indent_of(children[0][1])
	for position, (index, line) in enumerate(children):
		if indent_of(line) != inner:
			continue
		key = re.match(r"^\s*([\w-]+):(.*)$", line)
		if not key:
			raise Unreadable(f"line {index + 1}: cannot read {line.strip()!r} under `{event}:`")
		name, value = key.group(1), uncomment(key.group(2))
		if name not in FILTERS:
			continue
		if value:
			values[FILTERS[name]] = tuple(flow_list(value, f"{event}.{name}"))
			continue
		items = []
		for _, item in children[position + 1 :]:
			if indent_of(item) <= inner:
				break
			stripped = item.strip()
			if not stripped.startswith("- "):
				raise Unreadable(f"{event}.{name}: cannot read {stripped!r}")
			items.append(scalar(stripped[2:]))
		values[FILTERS[name]] = tuple(items)
	trigger = Trigger(**values)
	if trigger.paths is not None and trigger.paths_ignore is not None:
		raise Unreadable(f"{event} has both paths and paths-ignore, which GitHub refuses")
	if trigger.branches is not None and trigger.branches_ignore is not None:
		raise Unreadable(f"{event} has both branches and branches-ignore, which GitHub refuses")
	return trigger


# --- GitHub's filter patterns ------------------------------------------------------------------


def glob_regex(pattern: str) -> re.Pattern[str]:
	"""GitHub's filter pattern syntax: `*`, `**`, `**/` (zero or more directories), `?` and `+`
	(zero-or-one and one-or-more of the preceding character), `[...]`."""
	tokens: list[str] = []
	i = 0
	while i < len(pattern):
		c = pattern[i]
		if pattern.startswith("**/", i):
			tokens.append("(?:.*/)?")
			i += 3
		elif pattern.startswith("**", i):
			tokens.append(".*")
			i += 2
		elif c == "*":
			tokens.append("[^/]*")
			i += 1
		elif c in "?+" and tokens:
			tokens[-1] = f"(?:{tokens[-1]}){c}"
			i += 1
		elif c == "[":
			end = pattern.find("]", i + 1)
			if end == -1:
				raise Unreadable(f"pattern {pattern!r} has an unclosed [")
			tokens.append(pattern[i : end + 1])
			i = end + 1
		elif c == "\\" and i + 1 < len(pattern):
			tokens.append(re.escape(pattern[i + 1]))
			i += 2
		else:
			tokens.append(re.escape(c))
			i += 1
	return re.compile("".join(tokens) + r"\Z")


def matches(patterns: tuple[str, ...], value: str) -> bool:
	"""In order, a later pattern overriding an earlier one; `!` negates."""
	result = False
	for pattern in patterns:
		negated = pattern.startswith("!")
		if glob_regex(pattern[1:] if negated else pattern).match(value):
			result = not negated
	return result


def runs_for(trigger: Trigger, base_ref: str, files: list[str]) -> tuple[bool, str]:
	"""Whether GitHub starts this workflow for the pull request, and why."""
	if trigger.branches is not None and not matches(trigger.branches, base_ref):
		return (False, f"base {base_ref} is not in its branches")
	if trigger.branches_ignore is not None and matches(trigger.branches_ignore, base_ref):
		return (False, f"base {base_ref} is in its branches-ignore")
	if trigger.paths is not None:
		hit = next((f for f in files if matches(trigger.paths, f)), None)
		return (True, f"{hit} matches its paths") if hit else (False, "no changed file matches its paths")
	if trigger.paths_ignore is not None:
		hit = next((f for f in files if not matches(trigger.paths_ignore, f)), None)
		return (True, f"{hit} is outside its paths-ignore") if hit else (False, "every changed file is in its paths-ignore")
	return (True, "it has no path filter")


def expected_workflows(root: Path, self_path: str, base_ref: str, files: list[str]) -> tuple[dict[str, str], dict[str, str]]:
	"""({workflow path: why it runs}, {workflow path: why it does not}), this gate's own file left out."""
	expected, not_expected = {}, {}
	workflows = sorted([*root.glob(".github/workflows/*.yml"), *root.glob(".github/workflows/*.yaml")])
	for file in workflows:
		path = str(file.relative_to(root))
		if path == self_path:
			continue
		try:
			trigger = read_trigger(file.read_text(encoding="utf-8"))
		except Unreadable as err:
			raise Unreadable(f"{path}: {err}") from err
		if trigger is None:
			continue
		runs, why = runs_for(trigger, base_ref, files)
		(expected if runs else not_expected)[path] = why
	return expected, not_expected


# --- the verdict ---------------------------------------------------------------------------------


@dataclass
class Verdict:
	done: bool
	errors: list[str] = field(default_factory=list)
	warnings: list[str] = field(default_factory=list)
	pending: list[str] = field(default_factory=list)
	passed: list[str] = field(default_factory=list)


def latest_runs(runs: list[dict[str, Any]], head_sha: str, self_path: str, self_run_id: int) -> dict[str, dict[str, Any]]:
	latest: dict[str, dict[str, Any]] = {}
	for run in runs:
		if run.get("head_sha") != head_sha or run.get("event") not in AWAITED_EVENTS:
			continue
		if run.get("id") == self_run_id or run.get("path") == self_path:
			continue
		path = run.get("path") or run.get("name") or str(run.get("id"))
		if path not in latest or run["id"] > latest[path]["id"]:
			latest[path] = run
	return latest


def judge(
	expected: dict[str, str],
	not_expected: dict[str, str],
	runs: dict[str, dict[str, Any]],
	jobs: dict[int, list[dict[str, Any]]],
	other_checks: list[dict[str, Any]],
	elapsed: float,
	files_changed: int,
) -> Verdict:
	"""Where every awaited check stands. `jobs` holds the jobs of each completed run, by run id."""
	v = Verdict(done=True)
	for path, why in sorted(expected.items()):
		if path in runs:
			continue
		if files_changed > PATH_FILTER_LIMIT and "matches its paths" in why:
			v.warnings.append(f"{path} has no run, though {why}; GitHub reads only {PATH_FILTER_LIMIT} of the {files_changed} changed files for a path filter")
		elif elapsed >= GRACE_SECONDS:
			v.errors.append(f"{path} should have run ({why}), and GitHub started no run of it for this commit")
		else:
			v.done = False
			v.pending.append(f"{path}: no run yet ({why})")
	for path, run in sorted(runs.items()):
		label = f"{run.get('name') or path} ({path})"
		if path in not_expected:
			v.warnings.append(f"{label} ran though its trigger reads as not running ({not_expected[path]}): this gate's reading of the filter is wrong, fix scripts/pr_gate.py")
		if run.get("status") != "completed":
			v.done = False
			v.pending.append(f"{label}: {run.get('status')} {run.get('html_url', '')}".rstrip())
			continue
		# The run's conclusion is the verdict, as it is for a required check: a
		# job its workflow marks `continue-on-error` (deny.yml's advisories)
		# fails without failing the run, and is reported, not counted.
		failed = [j for j in jobs.get(run["id"], []) if j.get("conclusion") not in PASSING]
		named = [f"job {j.get('name')!r} {j.get('conclusion') or j.get('status')} {j.get('html_url', '')}".rstrip() for j in failed]
		if run.get("conclusion") in PASSING:
			v.passed.append(f"{label}: {run.get('conclusion')}")
			v.warnings += [f"{label}: {line}, which its workflow allows (continue-on-error)" for line in named]
		elif named:
			v.errors += [f"{label}: {line}" for line in named]
		else:
			v.errors.append(f"{label}: run {run.get('conclusion')} {run.get('html_url', '')}".rstrip())
	for check in sorted(other_checks, key=lambda c: c.get("name", "")):
		label = f"{check.get('name')} (check run from {((check.get('app') or {}).get('slug')) or 'another app'})"
		if check.get("status") != "completed":
			v.done = False
			v.pending.append(f"{label}: {check.get('status')}")
		elif check.get("conclusion") not in PASSING:
			v.errors.append(f"{label}: {check.get('conclusion')} {check.get('html_url', '')}".rstrip())
		else:
			v.passed.append(f"{label}: {check.get('conclusion')}")
	return v


# --- the API -----------------------------------------------------------------------------------------


class Api:
	def __init__(self, repo: str, token: str, base: str = "https://api.github.com") -> None:
		self.repo, self.token, self.base = repo, token, base.rstrip("/")

	def get(self, path: str) -> Any:
		url = path if path.startswith("http") else f"{self.base}/repos/{self.repo}/{path}"
		request = urllib.request.Request(url, headers={"Authorization": f"Bearer {self.token}", "Accept": "application/vnd.github+json", "X-GitHub-Api-Version": "2022-11-28"})
		for wait in (2, 4, 8, 16, None):
			try:
				with urllib.request.urlopen(request, timeout=30) as response:
					return json.load(response)
			except urllib.error.HTTPError as err:
				if err.code < 500 and err.code != 429 or wait is None:
					raise
			except (urllib.error.URLError, TimeoutError):
				if wait is None:
					raise
			time.sleep(wait)
		raise AssertionError("unreachable")

	def pages(self, path: str, key: str | None, limit: int = 30) -> list[Any]:
		out: list[Any] = []
		separator = "&" if "?" in path else "?"
		for page in range(1, limit + 1):
			body = self.get(f"{path}{separator}per_page=100&page={page}")
			items = body[key] if key else body
			out.extend(items)
			if len(items) < 100:
				break
		return out


def summary(lines: list[str]) -> None:
	where = os.environ.get("GITHUB_STEP_SUMMARY")
	if where:
		with open(where, "a", encoding="utf-8") as out:
			out.write("\n".join(lines) + "\n")


def gate(api: Api, pr: int, self_path: str, self_run_id: int, minutes: float, once: bool) -> int:
	pull = api.get(f"pulls/{pr}")
	head_sha, base_ref = pull["head"]["sha"], pull["base"]["ref"]
	# New names only: should GitHub also match a rename's old name, the run it
	# starts is awaited anyway and warned about, never missed.
	files = [f["filename"] for f in api.pages(f"pulls/{pr}/files", None)]
	expected, not_expected = expected_workflows(REPO_ROOT, self_path, base_ref, files)
	print(f"PR #{pr} at {head_sha[:12]} into {base_ref}, {len(files)} changed files")
	for path, why in sorted(expected.items()):
		print(f"  expects {path}: {why}")
	for path, why in sorted(not_expected.items()):
		print(f"  not expected {path}: {why}")

	if self_run_id:
		own_jobs = api.get(f"actions/runs/{self_run_id}/jobs")["jobs"]
		if len(own_jobs) > 1:
			print(f"::error title=PR gate::{self_path} has {len(own_jobs)} jobs; it may hold only the gate, since no check waits on the others")
			return 1

	started = time.monotonic()
	jobs: dict[tuple[int, int], list[dict[str, Any]]] = {}
	while True:
		runs = latest_runs(api.pages(f"actions/runs?head_sha={head_sha}", "workflow_runs", limit=5), head_sha, self_path, self_run_id)
		for run in runs.values():
			key = (run["id"], run.get("run_attempt", 1))
			if run.get("status") == "completed" and key not in jobs:
				jobs[key] = api.pages(f"actions/runs/{run['id']}/attempts/{key[1]}/jobs", "jobs", limit=5)
		by_run = {run["id"]: jobs.get((run["id"], run.get("run_attempt", 1)), []) for run in runs.values()}
		checks = api.pages(f"commits/{head_sha}/check-runs?filter=latest", "check_runs", limit=5)
		others = [c for c in checks if ((c.get("app") or {}).get("slug")) != "github-actions"]
		elapsed = time.monotonic() - started
		verdict = judge(expected, not_expected, runs, by_run, others, elapsed if not once else GRACE_SECONDS, len(files))
		if verdict.done or once or elapsed > minutes * 60:
			break
		if verdict.errors:
			# Nothing that failed will pass without a re-run, which starts this gate over anyway.
			break
		print(f"[{int(elapsed)}s] waiting on {len(verdict.pending)}: " + "; ".join(verdict.pending))
		time.sleep(POLL_SECONDS)

	for line in verdict.passed:
		print(f"passed: {line}")
	for line in verdict.warnings:
		print(f"::warning title=PR gate::{line}")
	timed_out = not verdict.done and not verdict.errors and not once
	if timed_out:
		verdict.errors += [f"{p} (not finished within {minutes:g} minutes)" for p in verdict.pending]
	elif once:
		for line in verdict.pending:
			print(f"pending: {line}")
	for line in verdict.errors:
		print(f"::error title=PR gate::{line}")
	summary(
		[
			"## PR gate",
			f"Head `{head_sha[:12]}`, {len(runs)} workflow runs and {len(others)} other check runs awaited.",
			*[f"- failed: {line}" for line in verdict.errors],
			*[f"- pending: {line}" for line in (verdict.pending if once else [])],
			*[f"- passed: {line}" for line in verdict.passed],
			*[f"- warning: {line}" for line in verdict.warnings],
		]
	)
	if verdict.errors:
		print(f"{len(verdict.errors)} check(s) did not pass; after re-running them, re-run this gate too.")
		return 1
	if not verdict.done:
		return 0 if once else 1
	print(f"every check on {head_sha[:12]} passed")
	return 0


# --- self-test ---------------------------------------------------------------------------------------


def self_test() -> int:
	failures: list[str] = []

	def same(name: str, got: Any, want: Any) -> None:
		if got != want:
			failures.append(f"{name}: got {got!r}, want {want!r}")

	routes = "name: Routes\n\non:\n  pull_request:\n    types: [opened, labeled]\n    branches:\n      - main\n    # why\n    paths:\n      - 'apps/servers/file_host/**'\n      - \"Cargo.toml\"  # root\n      - .github/workflows/routes.yml\n  push:\n    branches:\n      - main\n    paths:\n      - 'push-only/**'\n  workflow_dispatch:\n\njobs: {}\n"
	same("block trigger", read_trigger(routes), Trigger(branches=("main",), paths=("apps/servers/file_host/**", "Cargo.toml", ".github/workflows/routes.yml")))
	same("flow filters", read_trigger("on:\n  pull_request:\n    branches: [main, 'release/**']\n    paths-ignore: ['**.md']\n"), Trigger(branches=("main", "release/**"), paths_ignore=("**.md",)))
	same("bare event", read_trigger("on:\n  push:\n    branches: [main]\n  pull_request:\n    branches: [main]\n"), Trigger(branches=("main",)))
	same("scalar", read_trigger("on: pull_request\njobs:\n"), Trigger())
	same("list", read_trigger("on: [push, pull_request]\n"), Trigger())
	same("quoted on", read_trigger('"on":\n  pull_request:\n'), Trigger())
	same("no pull request", read_trigger("on:\n  push:\n    branches: [main]\n  workflow_call:\n    inputs:\n      pull_request:\n        type: string\n"), None)
	same("block list of events", read_trigger("on:\n  - push\n  - pull_request\n"), Trigger())
	for name, text in (
		("inline mapping", "on:\n  pull_request: {branches: [main]}\n"),
		("both paths", "on:\n  pull_request:\n    paths: [a]\n    paths-ignore: [b]\n"),
		("no on", "name: x\njobs:\n"),
		("odd list item", "on:\n  pull_request:\n    paths:\n      a: b\n"),
	):
		try:
			read_trigger(text)
			failures.append(f"{name}: read, should be unreadable")
		except Unreadable:
			pass

	for pattern, value, want in (
		("apps/servers/file_host/**", "apps/servers/file_host/src/main.rs", True),
		("apps/servers/file_host/**", "apps/servers/file_host_x/a", False),
		("Cargo.toml", "crates/x/Cargo.toml", False),
		("**/Cargo.toml", "Cargo.toml", True),
		("**/Cargo.toml", "crates/x/Cargo.toml", True),
		("docs/**/*.md", "docs/README.md", True),
		("docs/**/*.md", "docs/a/b.md", True),
		("*.md", "docs/a.md", False),
		("**.md", "docs/a.md", True),
		("migrations/**", "migrations/0001.up.sql", True),
		("**/*.{yml,yaml}", "a/b.yml", False),
		("*.jsx?", "a.js", True),
		("*.jsx?", "a.jsx", True),
		("v[12].*", "v2.0", True),
		("v[12].*", "v3.0", False),
		("main", "main", True),
		("main", "mainline", False),
		("release/*", "release/1/x", False),
	):
		same(f"glob {pattern!r} on {value!r}", bool(glob_regex(pattern).match(value)), want)
	same("negation wins when later", matches(("docs/**", "!docs/x.md"), "docs/x.md"), False)
	same("later positive wins", matches(("!docs/x.md", "docs/**"), "docs/x.md"), True)

	t = Trigger(branches=("main",), paths=("apps/**", "Cargo.toml"))
	same("paths hit", runs_for(t, "main", ["README.md", "apps/a.rs"])[0], True)
	same("paths miss", runs_for(t, "main", ["README.md"])[0], False)
	same("other base", runs_for(t, "dev", ["apps/a.rs"])[0], False)
	same("ignore all", runs_for(Trigger(paths_ignore=("**.md",)), "main", ["a.md", "b/c.md"])[0], False)
	same("ignore some", runs_for(Trigger(paths_ignore=("**.md",)), "main", ["a.md", "b.rs"])[0], True)
	same("no filter", runs_for(Trigger(), "main", [])[0], True)

	head = "abc"

	def run(rid: int, path: str, status: str = "completed", conclusion: str | None = "success", **extra: Any) -> dict[str, Any]:
		return {"id": rid, "path": path, "name": path, "head_sha": head, "event": "pull_request", "status": status, "conclusion": conclusion, **extra}

	def job(name: str, conclusion: str = "success", status: str = "completed") -> dict[str, Any]:
		return {"name": name, "status": status, "conclusion": conclusion}

	listed = [
		run(1, "lint.yml", conclusion="cancelled"),
		run(5, "lint.yml"),
		run(2, "gate.yml", status="in_progress", conclusion=None),
		run(3, "test.yml", event="push"),
		run(4, "old.yml", head_sha="other"),
		run(6, "dynamic/github-code-scanning/codeql", event="dynamic"),
	]
	same("latest per workflow, not self, not push", {p: r["id"] for p, r in latest_runs(listed, head, "gate.yml", 99).items()}, {"lint.yml": 5, "dynamic/github-code-scanning/codeql": 6})
	same("own run id left out", list(latest_runs([run(7, "x.yml")], head, "gate.yml", 7)), [])

	ok_runs = {"lint.yml": run(5, "lint.yml"), "test.yml": run(8, "test.yml")}
	ok_jobs = {5: [job("Rust lint"), job("Artifact retention")], 8: [job("rust_ci"), job("infra_ci", "skipped")]}
	expected = {"lint.yml": "no filter", "test.yml": "no filter"}
	codeql = {"name": "CodeQL", "status": "completed", "conclusion": "neutral", "app": {"slug": "github-code-scanning"}}

	def verdict(**kw: Any) -> Verdict:
		args: dict[str, Any] = {"expected": expected, "not_expected": {}, "runs": ok_runs, "jobs": ok_jobs, "other_checks": [codeql], "elapsed": 60.0, "files_changed": 3}
		args.update(kw)
		return judge(**args)

	all_ok = verdict()
	same("all passed", (all_ok.done, all_ok.errors, all_ok.warnings), (True, [], []))
	failed = verdict(runs={**ok_runs, "test.yml": run(8, "test.yml", conclusion="failure")}, jobs={**ok_jobs, 8: [job("rust_ci", "failure"), job("infra_ci", "skipped")]})
	same("failed job", (failed.done, len(failed.errors), "rust_ci" in failed.errors[0]), (True, 1, True))
	same("cancelled run", len(verdict(runs={**ok_runs, "lint.yml": run(5, "lint.yml", conclusion="cancelled")}, jobs={**ok_jobs, 5: [job("Rust lint", "cancelled")]}).errors), 1)
	same("timed out run", len(verdict(runs={**ok_runs, "lint.yml": run(5, "lint.yml", conclusion="timed_out")}, jobs={**ok_jobs, 5: [job("Rust lint", "timed_out")]}).errors), 1)
	allowed = verdict(jobs={**ok_jobs, 5: [job("Rust Deny Checks (advisories)", "failure"), job("Deny Checks Summary")]})
	same("continue-on-error job", (allowed.done, allowed.errors, len(allowed.warnings)), (True, [], 1))
	same("startup failure (no jobs)", len(verdict(runs={**ok_runs, "lint.yml": run(5, "lint.yml", conclusion="startup_failure")}, jobs={**ok_jobs, 5: []}).errors), 1)
	running = verdict(runs={**ok_runs, "test.yml": run(8, "test.yml", status="in_progress", conclusion=None)})
	same("in progress waits", (running.done, running.errors), (False, []))
	missing_early = verdict(runs={"lint.yml": ok_runs["lint.yml"]}, expected={**expected, "routes.yml": "apps/x matches its paths"})
	same("expected run not there yet", (missing_early.done, missing_early.errors), (False, []))
	missing_late = verdict(runs={"lint.yml": ok_runs["lint.yml"]}, expected={**expected, "routes.yml": "apps/x matches its paths"}, elapsed=GRACE_SECONDS + 1)
	same("expected run never came", (len(missing_late.errors), "routes.yml" in " ".join(missing_late.errors)), (2, True))
	big = verdict(runs=ok_runs, expected={**expected, "routes.yml": "apps/x matches its paths"}, elapsed=GRACE_SECONDS + 1, files_changed=301)
	same("over 300 files warns", (big.done, big.errors, len(big.warnings)), (True, [], 1))
	surprise = verdict(not_expected={"test.yml": "no changed file matches its paths"}, expected={"lint.yml": "x"})
	same("unexpected run awaited and warned", (surprise.done, surprise.errors, len(surprise.warnings)), (True, [], 1))
	skipped_by_path = verdict(expected={"lint.yml": "x"}, runs={"lint.yml": ok_runs["lint.yml"]}, not_expected={"routes.yml": "no changed file matches its paths"})
	same("path-filtered workflow not run passes", (skipped_by_path.done, skipped_by_path.errors), (True, []))
	same("other app failing", len(verdict(other_checks=[{**codeql, "conclusion": "failure"}]).errors), 1)
	same("other app pending", verdict(other_checks=[{**codeql, "status": "queued", "conclusion": None}]).done, False)

	real = expected_workflows(REPO_ROOT, ".github/workflows/gate.yml", "main", ["README.md"])
	if ".github/workflows/routes.yml" in real[0] or ".github/workflows/lint.yml" not in real[0]:
		failures.append(f"this repository's workflows read wrongly for a README-only change: {real}")
	real = expected_workflows(REPO_ROOT, ".github/workflows/gate.yml", "main", ["apps/servers/file_host/src/main.rs"])
	if ".github/workflows/routes.yml" not in real[0] or ".github/workflows/gate.yml" in real[0] or ".github/workflows/build-image.yml" in real[0]:
		failures.append(f"this repository's workflows read wrongly for a file_host change: {real}")

	if failures:
		print("::error::pr_gate.py's own tests failed:")
		for failure in failures:
			print(f"  {failure}")
		return 1
	print("pr_gate.py tests passed")
	return 0


def main() -> int:
	parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
	parser.add_argument("--self-test", action="store_true")
	parser.add_argument("--repo", default=os.environ.get("GITHUB_REPOSITORY"))
	parser.add_argument("--pr", type=int, help="the pull request; in Actions, read from the event")
	parser.add_argument("--minutes", type=float, default=60, help="how long to wait for every check")
	parser.add_argument("--once", action="store_true", help="evaluate once and report, without waiting")
	args = parser.parse_args()
	if args.self_test:
		return self_test()
	pr = args.pr
	if pr is None and os.environ.get("GITHUB_EVENT_PATH"):
		pr = json.loads(Path(os.environ["GITHUB_EVENT_PATH"]).read_text())["pull_request"]["number"]
	token = os.environ.get("GITHUB_TOKEN") or os.environ.get("GH_TOKEN")
	if not (args.repo and pr and token):
		parser.error("needs --repo (or GITHUB_REPOSITORY), --pr (or a pull_request event) and GITHUB_TOKEN")
	workflow_ref = os.environ.get("GITHUB_WORKFLOW_REF", "")
	self_path = re.sub(r"^[^/]+/[^/]+/|@.*$", "", workflow_ref) or ".github/workflows/gate.yml"
	try:
		return gate(Api(args.repo, token, os.environ.get("GITHUB_API_URL", "https://api.github.com")), pr, self_path, int(os.environ.get("GITHUB_RUN_ID") or 0), args.minutes, args.once)
	except Unreadable as err:
		print(f"::error title=PR gate::cannot read a workflow's trigger: {err}. Write it in the shape scripts/pr_gate.py reads, or teach it this one.")
		return 1


if __name__ == "__main__":
	sys.exit(main())

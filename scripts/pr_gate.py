#!/usr/bin/env python3
"""One check that passes only when every check a pull request runs here passed (GC5, #526).

`main`'s ruleset can require checks only by name, and the checks this
repository runs on a pull request live in several workflows, some of them
filtered by path (`routes.yml`'s `Emit the route snapshot` and `Check some-ui
against it`) and some skipped inside a run when their paths did not change
(`test.yml`'s `rust_ci` and `infra_ci`). A job cannot `needs` a job in
another workflow, so this script judges them through the API and writes the
verdict as a check run named `PR gate` (external id `pr-gate`), the name the
ruleset requires.

**Who runs it (#762).** No runner waits while the other checks run. Two short
jobs judge the head commit once each time something may have changed, and
write what they see, pending or decided:

- `.github/workflows/gate.yml` (`PR gate (open)`, `--open`) on the pull
  request's own events, so a push or a label that re-runs `routes.yml` turns
  the gate pending at once;
- `.github/workflows/gate-verdict.yml` (`PR gate (verdict)`, `--event`) on
  every completed workflow run it lists and every completed check run from
  another app, so the last check to finish decides the gate, and a re-run of
  a failed one decides it again with no re-run of the gate.

Both share one concurrency group per head commit, so their writes land in the
order they read. If the base branch has no `gate-verdict.yml` (the pull
request that adds it), `--open` falls back to waiting in the job as the gate
did before, up to `--minutes` (60).

1. **What should run.** It reads every `.github/workflows/*.y(a)ml` the pull
   request's merge commit holds (the ones GitHub runs for it) and, for each
   with a `pull_request` or `pull_request_target` trigger, decides from that
   trigger's `branches`/`branches-ignore` and `paths`/`paths-ignore` filters,
   against the base branch and the pull request's changed files, whether
   GitHub starts it. Those runs are **expected**. Each must also be listed by
   name in `gate-verdict.yml`'s `workflow_run`, or its finishing would decide
   nothing: one that is not fails the gate.
2. **What runs.** It lists the workflow runs for the head commit (events
   `pull_request` and `pull_request_target`, and `dynamic`: the runs GitHub
   starts from no file here, as CodeQL's default setup does), the latest per
   workflow file, and the check runs on the head commit that another app
   (code scanning's `CodeQL`, say) reports. Every one of them is awaited,
   expected or not, except the gate's own two workflows.
3. **The verdict.** It passes once every expected workflow has a run, and
   every awaited run and check run has completed with `success`, `skipped`
   or `neutral`. Any other conclusion (`failure`, `cancelled`, `timed_out`,
   `action_required`, `startup_failure`, `stale`) fails it as soon as it is
   seen, naming the run's failed jobs: nothing that failed passes without a
   re-run. A run is judged by its own conclusion, so a job its workflow lets
   fail (`continue-on-error`: deny.yml's advisories) is reported as a
   warning, as its workflow's author chose. A run that took longer than 60
   minutes fails it too, as one unfinished after 60 minutes did when the
   gate waited.

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
event when it handles the event, before any job starts. An expected run that
is missing keeps the gate pending while anything else is unfinished, and
fails it once nothing is (when waiting in the job: after GRACE), naming the
filter that expected it. One that GitHub started though step 1 did not
expect it is awaited and warned about, since then this reading of the filter
is wrong. GitHub evaluates a path filter on at most the first 300 changed
files, so on a larger pull request a missing path-filtered run is a warning,
not a failure.

**Not itself.** Runs of `gate.yml` are never awaited, and that workflow must
hold only the gate: any other job in it would be awaited by nothing, so the
gate fails if its run has more than one job. `gate-verdict.yml` runs on
`workflow_run`/`check_run`, events this gate never awaits, from the default
branch's copy: a pull request that edits it is judged by `main`'s.

    python3 scripts/pr_gate.py --open            # gate.yml, from the pull_request event
    python3 scripts/pr_gate.py --event           # gate-verdict.yml, from its event
    python3 scripts/pr_gate.py --repo paulgsc/server --pr 555 --once
    python3 scripts/pr_gate.py --self-test

`--once` evaluates once and prints where every check stands, writing
nothing. Needs `GITHUB_TOKEN` with `actions: read`, `contents: read` and
`pull-requests: read`, and `checks: write` to write the verdict (`checks:
read` for `--once`). Design and boundaries: docs/assurance.md.
"""

from __future__ import annotations

import argparse
import base64
import json
import os
import re
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from dataclasses import dataclass, field
from datetime import datetime
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
RUN_LIMIT_SECONDS = 60 * 60
OPENER = ".github/workflows/gate.yml"
VERDICT = ".github/workflows/gate-verdict.yml"
GATE_FILES = (OPENER, VERDICT)
CHECK_NAME = "PR gate"
EXTERNAL_ID = "pr-gate"
# Not a workflow file here: CodeQL's default setup runs as a `dynamic` workflow of this name.
DYNAMIC_WORKFLOWS = ("CodeQL",)


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


def branch_miss(trigger: Trigger, base_ref: str) -> str | None:
	"""Why this trigger's branch filters leave the base branch out, or None when they let it in."""
	if trigger.branches is not None and not matches(trigger.branches, base_ref):
		return f"base {base_ref} is not in its branches"
	if trigger.branches_ignore is not None and matches(trigger.branches_ignore, base_ref):
		return f"base {base_ref} is in its branches-ignore"
	return None


def runs_for(trigger: Trigger, base_ref: str, files: list[str]) -> tuple[bool, str]:
	"""Whether GitHub starts this workflow for the pull request, and why."""
	miss = branch_miss(trigger, base_ref)
	if miss:
		return (False, miss)
	if trigger.paths is not None:
		hit = next((f for f in files if matches(trigger.paths, f)), None)
		return (True, f"{hit} matches its paths") if hit else (False, "no changed file matches its paths")
	if trigger.paths_ignore is not None:
		hit = next((f for f in files if not matches(trigger.paths_ignore, f)), None)
		return (True, f"{hit} is outside its paths-ignore") if hit else (False, "every changed file is in its paths-ignore")
	return (True, "it has no path filter")


def local_workflows(root: Path) -> dict[str, str]:
	"""{workflow path: text} of the workflow files in a checkout."""
	files = sorted([*root.glob(".github/workflows/*.yml"), *root.glob(".github/workflows/*.yaml")])
	return {str(file.relative_to(root)): file.read_text(encoding="utf-8") for file in files}


def workflow_name(path: str, text: str) -> str:
	"""The workflow's top-level `name:`, which is what `workflow_run` lists it by; GitHub's default is its path."""
	found = re.search(r"^name:(.*)$", text, re.MULTILINE)
	return scalar(found.group(1)) if found and scalar(found.group(1)) else path


def verdict_listens(text: str) -> tuple[str, ...]:
	"""The workflow names in gate-verdict.yml's `workflow_run.workflows`. Raises Unreadable."""
	lines = text.replace("\t", "    ").split("\n")
	start = next((i for i, line in enumerate(lines) if re.match(r"^\s+workflow_run:\s*(#.*)?$", line)), None)
	if start is None:
		raise Unreadable(f"{VERDICT}: no `workflow_run:` trigger")
	for index, line in block(lines, start, indent_of(lines[start])):
		key = re.match(r"^\s*workflows:(.*)$", line)
		if not key:
			continue
		if uncomment(key.group(1)):
			return tuple(flow_list(key.group(1), "workflow_run.workflows"))
		column = indent_of(line)
		return tuple(scalar(item.strip()[2:]) for _, item in block(lines, index, column) if item.strip().startswith("- "))
	raise Unreadable(f"{VERDICT}: `workflow_run:` lists no `workflows:`")


def expected_workflows(texts: dict[str, str], base_ref: str, files: list[str]) -> tuple[dict[str, str], dict[str, str]]:
	"""({workflow path: why it runs}, {workflow path: why it does not}), the gate's own files left out."""
	expected, not_expected = {}, {}
	for path, text in sorted(texts.items()):
		if path in GATE_FILES:
			continue
		try:
			trigger = read_trigger(text)
		except Unreadable as err:
			raise Unreadable(f"{path}: {err}") from err
		if trigger is None:
			continue
		runs, why = runs_for(trigger, base_ref, files)
		(expected if runs else not_expected)[path] = why
	return expected, not_expected


def unheard(texts: dict[str, str], expected: dict[str, str]) -> list[str]:
	"""The expected workflows whose finishing gate-verdict.yml would not hear, by its own list in `texts`."""
	if VERDICT not in texts:
		return []
	listens = set(verdict_listens(texts[VERDICT]))
	return [f"{path} (name {workflow_name(path, texts[path])!r}) is not in {VERDICT}'s workflow_run list, so its finishing would decide nothing: add it there" for path in sorted(expected) if workflow_name(path, texts[path]) not in listens]


# --- the verdict ---------------------------------------------------------------------------------


@dataclass
class Verdict:
	done: bool
	errors: list[str] = field(default_factory=list)
	warnings: list[str] = field(default_factory=list)
	pending: list[str] = field(default_factory=list)
	passed: list[str] = field(default_factory=list)


def for_this_pr(run: dict[str, Any], pr: int) -> bool:
	"""Whether a run is this PR's. Two PRs can share a head SHA, and a run on the other one (a labelled
	routes.yml, say) must not stand in for this one's. A run GitHub lists no PR for (a fork's) is kept:
	it cannot be told apart."""
	if run.get("event") not in ("pull_request", "pull_request_target"):
		return True
	numbers = [p.get("number") for p in run.get("pull_requests") or []]
	return not numbers or pr in numbers


def latest_runs(runs: list[dict[str, Any]], head_sha: str, self_run_id: int, pr: int) -> dict[str, dict[str, Any]]:
	latest: dict[str, dict[str, Any]] = {}
	for run in runs:
		if run.get("head_sha") != head_sha or run.get("event") not in AWAITED_EVENTS:
			continue
		if run.get("id") == self_run_id or run.get("path") in GATE_FILES or not for_this_pr(run, pr):
			continue
		path = run.get("path") or run.get("name") or str(run.get("id"))
		if path not in latest or run["id"] > latest[path]["id"]:
			latest[path] = run
	return latest


def seconds(run: dict[str, Any]) -> float:
	"""How long a completed run's latest attempt took to its last update, from its creation for a first
	attempt (queueing counts, as it did against the waiting gate's clock) and from its start for a
	re-run; 0 when unknown."""
	begin = run.get("created_at") if run.get("run_attempt", 1) == 1 and run.get("created_at") else run.get("run_started_at")
	try:
		start = datetime.fromisoformat(str(begin).replace("Z", "+00:00"))
		end = datetime.fromisoformat(str(run["updated_at"]).replace("Z", "+00:00"))
	except (KeyError, ValueError):
		return 0.0
	return (end - start).total_seconds()


def judge(
	expected: dict[str, str],
	not_expected: dict[str, str],
	runs: dict[str, dict[str, Any]],
	jobs: dict[int, list[dict[str, Any]]],
	other_checks: list[dict[str, Any]],
	elapsed: float,
	files_changed: int,
	event: bool = False,
) -> Verdict:
	"""Where every awaited check stands. `jobs` holds the jobs of each completed run, by run id.

	`event`: judged once, by a job that does not wait (--open, --event), so there is no elapsed time:
	an expected run that is missing stays pending only while something else is unfinished (whose
	finishing judges again), and a run that took over RUN_LIMIT_SECONDS fails, as a run unfinished
	that long failed the gate that waited."""
	v = Verdict(done=True)
	missing: list[str] = []
	for path, why in sorted(expected.items()):
		if path in runs:
			continue
		if files_changed > PATH_FILTER_LIMIT and "matches its paths" in why:
			v.warnings.append(f"{path} has no run, though {why}; GitHub reads only {PATH_FILTER_LIMIT} of the {files_changed} changed files for a path filter")
		elif event:
			missing.append(path)
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
		if event and seconds(run) > RUN_LIMIT_SECONDS:
			v.errors.append(f"{label}: took {seconds(run) / 60:.0f} minutes, over the {RUN_LIMIT_SECONDS // 60} a check may take {run.get('html_url', '')}".rstrip())
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
	if missing and v.pending:
		v.done = False
		v.pending += [f"{path}: no run yet ({expected[path]})" for path in missing]
	else:
		v.errors += [f"{path} should have run ({expected[path]}), and GitHub started no run of it for this commit" for path in missing]
	return v


# --- the API -----------------------------------------------------------------------------------------


class Api:
	def __init__(self, repo: str, token: str, base: str = "https://api.github.com") -> None:
		self.repo, self.token, self.base = repo, token, base.rstrip("/")

	def send(self, method: str, path: str, body: Any = None) -> Any:
		url = path if path.startswith("http") else f"{self.base}/repos/{self.repo}/{path}"
		data = None if body is None else json.dumps(body).encode()
		headers = {"Authorization": f"Bearer {self.token}", "Accept": "application/vnd.github+json", "X-GitHub-Api-Version": "2022-11-28"}
		if data is not None:
			headers["Content-Type"] = "application/json"
		request = urllib.request.Request(url, data=data, method=method, headers=headers)
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

	def get(self, path: str) -> Any:
		return self.send("GET", path)

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

	def exists(self, path: str) -> bool:
		try:
			self.get(path)
			return True
		except urllib.error.HTTPError as err:
			if err.code == 404:
				return False
			raise


def remote_workflows(api: Api, ref: str) -> dict[str, str]:
	"""{workflow path: text} at `ref`, read as data through the API (gate-verdict.yml runs main's checkout)."""
	out = {}
	for entry in api.get(f"contents/.github/workflows?ref={urllib.parse.quote(ref)}"):
		if entry.get("type") == "file" and re.search(r"\.ya?ml$", entry.get("name", "")):
			body = api.get(f"contents/{urllib.parse.quote(entry['path'])}?ref={urllib.parse.quote(ref)}")
			out[entry["path"]] = base64.b64decode(body["content"]).decode("utf-8")
	return out


def summary(lines: list[str]) -> None:
	where = os.environ.get("GITHUB_STEP_SUMMARY")
	if where:
		with open(where, "a", encoding="utf-8") as out:
			out.write("\n".join(lines) + "\n")


@dataclass
class Look:
	"""One reading of where a pull request's head commit stands."""

	head_sha: str
	verdict: Verdict
	runs: int
	others: int
	files: int


def look(
	api: Api,
	pull: dict[str, Any],
	texts: dict[str, str],
	self_run_id: int,
	jobs: dict[tuple[int, int], list[dict[str, Any]]],
	elapsed: float,
	event: bool,
	files: list[str],
) -> Look:
	head_sha, base_ref, pr = pull["head"]["sha"], pull["base"]["ref"], pull["number"]
	expected, not_expected = expected_workflows(texts, base_ref, files)
	runs = latest_runs(api.pages(f"actions/runs?head_sha={head_sha}", "workflow_runs", limit=5), head_sha, self_run_id, pr)
	for run in runs.values():
		key = (run["id"], run.get("run_attempt", 1))
		if run.get("status") == "completed" and key not in jobs:
			jobs[key] = api.pages(f"actions/runs/{run['id']}/attempts/{key[1]}/jobs", "jobs", limit=5)
	by_run = {run["id"]: jobs.get((run["id"], run.get("run_attempt", 1)), []) for run in runs.values()}
	checks = api.pages(f"commits/{head_sha}/check-runs?filter=latest", "check_runs", limit=5)
	others = [c for c in checks if ((c.get("app") or {}).get("slug")) != "github-actions"]
	verdict = judge(expected, not_expected, runs, by_run, others, elapsed, len(files), event=event)
	verdict.errors += unheard(texts, expected)
	return Look(head_sha, verdict, len(runs), len(others), len(files))


def state_of(verdict: Verdict) -> str:
	"""`failure` as soon as anything failed, `pending` while anything is unfinished, else `success`."""
	if verdict.errors:
		return "failure"
	return "success" if verdict.done else "pending"


def lines_of(seen: Look, pending: bool) -> list[str]:
	v = seen.verdict
	return [
		f"Head `{seen.head_sha[:12]}`, {seen.runs} workflow runs and {seen.others} other check runs awaited, {seen.files} changed files.",
		*[f"- failed: {line}" for line in v.errors],
		*[f"- pending: {line}" for line in (v.pending if pending else [])],
		*[f"- passed: {line}" for line in v.passed],
		*[f"- warning: {line}" for line in v.warnings],
	]


def print_look(seen: Look) -> None:
	for line in seen.verdict.passed:
		print(f"passed: {line}")
	for line in seen.verdict.pending:
		print(f"pending: {line}")
	for line in seen.verdict.warnings:
		print(f"::warning title=PR gate::{line}")
	for line in seen.verdict.errors:
		print(f"::error title=PR gate::{line}")


TITLES = {"pending": "Waiting on other checks", "success": "Every check passed", "failure": "A check did not pass"}


def write(api: Api, head_sha: str, state: str, lines: list[str]) -> None:
	"""Set the `PR gate` check run on the head commit to `state`.

	An unfinished one is updated in place. A finished one is updated only to say the same thing again;
	any change writes a new check run, which is the one the ruleset reads (the newest of the name), as
	a re-run job's new check run is."""
	text = "\n".join(["## PR gate", *lines])
	if len(text) > 60000:
		text = text[:60000] + "\n… (cut; the job's log has the rest)"
	body: dict[str, Any] = {"name": CHECK_NAME, "external_id": EXTERNAL_ID, "output": {"title": TITLES[state], "summary": text}}
	server, run_id = os.environ.get("GITHUB_SERVER_URL"), os.environ.get("GITHUB_RUN_ID")
	if server and run_id:
		body["details_url"] = f"{server}/{api.repo}/actions/runs/{run_id}"
	if state == "pending":
		body["status"] = "in_progress"
	else:
		body.update(status="completed", conclusion=state)
	mine = [
		c
		for c in api.pages(f"commits/{head_sha}/check-runs?check_name={urllib.parse.quote(CHECK_NAME)}&filter=all", "check_runs", limit=3)
		if c.get("external_id") == EXTERNAL_ID and ((c.get("app") or {}).get("slug")) == "github-actions"
	]
	latest = max(mine, key=lambda c: c["id"], default=None)
	if latest and (latest.get("status") != "completed" or (state != "pending" and latest.get("conclusion") == state)):
		api.send("PATCH", f"check-runs/{latest['id']}", body)
		print(f"PR gate on {head_sha[:12]}: {state} (check run {latest['id']} updated)")
	else:
		made = api.send("POST", "check-runs", {**body, "head_sha": head_sha})
		print(f"PR gate on {head_sha[:12]}: {state} (check run {made.get('id')} written)")


def changed_files(api: Api, pr: int) -> list[str]:
	# New names only: should GitHub also match a rename's old name, the run it
	# starts is awaited anyway and warned about, never missed.
	return [f["filename"] for f in api.pages(f"pulls/{pr}/files", None)]


def announce(pull: dict[str, Any], files: list[str], texts: dict[str, str]) -> None:
	expected, not_expected = expected_workflows(texts, pull["base"]["ref"], files)
	print(f"PR #{pull['number']} at {pull['head']['sha'][:12]} into {pull['base']['ref']}, {len(files)} changed files")
	for path, why in sorted(expected.items()):
		print(f"  expects {path}: {why}")
	for path, why in sorted(not_expected.items()):
		print(f"  not expected {path}: {why}")


def own_jobs_problem(api: Api, self_run_id: int) -> str | None:
	if not self_run_id:
		return None
	own = api.get(f"actions/runs/{self_run_id}/jobs")["jobs"]
	if len(own) > 1:
		return f"{OPENER} has {len(own)} jobs; it may hold only the gate, since no check waits on the others"
	return None


def judge_once(api: Api, pull: dict[str, Any], texts: dict[str, str], self_run_id: int, write_it: bool) -> int:
	"""--open, --event and --once: judge now, write what is seen (pending or decided), and leave."""
	files = changed_files(api, pull["number"])
	try:
		announce(pull, files, texts)
		seen = look(api, pull, texts, self_run_id, {}, 0.0, True, files)
	except Unreadable as err:
		problem = f"cannot read a workflow's trigger: {err}. Write it in the shape scripts/pr_gate.py reads, or teach it this one."
		print(f"::error title=PR gate::{problem}")
		if write_it:
			write(api, pull["head"]["sha"], "failure", [f"- failed: {problem}"])
		return 1
	state = state_of(seen.verdict)
	print_look(seen)
	lines = lines_of(seen, pending=True)
	summary(["## PR gate", *lines])
	if write_it:
		write(api, seen.head_sha, state, lines)
	print(f"PR gate for #{pull['number']}: {state}")
	return 0


def wait(api: Api, pull: dict[str, Any], texts: dict[str, str], self_run_id: int, minutes: float) -> int:
	"""The fallback while the base branch has no gate-verdict.yml: wait in the job, as the gate did before #762."""
	files = changed_files(api, pull["number"])
	announce(pull, files, texts)
	head_sha = pull["head"]["sha"]
	write(api, head_sha, "pending", [f"{VERDICT} is not on {pull['base']['ref']} yet, so this job waits on every other check, up to {minutes:g} minutes."])
	started = time.monotonic()
	jobs: dict[tuple[int, int], list[dict[str, Any]]] = {}
	while True:
		elapsed = time.monotonic() - started
		seen = look(api, pull, texts, self_run_id, jobs, elapsed, False, files)
		verdict = seen.verdict
		if verdict.done or verdict.errors or elapsed > minutes * 60:
			# Nothing that failed will pass without a re-run, which starts this gate over anyway.
			break
		print(f"[{int(elapsed)}s] waiting on {len(verdict.pending)}: " + "; ".join(verdict.pending))
		time.sleep(POLL_SECONDS)
	if not verdict.done and not verdict.errors:
		verdict.errors += [f"{p} (not finished within {minutes:g} minutes)" for p in verdict.pending]
	print_look(Look(seen.head_sha, Verdict(True, verdict.errors, verdict.warnings, [], verdict.passed), seen.runs, seen.others, seen.files))
	state = state_of(verdict)
	lines = lines_of(seen, pending=False)
	summary(["## PR gate", *lines])
	write(api, head_sha, state, lines)
	if verdict.errors:
		print(f"{len(verdict.errors)} check(s) did not pass; after re-running them, re-run this gate too.")
		return 1
	print(f"every check on {head_sha[:12]} passed")
	return 0


def open_gate(api: Api, pr: int, minutes: float) -> int:
	"""gate.yml, on the pull request's own events."""
	pull = api.get(f"pulls/{pr}")
	texts = local_workflows(REPO_ROOT)
	self_run_id = int(os.environ.get("GITHUB_RUN_ID") or 0)
	problem = own_jobs_problem(api, self_run_id)
	if problem:
		print(f"::error title=PR gate::{problem}")
		write(api, pull["head"]["sha"], "failure", [f"- failed: {problem}"])
		return 1
	if not api.exists(f"contents/{VERDICT}?ref={urllib.parse.quote(pull['base']['ref'])}"):
		print(f"{VERDICT} is not on {pull['base']['ref']}: waiting in this job instead")
		return wait(api, pull, texts, self_run_id, minutes)
	return judge_once(api, pull, texts, self_run_id, write_it=True)


def event_pulls(api: Api, event_name: str, event: dict[str, Any]) -> tuple[str, list[int]]:
	"""The head commit an event is about and the open pull requests it may decide."""
	if event_name == "workflow_run":
		source = event["workflow_run"]
		head_sha = source["head_sha"]
		numbers = [p["number"] for p in source.get("pull_requests") or []]
		if not numbers and source.get("head_branch") and (source.get("head_repository") or {}).get("owner"):
			# A fork's run lists no pull request; find it by its head branch.
			head = f"{source['head_repository']['owner']['login']}:{source['head_branch']}"
			numbers = [p["number"] for p in api.get(f"pulls?state=open&head={urllib.parse.quote(head)}")]
	elif event_name == "check_run":
		source = event["check_run"]
		head_sha = source["head_sha"]
		numbers = [p["number"] for p in source.get("pull_requests") or []]
		if not numbers:
			numbers = [p["number"] for p in api.get(f"commits/{head_sha}/pulls") if p.get("state") == "open"]
	else:
		raise SystemExit(f"--event reads a workflow_run or check_run event, not {event_name!r}")
	return head_sha, sorted(set(numbers))


def event_gate(api: Api, head_sha: str, numbers: list[int]) -> int:
	"""gate-verdict.yml: something on `head_sha` finished, so judge each pull request it heads, once."""
	gate_trigger = read_trigger((REPO_ROOT / OPENER).read_text(encoding="utf-8")) or Trigger()
	if not numbers:
		print(f"no open pull request has {head_sha[:12]} as its head: nothing to judge")
	for pr in numbers:
		pull = api.get(f"pulls/{pr}")
		if pull.get("state") != "open":
			print(f"#{pr} is {pull.get('state')}: not judged")
			continue
		if pull["head"]["sha"] != head_sha:
			print(f"#{pr}'s head is now {pull['head']['sha'][:12]}, not {head_sha[:12]}: that commit's gate is its own")
			continue
		miss = branch_miss(gate_trigger, pull["base"]["ref"])
		if miss:
			print(f"#{pr}: {OPENER} does not run for it ({miss}): not judged")
			continue
		ref = pull.get("merge_commit_sha") or head_sha
		judge_once(api, pull, remote_workflows(api, ref), 0, write_it=True)
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
	same("latest per workflow, not self, not push", {p: r["id"] for p, r in latest_runs([{**r, "path": ".github/workflows/" + r["path"]} if r["path"] == "gate.yml" else r for r in listed], head, 99, 1).items()}, {"lint.yml": 5, "dynamic/github-code-scanning/codeql": 6})
	same("own run id left out", list(latest_runs([run(7, "x.yml"), run(8, VERDICT, event="workflow_run")], head, 7, 1)), [])
	shared = [
		run(10, "routes.yml", conclusion="failure", pull_requests=[{"number": 1}]),
		run(11, "routes.yml", pull_requests=[{"number": 2}]),
		run(12, "fork.yml", pull_requests=[]),
	]
	same(
		"another PR's newer run on the same head does not stand in for this one's",
		{p: r["id"] for p, r in latest_runs(shared, head, 99, 1).items()},
		{"routes.yml": 10, "fork.yml": 12},
	)

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

	# Judged once (--open, --event): nothing waits, so a missing run is pending only while something else
	# is unfinished, and a run that took over an hour fails.
	routes_expected = {**expected, "routes.yml": "apps/x matches its paths"}
	gone = verdict(runs=ok_runs, expected=routes_expected, event=True)
	same("event: expected run missing, nothing else running", (gone.done, len(gone.errors), "routes.yml" in " ".join(gone.errors)), (True, 1, True))
	not_yet = verdict(runs={**ok_runs, "test.yml": run(8, "test.yml", status="in_progress", conclusion=None)}, expected=routes_expected, event=True)
	same("event: expected run missing while another runs", (not_yet.done, not_yet.errors, len(not_yet.pending)), (False, [], 2))
	same("event: big pull request still warns", (verdict(runs=ok_runs, expected=routes_expected, event=True, files_changed=301).errors), [])
	slow = run(8, "test.yml", run_started_at="2026-11-10T00:00:00Z", updated_at="2026-11-10T01:00:01Z")
	same("event: a run over an hour fails", len(verdict(runs={**ok_runs, "test.yml": slow}, event=True).errors), 1)
	same("waiting: the hour is the wait's own", verdict(runs={**ok_runs, "test.yml": slow}).errors, [])
	quick = run(8, "test.yml", run_started_at="2026-11-10T00:00:00Z", updated_at="2026-11-10T00:59:00Z")
	same("event: a run under an hour passes", verdict(runs={**ok_runs, "test.yml": quick}, event=True).errors, [])
	queued = run(8, "test.yml", created_at="2026-11-09T23:30:00Z", run_started_at="2026-11-10T00:00:00Z", updated_at="2026-11-10T00:59:00Z")
	same("event: queueing counts on a first attempt", len(verdict(runs={**ok_runs, "test.yml": queued}, event=True).errors), 1)
	same("event: a re-run counts from its start", verdict(runs={**ok_runs, "test.yml": {**queued, "run_attempt": 2}}, event=True).errors, [])
	same("event: all passed", (verdict(event=True).done, verdict(event=True).errors), (True, []))
	same("event: failure decides while others run", state_of(verdict(runs={**ok_runs, "lint.yml": run(5, "lint.yml", conclusion="failure"), "test.yml": run(8, "test.yml", status="queued", conclusion=None)}, event=True)), "failure")
	same("state pending", state_of(running), "pending")
	same("state success", state_of(all_ok), "success")
	same("state failure", state_of(failed), "failure")

	listens = "on:\n  workflow_run:\n    workflows:\n      - Lint\n      - \"Detect, Build & Stage\"  # quoted\n    types: [completed]\n  check_run:\n    types: [completed]\n"
	same("verdict list, block", verdict_listens(listens), ("Lint", "Detect, Build & Stage"))
	same("verdict list, flow", verdict_listens("on:\n  workflow_run:\n    workflows: [Lint, 'CI Workflow']\n"), ("Lint", "CI Workflow"))
	texts = {VERDICT: listens, "lint.yml": "name: Lint\non: pull_request\n", "new.yml": "name: New\non: pull_request\n"}
	same("unlisted workflow", [line.split(" ")[0] for line in unheard(texts, {"lint.yml": "x", "new.yml": "x"})], ["new.yml"])
	same("no verdict file, nothing to check", unheard({"new.yml": texts["new.yml"]}, {"new.yml": "x"}), [])
	same("workflow name", (workflow_name("a.yml", "name: 'CI Workflow'  # x\n"), workflow_name("b.yml", "on: push\n")), ("CI Workflow", "b.yml"))
	same("branch filter", (branch_miss(Trigger(branches=("main",)), "main"), bool(branch_miss(Trigger(branches=("main",)), "dev"))), (None, True))

	here = local_workflows(REPO_ROOT)
	real = expected_workflows(here, "main", ["README.md"])
	if ".github/workflows/routes.yml" in real[0] or ".github/workflows/lint.yml" not in real[0]:
		failures.append(f"this repository's workflows read wrongly for a README-only change: {real}")
	real = expected_workflows(here, "main", ["apps/servers/file_host/src/main.rs"])
	if ".github/workflows/routes.yml" not in real[0] or OPENER in real[0] or ".github/workflows/build-image.yml" in real[0]:
		failures.append(f"this repository's workflows read wrongly for a file_host change: {real}")
	# Every workflow a pull request can start is one gate-verdict.yml hears finish, and it hears nothing else
	# but CodeQL's dynamic workflow, so a renamed workflow is not left listed under its old name.
	every = {path: "x" for path, text in here.items() if path not in GATE_FILES and read_trigger(text) is not None}
	same("gate-verdict.yml hears every pull-request workflow", unheard(here, every), [])
	names = {workflow_name(path, here[path]) for path in every} | set(DYNAMIC_WORKFLOWS)
	same("gate-verdict.yml lists nothing else", sorted(set(verdict_listens(here[VERDICT])) - names), [])

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
	parser.add_argument("--head-sha", help="with --event and --pr: the head commit to judge, instead of the event's")
	parser.add_argument("--minutes", type=float, default=60, help="--open's fallback: how long to wait for every check")
	mode = parser.add_mutually_exclusive_group()
	mode.add_argument("--open", action="store_true", help="gate.yml: judge on the pull request's own event")
	mode.add_argument("--event", action="store_true", help="gate-verdict.yml: judge on a finished workflow run or check run")
	mode.add_argument("--once", action="store_true", help="judge once and print where every check stands, writing nothing")
	args = parser.parse_args()
	if args.self_test:
		return self_test()
	token = os.environ.get("GITHUB_TOKEN") or os.environ.get("GH_TOKEN")
	if not (args.repo and token):
		parser.error("needs --repo (or GITHUB_REPOSITORY) and GITHUB_TOKEN")
	api = Api(args.repo, token, os.environ.get("GITHUB_API_URL", "https://api.github.com"))
	event_path = os.environ.get("GITHUB_EVENT_PATH")
	event = json.loads(Path(event_path).read_text()) if event_path else {}
	try:
		if args.event:
			if args.pr:
				head_sha = args.head_sha or api.get(f"pulls/{args.pr}")["head"]["sha"]
				return event_gate(api, head_sha, [args.pr])
			return event_gate(api, *event_pulls(api, os.environ.get("GITHUB_EVENT_NAME", ""), event))
		pr = args.pr or (event.get("pull_request") or {}).get("number")
		if not pr:
			parser.error("needs --pr, or a pull_request event")
		if args.open:
			return open_gate(api, pr, args.minutes)
		return judge_once(api, api.get(f"pulls/{pr}"), local_workflows(REPO_ROOT), 0, write_it=False)
	except Unreadable as err:
		print(f"::error title=PR gate::cannot read a workflow's trigger: {err}. Write it in the shape scripts/pr_gate.py reads, or teach it this one.")
		return 1


if __name__ == "__main__":
	sys.exit(main())

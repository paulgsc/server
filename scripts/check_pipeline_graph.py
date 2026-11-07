#!/usr/bin/env python3
"""The PIPELINES dashboard's graphs match the ledger and the alert rules (server#692).

`infra/grafana/dashboards/lib/pipelines.json` draws each chained process home
runs as a graph (`pipelines.jsonnet`), each node colored by the alerts that
fire for it. A graph that has drifted shows a pipeline that is not the one
running, or a node green because the alert it reads no longer exists, so this
fails, for every pipeline:

1. **It is a ledger process.** Its `process` is a process of paulgsc/some-ui's
   assurance ledger (`processes`).
2. **Its nodes are that process's segments.** The same ids, none missing and
   none added, each once.
3. **Its edges join its nodes.** Each edge's two ends are nodes of the graph,
   and no edge is drawn twice.
4. **Each alert it reads exists.** Each matcher is `label="value"` pairs with
   an `alertname`, and that alert is a rule in `infra/prometheus/rules`.

    python3 scripts/check_pipeline_graph.py --some-ui <checkout of paulgsc/some-ui>
    python3 scripts/check_pipeline_graph.py --self-test

CI runs it beside check_assurance_ledger.py (`.github/workflows/assurance.yml`),
which finds the ledger the same way.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path
from typing import Any

sys.path.insert(0, str(Path(__file__).resolve().parent))
from check_assurance_ledger import find_ledger  # noqa: E402

REPO_ROOT = Path(__file__).resolve().parent.parent
GRAPHS = REPO_ROOT / "infra" / "grafana" / "dashboards" / "lib" / "pipelines.json"
RULES = REPO_ROOT / "infra" / "prometheus" / "rules"

MATCHER = re.compile(r'^[a-zA-Z_][a-zA-Z0-9_]*="[^"\\]*"(,[a-zA-Z_][a-zA-Z0-9_]*="[^"\\]*")*$')
ALERT_RULE = re.compile(r"^\s*-\s*alert:\s*([A-Za-z_][A-Za-z0-9_]*)\s*$", re.MULTILINE)


def alert_names(rules: Path) -> set[str]:
	"""Every alert a rule file names (promtool tests, `*.test.yml`, are not rules)."""
	names: set[str] = set()
	for path in sorted(rules.glob("*.yml")):
		if not path.name.endswith(".test.yml"):
			names.update(ALERT_RULE.findall(path.read_text(encoding="utf-8")))
	return names


def segments_by_process(ledger: Any) -> dict[str, list[str]]:
	processes = ledger.get("processes") if isinstance(ledger, dict) else None
	found: dict[str, list[str]] = {}
	for process in processes if isinstance(processes, list) else []:
		if isinstance(process, dict) and isinstance(process.get("id"), str):
			found[process["id"]] = [s["id"] for s in process.get("segments", []) if isinstance(s, dict) and isinstance(s.get("id"), str)]
	return found


def check(graphs: Any, ledger: Any, alerts: set[str]) -> list[str]:
	errors: list[str] = []
	processes = segments_by_process(ledger)
	pipelines = graphs.get("pipelines") if isinstance(graphs, dict) else None
	if not isinstance(pipelines, list) or not pipelines:
		return ["pipelines.json has no `pipelines` list"]
	for index, pipeline in enumerate(pipelines):
		name = pipeline.get("process") if isinstance(pipeline, dict) else None
		where = f"pipeline {name or index}"
		if not isinstance(name, str) or name not in processes:
			errors.append(f"{where}: not a process of the ledger (it has {sorted(processes)})")
			continue
		nodes = [n.get("id") for n in pipeline.get("nodes", []) if isinstance(n, dict)]
		twice = sorted({n for n in nodes if nodes.count(n) > 1})
		if twice:
			errors.append(f"{where}: nodes drawn twice: {twice}")
		missing = [s for s in processes[name] if s not in nodes]
		extra = [n for n in nodes if n not in processes[name]]
		if missing:
			errors.append(f"{where}: the ledger's segments {missing} are not drawn")
		if extra:
			errors.append(f"{where}: nodes {extra} are not segments of the ledger's `{name}`")
		seen: set[tuple[str, str]] = set()
		for edge in pipeline.get("edges", []):
			if not (isinstance(edge, list) and len(edge) == 2 and all(isinstance(end, str) for end in edge)):
				errors.append(f"{where}: an edge is not [source, target]: {edge!r}")
				continue
			for end in edge:
				if end not in nodes:
					errors.append(f"{where}: the edge {edge[0]} -> {edge[1]} joins {end}, which is not a node")
			if tuple(edge) in seen:
				errors.append(f"{where}: the edge {edge[0]} -> {edge[1]} is drawn twice")
			seen.add((edge[0], edge[1]))
		for node in pipeline.get("nodes", []):
			for matcher in node.get("alerts", []) if isinstance(node, dict) else []:
				alert = re.search(r'(?:^|,)alertname="([^"]*)"', matcher) if isinstance(matcher, str) else None
				if not isinstance(matcher, str) or not MATCHER.match(matcher) or alert is None:
					errors.append(f"{where}, node {node.get('id')}: {matcher!r} is not label=\"value\" pairs with an alertname")
				elif alert.group(1) not in alerts:
					errors.append(f"{where}, node {node.get('id')}: reads {alert.group(1)}, which no rule in infra/prometheus/rules fires")
	return errors


def self_test() -> int:
	ledger = {"processes": [{"id": "p", "segments": [{"id": "a"}, {"id": "b"}]}]}
	alerts = {"Up", "Down"}

	def graph(**changes: Any) -> dict[str, Any]:
		pipeline = {
			"process": "p",
			"nodes": [{"id": "a", "alerts": ['alertname="Up"']}, {"id": "b", "alerts": ['alertname="Down",name="x"']}],
			"edges": [["a", "b"]],
		}
		pipeline.update(changes)
		return {"pipelines": [pipeline]}

	cases: list[tuple[str, dict[str, Any], str | None]] = [
		("a graph that matches holds", graph(), None),
		("an unknown process fails", graph(process="q"), "not a process of the ledger"),
		("a segment not drawn fails", graph(nodes=[{"id": "a", "alerts": []}], edges=[]), "are not drawn"),
		("a node the ledger lacks fails", graph(nodes=[{"id": "a"}, {"id": "b"}, {"id": "c"}]), "are not segments"),
		("a node drawn twice fails", graph(nodes=[{"id": "a"}, {"id": "b"}, {"id": "b"}]), "drawn twice"),
		("an edge to nowhere fails", graph(edges=[["a", "z"]]), "which is not a node"),
		("an edge drawn twice fails", graph(edges=[["a", "b"], ["a", "b"]]), "is drawn twice"),
		("an alert no rule fires fails", graph(nodes=[{"id": "a", "alerts": ['alertname="Gone"']}, {"id": "b"}]), "which no rule"),
		("a matcher with no alertname fails", graph(nodes=[{"id": "a", "alerts": ['name="x"']}, {"id": "b"}]), "with an alertname"),
		("a matcher that is not PromQL fails", graph(nodes=[{"id": "a", "alerts": ["alertname=Up"]}, {"id": "b"}]), "with an alertname"),
	]
	failed = 0
	for title, graphs, expected in cases:
		errors = check(graphs, ledger, alerts)
		ok = not errors if expected is None else any(expected in error for error in errors)
		if not ok:
			failed += 1
			print(f"FAIL {title}: {errors}")
	rules = alert_names(RULES)
	if "CallRenderFailed" not in rules or "GameFeedStale" not in rules:
		failed += 1
		print(f"FAIL reading infra/prometheus/rules found {sorted(rules)}")
	print(f"self-test: {len(cases) + 1 - failed} of {len(cases) + 1} passed")
	return 1 if failed else 0


def main() -> int:
	parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
	source = parser.add_mutually_exclusive_group()
	source.add_argument("--some-ui", type=Path, help="a checkout of paulgsc/some-ui; the ledger is found in it")
	source.add_argument("--ledger", type=Path, help="the ledger file itself")
	parser.add_argument("--self-test", action="store_true")
	args = parser.parse_args()
	if args.self_test:
		return self_test()
	try:
		path = args.ledger or (find_ledger(args.some_ui.resolve()) if args.some_ui else None)
	except LookupError as err:
		print(f"::error title=Ledger not found::{err}")
		return 1
	if path is None:
		parser.error("--some-ui or --ledger is required")
	errors = check(json.loads(GRAPHS.read_text(encoding="utf-8")), json.loads(path.read_text(encoding="utf-8")), alert_names(RULES))
	for error in errors:
		print(f"::error title=Pipeline graph::{error}")
	if errors:
		print(f"{len(errors)} finding(s): fix infra/grafana/dashboards/lib/pipelines.json, or the ledger it draws.")
		return 1
	print(f"the pipeline graphs match the ledger and the alert rules ({path.name})")
	return 0


if __name__ == "__main__":
	sys.exit(main())

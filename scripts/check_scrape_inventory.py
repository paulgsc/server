#!/usr/bin/env python3
"""Reconcile the compose stack, the scrape inventory, and prometheus.yml.

See #146 (C4). `infra/prometheus/inventory.yml` claims to be a checked-in
fact rather than a hand-kept list; this is the check that makes the claim
true. It fails when:

  * a service `docker compose config` resolves is missing from the
    inventory (a blind spot nothing else would catch), or the inventory
    claims a service compose does not have (a stale entry describing
    something that no longer runs);
  * an inventory entry with a `scrape.job`/`scrape.target` does not match a
    real job/target pair in `infra/prometheus/prometheus.yml`.

An inventory entry that opts out via `scrape.reason` is accepted without
a prometheus.yml match — "deliberately unscraped" is a valid answer here,
"unaccounted for" is not.

It also fails a watcher that waits on anything's health: a service in
`WATCHERS`, or any `*-exporter`, whose `depends_on` names a condition of
`service_healthy`. A watcher is how an unhealthy service is seen, so it
must start whether or not what it watches is healthy. On 2026-10-30
Prometheus waited on a degraded file_host and took every dashboard and
alert down with it (some-ui#2199).
"""

from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path

import yaml

REPO_ROOT = Path(__file__).resolve().parent.parent
INVENTORY_PATH = REPO_ROOT / "infra" / "prometheus" / "inventory.yml"
PROMETHEUS_CONFIG_PATH = REPO_ROOT / "infra" / "prometheus" / "prometheus.yml"

# What scrapes, evaluates, shows or delivers what is wrong. Every
# `*-exporter` is one too, named or not.
WATCHERS = frozenset({"prometheus", "grafana", "cadvisor"})


def compose_services() -> dict[str, dict]:
	"""The services `docker-compose.yml` actually resolves to, includes and all, by name."""
	result = subprocess.run(
		["docker", "compose", "config", "--format", "json"],
		cwd=REPO_ROOT,
		capture_output=True,
		text=True,
		check=False,
	)
	if result.returncode != 0:
		print("error: `docker compose config` failed:\n" + result.stderr, file=sys.stderr)
		sys.exit(1)
	return json.loads(result.stdout)["services"]


def watchers_waiting_on_health(services: dict[str, dict]) -> list[str]:
	"""`watcher -> dependency` for each watcher whose `depends_on` waits on that dependency's health."""
	waiting = []
	for name, service in sorted(services.items()):
		if name not in WATCHERS and not name.endswith("-exporter"):
			continue
		depends_on = service.get("depends_on") or {}
		# `docker compose config` writes the long form; a list is the short
		# form, which waits on nothing but the start.
		if isinstance(depends_on, dict):
			for dependency, how in sorted(depends_on.items()):
				if (how or {}).get("condition") == "service_healthy":
					waiting.append(f"{name} -> {dependency}")
	return waiting


def prometheus_jobs() -> dict[str, set[str]]:
	"""job_name -> set of scrape targets, from every static_configs block."""
	with PROMETHEUS_CONFIG_PATH.open() as f:
		config = yaml.safe_load(f)

	jobs: dict[str, set[str]] = {}
	for job in config.get("scrape_configs", []):
		targets: set[str] = set()
		for static_config in job.get("static_configs", []):
			targets.update(static_config.get("targets", []))
		jobs.setdefault(job["job_name"], set()).update(targets)
	return jobs


def main() -> int:
	with INVENTORY_PATH.open() as f:
		inventory = yaml.safe_load(f)

	entries = inventory.get("services", [])
	out_of_stack = {entry["service"] for entry in inventory.get("out_of_stack", [])}
	inventory_services = {entry["service"] for entry in entries}

	errors: list[str] = []

	# ── compose <-> inventory, both directions ──────────────────────────────
	services = compose_services()
	live_services = set(services)

	undeclared = live_services - inventory_services
	if undeclared:
		errors.append(
			"services `docker compose config` resolves but the inventory does not "
			f"list: {sorted(undeclared)} — add them to infra/prometheus/inventory.yml"
		)

	stale = inventory_services - live_services - out_of_stack
	if stale:
		errors.append(
			f"inventory entries with no matching compose service: {sorted(stale)} — "
			"they were removed or renamed; update infra/prometheus/inventory.yml, or "
			"move them to `out_of_stack` with a reason if that's deliberate"
		)

	overlap = inventory_services & out_of_stack
	if overlap:
		errors.append(f"listed both as a live service and as out_of_stack: {sorted(overlap)}")

	waiting = watchers_waiting_on_health(services)
	if waiting:
		errors.append(
			f"watchers that wait on a dependency's health: {waiting} — a watcher starts once what it "
			"watches has started (`condition: service_started`); see this script's docstring"
		)

	# ── inventory <-> prometheus.yml ─────────────────────────────────────────
	jobs = prometheus_jobs()

	for entry in entries:
		service = entry["service"]
		scrape = entry.get("scrape")
		if scrape is None:
			errors.append(f"'{service}': no `scrape` block — need a job/target pair or a `reason`")
			continue

		if "reason" in scrape:
			if not scrape["reason"].strip():
				errors.append(f"'{service}': `scrape.reason` is empty")
			continue

		job, target = scrape.get("job"), scrape.get("target")
		if not job or not target:
			errors.append(f"'{service}': `scrape` has neither a `reason` nor a complete job/target pair")
			continue

		if job not in jobs:
			errors.append(f"'{service}': scrape.job '{job}' has no matching job_name in prometheus.yml")
		elif target not in jobs[job]:
			errors.append(f"'{service}': target '{target}' not among prometheus.yml job '{job}' targets {sorted(jobs[job])}")

	if errors:
		print("scrape inventory check failed:\n", file=sys.stderr)
		for error in errors:
			print(f"  - {error}", file=sys.stderr)
		return 1

	print(f"scrape inventory OK: {len(live_services)} compose services, {len(entries)} inventory entries, {len(jobs)} prometheus jobs.")
	return 0


if __name__ == "__main__":
	sys.exit(main())

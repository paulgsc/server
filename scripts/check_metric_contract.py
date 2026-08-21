#!/usr/bin/env python3
"""Reconcile Grafana's queried metrics against what the workspace emits.

See #212 (G1/#217). Six dashboards used to query series that no crate in
this workspace has ever emitted — a panel that renders without error and
without data is a detector that always reports "fine". This is the check
that makes "the dashboard queries what the code emits" a checked fact
rather than something taken on faith, in both directions:

  * a dashboard querying a metric name nothing emits is a lie;
  * a crate emitting a metric name nothing reads is dead weight.

Both directions accept an explicit, commented exemption — an exemption is a
written decision, not a silent pass. See EXEMPT_QUERIED (series owned by an
exporter or Prometheus/Grafana itself, not by this workspace) and
EXEMPT_EMITTED (series this workspace emits deliberately ahead of a reader).

Deliberately source-level, matching apps/servers/file_host/src/routes/
inventory.rs's own reasoning: evaluating the jsonnet or standing up a real
recorder would need a built AppState and a running Grafana, which would make
the cheapest invariant in the repo the most expensive one to run. Instead:

  * the emitted set is every literal name passed to the `metrics` facade's
    `counter!`/`histogram!`/`gauge!` macros (the one exposure path #139
    chose) anywhere under `apps/` or `crates/`;
  * the queried set is every metric name lexically extractable from an
    `expr:`/`definition:` PromQL string under the *active* dashboard
    sources — `infra/grafana/dashboards/*.jsonnet` and
    `infra/grafana/dashboards/lib/**/*.libsonnet`. `dashboards/parked/` is
    excluded on purpose: scripts/build.sh never builds it either (parking a
    dashboard is exactly how this repo shelves a design without deleting
    it — see #220), so a metric a parked dashboard queries is not a claim
    Grafana is currently making.

Not in scope, both by the same reasoning inventory.rs states for routes:
label-set correctness (names only) and PromQL semantic validation (a
syntactically-real query over real series, not whether it means what its
title claims).
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
DASHBOARDS_DIR = REPO_ROOT / "infra" / "grafana" / "dashboards"
RUST_ROOTS = [REPO_ROOT / "apps", REPO_ROOT / "crates"]

# Series produced by an exporter, or by Prometheus/Grafana itself, rather
# than by any crate in this workspace. A prefix match against
# `node_exporter`/`cadvisor`/`process-exporter`/`redis_exporter`/
# `nats-exporter`/`blackbox-exporter`, plus the handful of built-ins.
EXEMPT_QUERIED_PREFIXES: dict[str, str] = {
	"node_": "node_exporter (infra/compose/monitoring.yml)",
	"container_": "cadvisor (infra/compose/monitoring.yml)",
	"namedprocess_": "process-exporter (infra/compose/monitoring.yml)",
	"redis_": "redis_exporter (infra/compose/monitoring.yml)",
	"gnatsd_": "prometheus-nats-exporter (infra/compose/monitoring.yml)",
	"probe_": "blackbox-exporter (infra/compose/monitoring.yml)",
}
EXEMPT_QUERIED_EXACT: dict[str, str] = {
	"up": "Prometheus's own per-target scrape-health series",
	"ALERTS": "Prometheus's own built-in firing-alerts series",
}

# Metrics this workspace emits through the `metrics` facade with no
# dashboard reader (yet). Each needs a reason — see #217's acceptance
# criteria: "a newly registered metric with no reader fails CI, or is
# exempted with a stated reason."
EXEMPT_EMITTED: dict[str, str] = {
	"operation_count_total": "file_host per-operation counter; operation_duration_seconds is the dashboarded signal for this same call site (instruments.rs's OperationTimer records both)",
	"operation_errors_total": "file_host per-operation error counter; no dedicated panel yet, tracked for a future operational-detail row rather than this uptime/SLA dashboard",
	"file_downloads_total": "file_host per-download counter; no dedicated panel yet, same as operation_errors_total",
	"file_size_bytes": "file_host download-size histogram; no dedicated panel yet, same as operation_errors_total",
	"nudge_waker_session_rows_read_total": "#262 (SLI1): the measurement for the waker's per-subject session read cost, landed deliberately ahead of a dashboard reader — the story is the counter and a characterisation test (nudge::waker's test module), not a panel. A panel is reasonable once #263 (SLI2) gives this series a shape worth graphing (today it just climbs with usage); tracked the same way as the other three entries above.",
}

METRICS_FACADE_CALL = re.compile(r'\b(?:counter|histogram|gauge)!\s*\(\s*(?:"([^"]+)"|([A-Z_][A-Z0-9_]*))')

# `histogram!(HTTP_DURATION_METRIC, ...)` — a call site naming its metric via
# a constant instead of a literal, so the name can't drift from wherever else
# that constant is used (e.g. a `PrometheusBuilder::set_buckets_for_metric`
# call needing the identical name). Resolved by first collecting every
# `const NAME: &str = "literal"` in the workspace, then substituting.
CONST_STR_DECL = re.compile(r'\bconst\s+([A-Z_][A-Z0-9_]*)\s*:\s*&(?:\'static\s+)?str\s*=\s*"([^"]+)"')

# Prometheus expands a histogram's base name into `_bucket`/`_sum`/`_count`
# series at scrape time; a dashboard querying one of those is querying the
# same `histogram!(...)` call site under a different literal name.
HISTOGRAM_SUFFIXES = ("_bucket", "_sum", "_count")

LABEL_VALUES_CALL = re.compile(r"label_values\(\s*([a-zA-Z_:][a-zA-Z0-9_:]*)")
LABEL_VALUES_CALL_FULL = re.compile(r"label_values\([^)]*\)")

# jsonnet single-quoted strings (no embedded escapes in this codebase's
# PromQL) and jsonnet's `|||...|||` triple-quoted "text block" strings, both
# assigned to the two field names dashboards actually put PromQL in.
PROMQL_FIELD = re.compile(
	r"(?:expr|definition)\s*:\s*(?:'([^']*)'|\|\|\|(.*?)\|\|\|)",
	re.DOTALL,
)

# Vector-matching / grouping clauses: their parenthesized contents are label
# names, not metric names — `sum(...) by (le, namespace)`,
# `foo{...} * on(service) bar{...}`.
GROUPING_CLAUSE = re.compile(r"\b(?:by|without|on|ignoring|group_left|group_right)\s*\([^)]*\)")

# A metric immediately followed by a label selector — matched first, and
# without descending into `{...}`, so a job-label regex value like
# `.*file-host.*` never gets scanned as if it were PromQL.
METRIC_WITH_SELECTOR = re.compile(r'(?<![a-zA-Z0-9_:."])([a-zA-Z_:][a-zA-Z0-9_:]*)\{')
LABEL_SELECTOR_BLOCK = re.compile(r"\{[^}]*\}")

# A bareword metric with no selector — `up`, `rate(cache_hits_total[5m])`,
# `sum(x) / sum(y)` — run only after selector blocks are stripped out.
BARE_METRIC_TOKEN = re.compile(r'(?<![a-zA-Z0-9_:."])([a-zA-Z_:][a-zA-Z0-9_:]*)\s*(?:\[|(?=[\s)+\-*/,;]|$))')

# PromQL functions, aggregation operators and keywords — never a metric
# name, but lexically indistinguishable from one without this list.
PROMQL_RESERVED = {
	"sum", "avg", "min", "max", "count", "stddev", "stdvar", "topk", "bottomk", "quantile", "count_values",
	"rate", "irate", "increase", "delta", "idelta", "deriv", "predict_linear", "holt_winters",
	"changes", "resets", "absent", "absent_over_time", "avg_over_time", "min_over_time", "max_over_time",
	"sum_over_time", "count_over_time", "quantile_over_time", "stddev_over_time", "stdvar_over_time",
	"last_over_time", "present_over_time",
	"label_replace", "label_join", "vector", "scalar", "time", "timestamp",
	"day_of_month", "day_of_week", "day_of_year", "days_in_month", "hour", "minute", "month", "year",
	"abs", "ceil", "floor", "round", "exp", "ln", "log2", "log10", "sqrt", "sort", "sort_desc",
	"sort_by_label", "sort_by_label_desc", "clamp", "clamp_max", "clamp_min", "sgn", "pi", "deg", "rad",
	"histogram_quantile", "label_values",
	"bool", "and", "or", "unless", "offset", "ignoring", "on",
}


def rust_source_files() -> list[Path]:
	files: list[Path] = []
	for root in RUST_ROOTS:
		files.extend(root.rglob("*.rs"))
	return sorted(files)


def const_str_declarations(files: list[Path]) -> dict[str, str]:
	"""`const NAME: &str = "literal"` -> literal, across every given file."""
	consts: dict[str, str] = {}
	for path in files:
		for match in CONST_STR_DECL.finditer(path.read_text()):
			consts[match.group(1)] = match.group(2)
	return consts


def emitted_metrics() -> dict[str, list[str]]:
	"""metric name -> sorted list of `path:line` call sites."""
	files = rust_source_files()
	consts = const_str_declarations(files)

	sites: dict[str, list[str]] = {}
	for path in files:
		text = path.read_text()
		# Whole-file, not line-by-line: `counter!(\n  "name", ...)` — the
		# facade's multi-arg calls routinely put the name on its own line.
		for match in METRICS_FACADE_CALL.finditer(text):
			name = match.group(1) if match.group(1) is not None else consts.get(match.group(2))
			if name is None:
				continue  # a non-const, non-literal first argument (e.g. a runtime String) — not lexically resolvable, and not a name this check can assert about either way
			lineno = text.count("\n", 0, match.start()) + 1
			sites.setdefault(name, []).append(f"{path.relative_to(REPO_ROOT)}:{lineno}")
	return sites


def active_dashboard_sources() -> list[Path]:
	"""Jsonnet/libsonnet sources scripts/build.sh actually builds — everything
	under dashboards/ except dashboards/parked/, which is excluded from every
	dashboard's PromQL claim the same way build.sh excludes it from output."""
	parked = DASHBOARDS_DIR / "parked"
	return sorted(p for p in DASHBOARDS_DIR.rglob("*.*sonnet") if parked not in p.parents)


def extract_metric_tokens(expr: str) -> set[str]:
	expr = LABEL_VALUES_CALL_FULL.sub(" ", expr)  # handled separately, via LABEL_VALUES_CALL
	expr = GROUPING_CLAUSE.sub(" ", expr)

	with_selector = {m.group(1) for m in METRIC_WITH_SELECTOR.finditer(expr)}
	bareword_text = LABEL_SELECTOR_BLOCK.sub(" ", expr)
	bare = {m.group(1) for m in BARE_METRIC_TOKEN.finditer(bareword_text)}

	tokens = with_selector | bare
	return {t for t in tokens if t not in PROMQL_RESERVED}


def queried_metrics() -> dict[str, list[str]]:
	"""metric name -> sorted list of `path:line` occurrences."""
	sites: dict[str, list[str]] = {}
	for path in active_dashboard_sources():
		text = path.read_text()
		for match in PROMQL_FIELD.finditer(text):
			expr = match.group(1) if match.group(1) is not None else match.group(2)
			lineno = text.count("\n", 0, match.start()) + 1
			names = {m.group(1) for m in LABEL_VALUES_CALL.finditer(expr)}
			names |= extract_metric_tokens(expr)
			for name in names:
				sites.setdefault(name, []).append(f"{path.relative_to(REPO_ROOT)}:{lineno}")
	return sites


def exempt_queried_reason(name: str) -> str | None:
	if name in EXEMPT_QUERIED_EXACT:
		return EXEMPT_QUERIED_EXACT[name]
	for prefix, reason in EXEMPT_QUERIED_PREFIXES.items():
		if name.startswith(prefix):
			return reason
	return None


def histogram_base(name: str) -> str | None:
	"""Prometheus's own `_bucket`/`_sum`/`_count` expansion of a histogram's
	base name — `operation_duration_seconds_bucket` querying
	`histogram!("operation_duration_seconds", ...)` is not a phantom."""
	for suffix in HISTOGRAM_SUFFIXES:
		if name.endswith(suffix) and len(name) > len(suffix):
			return name[: -len(suffix)]
	return None


def main() -> int:
	emitted = emitted_metrics()
	queried = queried_metrics()

	errors: list[str] = []

	phantom = sorted(set(queried) - set(emitted))
	phantom = [name for name in phantom if exempt_queried_reason(name) is None and histogram_base(name) not in emitted]
	for name in phantom:
		locations = ", ".join(queried[name])
		errors.append(f"queried but never emitted: '{name}' ({locations}) — fix the query, emit it, or add it to EXEMPT_QUERIED_* with a reason")

	queried_or_histogram_base = set(queried) | {histogram_base(name) for name in queried if histogram_base(name)}
	orphan = sorted(set(emitted) - queried_or_histogram_base)
	orphan = [name for name in orphan if name not in EXEMPT_EMITTED]
	for name in orphan:
		locations = ", ".join(emitted[name])
		errors.append(f"emitted but never queried: '{name}' ({locations}) — add a panel, or add it to EXEMPT_EMITTED with a reason")

	# An exemption that no longer matches anything is a stale written
	# decision — same "written decision, not a silent pass" bar applies to
	# removing one as to adding it.
	for name in EXEMPT_EMITTED:
		if name not in emitted:
			errors.append(f"EXEMPT_EMITTED['{name}'] no longer matches any emitted metric — remove the stale exemption")

	if errors:
		print("metric contract check failed:\n", file=sys.stderr)
		for error in errors:
			print(f"  - {error}", file=sys.stderr)
		return 1

	print(f"metric contract OK: {len(emitted)} emitted metrics, {len(queried)} queried metrics, {len(active_dashboard_sources())} active dashboard sources.")
	return 0


if __name__ == "__main__":
	sys.exit(main())

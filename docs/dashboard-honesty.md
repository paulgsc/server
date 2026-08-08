# Dashboard honesty (#212)

## The rule

> Absence of data is not evidence of health. A panel with no series must
> render as *unknown*, and unknown must be visually indistinguishable from
> broken — never from fine.

Grafana's own defaults are hostile to this: a `stat`/`gauge` panel with zero
series still reduces to a null value, and a threshold list that starts
`{ value: null, color: 'green' }` — the default shape of every threshold
list in `infra/grafana/` — paints that null the same color as "measured and
fine." A panel counting push failures reads as green when there are zero
failures, and green when the metric doesn't exist, and green when the
scrape target has been down for a week. Before #212, that was
`rate-limit.jsonnet`'s actual, years-long state: a panel titled *"Proof of
Failure (By Contradiction)"* that had never once displayed a real number.

Three states, not two, and they must be distinguishable at a glance from
across the room:

| State | Meaning | Reads as |
|---|---|---|
| Data, within bounds | measured, fine | green |
| Data, out of bounds | measured, bad | red |
| No data | not measured — cannot say | grey/dashed, never green |

This is fault condition #6 in this project's (not yet written) fault
taxonomy — "the dashboard is lying by omission" sits alongside the more
obvious ways a service can fail. If a `[FAULT]` epic or doc is ever filed,
this rule belongs in it verbatim; until then this note is its only home.

## How it's enforced

- **`infra/grafana/dashboards/lib/panel-defaults.libsonnet`** — `harden(panel)`
  merges a `null+nan` value mapping into any `stat`/`gauge` panel's
  `fieldConfig`, overriding Grafana's threshold-based fallback color for an
  empty result. `hardenAll(panels)` applies it to an entire dashboard's
  panel list in one call. Every dashboard under `infra/grafana/dashboards/`
  (excluding `parked/`, see `infra/grafana/README.md`) wraps its `panels:`
  list in `hardenAll`.
- **`panelDefaults.livenessPanel(title, jobs, id, gridPos)`** — one
  `up{job="…"}` stat per scraped job a dashboard depends on, so "no data
  everywhere" reads as a dead scrape target rather than a quiet system.
  Every surviving dashboard carries at least one.
- Time series panels are left alone — a gap in a line is already
  self-evident, and `spanNulls: false` (the convention already used
  throughout this repo) is enough.
- No Grafana alert rules exist to carry this rule further (see
  `lib/alerts.libsonnet`'s removal in #220) — the dashboard's visual state
  *is* the alert at this stage. `docs/dashboard-honesty.md` — this file —
  is where that deliberate non-goal is recorded.

## Checked, not just documented

`scripts/check_metric_contract.py` (#217) is a separate, narrower
guarantee: it only proves a panel's query names a series something in this
workspace actually emits. It says nothing about whether that panel *renders*
the three states above — that part is enforced by code review against this
note and the `panel-defaults.libsonnet` convention, not by CI.

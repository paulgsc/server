# Grafana dashboards

Sources live in `dashboards/*.jsonnet` (plus `dashboards/lib/`), built by
`scripts/build.sh` into `generated/*.json`, which Grafana loads read-only
via `infra/compose/monitoring.yml`'s `dashboard-builder` + `grafana`
services. `generated/*.json` is checked in so a fresh `docker compose up`
doesn't depend on the builder succeeding at exactly the right moment — but
it's build output, not a second source of truth: `scripts/build.sh` (with
`jsonnet -J lib`, then `jq .` to format) is what produced it, and that's
what any diff between the two should be resolved by re-running, not by
hand-editing the JSON.

Every panel's query is checked against what this workspace actually emits
by `scripts/check_metric_contract.py` (#217) — see
`docs/dashboard-honesty.md` for the rule that check exists to enforce, and
why "no data" and "healthy" have to be visually different states.

## Active dashboards

| Dashboard | Answers | Backed by | Owning epic |
|---|---|---|---|
| `dashboard.jsonnet` | Is file_host up, and is its one instrumented operation path within latency budget? | `probe_success`/`probe_duration_seconds` (blackbox-exporter), `operation_duration_seconds` (file_host, `metrics` facade) | #212 (G2/G3) |
| `sys-dashboard.jsonnet` | Is the host itself healthy — CPU, memory, disk, network? | `node_*` (node_exporter) | #212 (G2/G3) |
| `processor.jsonnet` ("WHO DUNNIT") | If the host hangs, which process/container did it? | `namedprocess_*` (process-exporter), `container_*` (cadvisor), `node_*`, `up`, `ALERTS` | #212 (G2/G3) |
| `cache-dashboard.jsonnet` | Is `some-cache`'s hit rate healthy, and is the dedup guard absorbing thundering herds? | `cache_hits_total`, `cache_misses_total`, `cache_fetch_duration_seconds`, `cache_dedup_waiters_total` (some-cache, `metrics` facade, namespace-labeled) | #212 (G2/G3/G4 — rewritten from a 14-fake-name version) |

Every dashboard above binds to the one provisioned Prometheus datasource —
`infra/grafana/provisioning/datasources/datasources.yml` pins `uid:
prometheus`, and `dashboards/lib/config.libsonnet` is the single jsonnet-side
definition every panel takes it from (#218).

## Parked

`dashboards/parked/` holds designs whose queries assume metrics that don't
exist yet — `scripts/build.sh`'s own `find . -maxdepth 1 -name '*.jsonnet'`
already skips anything not directly under `dashboards/`, so parking is
"excluded from the build, kept for the epic that will finish it," not
"deleted." `scripts/check_metric_contract.py` skips `parked/` for the same
reason: a parked dashboard's query isn't a claim Grafana is currently making.

| Dashboard | Assumes | Returns via |
|---|---|---|
| `rate-limit.jsonnet` | `http_requests_total*` — no axum-prometheus layer or manual counter emits this | the [ABUSE] epic, once it defines a real request counter |
| `ws-dashboard.jsonnet` | `ws_connection_*`/`ws_client_*` — registered in `apps/servers/file_host/src/websocket/connection/instrument.rs` via the raw `prometheus` crate, but every recording macro has zero call sites, so nothing ever dereferences the `lazy_static` | the [CLIENTS] epic, once those call sites exist (or the metrics move to the `metrics` facade) |

## Removed

`lib/health.libsonnet`, `lib/latency.libsonnet`, `lib/alerts.libsonnet` — all
three imported `grafonnet/grafana.libsonnet`, a library not vendored
anywhere in this repo, so none of them could have compiled if referenced.
Zero dashboards imported them. `alerts.libsonnet` additionally defined
Grafana alert rules with no provisioning directory to load them into. See
#220.

`forensic-panels.libsonnet`'s `requestRateInvariant` panel (queried
`http_requests_total`, same missing premise as `rate-limit.jsonnet`) was
removed rather than parked — it has no dashboard of its own to park into,
and `processor.jsonnet`'s other 17 panels are real.

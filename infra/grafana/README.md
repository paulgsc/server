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
`docs/fault-conditions.md` (#213) is the companion definition for the six
conditions `dashboard.jsonnet`'s HEALTH row renders a verdict on.

## Active dashboards

| Dashboard | Answers | Backed by | Owning epic |
|---|---|---|---|
| `dashboard.jsonnet` | Is file_host in a fault state (six conditions, `docs/fault-conditions.md`), is its instrumented traffic within latency budget, is anybody connected (a real device, not just the WS liveness probe), is the engagement waker actually landing notifications, and is it abusing (or being abused past) its own limits? | `up`, `dependency_up`, `http_requests_total`/`http_request_duration_seconds`, `refusals_total`, `service_info`, `process_start_time_seconds`, `ws_connections`/`ws_connection_lifecycle_total`/`ws_connection_duration_seconds`/`ws_client_connections{client_type="probe"\|...}`, `sqlite_pool_size`/`sqlite_pool_idle`, `rate_limit_decisions_total`/`rate_limit_tokens_available`/`rate_limit_active_clients`/`rate_limit_capacity_per_minute` (file_host, `metrics` facade), `nudge_waker_due_subjects`/`nudge_waker_verdicts_total` (file_host `nudge::waker`, `metrics` facade), `cache_hits_total`/`cache_misses_total` (some-cache), `probe_success`/`probe_duration_seconds` (blackbox-exporter), `operation_duration_seconds` | #212 (G2/G3), #213 (F1–F5), #214 (C1–C4), #215 (A1–A3) |
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
| `ws-dashboard.jsonnet` | `ws_connection_*`/`ws_client_*` — registered in `apps/servers/file_host/src/websocket/connection/instrument.rs` via the raw `prometheus` crate, but every recording macro has zero call sites, so nothing ever dereferences the `lazy_static` | superseded: #214 (C1) moved the surviving families onto the `metrics` facade and #214 (C4) reclaimed this file's panel designs into `lib/clients-panels.libsonnet`. Kept parked rather than deleted — `client_type_distribution`/`message_rate`/etc. are already dashboarded, but this file is the record of the original mock-up. |

## Removed

`lib/health.libsonnet`, `lib/latency.libsonnet`, `lib/alerts.libsonnet` — all
three imported `grafonnet/grafana.libsonnet`, a library not vendored
anywhere in this repo, so none of them could have compiled if referenced.
Zero dashboards imported them. `alerts.libsonnet` additionally defined
Grafana alert rules with no provisioning directory to load them into. See
#220.

`forensic-panels.libsonnet`'s `requestRateInvariant` panel (queried
`http_requests_total`, same missing premise `parked/rate-limit.jsonnet` had)
was removed rather than parked — it has no dashboard of its own to park
into, and `processor.jsonnet`'s other 17 panels are real.

`parked/rate-limit.jsonnet` and `parked/rate-limiting/rate-limit.libsonnet`
— #215 (A3/#232) is the "[ABUSE] epic" this file's own header said it was
waiting for. Unlike `ws-dashboard.jsonnet` above, nothing here was worth
keeping as a record: every panel's design (the four-stat refusal row, the
contradiction/invariant panel) is reclaimed into `lib/abuse-panels.libsonnet`
and wired into `dashboard.jsonnet`'s ABUSE row, on the metrics this epic
actually shipped rather than the client-IP-labelled ones the parked queries
assumed (`http_requests_total_by_client` is still unbounded cardinality, per
#222's own non-goals — the reclaimed `topClients` panel is gone for that
reason, not an oversight).

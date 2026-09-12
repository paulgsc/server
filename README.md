# Rust Dedicated Server

## Overview

A multi-crate Rust workspace built around `file_host`, an Axum service backed
by SQLx repositories, a Redis/NATS JetStream pipeline, and WebSocket
transport. It's the backend evidence behind
[`paulgsc/some-ui`](https://github.com/paulgsc/some-ui)'s résumé claims — the
table below maps each claimed area to where it actually lives in the tree, so
a claim can be checked against code rather than taken on faith.

| Area | What's implemented | Where |
| --- | --- | --- |
| **HTTP API** | Axum routes with a generated, tested route inventory — the declared surface is diffed against the live routers, not just documented | [`apps/servers/file_host/src/routes`](./apps/servers/file_host/src/routes), [route inventory](./apps/servers/file_host/docs/route-inventory.md) |
| **SQLx repositories** | Typed repositories with compile-time-checked queries and paired migrations, one crate per domain (activity, capture, engagement, mood event, presence, push, session) | [`crates/db`](./crates/db), [`migrations`](./migrations) |
| **Redis / NATS pipeline** | Redis caching and in-flight coalescing in front of NATS JetStream jobs, with explicit redelivery semantics on retryable failures | [`crates/some-cache`](./crates/some-cache), [`crates/some-transport`](./crates/some-transport), [`crates/push_kit`](./crates/push_kit) |
| **WebSockets** | Restart-aware connections with heartbeat/staleness detection, broadcast isolation, and bounded shutdown | [`apps/servers/file_host/src/websocket`](./apps/servers/file_host/src/websocket), [`crates/ws-connection`](./crates/ws-connection), [`crates/ws-conn-manager`](./crates/ws-conn-manager) |
| **Overload controls** | Token-bucket rate limiting, load shedding, and timeouts, with rejection tracked as a first-class fault condition rather than inferred from resource pressure | [`apps/servers/file_host/src/rate_limiter`](./apps/servers/file_host/src/rate_limiter), [fault conditions](./docs/fault-conditions.md) |
| **Observability** | Prometheus metrics and dashboards that render missing data as *unknown*, never as healthy by default | [`apps/servers/file_host/src/metrics`](./apps/servers/file_host/src/metrics), [dashboard honesty](./docs/dashboard-honesty.md) |
| **Tests against real dependencies** | Test suites that exercise SQLite, Redis, and NATS directly instead of mocking them away | [`crates/db/activity/tests`](./crates/db/activity/tests), [`crates/ws-connection/tests`](./crates/ws-connection/tests) |

## Status and limitations

No users, no traffic. This is a single-engineer workspace, exercised by its
own test suite and CI rather than by production load — every row above means
"the code demonstrates this," not "this has been operated at scale." See
[`docs/fault-conditions.md`](./docs/fault-conditions.md) for what is
instrumented and what isn't, and [`docs/WARNING.md`](./docs/WARNING.md) for
the standing caveat on trusting any of it blindly.

---

## Repository map

```text
apps/
├── servers/file_host/  Axum HTTP + WebSocket backend: routes, rate limiting,
│                       metrics, nudge scheduling, streaming
├── orchestrator/       Supervises per-stream orchestrators over the
│                       NATS-backed transport
└── some-obs/           OBS WebSocket automation service

crates/
├── db/                 One SQLx repository crate per domain (activity,
│                       capture, engagement, mood_event, presence, push,
│                       session)
├── ws-connection/, ws-conn-manager/, ws-events/
│                       WebSocket connection lifecycle and event types
├── some-cache/, some-transport/, some-metrics/, some-services/
│                       Shared caching, NATS transport, metrics, and service
│                       abstractions
├── push_kit/           Web Push (VAPID) actuation
├── intervention/, study_domain/
│                       Domain logic for when and what the system intervenes on
└── sdk/, cursorium/, file_reader/, obs-websocket/,
    enum-name-derive/, gsheet_derive/
                        Supporting libraries and derive macros

docs/        Design notes, fault taxonomy, dashboard conventions, SLAs
infra/       Compose files, Grafana dashboards, Prometheus, NATS config
migrations/  SQLx migrations
nix/         Reproducible dev shells (default / rust / ci)
scripts/     Metric-contract and scrape-inventory checks
```

`Cargo.toml`'s `[workspace] members` list is the source of truth for exact
crate membership; the grouping above is a map to orient from, not a
duplicate of it.

---

## Requirements
* Rust (latest stable) and Cargo
* SQLite — every `crates/db/*` repository builds with SQLx's `sqlite`
  feature only; migrations live under [`migrations/`](./migrations)
* Redis and NATS JetStream for the caching/messaging pipeline (see
  [`infra/compose`](./infra/compose))

**OR** just use Nix:
```bash
nix develop  # default shell: Rust + dev tools + Whisper + audio libs
```

Three shells are available (`default`, `rust`, `ci`) — see
[Nix Development Environment](./nix/README.md) for details.

---

## Development

The workflows CI actually runs, scoped to whatever package you're touching:

```bash
cargo check --workspace
cargo test --workspace
cargo clippy -p <changed-package> --all-targets --keep-going --no-deps
DATABASE_URL="sqlite://$PWD/dev.db" cargo sqlx prepare --workspace
```

The generated HTTP route inventory that [`paulgsc/some-ui`](https://github.com/paulgsc/some-ui)'s
contract harness consumes:

```bash
DATABASE_URL="sqlite://$PWD/dev.db" make routes  # regenerate routes.server.{json,ts}
make routes-check                                # assert the inventory still matches the routers
```

Local dependencies (Redis, NATS, Prometheus/Grafana, `file_host`,
`orchestrator`) are composed via [`docker-compose.yml`](./docker-compose.yml)
and [`infra/compose`](./infra/compose).

---

## Documentation

### Development environment
* [Nix setup & shells](./nix/README.md)
* [WSL / PowerShell cheat sheet](./docs/WSL-CHEATSHEET.md)
* [Redis & RedisInsight setup](./docs/REDIS_INSIGHT_SETUP.md)

### Backend design
* [Route inventory](./apps/servers/file_host/docs/route-inventory.md) – the generated, tested HTTP surface
* [Fault conditions](./docs/fault-conditions.md) – the six fault states and how each is observed
* [Dashboard honesty](./docs/dashboard-honesty.md) – why missing data must never render as healthy
* [WebSocket Service SLA](./apps/servers/file_host/docs/sla/WebsSocket_Service_SLA.md)
* [Study nudge design](./docs/study-nudge.md)
* [Google Sheets ingestion pipeline](./docs/system_design/gsheets_webhook/README.md)

### Models
* [Whisper Model Optimization Guide](./docs/models/whisper-optimization.md)

### Caveats
* [⚠️ Important Warnings](./docs/WARNING.md)

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
| **HTTP API** | Axum routes with a generated, tested route inventory — a compile-time check parses the route registrations in source and fails on anything undeclared, stale, or duplicated (deliberately source-level, not a live-router probe, since that would need a fully booted SQLite/NATS `AppState`) | [`apps/servers/file_host/src/routes`](./apps/servers/file_host/src/routes), [route inventory](./apps/servers/file_host/docs/route-inventory.md) |
| **SQLx repositories** | Typed repositories with compile-time-checked queries and paired migrations, one crate per domain (activity, capture, engagement, mood event, presence, push, session) | [`crates/db`](./crates/db), [`migrations`](./migrations) |
| **Redis caching** | A dedup/cache layer fronting read-heavy lookups, independent of the NATS job pipeline below | [`crates/some-cache`](./crates/some-cache), [`apps/servers/file_host/src/cache.rs`](./apps/servers/file_host/src/cache.rs) |
| **NATS JetStream publishing** | Axum handlers publish job envelopes (e.g. tab-capture processing) to JetStream, and `some-transport` provides reusable ack/nak/redelivery primitives (`AckHandle`) for a consumer to use — no in-repo consumer instantiates them yet, per `infra/prometheus/inventory.yml`'s own note that the `tabsched-pipeline` worker "has no compose file or crate in this repo yet" | [`crates/some-transport`](./crates/some-transport) |
| **WebSockets** | Connections with heartbeat/staleness detection and broadcast isolation; shutdown disconnects every tracked connection and gives cleanup a fixed grace window (individual removals aren't themselves timeout-bounded). State is in-memory only — a restart drops every connection and subscription, with no resume token or reconnection path (unlike the NATS client, which does reconnect automatically, a separate mechanism this doesn't extend to WebSockets) | [`apps/servers/file_host/src/websocket`](./apps/servers/file_host/src/websocket), [`crates/ws-connection`](./crates/ws-connection), [`crates/ws-conn-manager`](./crates/ws-conn-manager) |
| **Overload controls** | Token-bucket rate limiting, load shedding, and timeouts, with rejection tracked as a first-class fault condition rather than inferred from resource pressure | [`apps/servers/file_host/src/rate_limiter`](./apps/servers/file_host/src/rate_limiter), [fault conditions](./docs/fault-conditions.md) |
| **Observability** | Prometheus metrics and dashboards that render missing data as *unknown*, never as healthy by default | [`apps/servers/file_host/src/metrics`](./apps/servers/file_host/src/metrics), [dashboard honesty](./docs/dashboard-honesty.md) |
| **Tests against real dependencies** | Real SQLite databases and in-memory WebSocket-actor tests run directly, not mocked. NATS integration tests exist but skip themselves when no broker is reachable, and neither NATS nor Redis is provisioned in this repo's CI — see Status and limitations | [`crates/db/activity/tests`](./crates/db/activity/tests), [`crates/ws-connection/tests`](./crates/ws-connection/tests) |

## Status and limitations

No users, no traffic. This is a single-engineer workspace, exercised by its
own test suite and CI rather than by production load — every row above means
"the code demonstrates this," not "this has been operated at scale." See
[`docs/fault-conditions.md`](./docs/fault-conditions.md) for what is
instrumented and what isn't, and [`docs/WARNING.md`](./docs/WARNING.md) for
the standing caveat on trusting any of it blindly.

CI (`.github/workflows/test.yml`) doesn't provision Redis or a NATS broker, so
the NATS-dependent tests in `crates/some-transport` skip themselves there
(they run for real against `docker-compose`'s NATS service locally), and
`crates/some-cache` currently has no test suite that exercises Redis itself
rather than its in-process helpers.

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
* [`sqlx-cli`](https://crates.io/crates/sqlx-cli) (`cargo install sqlx-cli
  --version 0.7.4 --locked --no-default-features --features sqlite`, matching
  the version CI installs) — needed to create/migrate the database and run
  `cargo sqlx prepare`
* SQLite — every `crates/db/*` repository builds with SQLx's `sqlite`
  feature only; migrations live under [`migrations/`](./migrations)
* A NATS server with JetStream enabled — `file_host` opens a real
  connection to it at startup (`AppState::build`) and won't come up if it's
  unreachable. `AppState::build` only constructs a `JetStreamPublisher`; it
  doesn't create the `pipeline` stream `POST /tabs/pipeline` publishes to
  (`get_or_create_stream` is only called from `DurableConsumer::bind`,
  which nothing in this repo instantiates yet), so that stream needs to
  exist already — bootstrap it externally (e.g. `nats stream add`) before
  that route will work
* A running Redis instance — `AppState::build` only validates the URL at
  startup (`redis::Client::open` doesn't connect), so the process *starts*
  without Redis but reports it unhealthy via `/ready` and fails at first use
  (see [`infra/compose`](./infra/compose) for both services)

Nix covers the Rust/SQLx/SQLite toolchain above, not the Redis/NATS
services — those still need to be running separately:
```bash
nix develop  # default shell: Rust + dev tools + Whisper + audio libs
```

[`flake.nix`](./flake.nix) defines five shells — `default`, `rust`, `ci`,
`whisper`, and `llm` — see [Nix Development Environment](./nix/README.md)
for the first three.

---

## Development

What [`.github/workflows/test.yml`](./.github/workflows/test.yml) runs on a
clean checkout — order matters, since `sqlx::query!` checks SQL against a
real schema at compile time, so the database has to exist and be migrated
before `prepare`/`check`/`test` will build at all:

```bash
export DATABASE_URL="sqlite://$PWD/dev.db"
sqlx database create
sqlx migrate run --source migrations
cargo sqlx prepare --workspace
cargo check --workspace
cargo test --workspace
```

Clippy isn't part of that workflow. `.github/workflows/lint.yml` runs
`cargo clippy --workspace -- -D warnings --no-deps` but is currently paused
(scoped to a `foo` branch, not `main`). As a local check before pushing,
this repo's own `CLAUDE.md` recommends scoping it to whatever package
you're touching, with flags `lint.yml` doesn't pass:

```bash
cargo clippy -p <changed-package> --all-targets --keep-going --no-deps
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

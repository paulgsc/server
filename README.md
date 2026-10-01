# Rust Dedicated Server

## Overview

A multi-crate Rust workspace built around `file_host`, an Axum service backed
by SQLx repositories, an independent Redis caching layer and NATS JetStream
job pipeline, and WebSocket transport. It's the backend evidence behind
[`paulgsc/some-ui`](https://github.com/paulgsc/some-ui)'s résumé claims — the
table below maps each claimed area to where it actually lives in the tree, so
a claim can be checked against code rather than taken on faith.

| Area | What's implemented | Where |
| --- | --- | --- |
| **HTTP API** | Axum routes registered on a `RouteTable` that records each route as it registers it, so the route inventory is read from the same tables the server serves rather than kept alongside them; a raw `Router::route` call is a clippy error. CI (`routes.yml`) checks every PR's snapshot against the client repo's contracts and opens the sync PR there on merge | [`apps/servers/file_host/src/routes`](./apps/servers/file_host/src/routes), [route inventory](./apps/servers/file_host/docs/route-inventory.md) |
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
└── cursorium/, obs-websocket/
                        Supporting libraries

docs/        Design notes, fault taxonomy, dashboard conventions, SLAs
infra/       Compose files, Grafana dashboards, Prometheus, NATS config
migrations/  SQLx migrations
nix/         Reproducible dev shells (default / rust / ci / whisper / llm)
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

### Host-specific values

This server runs on one machine today: the NixOS dev host `nixos.local`, with
its data under `/mnt/storage` and NATS and Redis beside it. Some defaults,
paths and example values are right only there. They are allowed, but never
silent:

* **Marked.** Each one carries the word `HOST-SPECIFIC` where it is defined:
  `grep -rn HOST-SPECIFIC example.env infra/ apps/` lists every value a new
  host (a VPS, say) must replace.
* **Announced.** `file_host` logs one `HOST-SPECIFIC` warning at startup naming
  every variable it is running on a dev-host value for
  (`Config::host_specific_defaults_in_use`).
* **Tracked.** [#406](https://github.com/paulgsc/server/issues/406) is the
  inventory and the plan to move these out of code and compose into each
  host's own configuration.

The strongest one is `WEBAUTHN_RP_ID`: every passkey is bound to it, so
moving to a new domain strands every passkey made on the old one. Settle the
long-term domain before people create passkeys.

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

Clippy runs in its own workflow, [`.github/workflows/lint.yml`](./.github/workflows/lint.yml),
alongside `rustfmt` and the repo's own policy scripts, on pull requests to
`main` whose diff touches Rust-related paths (the workflow's `determine_jobs`
step lists them; a Markdown-only PR skips it). `.cargo/config.toml` enables
the `all`, `pedantic`, and `nursery` groups, and the workspace does not yet
meet that bar, so clippy runs as a ratchet rather than a clean gate: every finding is compared against a committed
baseline, [`scripts/clippy_baseline.json`](./scripts/clippy_baseline.json). A new
finding fails the PR, a fixed one fails until the baseline is regenerated, and
the baseline may only shrink relative to `main`, except in a PR that bumps
`lint.yml`'s pinned clippy toolchain, where it may grow by the new release's
lints (the workflow passes `--allow-growth` only then).
[`scripts/check_clippy_baseline.py`](./scripts/check_clippy_baseline.py)
explains the mechanism. The baseline is specific to one clippy release, so
`lint.yml` pins its toolchain (1.94.1 at the time of writing; the pin in that
file is authoritative) rather than using the "latest stable" above. Run the
same one to reproduce CI:

```bash
rustup toolchain install 1.94.1 --component clippy
cargo +1.94.1 clippy --workspace --all-targets --all-features --keep-going --message-format=json \
  -- --cap-lints=warn -A unknown-lints > clippy.json
python3 scripts/check_clippy_baseline.py clippy.json   # add --update after fixing debt
```

The generated HTTP route inventory that [`paulgsc/some-ui`](https://github.com/paulgsc/some-ui)'s
contract harness consumes:

```bash
export DATABASE_URL="sqlite://$PWD/dev.db"
make routes        # regenerate routes.server.{json,ts}
make routes-check  # assert the inventory still matches the routers (also needs DATABASE_URL: it builds the crate to run the test)
```

Local dependencies (Redis, NATS, Prometheus/Grafana, `file_host`,
`orchestrator`) are composed via [`docker-compose.yml`](./docker-compose.yml)
and [`infra/compose`](./infra/compose).

To run `file_host` from source while that container keeps port 3000:

```bash
make dev   # takes the next free port from 3000 and records it for some-ui's vite dev
```

It moves up to the next free port only because `make dev` asks it to
(`FILE_HOST_PORT_FALLBACK`); run any other way, a taken port is a startup
failure, as production needs. It records the port it got in
`$XDG_RUNTIME_DIR/file_host/dev-port.json` (`/tmp/...` without one), and
`paulgsc/some-ui`'s `vite dev` proxies there while it runs and back to the
container when it stops, reloading the page either way. The listen address
itself is `FILE_HOST_BIND`/`FILE_HOST_PORT` (default `0.0.0.0:3000`). `make dev`
reads the same environment as `cargo run`: pointed at the container's database
and VAPID keys, it runs a second nudge waker beside the container's.

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

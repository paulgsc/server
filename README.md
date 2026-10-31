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
| **HTTP API** | Axum routes registered on a `RouteTable` that records each route as it registers it, so the route inventory is read from the same tables the server serves rather than kept alongside them; a raw `Router::route` call is a clippy error. CI (`routes.yml`) checks every PR's snapshot against the client repo's contracts, runs those contracts against the PR's own `file_host`, and opens the sync PR there on merge | [`apps/servers/file_host/src/routes`](./apps/servers/file_host/src/routes), [route inventory](./apps/servers/file_host/docs/route-inventory.md) |
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
├── some-cache/, some-transport/, some-metrics/, some-services/, some-shutdown/
│                       Shared caching, NATS transport, metrics, service
│                       abstractions, and the shutdown signal
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

### Configuration

Each binary (`file_host`, `orchestrator`, `some-obs`) reads its environment
through one clap `Config`, in its `config.rs`, and nowhere else (#410, #484).
`<binary> --help` lists every variable it reads, with its default. A value
that does not parse stops the binary at start with an error naming the
variable; it never becomes the default. Libraries read no environment: they
take their config from the binary that uses them. A variable more than one
binary reads (`NATS_URL`, `RUST_LOG`, the `OTEL_*` ones) is declared once, in
[`crates/app-config`](./crates/app-config), and flattened into each `Config`
that reads it (#405).

That one declaration is also readable without starting anything (#503). Each
binary's tests hold a copy of its `Config`'s schema in
[`infra/env/<binary>.json`](./infra/env): every variable, whether it is
required, its default, its kind, its help, and whether it is a secret (its
value hidden, `hide_env_values`). [`example.env`](./example.env) is generated
from those copies and the compose files' `${...}` variables by
[`scripts/env_schema.py`](./scripts/env_schema.py); nobody writes a list of
variables by hand. `make env-schema` regenerates both after a `Config` or a
compose file changes; `make env` lists every variable the stack reads and
whether `.env` sets it.

- **ENV1: a variable that does not parse stops its binary at start, by name.**
  - _Claim:_ every environment variable this workspace's own code reads is
    declared on its binary's clap `Config`, and one whose value the declared
    parser refuses stops the binary before it does anything else, with an
    error naming the variable.
  - _Family:_ every binary (`file_host`, `orchestrator`, `some-obs`); every
    variable each declares; set or unset; a value of the wrong kind or out of
    range; read from the environment or given as a flag.
  - _Proof:_ each `Config`'s `every_variable_that_does_not_parse_is_refused_by_name`
    walks every declared variable: each is its own value name (clap's error
    names the value name, never the variable a value came from), each one that
    parses is refused with a bad value by an error naming it, and a variable
    in neither its refused nor its free-text list fails the test. Each
    binary's `tests/config.rs` runs the binary with one bad variable in its
    environment and requires exit 2 naming it. `clippy.toml`'s
    `disallowed-methods` makes reading the environment anywhere else
    (`std::env::var`, `var_os`, `vars`, `dotenvy::var`, `EnvFilter`'s and its
    builder's environment readers, `tracing_subscriber::fmt::init`) a lint
    error; the reads that are not configuration carry an `#[allow]` saying so.
  - _Tolerance:_ a free-text value (a URL, a key, a path) is checked by what
    uses it, not by clap. Dependencies read variables of
    their own that no `Config` declares, with their own fallbacks: tokio's
    `TOKIO_WORKER_THREADS`, polars' `POLARS_MAX_THREADS`, and the OTel SDK's
    `OTEL_RESOURCE_ATTRIBUTES`, `OTEL_EXPORTER_OTLP_HEADERS` and `OTEL_BSP_*`.

- **ENV2: the schema, `example.env` and each binary's `Config` agree.**
  - _Claim:_ `infra/env/<binary>.json` is what that binary's `Config`
    declares, and `example.env` is what `scripts/env_schema.py` writes from
    those files and the compose files; neither can fall behind its source.
  - _Family:_ every binary with a `Config`; every variable it declares and
    every `${...}` the compose files interpolate; a changed name, default,
    help, kind or secrecy; a hand edit of either generated file.
  - _Proof:_ each `Config`'s `the_checked_in_schema_is_this_config` compares
    the file with `app_config::schema` of the `Config` (`cargo test`), and
    `Lint`'s `Env schema` job runs `env_schema.py example --check`.
  - _Tolerance:_ none for what a binary declares. What clap cannot see, a
    requirement that depends on another variable (`WEBAUTHN_RP_ID` while
    `AUTH_ENABLED` is on) and a number's limits (`MAILBOX_SECONDS` at least
    60 while the mailbox is on), the binary declares beside its `Config` as
    `RULES`. `app_config::schema::parse` refuses to start on any that does not
    hold, naming all of them, and the same `RULES` are in the schema, so the
    preflight checks what start-up checks (#580, each binary's
    `tests/config.rs`). A relation between two values (the game feed's
    backoff cap at least its cadences) and a value's shape (the mailbox URL)
    stay the feature code's own check.

### Deploying the stack

On the host, from the checkout (#504):

```bash
cp example.env .env          # first time only
make preflight FILL=1        # names every gap at once, and asks for each
make up                      # the preflight again, then the deploy (below)
```

The preflight reads `docker compose config` as compose will, interpolating
each service's environment from `.env` and the shell, and checks every
service labelled `env-schema` against that binary's schema. It names every
required variable that is missing and every value its binary would refuse at
start, all in one pass (including the ones required only while another is
on, such as the mailbox's while `MAILBOX_ENABLED`), and exits non-zero if
there is any, so `make up`
never starts on a known gap. It also warns about each `.env` line nothing
reads: a renamed or retired variable is otherwise ignored without a word.
`FILL=1` writes what it asks for to `.env` in clear text, where compose reads
it, and leaves the file readable by its owner alone (mode 600); secrets kept
encrypted in git come with the NixOS modules (#408).
A `${NAME:?}` in any compose file stops every service, not only its own,
because compose interpolates the whole project before it starts one; the
preflight names those too.

- **ENV3: the preflight names every gap before anything starts.**
  - _Claim:_ given an `.env`, `make preflight` lists every variable a
    labelled service's binary requires and is not given, every value it
    would refuse, and every required compose variable unset, in one run, and
    exits 1 when there is any.
  - _Family:_ every service labelled `env-schema`; given by `.env`, by the
    service's own `environment:`, or by the shell; unset, or empty or of the
    wrong kind where its binary refuses that (an empty value is set: clap
    hands it to the parser, which takes it as text and refuses it as a
    number or a switch); required always, or while another variable is on.
  - _Proof:_ `env_schema.py --self-test` (`Lint`'s `Env schema` job).
  - _Tolerance:_ a free-text value (a URL, a key, a path) and a condition on
    another variable are checked by the binary at start, as ENV1 and ENV2
    say. A container without the label (the third-party ones) is checked
    only for the compose variables it interpolates.

### Deploying to home

`make up` is the deploy (AS18, #564), and `make up PULL=1` is the one that
takes a new image: it replaces the old habit of `docker compose pull &&
docker compose up -d`, which put whatever `:latest` was in place of a working
container whether or not it could become ready. After the preflight,
`scripts/deploy.py`:

1. records the image the running `file-host` container was started from;
2. pulls `file-host`'s image, with `PULL=1` (only that one: the rest of the
   stack is not rolled back, so it is not pulled either);
3. tags the serving image `pgathondu/server:previous` when a different one
   is about to start, so a prune cannot take it before it is needed;
4. runs the `migrate` one-shot (AS16, #563) while the old container still
   serves; a failure stops the deploy there, with nothing replaced;
5. runs `docker compose up -d`, and waits up to 90 s for the container's
   healthcheck to read `healthy`. That check is `file_host --health-check`,
   which passes on `/ready` saying ready or degraded (AS15, #560). A
   container that exits, restarts or reads `unhealthy` fails at once;
6. if it does not come up, tags the recorded image back as
   `pgathondu/server:latest` and recreates `file-host` alone from it, then
   waits the same 90 s for that.

It says what it did on its output, in lines starting `deploy:`, and exits
`0` deployed, `1` not deployed (pull or migration failed; the old container
still serves), `2` rolled back (the previous image serves), or `3` nothing
ready, which needs you: the previous image failed too, or there was none (a
first deploy), or the image did not change, so the cause is `.env`, a config
the binary refuses or a dependency down, which the same image would meet
again. Once AS17's alert channel exists, exits 2 and 3 are what it should
carry; until then the terminal is the only place they appear. After a
rollback the local `:latest` names the previous image, so a plain `make up`
keeps it; the next `make up PULL=1` tries the new one again.

The bound is the healthcheck's (`infra/compose/file_host.yml`): it probes
every 5 s through its 40 s `start_period` (`start_interval`, Docker Engine
25+), then every 30 s with a 10 s timeout, and 90 s covers all of those plus
one full interval and 10 s of slack. A bad image therefore costs about 3
minutes of AV1's 43-minute month, plus the pull and the migration, instead of
however long it takes someone to notice. The script lives here, beside the
compose files it drives, not in `paulgsc/dotfiles`: home runs this stack
from this checkout, and the NixOS modules (#408) that would move it into
`dotfiles` are not in use yet.

This is not a zero-downtime deploy: the old container stops before the new
one proves itself, and the rollback restarts it. Running two instances behind
Caddy so the old one serves until the new one is ready is #409 (deferred) and
AS20 (#566).

**The database is never restored by the deploy.** A migration that ran stays
applied, and the previous image serves the database as the migration left it:
a database ahead of its binary is designed to be one it serves
(`apps/servers/file_host/src/schema.rs`), which is what #564 records. Restoring
`backups/<stem>-<UTC>-before-<version>.db` is not done for a reason, not
overlooked: the old `file_host` keeps serving while `migrate` takes the copy,
and the new one serves (and its background passes write) from the moment it
starts, ready or not, so the live database can hold writes the backup lacks
at every point after the copy. A restore is safe only when nothing could have
written since the copy, which the deploy cannot show, so on its own it would
lose records. The one known case where the old binary does not serve a newer
schema, `drop_sharing_choice`, is not one a restore should fix either: its down
migration recreates the table it dropped, empty, so the old binary refuses
nothing and no record written since the copy is lost (`schema.rs`).
If you ever restore by hand, stop `file-host`, move the live file aside under
`backups/` with a name of its own (never delete it, and not a
`-before-` name, which `migrate` would count among its own and prune), then
copy the backup in. When the deploy ends with exit 3 it names the backup this
run took.

### NixOS modules

The flake exports each binary as a NixOS service, `nixosModules.file-host` and
`nixosModules.orchestrator` (#408), run as an OCI container. Their options are
generated from the same schema (`scripts/nixos_options.py` writes
`nix/modules/generated/`, and `Lint`'s `Env schema` job holds it in step), so
`nixos-option services.file-host.settings` lists every variable with its help,
and `nixos-rebuild` refuses a configuration with a required setting missing,
one required while another is on, or a number out of its limits, naming every
gap in one evaluation (`nix/modules/test.nix`, the `NixOS modules` job). A
secret has no option: it reaches the container only through
`environmentFiles`, a sops-nix template on the host.

Nothing uses them yet: the box still runs the compose stack (`make up`), and
each module is off unless `enable`. Moving the box onto them is its own step,
in its own configuration (`paulgsc/dotfiles`), and gives up what compose
gives, such as waiting on a dependency's healthcheck (#408).

### Host-specific values

This server runs on one machine today: the NixOS dev host `nixos.local`, with
its data under `/mnt/storage` and NATS and Redis beside it. Some defaults,
paths and example values are right only there. They are allowed, but never
silent:

* **Marked.** Each one carries the word `HOST-SPECIFIC` where it is defined:
  `grep -rn HOST-SPECIFIC example.env infra/ apps/ crates/` lists every value
  a new host (a VPS, say) must replace.
* **Announced.** `file_host` logs one `HOST-SPECIFIC` warning at startup naming
  every variable it is running on a dev-host value for
  (`Config::host_specific_defaults_in_use`).
* **Required where no default is right.** `ALLOWED_ORIGINS` and `APP_BASE_URL`
  have no default (#406): a host sets its own, and `make preflight` names them
  if it does not. The compose files' host paths are variables (`STORAGE`,
  `TRANSCRIBE_MODELS_DIR`, `METABASE_DATA_DIR`, `DATABASES_DIR`,
  `OLLAMA_MODELS_PATH`) whose dev-host defaults are marked where they are used.
* **Tracked.** [#406](https://github.com/paulgsc/server/issues/406) is the
  inventory and the plan to move these out of code and compose into each
  host's own configuration.

That host's own configuration is
[`paulgsc/dotfiles`](https://github.com/paulgsc/dotfiles): its `nixos/`
builds `nixos.local` (`nixos-rebuild switch --flake`), including the Caddy
that serves `file-host.home.maishatu.com` and the rest (`nixos/subdomains`)
and the machine's secrets, kept encrypted with sops-nix (`nixos/secrets`,
`secrets/nixos.yaml`). Owner, 2026-10-31. A setting this server needs from
the host belongs there, and the NixOS modules (#408) will be consumed from
there.

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

### Caveats
* [⚠️ Important Warnings](./docs/WARNING.md)

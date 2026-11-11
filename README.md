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
* [`sops`](https://github.com/getsops/sops) 3.9 or later, and a key
  `.sops.yaml` names, to deploy or run `make dev` (`nix shell nixpkgs#sops`;
  the default `nix develop` shell has it). See "Deploying the stack"
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
required, its default, its kind and its help. It is the contract
[`envctl`](./apps/envctl) reads, and it holds names and rules, never a
deployment's values. `make env-schema` regenerates it, and the NixOS options
from it, after a `Config` changes; `make env` lists every variable the stack
reads and whether `deploy/<role>/` sets it.

Every variable is a secret, set once per deployment (#613, the owner's
principles). A value that is not a secret is a constant in the source, not a
variable, and a value that changes while the app runs is data behind an
admin lever, not a variable. So a deployment's values are never plain text in
git, and CI never sees one: they are in `deploy/<role>/<app>.sops.env`, one
file per app and one for compose's own `${...}` variables
(`compose.sops.env`), encrypted with [sops](https://github.com/getsops/sops)
to the age recipients in [`.sops.yaml`](./.sops.yaml), the same as
`paulgsc/dotfiles`' (its admin key, and the box's SSH host key). Each file's
names are readable and its values are not. A new machine or shell needs only
its key: nothing is retyped and there is no `.env` (#614).

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
    their own that no `Config` declares, with their own fallbacks: the OTel
    SDK's `OTEL_RESOURCE_ATTRIBUTES`, `OTEL_EXPORTER_OTLP_HEADERS` and
    `OTEL_BSP_*`. Tokio's `TOKIO_WORKER_THREADS` is not one of them: each
    binary sets its worker threads in its `#[tokio::main]` (#617).
    An app's deployment file may set the OTel SDK's; its container gets them
    with the rest of the file.

- **ENV2: the schema, the NixOS options and each binary's `Config` agree.**
  - _Claim:_ `infra/env/<binary>.json` is what that binary's `Config`
    declares, and `nix/modules/generated/` is what `envctl nix` writes from
    those files; neither can fall behind its source.
  - _Family:_ every binary with a `Config`; every variable it declares; a
    changed name, default, help, kind or secrecy; a hand edit of either
    generated file.
  - _Proof:_ each `Config`'s `the_checked_in_schema_is_this_config` compares
    the file with `app_config::schema` of the `Config` (`cargo test`), and
    `Lint`'s `Env schema` job runs `envctl nix --check`.
  - _Tolerance:_ none for what a binary declares. What clap cannot see, a
    requirement that depends on another variable (`WEBAUTHN_RP_ID` while
    `AUTH_ENABLED` is on) and a number's limits (`MAILBOX_ARCHIVE_DAYS`
    from 30 to 36500 while the mailbox archives), the binary declares beside its `Config` as
    `RULES`. `app_config::schema::parse` refuses to start on any that does not
    hold, naming all of them, and the same `RULES` are in the schema, so the
    preflight checks what start-up checks (#580, each binary's
    `tests/config.rs`). A relation between two values (the game feed's
    backoff cap at least its cadences) and a value's shape (the mailbox URL)
    stay the feature code's own check.

### Deploying the stack

On the box, from the checkout, in any shell, with a key `.sops.yaml` names.
`make` builds envctl with the flake's cargo (through `nix develop .#rust`
when no `cargo` is on `PATH`), and envctl runs `sops` from `PATH` (the
`.#rust` and default shells have it), else through `nix shell
nixpkgs#sops`, else `$SOPS` (the admin key at `~/.config/sops/age/keys.txt`, as
`paulgsc/dotfiles`' `docs/secrets.md` sets up):

```bash
make up                       # DEPLOY=home: the preflight, then docker compose up -d
make compose ARGS="logs -f file-host"   # any other compose command, on the same files
```

`make up` decrypts `deploy/home/` with `sops decrypt` into envctl's memory and
runs the preflight on it. Only then does it start compose: compose's own
variables in its environment, and each labelled service's own file written
to `$XDG_RUNTIME_DIR/server/<pid>/<app>.env` (tmpfs, mode 600 in a mode 700
directory of that run's own, refused anywhere that is not tmpfs) for that
service's `env_file` alone, so two runs never touch each other's. They are removed
once compose returns, and a running container keeps its environment. Compose
runs with `--env-file /dev/null`, so a stray `.env` is read by nothing, and a
compose variable the shell happens to export is dropped unless the role's
file sets it. `REPO_ROOT` comes from make (`$(CURDIR)`), and so does
`ENV_DIR`. Every image compose starts comes from the role's release,
`deploy/<role>/release.json` (RELEASE #787, R1 #788): each service's
`image:` is `${IMAGE_<SERVICE>:?…}` (`file-host` reads `IMAGE_FILE_HOST`),
and envctl sets those from the release, never from a deployment file or the
shell. It is the one plain file under `deploy/`, since an image reference is
not secret. To move an image, change its line there; `make deploy-check`
refuses an `image:` written any other way, a service the release does not
name, and a release entry no service starts. A bare `docker compose` stops on
the first of these variables it lacks, naming it, which is why every compose
command goes through `make compose`.

The preflight checks every service labelled `env-schema` against that
binary's schema, as compose would start it: its file, and its service's own
`environment:` over it. It names every required variable that is missing
and every value its binary would refuse at start, all in one pass (including
the ones required only while another is on, such as the mailbox's while
`MAILBOX_ENABLED`), and exits non-zero if there is any, so `make up` never
starts on a known gap. It warns about a name a file sets that nothing reads
(a renamed or retired variable), and one compose's own `environment:` would
override. A `${NAME:?}` in any compose file stops every service, not only
its own, because compose interpolates the whole project before it starts one;
the preflight names those too. It prints names, never a value.

Every file states every variable of its binary but compose's own wiring,
so a deployment decides each one. An empty value (`NAME=`) is "decided:
unset" (#616): it reaches the binary unset, never set to nothing. To change
a value: `sops edit deploy/<role>/<app>.sops.env` (or `sops set`), then
`make up`. sops' dotenv does no quoting: `KEY="v"` gives
the container `"v"`, quotes and all, so write `KEY=v`. To prune variables,
`make prune APP=file_host` opens a checklist of the file's variables grouped
by the schema's sections, a page at a time (`PAGE=10`), names and states
only: a declared one pruned is stated unset, an undeclared one removed, and
the file is written through sops (`NAMES=A,B` prunes without it; #628).
To add a machine: add its recipient to
`.sops.yaml` here and in `paulgsc/dotfiles`, then `sops updatekeys` each
file.

**Commands run by hand (#634).** A tool run on the box, `sqlx migrate run`
say, reads host-side values no app file holds: compose gives file-host
`DATABASE_URL` as a path inside the container. Those go in
`deploy/<role>/tools.sops.env` (`sops edit` it; on home,
`DATABASE_URL=sqlite:///<STORAGE>/hopium.db`), whose names are free.
`nix develop .#rust` gives the shell a `tool` command:
`tool sqlx migrate run --dry-run` runs that one command with the file
decrypted into its environment only, for the role in `$DEPLOY` (home by
default, as for `make up`; `DEPLOY=dev tool …` for a scratch database). A
variable the shell sets itself wins. From any other shell,
`make tool ARGS="sqlx migrate run --dry-run"` does the same. Nothing is
exported into a shell, so no role's value reaches `make dev` or anything
else run there.

**Moving the box onto them, once (ENV2, #615).** The values exist today only
in the running containers and the box's plain-text `.env`. `make import`
(`DRY=1` to only name what would change) reads each running container with
`docker inspect` and writes `deploy/home/` straight through `sops encrypt`,
plain text only on its stdin: each labelled container's schema variables
(and the OTel SDK's own), less compose's own wiring and whatever else the
image sets, and every other variable of its schema stated too: at its
default, or empty (unset) when it has none; and compose's own variables, read back from the container
environment or bind mount compose put each into, else from `.env`. It names
each step on stderr and each variable it writes, never a value, and ends by
naming the `.env` to delete. Then commit `deploy/home/`, run `make up`, and
delete `.env`.

- **ENV3: the preflight names every gap before anything starts.**
  - _Claim:_ given a role's files, `make preflight` lists every variable a
    labelled service's binary requires and is not given, every value it
    would refuse, every required compose variable unset, and a labelled
    service with no file, in one run, and exits 1 when there is any.
  - _Family:_ every service labelled `env-schema`; given by its file or by
    the service's own `environment:`; unset, or empty or of the wrong kind
    where its binary refuses that (an empty value is set: clap hands it to
    the parser, which takes it as text and refuses it as a number or a
    switch); required always, or while another variable is on.
  - _Proof:_ `envctl`'s `preflight` tests, and its
    `preflight_names_what_is_missing_and_never_a_value` against stand-in
    `sops` and `docker` (`cargo test`).
  - _Tolerance:_ a free-text value (a URL, a key, a path) and a condition on
    another variable are checked by the binary at start, as ENV1 and ENV2
    say. A container without the label (the third-party ones) is checked
    only for the compose variables it interpolates.

- **ENV4: no deployment value is in the clear in git, in CI, or in the
  checkout.**
  - _Claim:_ every file under `deploy/` is one sops encrypted, and no other
    tracked file is a deployment's env file; CI reads names and never holds
    a key; `make up`, `make compose`, `make dev` and `make import` hold
    values in memory and write them only to tmpfs (one service's file, while
    compose reads it) or through `sops encrypt`; and none prints a value.
  - _Family:_ every role and app under `deploy/`; every compose service
    labelled `env-schema`, each reading exactly its own decrypted file;
    `import` from running containers; a value with quotes, `$`, `#` or a
    newline.
  - _Proof:_ `Lint`'s `Env schema` job runs `envctl check` (ENV3, #616),
    which holds no key and names each problem: a labelled service reading
    anything but its own `env_file`, or with no file in a role that runs
    compose; a `deploy/` file sops did not write (a value or comment in the
    clear, no MAC, version or recipient); a variable of its binary it does
    not state (empty for unset), a name in one that its binary or compose
    does not read, or one that compose's own `environment:` overrides; a
    binary reading `app-config` with no checked-in schema; and any tracked
    plain-text env file (`.env`, `.env.*`, `*.env`);
    `envctl`'s `stand_ins` tests run its binary against stand-in `sops` and
    `docker` scripts and find no value on disk or in its output after an
    import, a preflight or an `up`, the decrypted file mode 600 while compose
    runs and gone after, and a directory on a disk refused.
  - _Tolerance:_ the stand-ins are not sops or Docker; the first real run is
    the owner's, on the box. Docker itself keeps each container's environment:
    dockerd writes it in the clear to
    `/var/lib/docker/containers/<id>/config.v2.json` (root's alone), which is
    how a container restarts after a reboot without decrypting again, and
    whoever can `docker inspect` a container reads it. tmpfs pages can be
    swapped out. envctl survives Ctrl-C, SIGTERM and SIGHUP while compose
    runs and removes its files after; only a SIGKILL leaves them, until the
    next run replaces them or the session ends. Defaults
    are source constants, a `Config`'s (#617). `RUST_LOG` is each app's
    own: its binary's default filter, or the app's file in `deploy/<role>/`;
    neither compose nor an image sets one. The NixOS modules below put settings in the
    store, and nothing uses them.

### Deploying to home

A new `file_host` image takes one command and no other step (AS16 #563, AS18
#564; the owner's ask of 2026-11-10):

```bash
make home-deploy
```

It runs `envctl deploy` (`apps/envctl/src/deploy.rs`, built by make like the
rest of envctl; home has no python, and nothing home runs needs it):

1. **Pulls `pgathondu/server:latest`**, and stops there, having restarted
   nothing, when it is the image already serving. Only that image:
   `rust-runner` and `orchestrator` are pulled by hand
   (`make compose ARGS="pull rust-runner orchestrator"`, then `make up`), and
   nothing third-party is pulled. It does not update the checkout either:
   see "Needs a newer checkout", below.
2. **Skips an image that already failed here**: its id is in
   `~/.local/state/server-deploy/failed-images.json` (the on-demand unit's:
   `/var/lib/home-deploy/`), so `:latest` is tagged
   back to what serves and nothing restarts until a newer one is published.
3. Otherwise tags the serving image `pgathondu/server:previous` and runs
   **`make up`**: envctl's preflight on `deploy/<role>/`, then
   `docker compose up -d`. A preflight that refuses replaces nothing.
4. **`file_host` migrates as it starts** (`apps/servers/file_host/src/migrate.rs`):
   with nothing pending it does nothing; otherwise it copies the database
   (`VACUUM INTO ${STORAGE}/backups/hopium-<UTC>-before-<version>.db`),
   checks the copy (`PRAGMA integrity_check`, the same applied migrations),
   applies every pending migration, and keeps its newest 5 backups. So a
   backup is taken only by a start that has a migration pending, and each is
   a whole copy of the database less its free pages: the five take up to
   five times `sqlite_db_bytes - sqlite_freelist_bytes`. Removing the sixth is a discard
   site: a deleted account's rows live in the copies until then
   (`docs/identity.md`, invariant 9). It refuses a missing or
   never-migrated database file, which is what an unmounted volume looks
   like; a new host starts it once with `--init-database`. Any refusal or
   failed migration ends the process, so no build serves on a schema half
   applied. The same happens on every start: a reboot, a restart, `make up`,
   `make dev`.
5. **It waits up to 360 s** for `file-host`'s healthcheck to say `healthy`
   (`file_host --health-check`: `/ready` ready or degraded). A container
   that exits or restarts fails at once. The healthcheck's `start_period`
   is 300 s, so the backup and migrations of a large database can finish
   before any failed probe counts, and the wait outlasts it.
6. **A new image that does not come up is rolled back**: the previous image
   is tagged back as `pgathondu/server:latest`, `file-host` alone is
   recreated from it, it waits the same bound, and the failed image's id is
   recorded (step 2). It prints the container's last log lines.

It prints each step as `deploy: …` and exits `0` deployed or nothing to do,
`1` not deployed (the pull, the preflight or `up` failed; the old container
serves), `2` rolled back, or `3` nothing ready, which needs you: the
previous image failed too, there was none, or the image did not change (so
the cause is `deploy/<role>/` or a dependency).

**Needs a newer checkout.** The compose files and `deploy/<role>/` are the
checkout's, and the deploy does not pull it (the owner's call, 2026-11-10).
An image that needs a variable or compose change the checkout lacks fails
its preflight or its start, and is put back; the deploy then says "this
image needs a newer checkout on home". `git pull` in the checkout and run
`make home-deploy` again. An image that failed is skipped by later runs
until a newer one is published, so after a `git pull` that fixes it,
remove its id from `failed-images.json` (or wait for the next publish).

**The database is never restored automatically** (owner, 2026-11-10). A
migration that ran stays applied, and the previous image serves the
database as it left it: a database ahead of its binary is one it serves
(`apps/servers/file_host/src/schema.rs`). A backup is taken before the new
`file_host` starts serving, and its writes land after it, so a restore
would lose them; the deploy cannot show nothing was written since. To
restore by hand: stop `file-host` (`make compose ARGS="stop file-host"`),
move the live `${STORAGE}/hopium.db` aside under `${STORAGE}/backups/` with
a name of its own (never delete it, and never a `-before-` name, which the
prune would count among its own), copy the backup to
`${STORAGE}/hopium.db`, and `make up`. When a run ends with exit 3 it names
the backup that start took.

This is not a zero-downtime deploy: the old container stops before the new
one proves itself. Two instances behind Caddy is #409 (deferred) and AS20
(#566).

**On demand, with nobody typing it.** Home stays on the image it runs until
a client needs a newer one; nothing polls, and with no client newer than
home, home sends no request anywhere (the owner's ask, 2026-11-10):

1. **The contract epoch.** `apps/servers/file_host/src/contract.rs` holds
   `CONTRACT_EPOCH`, an ordered number that rises with every route change
   (its test and Lint's `Contract epoch` job hold it to the route surface),
   and `OLDEST_SUPPORTED`, raised when a change breaks older clients. The
   route snapshot carries both (`contract_epoch`, `oldest_supported`; the
   `.ts` module's `SERVER_CONTRACT_EPOCH`, `SERVER_OLDEST_SUPPORTED`), so
   paulgsc/some-ui's builds get them through the snapshot sync that already
   runs; nothing new runs in CI.
2. **The ask.** A client asks `GET /compat?needs=<its epoch>`, unversioned
   like `/health`. The answer is `{running: {epoch, oldest_supported,
   version}, needs, supported, deploy, latest_seen_epoch?, retry_after_s?}`,
   where `supported` is `oldest_supported <= needs <= epoch` and `deploy` is
   one of: `none` (served), `client_behind` (the client is older than home
   serves: it must update; pulling cannot help), `wanted` (a deploy was asked
   for now), `in_flight`, `cooldown` and `unavailable` (ask again after
   `retry_after_s`), `refused` (`needs` past the ceiling) or `cannot_request`
   (no `${STORAGE}/deploy`, as under `make dev`).
3. **The trigger.** For `wanted`, `file_host` writes `${STORAGE}/deploy/wanted`.
   A systemd path unit (`nixosModules.home-deploy`) starts
   `envctl deploy --on-demand ${STORAGE}/deploy` (built as the checkout's
   user first, `make envctl`): it pulls
   `pgathondu/server:latest`, reads the pulled image's epoch
   (`file_host --contract-epoch`), and deploys it, as above, only when that
   epoch is above the running one; a newer build of the same contract is not
   a reason to restart, and `:latest` goes back to what serves. It records
   what it did in `${STORAGE}/deploy/state.json`, which `file_host` reads for
   its answers and its metrics, and removes the marker.
4. **The bounds** (`apps/servers/file_host/src/deploy.rs`): one marker at a
   time, and one deploy at a time; 10 minutes at least between two pulls; a
   need the last pull found nothing to serve is answered `unavailable` for 6
   hours, so a client nothing published can serve costs at most 4 requests
   to Docker Hub in 6 hours; `needs` more than 1000 above the running epoch
   is refused, so a forged or mistyped one starts no pull at all; and no
   epoch at or below the running one is ever a reason to pull.
5. **Loud.** `DeployFailed` (a deploy rolled back or failed in the last
   hour) and `DeployStuck` (a marker 30 minutes unanswered) reach the owner
   like home's other alerts (`infra/prometheus/rules/home.yml`). The STORAGE
   dashboard's Deploy row draws the pattern's footprint: asks by answer,
   runs by outcome, pulls per 6 hours against Docker Hub's limit and their
   projection, bytes pulled, and the backups against the disk.

**Once, to install it (paulgsc/dotfiles).** With this flake as an input
named `server` (`inputs.server.url = "github:paulgsc/server";`, if
`dotfiles` does not have it yet), add to home's NixOS configuration:

```nix
imports = [inputs.server.nixosModules.home-deploy];
services.home-deploy = { enable = true; user = "<who runs make up>"; checkout = "<the checkout's path>"; storage = "<STORAGE>"; };
```

then `nixos-rebuild switch`. The unit runs as root (`${STORAGE}` is
file_host's uid's) and runs `make` as `user` (who holds the sops key);
`systemctl status home-deploy.path home-deploy` and
`journalctl -u home-deploy` show it.

### NixOS modules

The flake exports each binary as a NixOS service, `nixosModules.file-host` and
`nixosModules.orchestrator` (#408), run as an OCI container. Their options are
generated from the same schema (`envctl nix` writes
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
  `grep -rn HOST-SPECIFIC infra/ apps/ crates/` lists every value
  a new host (a VPS, say) must replace.
* **Required where no default is right.** `ALLOWED_ORIGINS`, `APP_BASE_URL`,
  `NATS_URL` and `REDIS_URL` have no default (#406, #617): a host sets its
  own, and `make preflight` names them if it does not. The dev host's values
  are its role's, in `deploy/dev/`, not defaults in the code. The compose files' host paths (`STORAGE`,
  `TRANSCRIBE_MODELS_DIR`, `METABASE_DATA_DIR`, `DATABASES_DIR`,
  `OLLAMA_MODELS_PATH`), Grafana's and Metabase's addresses and Grafana's
  admin password have no default either: each role's `compose.sops.env`
  sets them (#614).
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
(`--port-fallback true`, a flag and never a variable, so no deployment file
can set it); run any other way, a taken port is a startup failure, as
production needs. It records the port it got (`--port-file`) in
`$XDG_RUNTIME_DIR/file_host/dev-port.json` (`/tmp/...` without one), and
`paulgsc/some-ui`'s `vite dev` proxies there while it runs and back to the
container when it stops, reloading the page either way. The listen address
itself is `listen::BIND`/`listen::PORT` (default `0.0.0.0:3000`). `make dev`
runs it on `deploy/dev/file_host.sops.env`, decrypted into its environment
alone (`envctl run`), a variable the shell already sets winning
(`NUDGE_ENABLED=false make dev`). Write that file once with
`sops edit deploy/dev/file_host.sops.env`; file_host itself names every
variable it lacks or refuses at start. (The preflight checks a container's
view, with compose's own wiring, so it does not cover `make dev`.) Pointed at the container's database and VAPID keys, it
would run a second nudge waker beside the container's, so give dev its own
(`DATABASE_URL=sqlite://<checkout>/dev.db`, migrated as above). It names
its NATS and Redis too, which have no default since #617: beside the
compose stack, `NATS_URL=nats://localhost:4222` and
`REDIS_URL=redis://127.0.0.1:6379`, the ports the stack publishes.

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

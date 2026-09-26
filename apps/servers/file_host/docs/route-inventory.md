# Route inventory

`file_host` publishes a machine-readable description of its own HTTP surface.
The client repo (`paulgsc/some-ui`) consumes it to check that the requests it
makes still target routes that exist.

## Why

`axum::Router` is write-only. Once routes are registered there is no way to ask
it what it accepts, so the boundary this server exposes is invisible to
everything outside the process — including the client that has to agree with it.
Rename a path and the only thing that notices is a 404 in somebody's browser,
days later, on the other side of the boundary from the change that caused it.

Making the surface an artifact turns that into a diff.

## Pieces

| Path | Role |
| --- | --- |
| `src/routes/table.rs` | `RouteTable`: registers a route with axum and records it in one call; `Module`: a table plus its name, nesting and CORS |
| `src/routes/inventory.rs` | `modules()`, the one list of route modules `main.rs` serves; `snapshot()` reads the same list |
| `src/routes/ts_emitter.rs` | Renders the same inventory as a `.ts` module |
| `src/bin/dump_routes.rs` | Emits JSON (default) or TypeScript (`--ts`) on stdout |
| `.github/workflows/routes.yml` | Checks every PR's snapshot against the client, and opens the sync PR there on merge |
| `make routes` | Writes `routes.server.json` and `routes.server.ts` locally |
| `make routes-check` | Runs the inventory's own tests |

## Getting the snapshot to the client

CI does it; nothing is copied by hand. `.github/workflows/routes.yml`:

- **On every PR** that touches `file_host`, builds the snapshot and runs the
  client's `scripts/sync-server-routes.sh --verify` against the client's
  `main`. That runs `contract:drift` and the `server-routes` consistency test,
  so a rename that leaves a client contract pointing at nothing fails on the
  server PR that did it. The run summary shows the `ServerRoute` diff. If the
  break is intended, label the PR `client-breaking-route`: the check still
  reports but no longer fails, and the client catches up through the sync PR.
- **On every merge to `main`**, opens (or updates) one PR in `paulgsc/some-ui`
  on the `bot/server-route-snapshot` branch carrying both files. That PR runs
  the client's own CI, contract drift included. This needs the
  `SOME_UI_SYNC_TOKEN` secret (a fine-grained token on `paulgsc/some-ui` with
  Contents and Pull requests read/write); without it the job warns and does
  nothing.

The client owns where the files go (`scripts/sync-server-routes.sh`), so
moving them there is a change to that repo alone.

To look at the snapshot locally:

```sh
# sqlx checks queries at compile time, so building needs a migrated database
DATABASE_URL="sqlite://$PWD/dev.db" make routes
```

## Two renderings, one snapshot

`dump-routes` calls `inventory::snapshot()` exactly once per run and either
`serde_json`-serialises the result or hands it to `ts_emitter::render_ts`.
There is no second list of routes anywhere in the crate for the two outputs
to disagree about — a route missing from one is a route no module
registered, not a bug in one emitter.

The `.ts` module exports:

- `API_BASE_PATH` — the same `/api/v1` prefix as the JSON's `api_base_path`.
- `ServerRoute` — a union of every versioned route's full path.
- `UnversionedRoute` — the same for `/health`, `/ready`, `/ws`, and anything
  else this file marks `versioned: false`.

Parameter names (`:id`, `:tab_id`, ...) are not extracted into a separate
shape; the client derives them from the literal union with a
template-literal type (#266's own scope note), which keeps the generated
file a flat list of strings — reviewable without knowing what a
template-literal type is.

No `INVENTORY_SCHEMA_VERSION` bump accompanies this addition. See the doc
comment on that constant for why a second artefact is not a shape change to
the first one.

## Recorded, not declared

Until this change the inventory was a hand-written `ROUTES` list beside the
routers, kept honest by a test that parsed the `routes/*.rs` sources with a
regex. Every route was written twice, and the check had blind spots of its
own (a module missing from the test's file list, two identical calls
collapsing in a set).

Now a route is written once. Each route module builds a `RouteTable`:

```rust
let table = RouteTable::new()
	.get("/tabs", routes::get_all_tabs)
	.post("/tabs", routes::upsert_tab);

Module::versioned("tabs", table).with_cors(cors)
```

`.get(...)` hands the handler to axum and records `(GET, "/tabs")` in the same
call. `inventory::modules()` lists every module. `main.rs` builds its router
from that list (`inventory::routers`) and `dump-routes` reads the records from
the same list, so a route cannot be served but absent from the snapshot, or
the other way round. Whether a route sits under `/api/v1` comes from the
module (`Module::versioned` / `Module::unversioned`), which is also what
`main.rs` nests by.

Building the tables needs neither `AppState` nor `Config`: handlers are
registered, never called, and CORS, the only thing that reads `Config`, is a
function applied only when `main.rs` asks for the router. So `dump-routes`
still runs without a provisioned environment.

Two guards close the ways around it:

- `clippy.toml` disallows `axum::routing::Router::route`. A route registered
  anywhere except `RouteTable` is a lint error. The exceptions carry an
  `#[allow]` saying why: the recorder itself, `/metrics` (a scrape target
  built in `some_metrics`, outside the client-facing inventory), and
  throwaway test routers.
- `Module` accepts a table and a CORS layer, nothing else, so a module has no
  place to add a route that its table does not see.

The inventory's own tests cover what the type system does not:
`no_duplicate_method_path_pairs` and `the_surface_assembles_like_main_does`
(axum panics at boot on a duplicate or conflicting registration across
modules; better as a test failure), `module_names_are_unique`,
`every_module_registers_a_route`, `versioned_routes_carry_the_api_prefix`
and `snapshot_is_deterministic`.

## Adding a route

1. Register it on the relevant module's `RouteTable` in `src/routes/*.rs`.
2. That's all. The snapshot picks it up, and CI shows the client diff on your
   PR.

A whole new route module needs one line in `inventory::modules()`. Forget it
and the module is not served at all, which is loud, rather than served and
missing from the snapshot, which would not be.

## Schema versioning

`INVENTORY_SCHEMA_VERSION` is emitted in the JSON. Bump it when the shape
changes in a way consumers must react to — the client harness refuses a snapshot
version it does not recognise rather than misreading one.

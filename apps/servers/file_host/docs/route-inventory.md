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
| `src/routes/inventory.rs` | Declares the surface; tests prove the declaration matches the routers |
| `src/bin/dump_routes.rs` | Emits it as JSON on stdout |
| `make routes` | Writes `routes.server.json` |
| `make routes-check` | Runs the parity tests alone |

## Usage

```sh
# sqlx checks queries at compile time, so building needs a migrated database
DATABASE_URL="sqlite://$PWD/dev.db" make routes
```

Then copy `routes.server.json` into the client repo at
`packages/contract-harness/routes.server.json` and run `pnpm contract:drift`
there. The diff on that file is the reviewable record of what moved.

## The declaration is checked, not trusted

A hand-maintained list of routes rots exactly as fast as the thing it describes,
so `ROUTES` is not taken on faith.

`inventory_matches_route_sources` parses the sibling `routes/*.rs` sources at
compile time (via `include_str!`) and asserts set equality with the declaration,
in both directions:

- a `.route(...)` call that nobody declared fails the test — otherwise the
  client harness would be blind to that route;
- a declared route with no matching call also fails — otherwise the harness
  tests something that no longer exists and reports its 404 as a client bug.

Three further tests guard the checker's own blind spots: `module_coverage`
(a route module missing from `SOURCES` would contribute zero routes and pass
silently), `every_source_contributes_routes` (a stale `include_str!` path that
still compiles), and `no_duplicate_method_path_pairs` (axum panics at startup on
a duplicate registration — better as a test failure than a boot crash).

The check is source-level on purpose. Verifying against a live `Router` would
need a fully built `AppState` — SQLite, NATS, Google clients — which would make
the cheapest invariant in the crate the most expensive one to run.

## Adding a route

1. Register it in the relevant `src/routes/*.rs` as usual.
2. Add the matching `RouteDescriptor` to `ROUTES` in `src/routes/inventory.rs`.
   The test tells you if you forget, and tells you exactly which route.
3. Regenerate the snapshot and copy it to the client repo.

A whole new route module also needs an entry in the `SOURCES` table inside the
test module, otherwise its routes are unaudited.

## Schema versioning

`INVENTORY_SCHEMA_VERSION` is emitted in the JSON. Bump it when the shape
changes in a way consumers must react to — the client harness refuses a snapshot
version it does not recognise rather than misreading one.

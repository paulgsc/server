# LeetType execution: recorded runs, one runner, nothing executed in the request path

Design note for paulgsc/server#381 (LTY-EXEC, the server half of
paulgsc/some-ui#1226, X5) and paulgsc/server#328 (LTY-SRV4, the static
snapshot). It sits beside the route's module
(`src/routes/db/leetype.rs`), as #381's first acceptance criterion asks.

## The decision

A round's last artifact, `r`, is an execution result: the client's
`RunResult`. This server produces it by **recording runs offline with one
runner, and serving the recordings**. Nothing is compiled or executed while a
request is being answered.

| Piece | What it is |
| --- | --- |
| `crates/leetype_runner` | The runner. Builds `A` and every `A + d` from a stored round body, appends the round's `harness`, compiles each with `rustc`, runs each binary at the bounds of `constraintDiff.before` and of `constraintDiff.after`, and returns one `RunResult` per run. |
| `record-leetype-runs` (`src/bin/record_leetype_runs.rs`) | The only caller of the runner. An offline operator command, run after `import-leetype-rounds` on a machine with `rustc` (a developer's, or CI). Stores each round's transcript for the round version it ran. |
| `leetype_round_run` (`migrations/20260929000300_create_leetype_round_run.up.sql`) | One row per `(round, variant, bounds)`: the sizes and the `RunResult`, tagged with the round's `content_hash` at recording time. |
| `GET /api/v1/leetype/rounds/:id/runs` | The route. Reads the rows recorded for the round's **current** hash. |
| `dump-leetype-snapshot` (`src/bin/dump_leetype_snapshot.rs`) | #328. Writes the listed rounds and their runs as static files for the GitHub Pages build. |

The wire shape is `leetype_round_repo::RoundRuns`:

```json
{
  "roundId": "has-duplicate-sort-adjacent",
  "contentHash": "92f987e6…",
  "runs": [
    { "variant": "A",  "bounds": "before", "sizes": { "n": 1000 },
      "result": { "kind": "ok", "inputSize": 1000,
                  "observation": { "output": "false", "logs": [], "elapsed": { "milliseconds": 15 } } } },
    { "variant": "A",  "bounds": "after",  "sizes": { "n": 100000 },
      "result": { "kind": "error", "inputSize": 100000,
                  "error": { "errorClass": "budget-exceeded",
                             "message": "the program did not finish within the 2000 ms wall-clock ceiling, and was killed" } } },
    { "variant": "d0", "bounds": "before", "…": "…" }
  ]
}
```

`result` is the client's `RunResult` exactly (`lib/leetype/run-result` in
`paulgsc/some-ui`). `inputSize` is the bound of the constraint set's **first**
dimension. `sizes` carries every dimension, since a two-dimension round
(`n`, `q`) is not described by one number. Runs are ordered `A` first, then
`d0`, `d1`, …, and each variant's `before` run comes before its `after` run.
A round the table does not hold is a JSON `404`. A known round with nothing
recorded for its current version answers `runs: []`. The `ETag` is the hash of
the serialised body, and `If-None-Match` gets a `304`.

## Reinterpreting "the route executes a round"

#381's second criterion reads "the route executes a round from #325's table
by id". **This implementation changes what that means, on purpose.** The route
answers, by round id, with **the runner's result for that round**. The
runner ran offline, before the request. Three reasons:

1. **No sandbox in the request path.** The other reading needs `rustc`, a
   linker and a process sandbox inside the production container. It also
   puts the compilation and execution of programs (reviewed ones, but still
   programs) on the path that answers the public internet. That buys
   nothing a recording doesn't: the programs are fixed corpus artifacts, so
   every run the route could do on request is known before anyone asks.
2. **One runner for the live build and the static one.** #381 itself
   suggests that #328's generator should call "the **same runner** this route
   uses, offline". Taken seriously, that means one runner and one set of
   recordings behind both builds. A live answer and a static one are then the
   same bytes, `RoundRuns` from the same rows, rather than two code paths
   that agree only by review.
3. **Thm. 4.1.** No finite set of runtime observations entails a complexity
   class. A recorded run and a live one are equally unable to establish a
   claim, so nothing pedagogical is lost by serving a recording. Prop. 4.1
   allows a run exactly one job: establishing a concrete fact about one input.
   A recording does that job equally well.

The **error classes keep their meaning.** `compile`, `runtime` and
`budget-exceeded` are what the program did when it was recorded. A
`budget-exceeded` in a response is a recorded ceiling hit, not a transport
error, which is how #381 wants it treated.

## Technology: `rustc` in a subprocess, offline

The runner shells out to the machine's `rustc` through `std::process`. It
adds no third-party dependency (`tempfile` was already in the lockfile) and
uses no `unsafe`. Rust is the only language (decided 2026-09-26 on #381,
enforced by `RoundSchema`), so one toolchain covers the whole corpus.

### Rejected options

- **A live `rustc` subprocess sandbox per request.** This is the obvious
  reading of #381. It needs the toolchain (about 1 GB) and a sandbox (seccomp,
  namespaces or cgroups) in the production image, which is
  `rust:…-slim-bookworm` for building and a runtime stage that ships one
  binary. It puts compilation (seconds) and execution (up to the ceiling)
  inside a request. Under the global rate limiter, a handful of clients could
  then hold the CPU for the ceiling's length each. And every answer it could
  give is fixed by the corpus anyway.
- **Wasmtime with fuel.** A large dependency tree for a server that otherwise
  has none like it. Publishing would need a `wasm32` target and toolchain,
  since every `A + d` would be compiled to wasm ahead of time. The most
  tempting reason for it, fuel as a deterministic operation count, does not
  hold up: fuel counts wasm instructions, which are not the canon's operations
  either (Def. 1.3's budget `B` counts abstract operations; Ax. 3.1 makes the
  budget an order-of-magnitude heuristic). A deterministic number that looks
  like the canon's count and isn't would read as authoritative, which is the
  kind of field never #3 forbids.
- **nsjail or bubblewrap.** A host dependency, and one that isn't available
  in every place the recorder runs (developer machines, CI runners, this
  sandbox). They solve isolation from untrusted code. Here there is none: the
  only programs are reviewed corpus artifacts, and the only machine that runs
  them is one somebody chose to run the recorder on.

## The runner

For a round body with a `harness`:

1. **Programs.** `A` is `algorithm.source`. `A + d` applies
   `diffOptions[i].member.hunk` exactly as the client's `applyHunk`
   (`lib/leetype/round-assembly`) does. The hunk's context and deletion text,
   concatenated, must match verbatim at the start of line `oldStart` (from 1).
   It is replaced by the context and addition text, and `newStart` must equal
   `oldStart`. A hunk that does not apply is a structured error naming its
   variant (`variant d1 does not apply: …`), and the round is refused before
   anything compiles. Each program is the variant's code, a newline, and the
   harness. That is the same text the client's
   `check-round-programs-compile.ts` compiles in CI.
2. **Compile.** Each program is compiled once, with
   `rustc --edition 2021 -C opt-level=0 -A warnings --crate-type bin -o <bin> <file>`,
   in a fresh temporary directory per round (removed afterwards).
3. **Run.** Each binary runs twice, with one `dimension=bound` argument per
   constraint: first for `constraintDiff.before`, then for
   `constraintDiff.after`. The bound is the argument because admissibility is
   a worst-case relation (Def. 3.1), and the harness builds the worst-case
   input of that size.

### Why `opt-level=0`

Measured on 2026-09-29, on the fixture corpus (5 rounds, 18 variants), with
each variant run at `C′` (`constraintDiff.after`), n = q = m = 100,000:

| | `opt-level=0` | `opt-level=3` |
| --- | --- | --- |
| Admissible member (5) | 4–25 ms | 3–5 ms |
| `A` and distractors (13) | all over 20 s | 812 ms – over 20 s; **4 of 13 under the 2 s ceiling** (812, 1335, 1384, 1662 ms), one more at 2125 ms |

At `-O`, LLVM vectorises or strength-reduces some quadratic loops enough to
finish under the ceiling at `C′`. For example, `count-present-sorted-lookup`'s
`A` finishes in 812 ms, and `range-sums-prefix`'s `A`, `d1` and `d2` in
1.3–1.7 s. That would make a recording say that an inadmissible program
"finished" at `C′`. At `opt-level=0` every inadmissible variant is past the
ceiling by an order of magnitude, and every admissible one finishes in tens of
milliseconds. There is a 100× gap on both sides of 2 s.

The run is **of the program as written**, not of what an optimiser made of
it. That is the object the canon reasons about, and the object the learner
reads. Debug assertions and overflow checks are on at `opt-level=0`, so an
arithmetic overflow is a `runtime` error. That is also the program as
written, in Rust's own debug semantics.

In the recording above (`record-leetype-runs` on this corpus, 2 s ceiling,
default limits), every `A` and every distractor is `budget-exceeded` at
`after`. Every admissible member is `ok` at both bounds, and each prints what
`A` printed at `before`. The recording takes about 30 s, most of it the 13
ceiling hits.

### Limits

All are in `leetype_runner::Limits`, with defaults, and each is a flag on
`record-leetype-runs`.

| Limit | Default | At the limit |
| --- | --- | --- |
| Compile timeout | 60 s | `compile`, "rustc did not finish within 60 s, and was killed" |
| Run wall-clock ceiling | 2000 ms | `budget-exceeded`; the process group is sent `SIGKILL` and reaped |
| stdout | 4 KiB | kept to the ceiling, the rest drained and dropped, never streamed; `logs` gets "stdout truncated: kept the first 4096 of N bytes" |
| stderr | 4 KiB | the same, in `logs`. A `runtime` error's message is its exit status plus stderr's **tail**. A `compile` error's message is rustc's stderr **head**, where the first error is. |
| Address space | 512 MiB | applied through `prlimit --as` when `prlimit` is installed. An allocation past it aborts, which is `runtime`. |
| Environment | cleared | a run gets no environment at all. The compile gets only `PATH`, `HOME`, `RUSTUP_HOME`, `RUSTUP_TOOLCHAIN` and `CARGO_HOME` (what rustup's proxy and the linker need), plus `TMPDIR` set to the round's directory. |
| Working directory | fresh per round | a `tempfile` directory, removed when the round is done |

Other outcomes map as follows: a non-zero exit or a signal is `runtime`, with
the status and stderr's tail. A compile failure is `compile`, with rustc's
stderr. `rustc` missing or unstartable is **not** a result: the recorder stops
with exit code 2, since no round would do better.

The child runs in its own process group (`CommandExt::process_group(0)`), so
the kill takes a compile's linker along with `rustc`. It is polled with
`try_wait` every millisecond, so `elapsed` is good to about a millisecond.
Timings are illustrative only (Ax. 3.1). They come from the machine that
recorded them, and nothing grades them.

### Known gaps

- **Memory.** Without `prlimit` (util-linux), runs have no memory ceiling.
  The recorder prints a note when that happens. Getting `setrlimit` without
  `prlimit` would need `unsafe` in a `pre_exec` hook, which this workspace
  avoids. The corpus's harnesses allocate a few MB at most.
- **Network and filesystem.** A run is not in a network or mount namespace.
  It runs as whoever runs the recorder, in an empty directory, with an empty
  environment. That is acceptable only because the programs are reviewed
  corpus artifacts and the recorder is an operator's choice. Learner-authored
  code stays out of scope (#381, paulgsc/some-ui#1201). If it ever came in
  scope, this whole design would need revisiting, starting with this bullet.
- **Timing noise.** `elapsed` varies between recordings. That is harmless:
  the snapshot is a function of the database, not of a new recording (see
  below), and the budget verdicts sit two orders of magnitude from the
  ceiling.

## The four nevers, and the two that are this repo's

| # | Never | How it holds here | Checked by |
| --- | --- | --- | --- |
| 1 | executes caller-supplied source | The route's extractors are `State`, `HeaderMap` (read for `If-None-Match` only) and `Path<String>`. It has no `Json`, `Form`, `Query` or body, and is registered for `GET` alone. The runner only ever reads a stored body, which came in through the reviewed import path or the operator's route. The route executes nothing anyway. | `the_runs_route_takes_no_input_but_the_round_id`: `POST`/`PUT`/`PATCH`/`DELETE` with a `source` body are `405`, and a `GET` with `?source=` or a body answers the same bytes |
| 2 | selects a round | The route answers for the id it is asked about. The recorder records every listed round (or `--round <id>`), and neither decides what a learner sees next. | By construction: nothing reads the runs table but the route and the dump |
| 3 | returns a complexity claim | `RoundRuns` holds the round's id and hash, and per run the variant, bounds, sizes and `RunResult`. There is no class, no Θ, no "admissible" and no verdict field. The messages state what happened ("did not finish within the 2000 ms wall-clock ceiling"), never why. | `a_runs_response_carries_no_complexity_claim`: every key of a sample response covering both branches is on an explicit allowlist |
| 4 | becomes required | A round with nothing recorded answers `runs: []`, and an unknown one a `404`. The static build gets recordings (`dump-leetype-snapshot`) and needs no server. The client stays playable when every run is missing or failed (paulgsc/some-ui#1226's own criterion). | The client's side |
| 5 | produces a `StudySignal` | `leetype_runner`'s dependency closure contains none of `study_domain`, `intervention`, `outcome_repo`, `session_repo` or `engagement_repo`, so it cannot even name the type. The route, its module and both commands name none of them either. | `crates/leetype_runner/tests/nevers.rs` (walks `cargo metadata`'s resolved graph) and `the_runs_surface_reaches_neither_a_study_signal_nor_the_runner` (scans the sources) |
| 6 | breaks `docs/identity.md`'s privacy invariants | `leetype_round_run` holds nothing about who asked. Runs are recorded offline, not per request. It is classified `NOT_SUBJECT_SCOPED` in `privacy.rs`. Rate limiting is the global middleware on every versioned route, keyed by `net.rs`'s `PeerKey`, not a second mechanism. | `privacy.rs`'s schema test |
| — | **no execution in the request path** (this design's own) | `leetype_runner` is a dependency of `file_host` only for the `record-leetype-runs` binary. No other file under `src/` names it, so the server binary never links it. | `the_runs_surface_reaches_neither_a_study_signal_nor_the_runner` |
| — | **a stale run is never served** (this design's own) | Each row carries the `content_hash` it was recorded for. The route and the dump read only rows matching the round's current hash, in one query. `replace_runs` refuses (writes nothing) if the hash moved while recording. | `stale_runs_are_never_served_or_written`, `the_runs_route_serves_the_current_versions_transcript` |

### Invariants this design relies on

Declared per this repo's `CLAUDE.md` ("Drift is loud, not silent"). A reviewer
settles each one with a single check against a hunk.

- **EX1: nothing the server serves compiles or runs a program.**
  - _Claim:_ under `apps/servers/file_host/src/`, only
    `bin/record_leetype_runs.rs` names `leetype_runner`, and no handler spawns
    a process.
  - _Falsified by_ a hunk that adds a `leetype_runner` path, a
    `std::process::Command`, or a `tokio::process` call to any other file
    under `src/`, or that deletes or weakens the source-scan test.
  - _Why not enforced mechanically:_ the `leetype_runner` half is enforced
    (the test above). The `Command` half is "mechanical; not yet a rule":
    other binaries in `src/bin/` could legitimately spawn processes, and no
    handler does today.
- **EX2: runs are keyed to the bytes they ran.**
  - _Claim:_ every read of `leetype_round_run` that serves runs filters on
    the round's current `content_hash`, and every write goes through
    `replace_runs`. One reader is exempt by name: `RoundRepository::has_runs`
    answers "is this exact hash recorded?" for the recorder, which passes the
    hash it just read; it serves nothing.
  - _Falsified by_ a hunk that queries `leetype_round_run` without joining or
    filtering on `leetype_round.content_hash` (other than `has_runs`), that
    serves `has_runs`' answer or a row it selects to a client, or that
    inserts into it outside `RoundRepository::replace_runs`.
  - _Why not enforced mechanically:_ SQL text is not linted. The tests cover
    the two readers that exist, not ones added later.

## #328: the static snapshot

`dump-leetype-snapshot <out-dir>` writes:

```text
<out>/rounds/manifest.json   {"rounds": [<id>, …]}   listed rounds, by id
<out>/rounds/<id>.json       the stored body, byte for byte
<out>/runs/<id>.json         GET /leetype/rounds/:id/runs's body (RoundRuns)
```

It uses the export's layout (`packages/ui/leetype/corpus/rounds/` in
`paulgsc/some-ui`, written by `export-round-corpus.ts`). JSON is written the
way the client's export writes it: two-space indent and a final newline.

**Deterministic.** Rounds are listed by id and runs in transcript order.
`recorded_at` is not part of the dump. A file whose bytes would not change is
not rewritten, and a file the snapshot no longer contains (a retired round's)
is removed. Regenerating from an unchanged database is therefore a
byte-identical no-op, and so is re-recording an unchanged round, as long as
its results come out the same. `a_dump_is_the_export_byte_for_byte_and_regenerating_it_is_a_no_op`
proves both, plus the round files being the fixture's bytes.
`--check` writes nothing and exits 1 if the directory differs. That is the CI
check #328's definition of done asks for, for whichever repo holds the
checked-in copy.

**How drift is caught.**

- **Rounds.** A snapshot round file is the stored body, and the stored body
  is the export's file byte for byte (`round_corpus_parity.rs` proves the
  import direction). So `rounds/<id>.json` must equal `corpus/rounds/<id>.json`
  in `paulgsc/some-ui`, which that repo's `export-round-corpus.ts --check`
  already pins to its authored rounds. **One exception:** `manifest.json` is
  ordered by id, because the database keeps no authored order. The client's
  export lists rounds in `AUTHORED_ROUNDS` order. Compare manifests as sets,
  or have the export sort.
- **Runs.** They are keyed by content hash. `runs/<id>.json` names the
  `contentHash` it was recorded for, so a client can check it against the
  round file's own hash (SHA-256 of its bytes). A transcript left behind when
  a round changed is detectable on its face, and the server never produces
  one (EX2).

The transcripts are therefore "machine-produced, never hand-committed"
(Rem. 11.3). They come from the same runner and the same rows as the live
route, which is the relationship #381 proposed for #328.

**What #328 asked for that this does not do.** #328 was drafted against the
M20 step corpus and asked for generated `seed/*.ts` modules and
`dump-leetype-exercises`. The corpus is now rounds (#324's amendment), and the
client already consumes a JSON export layout. The snapshot emits that layout
instead of TypeScript. Wiring it into the Pages build, and the CI step that
runs `--check` against a checked-in copy, are `paulgsc/some-ui`'s side.

## Operating it

```sh
export DATABASE_URL=sqlite:///path/to/file_host.db
cargo run -q --bin import-leetype-rounds -- path/to/corpus/rounds
cargo run -q --bin record-leetype-runs            # rounds whose current version has no runs
cargo run -q --bin record-leetype-runs -- --all   # re-record everything
cargo run -q --bin dump-leetype-snapshot -- path/to/snapshot [--check]
```

`record-leetype-runs` exits 0 when every selected round was recorded, up to
date, or skipped for having no harness. It exits 1 when a round could not be
run (a hunk that does not apply) or changed while it was being recorded, and 2
when it could not start or had to stop (no database, no `rustc`, an unknown
`--round`).

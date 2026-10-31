# LeetType execution: recorded runs, one runner, nothing executed in the request path

Design note for paulgsc/server#381 (LTY-EXEC, the server half of
paulgsc/some-ui#1226, X5) and paulgsc/server#328 (LTY-SRV4, the static
snapshot). It sat beside the route's module (`src/routes/db/leetype.rs`), as
#381's first acceptance criterion asked.

> **The route is retired** (paulgsc/some-ui#2032, the owner's decision of
> 2026-10-30). Nothing on the phone has played a Def. 1.7 round since
> some-ui#1941, so `GET /leetype/rounds*` and the operator's
> `/leetype/operator/rounds*` were removed. The recordings stay: the runner,
> `record-leetype-runs`, `leetype_round_run` and `dump-leetype-snapshot` are
> unchanged, and no table or row was dropped. Where this note describes the route,
> it says how it was served. The tests that held the nevers and EX1 over the
> snapshot's `RoundRuns`, the source and the dependency graph moved to
> `tests/leetype_runs.rs`; the ones that exercised the route itself went with it.

## The decision

A round's last artifact, `r`, is an execution result: the client's
`RunResult`. This server produces it by **recording runs offline with one
runner, and serving the recordings**. Nothing is compiled or executed while a
request is being answered.

| Piece | What it is |
| --- | --- |
| `crates/leetype_runner` | The runner. Builds `A` and every `A + d` from a stored round body, appends the round's `harness`, compiles each with `rustc`, runs each binary at the bounds of `constraintDiff.before` and of `constraintDiff.after`, and returns one `RunResult` per run. The compile and the run themselves are `rust_runner`'s (below). |
| `crates/rust_runner` | The neutral runner both recorders call (paulgsc/some-ui#2234): compiles one program (`rustc -C opt-level=0 --error-format=json`, keeping each diagnostic's code and spans) and runs it under the limits below, killing its whole process group. Also Rust Rounds' `rust-runner` service ("Rust Rounds: the rust-runner", below). |
| `record-leetype-runs` (`src/bin/record_leetype_runs.rs`) | The only caller of the runner. An offline operator command, run after `import-leetype-rounds` on a machine with `rustc` (a developer's, or CI). Stores each round's transcript for the round version it ran. |
| `leetype_round_run` (`migrations/20260929000300_create_leetype_round_run.up.sql`) | One row per `(round, variant, bounds)`: the sizes and the `RunResult`, tagged with the round's `content_hash` at recording time. |
| `GET /api/v1/leetype/rounds/:id/runs` | The route, **retired** (some-ui#2032). Read the rows recorded for the round's **current** hash. |
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
No route serves it. `dump-leetype-snapshot` writes one `runs/<id>.json` per
listed round, read for the round's current hash; a round with nothing recorded
for its current version gets `runs: []` and is named in the dump's report.

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
adds no third-party dependency (`tempfile` was already in the lockfile;
`rustix`, for the process-group kill, already was too) and uses no `unsafe`. Rust is the only language (decided 2026-09-26 on #381,
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
   `rustc --edition 2021 -C opt-level=0 -A warnings --error-format=json --crate-type bin -o <bin> <file>`,
   in a fresh temporary directory per round (removed afterwards). A failed
   compile's message is the diagnostics' `rendered` text, which is byte for
   byte what rustc prints without `--error-format=json`
   (`the_rendered_text_is_what_rustc_prints_without_json`), so moving the
   compile into `rust_runner` changed no recorded message.
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

All are in `leetype_runner::Limits` (`rust_runner`'s), with defaults. Each
is a flag on `record-leetype-runs` but the compile's address space and the
file size, which keep their defaults.

| Limit | Default | At the limit |
| --- | --- | --- |
| Compile timeout | 60 s | `compile`, "rustc did not finish within 60 s, and was killed" |
| Run wall-clock ceiling | 2000 ms | `budget-exceeded`; the process group is sent `SIGKILL` and reaped |
| stdout | 4 KiB | kept to the ceiling, the rest drained and dropped, never streamed; `logs` gets "stdout truncated: kept the first 4096 of N bytes" |
| stderr | 4 KiB | the same, in `logs`. A `runtime` error's message is its exit status plus stderr's **tail**. A `compile` error's message is rustc's stderr **head**, where the first error is. |
| Address space | 512 MiB | applied through `prlimit --as` when `prlimit` is installed. An allocation past it aborts, which is `runtime`. |
| Compile address space | 768 MiB | rustc under `prlimit --as`, likewise. A source that makes rustc need more aborts it, which is `compile`, "rustc was killed by signal 6". |
| File size | 64 MiB | `prlimit --fsize` for the compile and the run; neither gets a POSIX message queue (`--msgqueue=0`) or a core. |
| Environment | cleared | a run gets no environment at all. The compile gets only `PATH`, `HOME`, `RUSTUP_HOME`, `RUSTUP_TOOLCHAIN` and `CARGO_HOME` (what rustup's proxy and the linker need), plus `TMPDIR` set to the round's directory. |
| Working directory | fresh per round | a `tempfile` directory, removed when the round is done |

Other outcomes map as follows: a non-zero exit or a signal is `runtime`, with
the status and stderr's tail. A compile failure is `compile`, with rustc's
stderr. `rustc` missing or unstartable is **not** a result: the recorder stops
with exit code 2, since no round would do better.

The child runs in its own process group (`CommandExt::process_group(0)`), so
the kill takes a compile's linker along with `rustc`. The kill is a system
call (`kill(-pgid)`, through `rustix`), not a `kill` process, so a program
that has used up its process ceiling cannot stop it. It is polled with
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
  code stays out of scope (#381, paulgsc/some-ui#1201).
  **Code nobody reviewed came in scope with Rust Rounds** (the owner's D1,
  paulgsc/some-ui#2234): programs a tutor wrote. They never run here. They
  run only in the `rust-runner` container, which closes these gaps for them
  (RX1, below). `record-leetype-runs` keeps these gaps, because what it runs
  is still the reviewed corpus.
- **Timing noise.** `elapsed` varies between recordings. That is harmless:
  the snapshot is a function of the database, not of a new recording (see
  below), and the budget verdicts sit two orders of magnitude from the
  ceiling.

## The four nevers, and the two that are this repo's

| # | Never | How it holds here | Checked by |
| --- | --- | --- | --- |
| 1 | executes caller-supplied source | No route serves rounds or runs, and no file the server serves names the runner, so no request reaches it. The runner only ever reads a stored body, which came in through `import-leetype-rounds`, the reviewed import path. | `the_runs_surface_reaches_neither_a_study_signal_nor_the_runner` (`tests/leetype_runs.rs`): no file under `src/` but `record-leetype-runs` names the runner |
| 2 | selects a round | The recorder records every listed round (or `--round <id>`), and the dump writes every listed round. Neither decides what a learner sees next. | By construction: nothing reads the runs table but the dump (and the recorder's `has_runs`) |
| 3 | returns a complexity claim | `RoundRuns` holds the round's id and hash, and per run the variant, bounds, sizes and `RunResult`. There is no class, no Θ, no "admissible" and no verdict field. The messages state what happened ("did not finish within the 2000 ms wall-clock ceiling"), never why. | `a_runs_transcript_carries_no_complexity_claim` (`tests/leetype_runs.rs`): every key of a sample `RoundRuns` covering both branches is on an explicit allowlist |
| 4 | becomes required | Nothing refuses a round for lacking runs: the dump writes a listed round with nothing recorded as `runs: []` and names it in its report. The static build gets recordings (`dump-leetype-snapshot`) and needs no server. The client stays playable when every run is missing or failed (paulgsc/some-ui#1226's own criterion). | `a_dump_is_the_export_byte_for_byte_and_regenerating_it_is_a_no_op` (`leetype_round_repo::snapshot`) for the dump; the client's side for the rest |
| 5 | produces a `StudySignal` | `leetype_runner`'s dependency closure contains none of `study_domain`, `intervention`, `outcome_repo`, `session_repo` or `engagement_repo`, so it cannot even name the type. Both commands name none of them either. | `crates/leetype_runner/tests/nevers.rs` (walks `cargo metadata`'s resolved graph) and `the_runs_surface_reaches_neither_a_study_signal_nor_the_runner` (`tests/leetype_runs.rs`, scans the sources) |
| 6 | breaks `docs/identity.md`'s privacy invariants | `leetype_round_run` holds nothing about who asked. Runs are recorded offline, not per request. It is classified `NOT_SUBJECT_SCOPED` in `privacy.rs`. No route reads it. | `privacy.rs`'s schema test |
| — | **no execution in the request path** (this design's own, EX1 below) | `leetype_runner` is a dependency of `file_host` only for the `record-leetype-runs` binary. No other file under `src/` names it, so the server binary never links it. No file names `rust_runner`, which reaches `file_host` only through `leetype_runner`, and only the files in `SPAWNS` start a process. | `the_runs_surface_reaches_neither_a_study_signal_nor_the_runner` and `the_runner_reaches_file_host_only_through_leetype_runner` (`tests/leetype_runs.rs`) |
| — | **a stale run is never served** (this design's own) | Each row carries the `content_hash` it was recorded for. The dump reads only rows matching the round's current hash, in one query. `replace_runs` refuses (writes nothing) if the hash moved while recording. | `stale_runs_are_never_served_or_written` |

### Invariants this design relies on

Declared per this repo's `CLAUDE.md` ("Drift is loud, not silent"). A reviewer
settles each one with a single check against a hunk.

- **EX1: nothing file_host serves compiles or runs a program in a request,
  and the server spawns no compiler; only the offline `record-leetype-runs`
  command does, and Rust Rounds' runner does, sandboxed.**
  (Amended for Rust Rounds, paulgsc/some-ui#2234, with the owner's D1. It
  read "nothing the server serves compiles or runs a program".)
  - _Claim:_ under `apps/servers/file_host/src/`, only
    `bin/record_leetype_runs.rs` names `leetype_runner`; no file names
    `rust_runner`, the crate that compiles and runs; `rust_runner` reaches
    file_host's dependency graph only through `leetype_runner`; and only the
    files listed in the test's `SPAWNS` start a process at all, each with
    its reason. Today that is `transcribe/engine.rs`, which runs `ffmpeg` (a
    fixed decoder of a stored note's audio, on the transcriber's blocking
    thread, never a program and never in a request). `record-leetype-runs`
    compiles the reviewed LeetType corpus through `leetype_runner`, on an
    operator's command, never in the server. Rust Rounds' programs are
    compiled and run by the `rust-runner` container, and file_host only
    asks it over its socket, off any request's path (`rust_rounds::pass`,
    RP1), through `rust_rounds_wire`, the runner's client half, which
    starts no process either.
  - _Falsified by_ a hunk that adds a `leetype_runner` path to any other
    file under `src/`, a `rust_runner` path to any file under `src/`, a
    dependency through which file_host reaches `rust_runner` other than
    `leetype_runner`, or a process start (`Command::new`,
    `std::process::Command`, `tokio::process`) to a file not in `SPAWNS` or
    to any file of `crates/rust_rounds_wire/src`; by
    an entry added to `SPAWNS` that this claim does not name; or by a hunk
    that deletes or weakens either test.
  - _Enforced mechanically_ by two tests in `tests/leetype_runs.rs`:
    `the_runs_surface_reaches_neither_a_study_signal_nor_the_runner` (the
    source scan, of `src/` and of `rust_rounds_wire`, and that this claim
    names every `SPAWNS` file) and
    `the_runner_reaches_file_host_only_through_leetype_runner` (`cargo
    metadata`'s resolved graph). They sat in the round handler's tests until
    that handler was retired (some-ui#2032); the test file is outside
    `src/`, so the scan never reads its own lists. Cargo's graph is per package, not per
    binary, so which binary links `leetype_runner` is the scan's to hold.
    The scan is lexical: it skips comments and everything after a file's
    first `#[cfg(test)]` (tests may start processes), and a process started
    through an alias the markers miss is not seen. The markers include an
    aliased `use` line (`use std::process::{Command as Run}` fails it).
  - _The drift this amendment fixed:_ the falsifier used to forbid any
    `std::process::Command` under `src/`, while `transcribe/engine.rs`
    already ran `ffmpeg` that way. The claim ("no handler spawns a
    process") held; the falsifier was wider than it, and was "not yet a
    rule" for that reason. It now matches the claim, within what a lexical
    scan sees, and is enforced.
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

## Rust Rounds: the rust-runner (paulgsc/some-ui#2234, RR3)

Rust Rounds' cards show what `rustc` really did with a toy program a tutor
wrote. **Every program is hostile**, whichever tutor wrote it (the Claude
routine, ChatGPT by paste, the API later) and however it arrived (the owner,
D1: "we always treat that code as unsanitized"). Nobody reviewed it, so none
of LeetType's known gaps is acceptable for it, and it never runs anywhere
but the `rust-runner` container.

| Piece | What it is |
| --- | --- |
| `crates/rust_runner` (`recording`) | One program, `main.rs`, in a directory of its own, compiled with `--edition 2024 -C opt-level=0 --error-format=json` (warnings kept, as the owner's own code builds) and, if it builds, run once with no arguments, stdin or environment. A `Recording` keeps the outcome, rustc's diagnostics (codes and spans) and text, the program's stdout (first 4 KiB) and stderr (last 4 KiB, where a panic is), and how it ended. |
| `RustcRun` (`Recording::rustc_run`) | What the phone reads: some-ui's `RustcRunSchema` (RR1, `packages/ui/rust-rounds`) field for field: `compile-error` (`code`, `stderr`, `span`), `prints`, `panics` (exit 101), `stopped` (`why`: `time`, `memory`, `output`, `exit`), each with the release (`1.97.0`) and, past a compile, `compileStderr`. Derived, never stored alone, so a change to it needs no re-recording. |
| `crates/rust_rounds_wire` | The half a client links: the protocol (`Request`, `Response`, `Refusal`, and `exchange`, the client, which tells a runner that cannot be reached from one that went away holding the job), what a `Recording` and a `RustcRun` hold, and `key`. It starts no process and compiles nothing; `rust_runner` depends on it and re-exports it, so the service and its client speak one definition, and file_host reaches the runner through it alone (EX1). |
| `rust-runner serve` (`src/bin/rust_runner.rs`, `service`) | The service: one JSON request line, one JSON answer line, one connection, over a Unix socket. `{"ask":"toolchain"}`, or `{"ask":"record","toolchain":…,"source":…}`, answered with the recording and its `RustcRun`. A `record` naming another toolchain is refused, and only the client's uid (file_host's, 10000) and root are served. It runs as root with two capabilities and starts every compile and run as 10101:10101; after each job, a helper kills that uid's processes and its hidden `clean` mode removes what the program left, both as 10101. |
| `infra/docker/Dockerfile.rust-runner` | `rust:1.97.0-slim-bookworm`, pinned by digest (rustc 1.97.0 (2d8144b78 2026-07-07), the owner's), plus the binary, and the programs' uid 10101; no setuid or setgid binary, and a toolchain only root can write. |
| `infra/compose/rust_runner.yml` | The sandbox (RX1). Not in `docker-compose.yml`'s stack until file_host asks it anything. |
| `scripts/check_rust_runner_sandbox.py` (`rust-runner.yml`) | Brings the compose file up as written and runs the worked example and the hostile programs against it. |

**Recording on arrival** (RR3's second slice; "Rust Rounds: recording on
arrival", below): the phone puts a round, home keeps it as an append-only
row and queues each distinct program, and a background pass in file_host
hands each to the runner over its socket and keeps how it ended. The runner
is in `docker-compose.yml`'s stack, its socket's volume mounted read-only
into file-host (`infra/compose/file_host.yml`); `RUST_ROUNDS_ENABLED` stays
off until home turns it on, and until then file_host asks it nothing. Not
yet done: the rounds, runs and task placed in some-ui's assurance ledger.

### Why a socket and no network, not "an internal network"

The design proposed an internal-only network. A program shares the
runner's network namespace, so on any network, internal or not, it could
reach everything else on it: file_host's routes, Redis, NATS. With
`network_mode: none` there is nothing to reach but loopback, and the only
way in is a Unix socket on a volume shared with whoever asks. It is
stricter than the proposal, and simpler: no port, no HTTP.

### The runner's invariants

Declared per this repo's `CLAUDE.md` ("Drift is loud, not silent"), in the
same shape as EX1 and EX2. Each holds of the code and configuration as they
are; "the runner" is `rust-runner serve`, "a program" is any source it is
asked to record, and "the programs' uid" is 10101 (and gid 10101).

- **RX1: every program runs only in the sandbox, as a uid that can touch
  nothing of the runner's, and nothing relaxes it.**
  - _Claim:_ Rust Rounds' programs are compiled and run only by
    `rust-runner serve` in the container `infra/compose/rust_runner.yml`
    declares: `network_mode: none`; `ipc: none` (no `/dev/shm`) with
    `kernel.shm_rmid_forced=1`; not privileged, no device, the pid and user
    namespaces Docker gives by default, no unconfined seccomp or AppArmor;
    one mount, the socket's volume; a read-only root with no setuid or
    setgid binary and a toolchain only root can write; the runner as root
    with only `SETUID` and `SETGID` (`cap_drop: ALL`), not dumpable and with
    no new privileges (its own `PR_SET_NO_NEW_PRIVS` and the container's),
    and the container's PID 1 (no init; `restart: unless-stopped`);
    `pids_limit: 128`; 1 GiB of memory and swap; one CPU; no core
    dumps. rustc and the program run as uid and gid 10101, a pair nothing
    else in the stack runs as (file_host is 10000:10001), with no
    supplementary groups and no capabilities, so they cannot signal, trace
    or read the runner, and under `prlimit`: 768 MiB of
    address space for rustc and 512 MiB for the program, 64 MiB per file,
    64 processes and threads for the uid, no core, no POSIX message queue.
    They can write only in the program's own directory under `/work`
    (root's, mode 0755); the socket's directory is root's, 0755, and the
    runner refuses a `--work` or socket directory that is not its own
    (root's, writable by nobody else). The socket (0666 in a 0755
    directory, on a volume file_host shares) can be reached by anything in
    either container, a program included, so **the peer's uid is the
    primary defence**: each connection's `SO_PEERCRED` uid is checked before
    anything is read, and only file_host's (10000) and root's are served.
    No source is exempt and no request
    field changes how a program is treated (the request type denies unknown
    fields). The container's environment holds only the names
    `check_sandbox` allows, so nothing secret, since a compile's
    `include_str!` can read what the image holds and a program its own
    environment (empty).
  - _Falsified by_ a hunk that loosens any of those settings, adds a
    capability (`KILL`, `DAC_OVERRIDE`, `FOWNER` and `CHOWN` included: the
    sweep and `clean` run as 10101 and need none), gives 10101 to anything
    else in the stack, serves another uid on the socket, starts a compile
    or run as the runner's uid, publishes a port, joins a
    network or an IPC namespace, passes an `env_file`, a secret or another
    variable, adds a request field or branch that skips a limit for some
    source ("trusted" tutors, say), or runs a Rust Rounds program anywhere
    else.
    **Who can signal whom.** No program can signal the runner: it is a
    different uid with no `CAP_KILL`, and as PID 1 the runner receives no
    signal it has no handler for.
    The sweep's helper (RX2) is a process of the programs' uid from its
    fork on, so a program's survivor *can* stop or kill it; that is why the
    helper is bounded and RX3's watchdog, a thread of the root runner that
    nothing of the programs' uid can reach, is the backstop.
  - _Scope:_ the `rust-runner` container and the crate's `serve` path.
    `record-leetype-runs` uses the same `Compiler` without a uid of its own
    or a container (the reviewed corpus, LeetType's known gaps above).
    **Known limit:** the container has no user namespace, so uid 10101 is
    the host's uid 10101, and `RLIMIT_NPROC` (64) counts every process of
    it on the host. `check_ids` holds the stack's own files to 10101 being
    the runner's alone; another tenant of the host running as 10101 would
    share the count (and could exhaust it, failing a job's runs, never
    widening what a program may do).
  - _Why not enforced mechanically:_ settings in a compose file are not
    linted. `check_sandbox` in `scripts/check_rust_runner_sandbox.py` (the
    `Rust runner` workflow) checks them against the container compose
    starts, that no other Dockerfile or compose file names 10101, and that
    another uid is refused on the socket; the hostile programs check their
    effect (a program's identity, a stop, kill and `/proc` read of the
    runner that fail, a request a program queues that is refused);
    `the_wire_reads_as_documented` checks the request type. Neither sees a
    setting the check does not name.
- **RX2: nothing a program does outlives its job.**
  - _Claim:_ at a ceiling and after every compile and every run, every
    process of the programs' uid is killed, by a helper running as that uid
    (`sh -c 'kill -KILL -1'`, repeated until `/proc` lists no live process
    of it): the runner signals nothing and needs no `CAP_KILL`, and nothing
    is signalled by a pid that could have been reused. The runner never
    walks a tree a program made: after every job `clean`, as the programs'
    uid, removes everything in each program's directory, however deep (by
    relative name, one level at a time) and opening directories the uid
    owns before reading them (a directory a full-path host such as
    AppArmor will not let it enter is removed as it is: if it is not
    empty, removal fails and the runner restarts, RX2); the runner then
    removes the empty
    directories, and checks that `/proc/sysvipc`
    lists no System V object and `/dev/shm` and `/dev/mqueue` are empty,
    and that the socket's directory holds only the socket it bound. If any
    of that fails, the runner exits, and the container's restart starts it
    clean: fresh tmpfs, fresh IPC namespace, and, since the runner is PID 1,
    every process in the container killed with it. **The restart is the
    cleanup of last resort**, and these cases take it: a System V semaphore
    or queue a program leaves (shared memory goes with the process that
    made it, `shm_rmid_forced`; POSIX queues are refused, `--msgqueue=0`);
    something left in `/dev/shm` or `/dev/mqueue`, `/work` or the socket's
    directory that cannot be removed; a sweep that cannot finish (a
    survivor that stops the helper past its 5 s, or kills it every pass);
    and a job past the watchdog's deadline (RX3). The client of a job that
    ends this way sees its connection dropped, with no answer. The one
    restart that is answered first is a machine fault before the program
    ran (RX3's `unavailable`), so no program is charged with it. **What an
    attempt costs:** each attempt at a program that wedges the runner holds
    it for up to the job deadline (`serve_job_deadline`, 132 s) before the
    watchdog restarts it, which is why the pass's `DROPS` cap (2) bounds
    what one program can cost at two job deadlines per toolchain (RP3).
  - _Falsified by_ a hunk that skips the sweep, the clean or any of those
    checks for any request, catches their failure and goes on serving,
    makes a sweep failure read as the program's (`TimedOut`,
    `InvalidInput`), or puts an init back in front of the runner.
  - _Scope:_ processes, files under `/work` and the socket's directory, and
    IPC objects of the container's namespace. Time a program spent and the
    CPU's caches are not undone; nothing reads them.
  - _Why not enforced mechanically:_ the runner's loop is short enough to
    read. The hostile programs check it: a loop, unbounded allocation, a
    fork bomb, a thread bomb, processes that leave the group (`setsid`), a
    socket, writes everywhere, a directory made mode 000 and one 20000
    levels deep (as deep as the host allows), System V shared memory,
    semaphores and queues, survivors looping `kill(-1, SIGSTOP)` and
    `kill(-1, SIGKILL)`, and compiles that blow up rustc. Each ends as a
    recorded outcome or a dropped connection, with no process, file or IPC
    object left; only the semaphore and queue and the two loopers restart
    the runner, once each at most.
- **RX3: one job at a time, never on a request's path.**
  - _Claim:_ the service answers a connection before it accepts the next,
    and every job is bounded: the compile timeout (60 s), the run ceiling
    (2 s), a 64 KiB request read within one 5 s deadline for the whole of
    it, the answer written within one 5 s deadline for the whole of it,
    `clean` within 30 s, and each sweep helper within 5 s. **The backstop
    is the watchdog:** a thread of the runner arms each job's whole life,
    from accepting its connection to the end of its tidy, with the sum of
    those ceilings plus a 30 s sweep allowance (132 s with the defaults,
    `serve_job_deadline`, which is `rust_rounds_wire::protocol::job_deadline`
    of the runner's ceilings), and exits the runner if the job is still going
    past it. That bounds the one wait nothing on the job's thread can: a
    helper a survivor stopped before its `exec`, which blocks `spawn`. The
    exit is RX2's restart; the client sees its connection dropped. **What a
    client does with a dropped connection:** treat it as "the runner
    restarted while running this job" and retry it, a bounded number of
    times, then record that as the outcome (`rust_rounds::pass`, RP3); the
    protocol has no answer for it. **A fault that is the machine's alone is
    said:** when `record` fails before the program ran (rustc missing or
    failing to run, no directory or file for the program, or the program's
    own process not started: no pipes, descriptors or memory, a fork or
    exec the kernel refuses, `process::not_started`; `SpawnError`'s
    `Toolchain` and `Io`), the service first answers `refused`, reason
    `unavailable`, then exits as before; the client keeps nothing against
    the program (RP3). A fault once the program ran (a run or a sweep it
    could not supervise to its end, `SpawnError::Unsupervised`, which a
    program's survivors can cause) is answered with nothing, so the client
    counts a drop. This is the only change to the service since slice 1:
    one refusal kind, and one variant that tells the two phases apart. Each
    retry of a program that wedges the
    runner holds the one job slot for up to the job deadline, so the pass's
    `DROPS` cap (2) bounds that program at two job deadlines per toolchain.
    **A client waits past the watchdog:**
    `ASK_TIMEOUT` (137 s) is `DEFAULT_JOB_DEADLINE`, the job deadline of the
    default ceilings the image runs with, plus one client timeout, all
    `rust_rounds_wire` constants the runner's own deadline is summed from,
    and `ask`'s default wait. A client that gave up before the watchdog
    would ask again while the runner was still wedged, queue behind it, and
    spend a second attempt on one hostile program. A connection from any uid but the client's and
    root's is refused before anything is read (`SO_PEERCRED`), so a program
    cannot queue jobs that outlive it.
    file_host asks the runner only from a background pass (EX1), never from
    a handler, and `ask` reads at most 4 MiB of answer.
  - _Falsified by_ a machine fault before the program ran answered with
    anything but `unavailable`, or one after it (`Unsupervised`) answered
    `unavailable`, a client wait (the pass's `Waits::record`, `ask`'s default)
    that is a literal or shorter than the job deadline, a job deadline summed
    from anything but `rust_rounds_wire`'s constants and the runner's
    ceilings, a thread or task per connection, a removed ceiling or
    one per read or write, a helper waited without a deadline, a watchdog
    removed, disarmed before the tidy ends or depending on any process of
    the programs' uid, a peer check removed or moved after the read, or a
    handler that asks the runner.
  - _Scope:_ the service loop and its client. How often the pass asks, and
    what it does when the runner goes away holding a program, is RP3 and RP4
    (below).
  - _Why not enforced mechanically:_ the loop is short enough to read.
    `a_trickling_client_is_refused_at_the_client_timeout` checks the read's
    deadline (the write's is the same type), `only_a_clients_uid_is_served`
    and the hostile programs check the peer refusal, the two looper cases
    check that a job always ends and the runner comes back (by the helper's
    deadline or the watchdog's), and EX1's tests the request path.
- **RX4: a recording is keyed to the bytes and the toolchain that made it.**
  - _Claim:_ `key` is SHA-256 of the toolchain (`rustc -V` and the profile,
    `rustc 1.97.0 (2d8144b78 2026-07-07); --edition 2024 -C opt-level=0`),
    a NUL, and the source. The runner refuses a `record` naming a toolchain
    other than its own. The client does not take the answer's word for the
    key: file_host's pass checks the answer's toolchain is the one it sent
    and stores the recording under `key` of that toolchain and the source
    it sent, computed itself. A toolchain bump is a new key for every
    program. One normalisation is applied: a panic header's thread id is
    dropped, so the same failure records the same bytes.
  - _Falsified by_ a key over anything else (a round, a card, a tutor), or
    a recording stored under a key or toolchain the client did not compute
    and send itself.
  - _Scope:_ the runner, and the client's half in `rust_rounds::pass` (RP2).
  - _Why not enforced mechanically:_ which key a future writer computes is
    not visible to a type. The runner's half is checked by
    `the_key_is_the_toolchain_and_the_source_and_nothing_else`,
    `the_same_program_records_the_same_bytes_but_its_time` and
    `the_service_answers_one_request_per_connection`, and the client's by
    `a_stored_rounds_programs_are_recorded_by_the_runner_in_one_pass` (the
    fake runner's own `key` is never the one kept).

## Rust Rounds: recording on arrival (paulgsc/some-ui#2234, RR3)

| Piece | What it is |
| --- | --- |
| `PUT /api/v1/rust-rounds/:id` (`routes::rust_rounds`, `handlers::rust_rounds`) | A paired phone puts a round as the tutor wrote it (`some-ui.rust-round/1`). Home checks what it needs (`rust_round_repo::round::read`: at most 32 KiB, the tag, the id, each card's kind and id, and every program: a read card's, and a variant card's built as the format builds it) and, in one transaction, keeps the round and queues each program it does not hold yet. `201` new, `200` the same bytes again, `409` another body under an id the subject holds, `422` naming each field, `413` for a body past one byte over the ceiling, before it is buffered. |
| `GET /api/v1/rust-rounds`, `GET /api/v1/rust-rounds/:id` | The caller's subject's newest rounds; one of its rounds, byte for byte, with each read and variant card's program and every run kept for it (`toolchain`, `ended`, `refusal`, and `rustc`, some-ui's `RustcRunSchema`; never when it was kept, since a run is shared across subjects). The `ETag` is the body's hash, so it moves when a run is kept; `If-None-Match` answers `304`. Another subject's id is a `404`. |
| `rust_round`, `rust_round_source`, `rust_round_run`, `rust_round_drop` (`migrations/20261031000100_create_rust_round.up.sql`, `crates/db/rust_round`) | The rounds; each distinct program once, in arrival order (the queue); one run per program and toolchain (`recorded`, `refused` or `runner-lost`, RP3); and each time the runner was lost holding a program. None is ever updated. `rust_round` is subject-scoped, read only by its subject and deleted only with its account; the other three name no subject, are never deleted, and are shared by every round holding the program (docs/identity.md invariant 28). |
| `rust_rounds` (`src/rust_rounds.rs`, `src/rust_rounds/pass.rs`) | The background pass, every `RUST_ROUNDS_SECONDS`: asks the runner its toolchain, reads what waits under it, and hands each program over the socket (`rust_rounds_wire::protocol::exchange`), keeping how it ended. Off unless `RUST_ROUNDS_ENABLED`. |

**A round's id is never reused, so another body under it is refused.** The
format says ids are "unique across rounds, never reused", so a second body
under a held id is a tutor's mistake, not a new version: home answers `409`
and keeps the first. Never a revision and never an overwrite, so what the
phone played is what home keeps. Ids are unique per subject, since a round
is its putter's: another subject's round under the same id is its own
round, and neither can see the other's.

**The pass's invariants.** Declared per this repo's `CLAUDE.md` ("Drift is
loud, not silent"), in the shape of EX1, EX2 and RX1–RX4; they are the
study nudge's scaling rules ("Scaling invariants", `docs/study-nudge.md`)
applied to this pass. "The pass" is `rust_rounds::pass::run_once`.

- **RP1: no request waits on a compile, and file_host spawns no process.**
  - _Claim:_ no handler asks the runner: `handlers::rust_rounds` reads and
    writes rows only, and a `PUT` answers once its transaction commits. Only
    the pass asks, from the task `rust_rounds::start` spawns, each ask on a
    blocking thread (`spawn_blocking`) through `rust_rounds_wire`, which
    starts no process. The runner compiles and runs, in its container (RX1).
  - _Falsified by_ a handler, extractor or middleware that names
    `rust_rounds::pass`, `Runner`, `exchange` or `ask`; a process start
    under `src/` or in `rust_rounds_wire` (EX1's `SPAWNS`); or file_host
    depending on `rust_runner` other than through `leetype_runner`.
  - _Why not enforced mechanically:_ EX1's two tests hold the process half
    and the dependency half. That no handler calls the pass is a reading of
    one module's imports.
- **RP2: runs are keyed to the bytes and the toolchain, and what waits is
  one range read.**
  - _Claim:_ every run is kept by `rust_round_repo::keep_run` under the key
    the pass computes (`key(toolchain, source)`, from the toolchain it asked
    the runner and the source it sent; RX4), for the program that is next in
    arrival order under that toolchain, and refused otherwise. So, per
    toolchain, the recorded programs are always the first to arrive, and
    what waits is every program after the toolchain's last run:
    `rust_round_repo::waiting` reads it with two indexed reads, whatever has
    been recorded, and a pass with nothing to do reads nothing. A toolchain
    bump is a new key for every program, recorded again from the first.
  - _Falsified by_ a run kept under a key or toolchain the pass did not
    compute or ask for, a write to `rust_round_run` or `rust_round_drop`
    outside `keep_run` and `keep_drop`, either of them writing for a program
    other than the next waiting, or a read of what waits that scans what is
    recorded.
  - _Why not enforced mechanically:_ SQL text is not linted.
    `programs_wait_in_arrival_order_until_recorded_under_the_toolchain`
    checks the order and the refusal, and `what_waits_is_an_indexed_range_read`
    the query plan.
- **RP3: a program is blamed only for what it did.** Every answer to a
  program's ask keeps one of three things, or nothing, and every stop is
  said.
  - _Claim:_ for each program asked about, under the toolchain the pass
    asked the runner first (`rust_rounds::pass::run_once`'s table):
    - **recorded** (a recording under that toolchain): kept as `recorded`,
      with the recording and the `RustcRun` the pass derives from it; the
      pass goes on.
    - **refused for the request** (`malformed`, `too-large`): kept as
      `refused`, with the reason and a `stopped` `RustcRun` (`exit`); the
      pass goes on, so the queue moves.
    - **lost** (the runner went away after the whole request was sent,
      `Dropped`; or no answer within `ASK_TIMEOUT`, `TimedOut`, which
      outlasts the runner's watchdog, so a hang is the watchdog failing on
      this program): one `rust_round_drop` row (`dropped` or `timed-out`);
      at the `DROPS`th (2) of either kind for one program under one
      toolchain, its run is kept in the same transaction as `runner-lost`,
      with a `stopped` `RustcRun` (`exit`). The pass stops.
    - **an outage** (nothing listening, no socket, the request not written
      whole; `unavailable`, the runner's own machine failing before the
      program ran, RX3; `peer`; an answer that is no answer, or a recording
      under another toolchain): nothing kept; the pass stops, and the next
      asks again.
    - **`wrong-toolchain`** (the runner was replaced mid-pass): nothing
      kept; the pass stops, and the next asks the new toolchain.
    The toolchain ask keeps nothing whatever happens: any failure of it is
    an outage. Every stop is an ERROR on the log and one count on
    `operation_errors_total{operation="rust_rounds", error_type=<the
    stop's kind>}`, every pass it lasts: `runner_unreachable`,
    `runner_unavailable`, `runner_refused`, `runner_invalid`,
    `toolchain_changed`, `runner_dropped`, `runner_timed_out`, and
    `pass_deadline` when the deadline left no room to ask a single program.
    The one stop that is a WARN and not counted is `pass_deadline` after at
    least one program was asked about: what is left waits for the next
    pass. So one hostile program can lose the runner at most `DROPS` times
    per toolchain, and each loss costs a pass at most `ASK_TIMEOUT`
    (137 s; one more `CLIENT_TIMEOUT` if the request's write was slow,
    RP4): a dropped connection comes by the watchdog's job deadline (132 s,
    RX3), and a timed-out one at the wait's end. A program is kept as
    `refused` only for its own request, and never for an outage. State per
    program and toolchain is at most `DROPS` loss rows and one run.
    **Tolerance:** a loss is the program's most likely cause, not a proven
    one. A `TimedOut` can be the machine's (a host frozen or starved past
    137 s), and a `Dropped` a runner ended from outside while it held the
    program (the kernel's OOM killer, the container stopped or restarted).
    A program that meets `DROPS` such faults under one toolchain is kept as
    `runner-lost` though it did nothing; the record says only that the
    runner was lost while holding it, and the next toolchain asks again.
    A machine fault the runner can see before the program ran, its start
    included, is `unavailable`, never a loss (RX3).
  - _Falsified by_ a loss kept for anything but `Dropped` or `TimedOut` on a
    program's ask, anything kept for an outage, `unavailable`, `peer` or
    `wrong-toolchain`, a `refused` kept for any reason but `malformed` or
    `too-large`, a `RustcRun` taken from the runner's answer rather than
    derived from the recording, a loss limit that grows with anything, a
    second `runner-lost` path, a stop that is not logged as an error and
    counted (but `pass_deadline` after an ask), or a program ask that waits
    less than `ASK_TIMEOUT`.
  - _Why not enforced mechanically:_ which failure is whose is a reading of
    `exchange`'s phases and the runner's refusals.
    `an_exchange_says_how_far_it_got` checks the phases,
    `a_machine_fault_before_the_program_runs_is_answered_unavailable` the
    runner's side, and the pass's tests each row:
    `a_stored_rounds_programs_are_recorded_by_the_runner_in_one_pass`,
    `a_refused_program_is_kept_as_that_and_the_queue_moves_on`,
    `an_outage_keeps_nothing_and_every_program_waits`,
    `a_program_that_loses_the_runner_is_kept_as_that` and
    `a_toolchain_changed_mid_pass_keeps_nothing`.
- **RP4: every pass is bounded by `BATCH` and the pass deadline, and
  nothing else.**
  - _Claim:_ a pass reads at most `BATCH` (20) waiting programs and asks
    the runner about each in turn. It asks the toolchain (waiting
    `CLIENT_TIMEOUT`, 5 s) only if that wait and one program's fit in its
    deadline, and each program (waiting `ASK_TIMEOUT`, 137 s) only while
    that wait is left before it. The wait is for the answer; before it, the
    request's write has its own `CLIENT_TIMEOUT`, so an ask ends at most
    `CLIENT_TIMEOUT` (5 s) past the deadline, and a pass at most that past
    `RUST_ROUNDS_PASS_DEADLINE_SECONDS`. (A request is at most 64 KiB to a
    socket on the same host, so the write is short unless the runner is
    wedged; the text says what holds rather than widening every wait.)
    `RUST_ROUNDS_PASS_DEADLINE_SECONDS` is at least both waits together
    (142 s, `rust_rounds::MIN_DEADLINE`, `Waits::least_deadline`), so a
    pass at the minimum asks at least one program. A recorded or refused
    program lets the pass go on; anything else ends it, and the next starts
    from the first program waiting. There is no other cap, cursor or
    deadline: the queue is the arrival order and the toolchain's last run,
    and `DROPS` bounds state per program, not work per pass.
  - _Falsified by_ a per-pass cap, cursor or deadline of the mechanism's
    own, a pass that reads past `BATCH`, a toolchain ask that spends a
    program's wait, an ask started with less than its wait left before the
    deadline, a request write without `CLIENT_TIMEOUT`, a minimum deadline
    below both waits, or a pass that goes on after a loss or an outage.
  - _Why not enforced mechanically:_ the pass is short enough to read.
    `a_pass_is_bounded_by_its_batch_and_its_deadline` checks both bounds,
    and that a pass at exactly the minimum deadline records a program;
    `a_deadline_reached_after_work_is_no_error` the warning.

## #328: the static snapshot

`dump-leetype-snapshot <out-dir>` writes:

```text
<out>/rounds/manifest.json   {"rounds": [<id>, …]}   listed rounds, by id
<out>/rounds/<id>.json       the stored body, byte for byte
<out>/runs/<id>.json         the round's RoundRuns
```

It uses the layout `import-leetype-rounds` reads, the one `paulgsc/some-ui`'s
round export wrote until that export was retired (some-ui #1941);
`crates/db/leetype_round/testdata/rounds/` is in it. JSON is written the way
that export wrote it: two-space indent and a final newline.

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

- **Rounds.** A snapshot round file is the stored body, byte for byte, and
  an imported body is its file byte for byte (`leetype_round_repo::importer`'s
  tests). `paulgsc/some-ui` retired the corpus export this was once compared
  with (#1941), and the operator routes that wrote rounds after it were
  retired (some-ui#2032), so the database is their only source and
  `import-leetype-rounds` the one way in. `manifest.json` is ordered by id,
  because the database keeps no authored order.
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

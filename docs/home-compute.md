# Home's compute: who runs, who waits, who is killed first

Home is one machine: Intel x86_64, with an NVMe root and an NTFS data disk.
It has **no swap** (paulgsc/dotfiles `nixos/hardware-configuration.nix`,
`swapDevices = []`), and no earlyoom or oomd policy. Several services share
it, and each has its own clock, memory cap and retries. This doc says how
they share it. Epic #705 (LANE) is the design and its record.

What this doc declares is true as written. **Planned** marks what is not
built yet, with its issue.

## Invariants

These hold for everything that runs on home. A change that would break one is
raised on its issue before it is built (CLAUDE.md, "Drift is loud"). Each one
says what holds it, or **Planned** with its issue where nothing does yet.

1. **At most one batch job runs on home at a time, across every kind.** The
   render worker is the one batch worker. It runs one job at a time and
   names it in `<lane>/job`, one file for every kind (the call, Playoff
   Delta, previews, essays, clip walks), and the lane admits one job per
   run (LA2). Held by `crates/lane`'s tests and `render-calls.test.sh`. A
   second batch worker joins this lane, never one of its own (L5,
   some-ui#2084).
2. **Interactive work never waits behind batch work.** While a voice note
   is heard or a Rust Rounds program recorded, the lane pauses the render or
   does not start it (LA1), and `render` and `kokoro` take a quarter of
   the default CPU weight. Held by LA1's tests and the worker's pause case.
3. **A yield is never a failure, and resumed work never pays twice for a
   step it kept.** A pause or a stop leaves no `failed.log`, and the job
   resumes from its step cache under the same id. Held by
   `render-calls.test.sh`'s pause and stop cases (L4c, #708). Seeing the
   steps that were paid twice is L7 (#712, **Planned**).
4. **A collision never becomes a recorded result.** A verdict of time, a
   lost runner or a hearing's deadline taken while the box was contended is
   nobody's, and is asked again ("The contention guard", L1 #706, which
   lands with #822).
5. **The caps fit home's RAM, checked on the box at deploy.** **Planned**,
   #709: home's RAM is in neither repo yet. Today the sum of every
   container's cap, ollama's 8 GiB included, is not checked against
   anything.
6. **No schedule starts batch work.** The render worker looks for work
   every minute, but only the lane's `run` starts a job: room, not a clock
   (the owner, 2026-11-07). Held by the worker's "no render starts on a
   missing or stale decision" cases.

## Classes: who is low priority, and what that means

Every service is in one class. A new service picks one in the same change
that adds it.

| Class | Services | What it means today |
|---|---|---|
| **Protected** | `file-host`, `nats`, `redis`, `prometheus`, `grafana`, `node_exporter`, `cadvisor` | Home's state, and the path that tells the owner what happened: Prometheus evaluates the alerts, Grafana delivers them, node_exporter and cAdvisor count the kills. Killed last. |
| **Interactive** | `rust-runner`, `orchestrator`, `home-deploy` | Short work someone may be waiting on. The default. The voice-note transcriber is interactive too, but it runs inside `file-host`, so it is protected with it. `home-deploy` is a host systemd unit, not a container (`nix/modules/home-deploy.nix`): it runs for minutes, only when a client needs a newer server (README.md, "Deploying to home"), and a client is waiting on it. |
| **Tooling** | `metabase`, `redisinsight`, `dashboard-builder`, `redis-exporter`, `nats-exporter`, `process-exporter`, `blackbox-exporter` | For looking at home; nothing is owed by them. |
| **Batch**, the lowest | `render`, `kokoro`, `ollama` | Long work with deadlines in days, and nobody waiting by the minute. Killed first. `render` and `kokoro` take a quarter of the default CPU weight (`cpu_shares: 256`), so any other service gets the CPU first when it is contended. |

The lane (below) decides when the batch class may run: a render starts
only when no interactive work waits and its peak fits, pauses for
interactive work, and is evacuated when memory runs short. The render
worker obeys it (L4c #708, "What the render worker does" below), and
resumes from its step cache: no clock starts a render.

## The lane

The lane decides, each tick, whether the batch class may run now, must
pause, or must stop and let go of its memory. It decides from the kernel's
own signals (`/proc/meminfo`'s `MemAvailable`, `/proc/pressure/memory`'s
`some avg10`) and from whether interactive work is waiting. A render
runs when there is room and gives way when anything else needs it. No
schedule starts batch work; the only time the lane measures is how long
room has held.

`crates/lane` is that decision, a pure function (`step`) with no I/O. The
capacity simulation (#710) is meant to replay real traces through the same
code. What it promises, each held by a test in `crates/lane/src/tests.rs`:

- **LA1.** Batch work never runs while interactive work waits: it pauses, or
  does not start.
- **LA2.** Batch work never starts, or starts again after a stop, without
  room for its peak above the floor, held for the hold-down. The peak is
  the one measured on earlier runs, or, before any run is measured, the
  ceiling the caller declares (the batch containers' memory caps), so the
  first run can start and be measured. While the lane says run, a job not
  seen before (or none yet) keeps that permission only while that room
  holds: a run is a permission to start one job, not every job after it. A
  signal that could not be read, or no peak at all, is never room.
- **LA3.** Batch work running or paused stops and lets go of its memory
  once free memory falls under the floor, memory pressure passes its limit,
  or a signal can no longer be read. Free memory read under the floor is a
  memory stop even when the pressure reading is lost, and a pressure that is
  not a finite number counts as unreadable.
- **LA4.** Stops for memory or pressure are counted against the job in
  progress, named by an id the worker gives each job when it starts it and
  keeps through pauses and evacuations until the job finishes or is given
  up. From the policy's `stuck_after` (at least one) in a row the decision
  says the job is stuck, so one that never fits is reported, not retried
  silently. A tick with another id, or none, starts the count again, so one
  job's stops are never another's and an idle lane is never stuck.

A kernel without PSI (`/proc/pressure` missing) leaves the pressure signal
unreadable, and under LA2 batch work then never runs: home's kernel must have
PSI on. If it does not, batch work never runs, and when nothing else holds
it back (interactive work, memory under the floor) the lane's reason is
`unknown`, as it is for an unreadable `/proc/meminfo` or a missing peak.

### What runs it

file_host runs the lane (`apps/servers/file_host/src/call_weeks/lane.rs`,
started with the call weeks worker) every **5 s**, the lane's tick. Each
tick it reads:

- `/proc/meminfo` and `/proc/pressure/memory`. Inside file_host's container
  both are the host's: `/proc/meminfo` is not namespaced, `/proc/pressure`
  is the system-wide PSI rather than the container's cgroup, and Docker
  masks neither.
- Whether interactive work is waiting: a counter in file_host's `AppState`
  that the transcriber holds while it hears a note and Rust Rounds holds
  while the runner records a program. Rust Rounds' toolchain ask, made at
  the start of every pass whether or not a program waits, holds nothing:
  held, it would mark home busy every minute, and a render could never pass
  its hold-down. A pass with nothing to do holds nothing, and while the
  operator has paused Rust Rounds (`settings::RustRounds`) nothing is asked,
  so nothing is held. Work that started and ended between two ticks still
  counts at the next one.
- The render worker's status folder, `${STORAGE}/renders/call-worker/`:
  `job`, the id of the job in progress (a decimal number), and `peak`, the
  bytes its last finished run measured. Both are written by the worker
  (below); with none written there is no job, and the peak is the declared
  ceiling below. A file there that is not a number is an unreadable signal,
  and an unreadable `job` is never room: the lane says stop, `unknown`.

It runs `lane::step`, and writes the decision whole (a hidden file, then a
rename) to `${STORAGE}/renders/call-worker/lane`, one `key=value` a line:

```
verb=stop
why=settling
stops=0
stuck=false
at=1731000000
```

`at` is Unix seconds. A decision older than **30 s**, or none, means stop:
the worker never runs on a lane that has stopped deciding.

**Tolerance:** a pause lands within one lane tick (5 s) of interactive work
starting, plus the worker's own read interval (2 s): within 7 s. Interactive work
starts when the transcriber starts hearing a note or the runner is asked
to record a program; a note waits for the transcriber's minute poll before
that, and that wait is not interactive work.

### Home's policy

| Value | Home's | Why |
|---|---|---|
| Floor | 2 GiB | Interactive work must fit beside a render that holds its memory while paused: the rust-runner's whole 1 GiB cap, and as much again for the transcriber's working memory (its model's 500 MB stays loaded, so is already counted), file_host's requests and the error in `MemAvailable`'s estimate. |
| Declared peak | 5.84 GiB | Until the worker measures one: the render's 4 GiB cap and the speaker's room above the 1.25 GB it keeps between renders (its 3 GiB cap less that, #691). The stage is served inside the render's container, so its cap holds it (#718; 6.09 GiB while it was a container of its own). What the speaker keeps is already out of `MemAvailable`, so it is not counted twice. Inside the render's cap, ffmpeg's joins peak under 1 GB since VR1 (some-ui#2657; 4.17 GB before it, #694). |
| Pressure limit | 10% `some avg10` | Some task stalled on memory for a tenth of the last ten seconds. Home has no measurement of memory PSI under a render yet: the CALL dashboard's LANE row draws it against this limit, which is where it is tuned. |
| Hold-down | 30 s | Three of `avg10`'s windows, so room that comes back is not a lull inside one. |
| Stuck after | 3 stops | A job stopped three times in a row for memory or pressure is reported (LA4). |

Free memory needed to start a render is the floor plus the peak, 8.09 GiB
with the declared one. A box that never has that free never starts a
render, and says so: the lane's reason stays `memory`, the LANE row shows
how long, and `CallRenderOverdue` fires on the week left waiting.

### What it says

- **Metrics** (`metrics/lane.rs`): the verb and reason (`lane_verb`,
  `lane_why`, and `lane_ticks_total` counting each tick by both, so the time
  spent held back and why is exact), `lane_stops` and `lane_stuck`, every
  signal it read and whether each could be (`lane_signal_readable`), the
  policy, and when it last wrote a decision.
- **The CALL dashboard's LANE row:** what the lane said and why over time,
  the time in each decision, who was working, free memory against the floor
  and the peak, and pressure against its limit.
- **Alerts** (`home.yml`, group `home-lane`, objective LN3), each with a
  firing and a quiet promtool case in `home.test.yml`:
  - `LaneStale`: no decision written for over a minute, held a minute (one
    never written included).
  - `LaneStuck`: the job in progress is stuck (LA4).
  - `LaneSignalsUnreadable`: a signal unreadable for 5 minutes, named. Every
    `unknown` decision comes from one, so a kernel without PSI is heard
    here.
  - file_host not answering a scrape is `FileHostDown`'s, not these.

### What the render worker does

`infra/render/render-calls.sh` (L4c #708), for each of its jobs alike (the
call's, Playoff Delta's, a template's preview's, a Rust Rounds essay's and a
clip's walk). A clip's walk (paulgsc/some-ui#2670's clips) is batch class
like the others: the motion replays an app's scripted walk in Chromium and
keeps its stills, no speaker and no video, so it is the smallest job the
lane admits, measured like any other once it has run. `infra/render/render-calls.test.sh` holds each
point below with a fake lane and a recorder slow enough to be caught half
way; CI runs it on every PR.

- **A job** is one episode's attempt, from its first try until it renders,
  fails or is let go. Its id (the time in nanoseconds, or one past the last
  id given, kept in `job-seq`) is never reused, and it keeps it through every
  pause and stop, so LA4 counts its stops alone and a resumed job is never
  admitted as new. Whichever job runs writes the one `job`: the lane admits
  one batch job at a time. `job.state` says which job it is (`kind`, `key`,
  `since`) and where (`state`: `waiting`, `rendering` or `paused`;
  `state_since`; `yields`, the times it gave way; `ran`, the seconds it
  rendered before `state_since`), for the phone's Renders page
  (some-ui#2761). A job that renders leaves `took`, the seconds it rendered
  with pauses and waits left out, in its kind's status folder: the next
  one's rough bar measures against it. While a run records, `job.progress`
  keeps the recorder's own count beside it (its job's `id`; `station`,
  `recording` or `assembling`; `scene`, the scenes recorded this run;
  `scenes`, how many, once the recorder says; `since`), a count and never a
  scene's name, gone with the job and counted afresh by each run (C138).
- **It starts** only on a `run` written after its id was, so the lane has
  seen it and found room for its peak (LA2). A decision missing, unreadable,
  or more than 30 s from now either way is stop.
- **It reads the decision every 2 s** while a job waits or runs.
  - `pause` freezes the recorder and every process under it (SIGSTOP; SIGCONT
    on `run`). The tree is read from `/proc`, not the process group:
    Playwright starts Chromium in a session of its own. Frozen, it keeps its
    memory and takes no CPU. The speaker is another container and is not
    frozen: a line it is saying finishes.
  - `stop` freezes the tree, then kills it, so nothing escapes by outliving
    its parent. The job keeps its work, its step cache included (each entry
    is renamed into place whole, so a kill leaves no half entry), and waits
    for the next `run`; the recorder then reruns over it, and every step it
    had finished is a cache hit.
  - An attempt that fails within 5 minutes of a pause or a resume, or dies
    while frozen, is run again the same way, not failed: a page's timeout runs on while the
    recorder is frozen. At most 3 times a job; past that, or later, it
    failed, so a real failure never loops.
  - **A job that let go of its memory without the lane's stop** (a restart
    killed it, it failed of a pause, or its decision went stale) is taken
    away (`job` removed) until the lane writes a decision without it, then
    named again under the same id. The lane checks room only for a job it
    has not seen running (LA2), so this is what makes it check before the
    job takes its memory again. Its count of stops starts again (LA4); a
    stop the lane decided never goes this way, so a job that never fits is
    still counted to stuck. A restart in the moment the job is taken away
    loses its id: it comes back as a new job, its count, `yields` and `ran`
    started again (the phone's bar too), its work kept.
  - **A yield is never a failure:** no `failed.log`, no `CallRenderFailed`.
- **Its peak**, once it renders: the render container's `memory.peak` (its
  most since the container started, so never less than this render's) plus
  the speaker's room, as the declared peak counts it.
  An unreadable `memory.peak` writes none, and the lane keeps the declared
  one.
- **A restart keeps the job.** `job.state` names it, it runs before any
  other, under the same id, and its work is never removed for the
  `WORK_MAX_MB` cap while it waits. Docker stopping the container kills the
  recorder's tree and leaves the job.
- **The first render after a boot** waits for the lane's first `run`, which
  is never sooner than its hold-down (30 s after file_host starts): home's
  own services start first.
- **It looks for work every minute**, a listing of three folders. Looking
  never starts a render; the lane does. A failed episode is tried again an
  hour after its failure, not at every look. While a job waits for room or
  is paused, the worker stamps `last-pass` at least every minute, so a
  worker the lane holds back is not `CallRenderWorkerStale`'s (a job that
  never fits is `LaneStuck`'s). It never stamps while a render runs, so a
  render that hangs still goes stale.
- **Between jobs**, it clears every attempt log (`.<key>.log`) a killed
  attempt left: none runs then, and the lane would read one as batch work
  in progress for good.
- Each recorder run has its own `TMPDIR`, removed after it, killed or not:
  a killed Chromium leaves its profile behind, and `/tmp` is held in memory.
- **A template's preview** (SFEED12, paulgsc/some-ui#2757) is its third job,
  after the call's and Playoff Delta's, with no priority over either: the
  owner asks for one from Studio (`POST /previews/:template`), file_host
  writes the request to `${STORAGE}/preview-requests`, and the worker makes
  it from the template's sample under the lane like any episode, counted
  against the same declared peak until one is measured. Unlike an episode, a
  preview that fails is never tried again (the owner asks for another), and
  it is not kept: file_host removes it a day after it is made, and an older
  one of its template as soon as a newer one is ready (`previews::prune`,
  every 10 minutes, never the job in progress), at the owner's word, with no
  switch and no receipt. One not ready a day after it was asked fails as
  `overdue`, the job in progress included, so a render that died never
  holds its template's one request for good.

## At rest: what home costs while nothing is worked on

Home's idle footprint is what its services take while nobody's work is
running. The lane knows when that is, so it says it: `lane_idle` is 1 in a
tick with no interactive work held and no batch work in progress (a job
named in `job`, or an attempt of either of the render worker's jobs
running, the call's or Playoff Delta's, which the worker marks with a
`.<key>.log` in that job's status folder until the attempt ends; a log left
by an attempt that was killed counts until that key's next attempt ends,
when that key's next attempt ends).
The polls (mailbox, call weeks,
game feed, the nudge waker), the mailbox's held bell connection (a `ping`
every 30 seconds), requests and scrapes keep running at rest:
they are part of what keeping home up costs, and are counted.

WHO DUNNIT's AT REST row keeps each series only at those moments and
averages it over the dashboard's range: the share of time at rest, home's
memory in use and its busy cores at rest, and each container's memory and
CPU at rest. A batch service that stays up between jobs (the speaker keeps
its model, about 1.25 GB) is counted there.

## Who is killed first

When home runs out of memory, the kernel kills the process with the highest
OOM score: its share of memory, plus the service's `oom_score_adj`. Each
service sets one by its class, in `infra/compose/*.yml`:

| Class | `oom_score_adj` |
|---|---|
| Batch | `ollama` 900, `render` 800, `kokoro` 800 |
| Tooling | 500 |
| Interactive | 0 (`home-deploy` takes systemd's default, which is 0) |
| Protected | −500 |

`ollama` goes before the render's services: it is the largest when a model
is loaded, and nothing in this repo or paulgsc/some-ui calls it (#705, D3).
Protected is −500, not −1000: a protected service that leaks can still be
killed, rather than taking the whole host down with it.

A container's own `mem_limit` is separate: past it, the kernel kills the
largest process *inside that container*, whatever the host has free. The
order above decides only when the host as a whole runs out.

## What a kill costs

| Killed | What it costs | What retries it | What says so |
|---|---|---|---|
| a process in `render` (ffmpeg, Chromium, the recorder and the stage it serves itself, #718) | the week's attempt fails (one the lane stops is no failure: it resumes, "What the render worker does") | an hour after the failure; every step it finished is in the step cache, so the retry pays only for the step in flight and the stills | `HomeOomKill`, `CallRenderFailed` |
| `kokoro` | the line being said; the container restarts, and the render's attempt fails on the dropped connection | as above; takes already said are cached | `HomeOomKill`, `CallServiceRestarted`, `CallRenderFailed` |
| `ollama` | a request in flight; the container restarts | nothing in this repo calls it | `HomeOomKill` |
| `file-host` | the API and every poll stop until Docker restarts it. A voice note being heard keeps nothing and is heard again (`transcribe/pass.rs`, "A shutdown keeps nothing") | Docker's restart; each poll's first pass runs at start | `HomeOomKill`, `FileHostDown` |
| `rust-runner` | the program it was running is kept as a loss (`dropped`), and at the second loss of one program it is kept as `runner-lost` for good under that toolchain (`rust_rounds/pass.rs`, `DROPS`) | Docker's restart; the next pass asks again until `DROPS` | `HomeOomKill`. A program lost to *another* service's memory is blamed on nothing yet: #706 |
| `prometheus` | no alert is evaluated until Docker restarts it | Docker's restart | `HomeOomKill` once it is back, if within the hour |
| `grafana` | no alert is delivered until Docker restarts it | Docker's restart | `HomeOomKill`, delivered once it is back |
| `nats`, `redis` | what file_host keeps in them, and its realtime path, until Docker restarts them | Docker's restart | `HomeOomKill` |
| tooling | nothing owed | Docker's restart | `HomeOomKill` |
| `home-deploy` | the deploy in flight, before it records anything: a `make up` cut short can leave `file-host` stopped (then `FileHostDown`) | the path unit starts it again while the marker exists, and the run repeats from the pull | `HomeOomKill`; `FileHostDown` if it stopped `file-host`; `DeployStuck` once the marker is 30 minutes old |

## How a kill is heard

The kernel's own count, `node_vmstat_oom_kill` (`/proc/vmstat`'s `oom_kill`),
is the truth: it counts every OOM kill, whether inside a container's cap or
across the host. Two alerts in `infra/prometheus/rules/home.yml` (LN2) read
it, and `home.test.yml` proves each:

- **`HomeOomKill`** names the container. cAdvisor attributes each kill
  from the kernel's log (`container_oom_events_total`, by `name`).
- **`HomeOomKillUnattributed`** fires when the kernel counted more kills
  than cAdvisor named, held 2 minutes so that one counter being scraped
  before the other is not a kill. A kill outside any container, or one
  cAdvisor missed, is still heard.

Both look back an hour. A second kill within the hour of the first is the
same firing alert; the WHO DUNNIT dashboard's OOM tile counts each kill.

**Tolerance:** a kill made while Prometheus was down for more than an hour
shows on the tile, but no alert says it. Nothing watches Grafana itself
(paulgsc/some-ui#2289).

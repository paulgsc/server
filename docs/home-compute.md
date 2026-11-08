# Home's compute: who runs, who waits, who is killed first

Home is one machine: Intel x86_64, with an NVMe root and an NTFS data disk.
It has **no swap** (paulgsc/dotfiles `nixos/hardware-configuration.nix`,
`swapDevices = []`), and no earlyoom or oomd policy. Several services share
it, and each has its own clock, memory cap and retries. This doc says how
they share it. Epic #705 (LANE) is the design and its record.

What this doc declares is true as written. **Planned** marks what is not
built yet, with its issue.

## Classes: who is low priority, and what that means

Every service is in one class. A new service picks one in the same change
that adds it.

| Class | Services | What it means today |
|---|---|---|
| **Protected** | `file-host`, `nats`, `redis`, `prometheus`, `grafana`, `node_exporter`, `cadvisor` | Home's state, and the path that tells the owner what happened: Prometheus evaluates the alerts, Grafana delivers them, node_exporter and cAdvisor count the kills. Killed last. |
| **Interactive** | `rust-runner`, `orchestrator` | Short work someone may be waiting on. The default. The voice-note transcriber is interactive too, but it runs inside `file-host`, so it is protected with it. |
| **Tooling** | `metabase`, `redisinsight`, `dashboard-builder`, `redis-exporter`, `nats-exporter`, `process-exporter`, `blackbox-exporter` | For looking at home; nothing is owed by them. |
| **Batch**, the lowest | `render`, `kokoro`, `call-stage`, `ollama` | Long work with deadlines in days, and nobody waiting by the minute. Killed first. `render` and `kokoro` take a quarter of the default CPU weight (`cpu_shares: 256`), so any other service gets the CPU first when it is contended. |

**Planned** (L4 #708): the batch class runs on room, not a clock. A render
starts only when no interactive work waits and its measured peak fits. It
pauses for interactive work, is evacuated when memory runs short, and
resumes from its step cache. Until then, a render starts on its own hourly
loop (`infra/render/render-calls.sh`), whatever else is running.

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
  first run can start and be measured. A signal that could not be read, or
  no peak at all, is never room.
- **LA3.** Batch work running or paused stops and lets go of its memory
  once free memory falls under the floor, memory pressure passes its limit,
  or a signal can no longer be read.
- **LA4.** Stops for memory or pressure of a job in progress are counted
  until that job finishes or is given up; from the policy's `stuck_after`
  (at least one) in a row the decision says the job is stuck, so one that
  never fits is reported, not retried silently. A job is in progress from
  its start until it finishes or the worker gives it up: being paused or
  evacuated does not end it. With no job in progress the count is zero, so
  an idle lane, or the job after one given up, is never stuck for another
  job's stops.

A kernel without PSI (`/proc/pressure` missing) leaves the pressure signal
unreadable, and under LA2 batch work then never runs: home's kernel must have
PSI on. If it does not, the lane's reason stays `unknown`, the reason it
also gives for an unreadable `/proc/meminfo` or a missing peak.

**Planned** (L4 #708): nothing runs the lane yet. file_host is to read the
signals and write the decision where the render worker reads it, and the
render worker is to pause, stop and resume on it.

## Who is killed first

When home runs out of memory, the kernel kills the process with the highest
OOM score: its share of memory, plus the service's `oom_score_adj`. Each
service sets one by its class, in `infra/compose/*.yml`:

| Class | `oom_score_adj` |
|---|---|
| Batch | `ollama` 900, `render` 800, `kokoro` 800, `call-stage` 800 |
| Tooling | 500 |
| Interactive | 0 |
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
| a process in `render` (ffmpeg, Chromium, the recorder) | the week's attempt fails | the next hourly pass; every step it finished is in the step cache, so the retry pays only for the step in flight and the stills | `HomeOomKill`, `CallRenderFailed` |
| `kokoro` | the line being said; the container restarts, and the render's attempt fails on the dropped connection | as above; takes already said are cached | `HomeOomKill`, `CallServiceRestarted`, `CallRenderFailed` |
| `call-stage` | the render's attempt fails | as above | `HomeOomKill`, `CallServiceRestarted`, `CallRenderFailed` |
| `ollama` | a request in flight; the container restarts | nothing in this repo calls it | `HomeOomKill` |
| `file-host` | the API and every poll stop until Docker restarts it. A voice note being heard keeps nothing and is heard again (`transcribe/pass.rs`, "A shutdown keeps nothing") | Docker's restart; each poll's first pass runs at start | `HomeOomKill`, `FileHostDown` |
| `rust-runner` | the program it was running is kept as a loss (`dropped`), and at the second loss of one program it is kept as `runner-lost` for good under that toolchain (`rust_rounds/pass.rs`, `DROPS`) | Docker's restart; the next pass asks again until `DROPS` | `HomeOomKill`. A program lost to *another* service's memory is blamed on nothing yet: #706 |
| `prometheus` | no alert is evaluated until Docker restarts it | Docker's restart | `HomeOomKill` once it is back, if within the hour |
| `grafana` | no alert is delivered until Docker restarts it | Docker's restart | `HomeOomKill`, delivered once it is back |
| `nats`, `redis` | what file_host keeps in them, and its realtime path, until Docker restarts them | Docker's restart | `HomeOomKill` |
| tooling | nothing owed | Docker's restart | `HomeOomKill` |

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

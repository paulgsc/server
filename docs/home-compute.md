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
   Delta, previews, essays, clip walks, cards essays, engagement reports, and an
   episode's asset made alone),
   and the lane admits
   one job per run (LA2). Held by `crates/lane`'s tests and `render-calls.test.sh`. A
   second batch worker joins this lane, never one of its own (L5,
   some-ui#2084).
2. **Interactive work never waits behind batch work, and a backlog of it
   never holds batch work for good.** While a voice note
   is heard or a Rust Rounds program recorded, the lane pauses the render or
   does not start it (LA1), and `render` and `kokoro` take a quarter of
   the default CPU weight. Held by LA1's tests and the worker's pause case.
   The one bound: the transcriber's backlog is not someone waiting, so once
   it has heard for `transcribe::turn::TURN` (20 minutes) in a row while
   batch work waits, it hears nothing for `transcribe::turn::GIVE`
   (10 minutes) or until no batch work waits, checked before each note and
   never midway through one (TR4, C899; 2026-11-20, when a run of deadlines
   held every render for hours). Decided with the owner's ask that renders
   not be starved by a busy transcriber, through paulgsc/some-ui#3333's
   coordinator (2026-11-21); `TURN` and `GIVE` are policy constants. A run ends when the queue is empty, so
   notes heard as they arrive never add up to a turn. A give holds every
   note, a take recorded during it included, for at most `GIVE`, with one
   exception: a note the owner fast-tracked (`POST /voice-notes/:id/fast-track`,
   paulgsc/server#1090) is heard through the give on its one try, while the
   queue's model has never tried it, and only that one. The authority is the
   owner's ruling on paulgsc/server#1090 (2026-11-21): "a fast-tracked take
   skips the give"; C1017 bounds it to one try. It is bounded by the owner's
   own hand: within one run of `file_host`, at most one deadline per marked
   note per model, since marking a
   tried note again never makes it `FastTrack`, and a note excused by the
   contention guard gives way as every other note does. (The guard's streak is
   kept in memory, so a restart during an excused try brings the note back
   `FastTrack` and not excused: one more try per restart.) Its retries wait for the give like the
   rest. And a job
   the lane holds stopped still counts as waiting (paulgsc/server#1088,
   bounded by `GIVE` either way). Held by
   `transcribe::turn`'s and `transcribe::pass`'s tests.
3. **A yield is never a failure, and resumed work never pays twice for a
   step it kept.** A pause or a stop leaves no `failed.log`, and the job
   resumes from its step cache under the same id. Held by
   `render-calls.test.sh`'s pause and stop cases (L4c, #708). Seeing the
   steps that were paid twice is L7 (#712, **Planned**).
4. **A collision never becomes a recorded result, within a bound.** A
   verdict of time, a lost runner or a hearing's deadline taken while the
   box was contended is nobody's, and is asked again, at most `NOBODYS` (3)
   times in a row for one program or note; after that it is judged as on a
   calm box, so a box that never calms cannot hold the queue ("The
   contention guard", L1 #706, #822).
5. **The caps fit home's RAM, checked on the box at deploy.** **Planned**,
   #709: home's RAM is in neither repo yet. Today the sum of every
   container's cap is not checked against anything.
6. **No schedule starts batch work.** The render worker looks for work
   every minute, but only the lane's `run` starts a job: room, not a clock
   (the owner, 2026-11-07). Held by the worker's "no render starts on a
   missing or stale decision" cases.
   A request written once a time has passed (a week ended, as `report_weeks`
   writes one) only makes work available; the lane still starts it.
7. **Nothing does work it does not need: a failure that would come out the
   same is not tried again.** A video the render policy refused (too long,
   too large, or scenes past the frame, #975: the recorder's own refusal,
   for the same input recorded with
   the same flags by the same recorders and speaker over the same brand
   files, is the same refusal) is held in `<status>/<key>.refused` with what
   its last attempt was refused for, and tried again only once one of those
   changes, never on the hourly clock. So is an episode's asset the recorder
   itself refused (its `asset refused:` line: a title no size fits, files not
   there), the same folder giving the same answer (`reason=recorder`), held
   until what the hold sums changes (the render and speaker images, the
   worker, the recorders' flags, the brand files, the episode's own files); any other failure of it
   (a crash, which exits 1 too) keeps the hour. Only the last attempt's own refusal
   counts: an earlier attempt of the same job refused is not what a later
   one failed of. Every other
   failure may clear on its own and is tried again an hour after it.
   Held by `render-calls.test.sh`'s refusal cases (RF1, #928; #975).
8. **Every failure is counted, once, where it happens, and reaches the
   owner.** However deep the step, a failure is one increase of
   `failures_total{process,step}` and one `error!` line, made together by
   `file_host::failure::Failure::record`, the only maker of the `Failure`
   token. An outcome that is a failure carries the token, so it cannot be
   stored uncounted (the mailbox pass's `Handing`, #1172, first).
   `HomeFailures` (`infra/prometheus/rules/home.yml`) sends any increase to
   Slack. Counting changes no function's shape and no outcome stored truly,
   and adds no clock: a deterministic refusal is counted once and held
   (invariant 7), and a yield is not a failure (invariant 3), so nothing
   converts one into a `Failure`. One outcome stored falsely was corrected
   with it: a file the mailbox can never take is now `unfit`, final, not
   `refused` and handed again (the coordinator's call on #1192's review).
   The labels are closed sets (`Process`, the ledger's process ids, and
   `Step`), started at 0 at boot. The cause is logged, so keeping what
   docs/identity.md keeps out of logs out of it is the caller's duty. Held
   by `failure`'s tests and `HomeFailures`' promtool cases. Not yet held
   everywhere: about 140 sites still absorb a failure into an ordinary
   value, each in a child of epic #1174, and nothing yet stops a new one
   (the erasure ratchet, #1189, **Planned**). The owner's rule
   (2026-11-24): a failure is like energy, never destroyed, and the
   bookkeeping must reconcile.
9. **A failure that persists is a level, never a recount.** A condition
   every pass or every restart meets again until it is fixed (an ask home
   cannot answer, a refused file newest in its folder, an index the mailbox
   did not take) is not counted each time it
   is met, since one condition would be many failures (invariant 8: one
   event, one count). It is `failures_open{process,step}`, how many are open
   now, set by `file_host::failure::open` afresh by each pass that looks and
   never added to, so a restart cannot count one twice and a fixed one reads
   0 at the next pass that looks; a pass that could not look leaves it as it
   stood, and its own failure is its process's `*Failing` alert. Its steps
   are in the same closed `Step` table, marked `open`, and a step is one or
   the other, never both. A condition that ends in a loss is an event at the
   end: a refused file a newer kept file passes is never read again, so the
   pass whose keep passes it counts it once (`passed_over`); a study nudge
   one device accepted is spent, so each device that did not take it is
   counted once (`device_missed`). So is what no later pass meets again,
   though it was never kept: a nudge attempt every device refused, whose
   subject is tried again only at its next eligibility (`no_device_accepted`),
   and a walk render the clips pass rejected and removed (`walk_rejected`);
   a level for either would read 0 at the next pass while nothing was fixed
   (Codex on #1221). The open steps beyond the mailbox's (#1179, #809): the
   call's, Playoff Delta's, the report weeks' and the cards essays' render
   signals a tick could not read (`report-weeks` and `cards-essays`
   `observe`, #1227; the report's also when its worker's last pass does
   not read), the
   call's next-week answer, what a clips pass could not take and asks
   again, the Jobs run records waiting on a phone (`jobs-notice`'s
   `refused` and `unanswered`, #1176) and the Snips keys waiting on a fetch
   that may clear (`snips-pickup`'s `retry`, #1177). Nothing is
   stored for it and it adds no clock: the
   passes that already run set it. `HomeFailuresOpen`
   (`infra/prometheus/rules/home.yml`) tells the owner while one stays open
   for about 2 hours (1h45m and the 15 minutes its window holds a cleared
   level). One that clears sooner with nothing lost (an ask the routine
   withdrew, an index a newer one replaced) never tells; that is the bound.
   A condition nothing can fix is not a level, since it would page for good
   with no action behind it: a held Rust Rounds week that does not read
   stays held (nothing rewrites `mailbox_item`), so it is counted once, as
   `read_body`, when it is kept, and so is a mock interview's week (#1178),
   since neither is then owed an essay, and the once-per-process sweep that meets it
   again counts nothing. The bound there: weeks kept before #1198 were never
   counted, since process memory cannot tell the first boot after a deploy
   from any other restart, and a record of it would be a new store
   (the coordinator's call on #1201, 2026-11-24). The owner's choice on #1201 (2026-11-24): a gauge, not a
   counted-once table. The deploy's gauge sweep (`metrics::deploy`) holds
   home's deploy record or marker open while it can't be read
   (`home-deploy`'s `read_state`/`read_marker`, #1208), and logs it when it
   turns unreadable, not on every tick. Held by `failure`'s, the mailbox
   pass's and `metrics::deploy`'s tests and `HomeFailuresOpen`'s promtool
   cases.

## Classes: who is low priority, and what that means

Every service is in one class. A new service picks one in the same change
that adds it.

| Class | Services | What it means today |
|---|---|---|
| **Protected** | `file-host`, `file-host-migrate`, `litestream`, `nats`, `redis`, `prometheus`, `grafana`, `node_exporter`, `cadvisor` | Home's state, and the path that tells the owner what happened: Prometheus evaluates the alerts, Grafana delivers them, node_exporter and cAdvisor count the kills. `litestream` replicates file_host's database to its backup, and `file-host-migrate` is file-host's own migrate step, a one-shot run before it on every `up` (C1620). Killed last. |
| **Interactive** | `rust-runner`, `orchestrator`, `www`, `resume-render`, `home-deploy` | Short work someone may be waiting on. The default. `www` serves every page of the web app and each call it proxies to file_host while someone is on it; `resume-render` renders a résumé while the owner waits, and does nothing between. Anything that runs inside `file-host` takes its class and adds no service: the voice-note transcriber, which would be interactive on its own, and every worker loop file_host runs. `home-deploy` is a host systemd unit, not a container (`nix/modules/home-deploy.nix`): it runs for minutes, only when a client needs a newer server (README.md, "Deploying to home"), and a client is waiting on it. |
| **Tooling** | `metabase`, `redisinsight`, `dashboard-builder`, `redis-exporter`, `nats-exporter`, `process-exporter`, `blackbox-exporter` | For looking at home; nothing is owed by them. |
| **Batch**, the lowest | `render`, `kokoro` | Long work with deadlines in days, and nobody waiting by the minute. Killed first. `render` and `kokoro` take a quarter of the default CPU weight (`cpu_shares: 256`), so any other service gets the CPU first when it is contended. |

`file-host` may use three of home's four cores (`cpus: 3.0`; four, as #710
has it), so the voice-note transcriber inside it hears on two threads
(`transcribe::default_threads`), and holds the lane (LA1) for a little over
half as long as on one (`transcribe`, "What it costs"). It is a cap, not a
reservation. The owner chose it (C919).

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
- Whether interactive work is waiting: a counter per kind (`Kind`: voice,
  rust_rounds) in file_host's `AppState` that the transcriber holds while it
  hears a note and Rust Rounds holds while the runner records a program. The
  lane decides on any kind held; the kinds are kept apart only to say which
  work held it (`lane_busy_ticks_total`). Rust Rounds' toolchain ask, made at
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
  spent held back and why is exact; `lane_busy_ticks_total` counts the ticks
  each kind of interactive work held, so `busy` splits exactly by kind too,
  paulgsc/some-ui#3332), `lane_stops` and `lane_stuck`, every
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
call's, Playoff Delta's, a template's preview's, a Rust Rounds essay's, a
clip's walk, a cards essay's and an episode's asset alone). A clip's walk (paulgsc/some-ui#2670's clips) is batch class
like the others: the motion replays an app's scripted walk in Chromium and
keeps its stills, no speaker and no video, so it is the smallest job the
lane admits, measured like any other once it has run. A cards essay
(CARDS7, #854) is batch class too: the recorder draws each card on the
stage's burn-ins page in Chromium, each scene of a spoken (v2) essay said
by home's speaker as the call's are (EXPLAIN6, #878; an older v1 essay is
silent), and renders the video with ffmpeg, measured like
any other once it has run. Its declared peak is every job's, the speaker's
room included, so voicing it changed neither its peak nor its class. A
week's engagement report (ENG12, paulgsc/some-ui#2700, #821) is batch class
too, and needs no class or cap of its own: the recorder writes the report
from the counts `report_weeks` gave it and records it as the call's
recorder does, said by home's speaker, under the same lane and the same
caps. What `report_weeks` does in file_host to give it a week (a few
indexed reads and one small file an hour, and each minute a read of the
weeks held and a look at each one's files) is not batch work and takes no
room the lane counts. `infra/render/render-calls.test.sh` holds each
point below with a fake lane and a recorder slow enough to be caught half
way; CI runs it on every PR.

- **A job** is one episode's attempt, from its first try until it renders,
  fails or is let go. A week of the call that is a series of parts
  (some-ui#2918) is one job a part, keyed `<from>` for part 1 and
  `<from>-p<k>` after, so a part is admitted, paused, stopped and resumed
  from its own step cache on its own, never a whole week at once; the week's
  first job asks the recorder's `render call-parts` how many parts there
  are, once, inside the run the lane admitted, so no work starts outside the
  lane. Its id (the time in nanoseconds, or one past the last
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
- **A failure is tried again an hour later; a refusal by the policy once
  what it was refused for changes** (invariant 7). A refusal keeps
  `<key>.refused` beside its `failed.log`: its reason (`too-long` with the
  minutes and the cap, `too-large` with the MB and the cap, or `overflow`
  with the scenes past the frame, at most twelve by id, and how many; the
  recorder's numbers and ids, never its words) and `held`, one sum, taken before
  each attempt, of every flag the recorder is handed (the job's caps among
  them), the render and speaker images (`RENDER_IMAGE`, `KOKORO_IMAGE`, the
  release's pins), and each input's and brand file's name, size and time. A
  pass whose sum differs tries it at once; so a cap
  raised and deployed renders the refused episode on its next pass, with no
  new input.
- **Run again** (`POST /feeds/:feed/videos/:key/produce` with `{asset:
  video}`, file_host's `renders_again`) is the owner asking for a failure's
  hour to be over now: file_host moves `<key>.failed.log` aside to
  `<key>.again.log`, its time the ask's, which is no failure, so the
  worker's next look takes the episode, still only on the lane's `run`
  (invariant 6). The worker removes `<key>.again.log` once that attempt
  ends, however it ends, so there is one at most per episode, and only
  while it is asked: the log keeps `failed.log`'s own life, nothing longer.
  A refusal is never moved: by the policy it would come out the same
  (invariant 7), so Run again answers `409` with its reason; and no log with
  any `.refused` beside it moves: the worker writes the refusal from the
  attempt's own log before moving the log into place, and a refusal that
  appears as file_host moves a log puts it back. A cards essay home refused for its media is
  file_host's own and never the worker's: Run again looks at it again as
  home holds its media now, and only when that would no longer refuse it
  does it run the cards pass at once rather than on the hour (never during
  the pass's failure backoff).
- **An asset alone** (the `asset` job, `{asset: thumbnail}`): file_host
  writes one empty ask per `(feed, key, asset)`, `renders/asks/<asset>-<feed>-<key>`,
  once (a second ask is the first). The job draws it with some-ui's
  `record:asset` from the episode's own folder, read-only, into its own work
  folder, then puts it beside the video whole: copied to
  `.<file>.partial` in the episode's folder, linked to its name (a link
  never replaces a file there, so a made asset is never overwritten), the
  partial removed. The video is never rendered again. Present means done: an
  ask whose asset is there, or whose episode has no video, is over, with its
  log and any partial a kill left. One the recorder refused (its `asset
  refused:` line, never its exit alone) is
  held, as a policy refusal is, by `reason=recorder` and `held`, until what
  `held` sums changes (the render and speaker images, this worker, the
  recorders' flags, the brand files, the episode's own files; never its
  folder's time, which a placing moves); any other failure keeps the hour, and Run
  again moves its log aside as a video's. A drawn file a restart left in the
  work folder is put in place without drawing it again. A description is the
  render's own chapters and is made only with its video.
- **Each job's length cap is its own** (`max_minutes`): an essay, Rust
  Rounds or mock interview, may run 60 minutes (C780: the owner's 30 of
  2026-11-17, #927, raised on 2026-11-19 when interview takes went to 3
  minutes, under the owner's ruling that a length cap is a suggestion to
  move with the story); every other job, an engagement report too, 15. The
  other caps are every job's.
- **A restart keeps the job.** `job.state` names it, it runs before any
  other, under the same id, and its work is never removed for the
  `WORK_MAX_MB` cap while it waits. Docker stopping the container kills the
  recorder's tree and leaves the job, marked `job.termed`: a stop that was
  asked for. A restart without that mark (the container died under the job,
  an OOM inside it, say) is counted in `job.state`'s `restarts`, and at
  `RESTARTS` (3) the job is taken as failed, as an exit is: `failed.log`,
  `CallRenderFailed`, and a retry an hour later as a new job. So a render
  that kills its own container is not resumed first on every restart for
  good (#740).
- **The first render after a boot** waits for the lane's first `run`, which
  is never sooner than its hold-down (30 s after file_host starts): home's
  own services start first.
- **It looks for work every minute**, a listing of three folders. Looking
  never starts a render; the lane does. A failed episode is tried again an
  hour after its failure, not at every look. While a job waits for room or
  is paused, the worker stamps `last-pass` at least every minute, so a
  worker the lane holds back is not `CallRenderWorkerStale`'s (a job that
  never fits is `LaneStuck`'s). It never stamps while a render runs, so a
  render that hangs still goes stale. Its sign of life for the Renders page
  is a separate file, `job.alive` (the job's `id` and `at`), which its own
  loop stamps every 10 s while a job renders or is paused, whatever the
  recorder says. A stamp more than 60 s old reads as `heldBy: silent`, so a
  worker killed mid-render stops reading as rendering within a minute (#739).
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
| Batch | `render` 800, `kokoro` 800 |
| Tooling | 500 |
| Interactive | 0 (`home-deploy` takes systemd's default, which is 0) |
| Protected | −500 |

Protected is −500, not −1000: a protected service that leaks can still be
killed, rather than taking the whole host down with it.

A container's own `mem_limit` is separate: past it, the kernel kills the
largest process *inside that container*, whatever the host has free. The
order above decides only when the host as a whole runs out.

### The caps check

`make up` runs on home, so it can read home's `MemTotal`
(`apps/envctl/src/caps.rs`, L3 #709). It sums the cap of every service
`up` starts and warns when the sum is above `MemTotal`. A service's cap is
its `mem_limit`, or else its `deploy.resources.limits.memory`, times its
`scale` or `deploy.replicas`. A service under `profiles:` counts only when
`COMPOSE_PROFILES` turns one of them on (`*` turns on all). Sizes are read
as compose reads them (`1.5g`, `512MiB`, `1t`, `.5g`); a cap of 0 is no cap,
as Docker takes it. The warning names the three largest caps,
which bind first, with each one's class (read from its `oom_score_adj`, as
above). It also warns of any service whose cap or class it can't read, such
as a `${…}` value, rather than leave it out unsaid.

Every service counts at its cap, batch included. The batch caps are its
peak: the render's 4 GiB and the speaker's 3 GiB are the lane's declared
peak plus what the speaker keeps between renders. The room the lane's floor
holds for interactive work is already inside the interactive caps. So no
headroom is added on top for the lane. Nothing is set aside yet for what
runs outside compose either: the kernel, dockerd, systemd and `home-deploy`.
How much to reserve for them is decided with the refusal below, from home's
published facts.

It warns and never refuses. **Planned** (#709): a refusal, once home's
facts are published nightly (part 1). The caps total 25.13 GiB (www's
256 MiB and resume-render's 1 GiB joined with R8, #853; Litestream's
512 MiB and `file-host-migrate`'s 1 GiB with C1620, though the step has
exited before `file-host` starts); `ollama`'s 8 GiB left with it (#705, D3).

## What a kill costs

| Killed | What it costs | What retries it | What says so |
|---|---|---|---|
| a process in `render` (ffmpeg, Chromium, the recorder and the stage it serves itself, #718) | the week's attempt fails (one the lane stops is no failure: it resumes, "What the render worker does") | an hour after the failure; every step it finished is in the step cache, so the retry pays only for the step in flight and the stills | `HomeOomKill`, `CallRenderFailed` |
| `kokoro` | the line being said; the container restarts, and the render's attempt fails on the dropped connection | as above; takes already said are cached | `HomeOomKill`, `CallServiceRestarted`, `CallRenderFailed` |
| `file-host` | the API and every poll stop until Docker restarts it. A voice note being heard keeps nothing and is heard again (`transcribe/pass.rs`, "A shutdown keeps nothing") | Docker's restart; each poll's first pass runs at start | `HomeOomKill`, `FileHostDown` |
| `rust-runner` | the program it was running is kept as a loss (`dropped`), and at the second loss of one program it is kept as `runner-lost` for good under that toolchain (`rust_rounds/pass.rs`, `DROPS`). A loss while the kernel killed anything for memory, or while the CPU was contended, is nobody's and keeps nothing ("The contention guard") | Docker's restart; the next pass asks again until `DROPS` | `HomeOomKill`; `contention_outcomes_total{what="excused"}` |
| `prometheus` | no alert is evaluated until Docker restarts it | Docker's restart | `HomeOomKill` once it is back, if within the hour |
| `grafana` | no alert is delivered until Docker restarts it | Docker's restart | `HomeOomKill`, delivered once it is back |
| `nats`, `redis` | what file_host keeps in them, and its realtime path, until Docker restarts them | Docker's restart | `HomeOomKill` |
| tooling | nothing owed | Docker's restart | `HomeOomKill` |
| `home-deploy` | the deploy in flight, before it records anything: a `make up` cut short can leave `file-host` stopped (then `FileHostDown`) | the path unit starts it again while the marker exists, and the run repeats from the pull | `HomeOomKill`; `FileHostDown` if it stopped `file-host`; `DeployStuck` once the marker is 30 minutes old |

## The contention guard: a busy box is nobody's fault

L1 (#706). The lane keeps batch work off the CPU while interactive work
runs (LA1), but not every service is batch, and a second interactive job
can still contend. So what interactive work *records* is guarded too.

- **The rule.** A busy CPU or another service's memory only makes home
  slower; it never changes what home records. An outcome a wall clock
  decided, or a lost runner, taken while the box was contended, is
  nobody's: nothing is kept, and it is asked again.
- **The evidence**, read before and after each measurement
  (`file_host`'s `contention` module): `/proc/pressure/cpu`'s `some`
  total, contended at a stall share of 10% of the window or more
  (`CONTENDED_SHARE`); and `/proc/vmstat`'s `oom_kill`, which moved if the
  kernel killed anything. Both are the host's inside `file_host`'s
  container, as the lane's signals are.
- **What it excuses:**

  | Worker | Excused when contended | Never excused |
  |---|---|---|
  | Rust Rounds (`rust_rounds/pass.rs`, RP3) | a recording that ran out of time (CPU); a lost runner, `dropped` or `timed-out` (CPU, or an OOM kill anywhere) | every other verdict: a panic, a compile error, a memory ceiling, output |
  | Transcriber (`transcribe/pass.rs`) | a `deadline` (CPU) | `failed`, `unreadable`, `too_long` |

  A recording kept from a contended window has its run's `milliseconds`
  marked `contended` in the stored recording.
- **Bounded.** One program or note is excused at most `NOBODYS` (3) times
  in a row, then judged as on a calm box. The state is the last one
  excused and its count, O(1). An excused outcome stops its pass, so the
  next pass starts with the same one.
- **An unreadable signal is no evidence**: the outcome is judged as before
  the guard. The lane already reports a kernel without PSI
  (`LaneSignalsUnreadable`).
- **What says so:** `contention_outcomes_total{worker, what}` (`excused`,
  `judged_anyway`, `marked`, `unreadable`) on the CALL dashboard's LANE
  row; an INFO line for each pass that gave way, and a WARN for each
  outcome judged anyway. No alert: an excused outcome waits a pass, and
  one judged anyway is the bound working.

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

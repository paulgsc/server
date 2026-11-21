# Shared stages

A shared stage is one piece of home's work that other pipelines join at, so when it is slow or
broken everything downstream waits (paulgsc/some-ui#3333, which absorbs #3332). The transcriber
is the first: interview answers, reflections, jobs notes and Studio takes all wait on it before
anything reads their words.

## The contract

`crate::stage::Stage` in file_host is a closed enum. Each case declares, by exhaustive match, so
a new stage is a new case or it does not build:

| What | Where it lives | Transcriber |
| --- | --- | --- |
| Its promise: a ledger objective and its bound | `Stage::promise`, the bound a policy constant | VH7, `transcribe::HEARD_WITHIN` (15 min, C900) |
| Its headline signal: when the oldest item still waiting arrived, and how many wait | `metrics::stage::record_waiting`, once a pass, over the whole queue | `voice_repo::transcript::backlog` |
| How an attempt ends: done, refused (the item can't be done), failed | `stage::Ended`, mapped from the stage's own outcomes | heard/silent, unreadable/too long, deadline/failed |
| Who waits on it | `Stage::waiters`, each a `Waiter` with its series | `cards_essays_waiting_on_takes`, `lane_busy_ticks_total{kind}` |

The series are the same for every stage, told apart by the `stage` label:
`stage_oldest_waiting_since_timestamp_seconds`, `stage_waiting`, `stage_attempts_total{result}`
and `stage_promise_seconds{objective}`. Prometheus ages the first into
`stage:oldest_waiting:seconds` (`infra/prometheus/rules/home.yml`, group `home-stages`), and two
rules hold every stage: `StageDegraded` (the oldest waiting is older than the promise, 5 min) and
`StageBroken` (more attempts in the last hour than one item gets, `stage_attempts_per_item`, and
under a tenth done or refused, 5 min). Both
take their `objective` from `stage_promise_seconds`, so no rule names a stage or a bound.

The owner sees them as the SHARED STAGES row at the top of the file_host dashboard
(`infra/grafana/dashboards/lib/stage-panels.libsonnet`): per stage, STATE (OK / SLOW / STUCK /
NO DATA), OLDEST WAITING and WHO WAITS. The per-stage dashboards (TRANSCRIBE and the rest) stay
the drill-down. `stage::tests` fails while a case has no alert test, no tile, or a tile that
does not read each of its waiters.

## Invariants

1. **One read a pass, over the whole queue.** A stage reads its oldest arrival and count once a
   pass, over every item waiting, never from the pass's batch, whose order is the pass's own
   business. The read is the queue's own predicate with no limit, so it does no more work than
   the batch's read whenever fewer than a batch wait. No per-item series.
2. **A time, not an age.** The server exports when the oldest item arrived, never how long it
   has waited, because a broken stage's pass is long (the transcriber's: 20 notes, 600 s each)
   and an age set per pass would stand still for all of it. Prometheus ages it.
3. **Nothing waiting reads 0; a stage that never reported reads no data.** A switched-off stage
   is never OK on the dashboard and never fires.
4. **Fresh within an attempt.** The transcriber reads its backlog at a pass's start, before
   anything else the pass can fail on, and again after each attempt it keeps, so neither a
   failing pass nor a long one leaves an age standing from a note already heard. A pass that
   fails on storage leaves the last reading; `TranscriberFailing` says so.
5. **Closed labels.** `stage`, `objective`, `result` and the lane's `kind` come from closed
   enums; no label names an item or a person.
6. **The bound is policy.** It is a constant in the source, moved by a call, and reaches the
   alerts as `stage_promise_seconds`; it is never written into a rule.

## The cases to come

Declared, not built; each is a new case of `Stage` with its own promise:

- **The render lane**: the oldest render job waiting, from the render worker's status folder;
  its waiters the essays and weeks behind it.
- **Kokoro's voicing**: the oldest line waiting to be voiced; its waiters the renders that need it.
- **The mailbox**: the oldest item the Worker holds that home has not filed, its promise the poll
  interval and budget; its waiters every routine that reads home's copy.

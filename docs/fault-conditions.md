# Fault conditions (#213)

## The question, and why it needed a strict definition

"Is my server in a fault state" doesn't compile as a dashboard panel until
it's broken into predicates — each one either true or not, each separately
observable and separately falsifiable. Before this epic, the only thing
claiming to answer this question was `/health`, and it answered with a
string literal that consulted nothing. A service in any of the six states
below looked identical to a healthy one.

**Resource pressure is not a fault state.** High CPU, high RSS, and a
growing disk footprint are *sizing* facts, not fault conditions — a service
at 90% of its memory limit and serving every request correctly is not
faulted, it is expensive. The only overlap with sizing is condition #4
below, and only where saturation is *observed as refused work*, never
inferred from a resource ceiling.

## The six conditions

### 1. Unreachable {#unreachable}

`up{job=...} == 0`. The process is gone or the scrape path broke.
Distinguishable from every other row *only* because the others require a
live scrape to even be false — this is the one that fires when nothing else
can be evaluated at all.

### 2. Dependency down {#dependency-down}

`/ready` non-200, per-dependency gauge. SQLite pool exhausted, NATS
disconnected, Redis unreachable — before #221 (F1), all three were invisible
until a request happened to fail. `handlers::readiness` checks each with a
bounded timeout and exports `dependency_up{dependency="sqlite"|"nats"|"redis"}`.

### 3. Rejecting {#rejecting}

Elevated 5xx rate. `main.rs`'s tower stack already sheds and times out
(`LoadShedLayer`, `TimeoutLayer`) — before #222 (F2), both outcomes were
indistinguishable from success in every aggregate that existed.
`http_requests_total{status=~"5.."}` is what makes them visible.

### 4. Saturated {#saturated}

Admission-control refusals: load shedding, WS `ConnectionGuard` 503s,
permit-acquire timeouts. This is the fault state that *looks* healthiest
from every other angle — every request that *is* served stays fast, because
the slow ones were never admitted. `refusals_total{stage,reason}` (#223/F3)
is the only place this was ever counted.

### 5. Stalled {#stalled}

Age of the last successful background pass exceeds 3× its interval. The
`nudge` waker (see the [PUSH] epic, #236) is designed so a quiet day and a
dead loop produce byte-identical output: nothing. A last-success timestamp
gauge turns "nothing happened" into a comparison against a known interval
instead of a silence nobody can distinguish from correctness.

### 6. Blind {#blind}

A series the dashboard depends on has no data. Established by the [GLANCE]
epic (#212, `docs/dashboard-honesty.md`). Listed here because being unable
to evaluate conditions #1–#5 is itself a fault state — and, before #212, was
the one this repo was actually in. A monitoring system that cannot
distinguish "nothing is wrong" from "I cannot tell" is not a monitoring
system.

## Where this is enforced

The HEALTH row at the top of `infra/grafana/dashboards/dashboard.jsonnet`
(#225/F5) renders all six as one row of stat panels: UP (#1), DEPS (#2),
ERRORS (#3), REFUSALS (#4), LOOPS (#5), SIGNAL (#6 — red the instant any of
the other five goes grey, so "five greens" and "five greys" can never be
mistaken for each other at a glance). Each panel links back to its
condition's anchor on this page.

//! `refusals_total{stage,reason}` — see #213 (F3).
//!
//! This server declines work in several distinct ways, and refusal is fault
//! condition #4 — **saturated** — the one that looks healthiest from every
//! other angle: every request that *is* served stays fast, because the slow
//! ones were never admitted. Before this, the only evidence was a `warn!`
//! or `error!` line nobody was watching. These counters sit next to those
//! log lines, not in place of them — the log says what happened, the
//! metric says when to look.
//!
//! `stage` is `"http"` or `"ws"`. `reason` is one of:
//!
//!   http: `timeout`, `load_shed`, `body_limit`
//!   ws:   `global_capacity`, `queue_full`, `permit_timeout`
//!
//! One row from #213's refusal table is deliberately not counted here:
//!
//! - **Concurrency limit** (`tower::limit::ConcurrencyLimitLayer`) — in the
//!   tower 0.4 this workspace pins, `ConcurrencyLimit::poll_ready` never
//!   produces an error; it is pure backpressure (`Poll::Pending` on a
//!   semaphore) with no discrete "refused" event to count. When sustained
//!   concurrency pressure does turn into an actual refusal, that refusal
//!   happens at a layer this module *does* count — `load_shed` if
//!   `LoadShedLayer` sheds it, `timeout` if `TimeoutLayer` elapses first.
//!   Inventing a synthetic "queued" counter here would mean fabricating a
//!   signal tower itself doesn't produce, which is exactly what this story
//!   (and #223's "observe first" non-goal) argues against.

use metrics::counter;

pub fn record_http(reason: &'static str) {
	counter!("refusals_total", "stage" => "http", "reason" => reason).increment(1);
}

pub fn record_ws(reason: &'static str) {
	counter!("refusals_total", "stage" => "ws", "reason" => reason).increment(1);
}

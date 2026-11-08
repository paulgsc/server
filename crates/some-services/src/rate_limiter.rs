//! A per-client rate limiter built on GCRA, the generic cell rate algorithm.
//!
//! # Why this shape
//!
//! The token bucket this replaces kept a token count and a "last refill"
//! timestamp, and on every read (a request, or the 10 s gauge sweep) added
//! `floor(elapsed * rate)` tokens and moved the timestamp to `now`. The floor
//! threw away the remainder every time, so how many tokens a client got back
//! depended on how often anything *looked* at its bucket. A phone sending
//! every 1-2 s at 60/min got one token back per request and never recovered
//! from a burst: refused at well under its limit, and only for that traffic
//! shape. That is a correctness property of the state representation, not
//! of one line, so the state changed.
//!
//! GCRA keeps one number per client: the *theoretical arrival time* (TAT),
//! the moment the client's bucket would be full again if it sent nothing
//! more. Each allowed request pushes it one emission interval (`period /
//! limit`) further out; a request is refused if accepting it would push the
//! TAT more than `burst` intervals past now. Everything a token bucket says
//! is derivable from it (`tokens available = burst - (TAT - now) / interval`),
//! but:
//!
//! - **Time is continuous.** Nothing is rounded into state, so nothing is lost
//!   between reads, however they interleave. Rounding happens only when
//!   *reporting* (the gauge's available count), never when *deciding*.
//! - **Reads are pure.** [`KeyedLimiter::snapshot`] and the eviction sweep
//!   compute from the TAT without writing it, so observing the limiter cannot
//!   change what it decides (the property the old one broke, and the one its
//!   tests now pin: `reading_the_limiter_never_changes_a_decision`).
//! - **A refusal knows exactly when to come back.** The excess over the
//!   tolerance *is* the wait, so a 429 carries an exact `Retry-After`.
//! - **One write per decision, under the key's shard lock** (`DashMap`'s
//!   entry API), instead of compare-and-swap loops that gave up and refused
//!   after ten collisions: contention can no longer turn into a refusal.
//! - **A monotonic, injected clock** ([`Clock`]). Wall-clock time can step
//!   backwards; `Instant` cannot. And because the clock is a parameter, the
//!   tests drive hours of traffic in microseconds, exactly, with no sleeps
//!   and no timing slack.
//!
//! The same algorithm backs `governor` and `redis-cell`; it is written out
//! here rather than imported because it is about forty lines, and its tests
//! are the point.
//!
//! # Invariants
//!
//! Each is pinned by a test in `keyed.rs`, on a manual clock:
//!
//! 1. **A client under its quota is never refused**, whatever the shape of its
//!    traffic (`a_client_under_its_limit_is_never_refused`).
//! 2. **Reading never changes a decision.** Snapshots and eviction only read,
//!    or forget clients whose next decision is the same either way
//!    (`reading_the_limiter_never_changes_a_decision`).
//! 3. **Decisions are an exact token bucket's**: no rounding, no drift
//!    (`decides_exactly_as_an_exact_token_bucket_would`).
//! 4. **Contention admits exactly the burst**, never more and never a refusal
//!    a client didn't earn (`contention_admits_exactly_the_burst`).
//! 5. **State is bounded** by the clients that sent within the last
//!    `burst × interval`, and a refusal writes nothing.
//!
//! A change that needs to break one of these is a design change: raise it,
//! don't patch around the test.

mod clock;
mod gcra;
mod keyed;

pub use clock::{Clock, ManualClock, MonotonicClock};
pub use gcra::{Decision, Quota};
pub use keyed::{KeyedLimiter, Snapshot};

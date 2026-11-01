//! The study nudge: deciding when to interrupt someone, and doing it.
//!
//! Read the modules in this order — each depends only on the ones above it:
//!
//! | Module      | Question it answers                                       |
//! |-------------|-----------------------------------------------------------|
//! | [`clock`]   | What does "today" mean, and whose clock decides?           |
//! | [`presence`]| Is someone looking at the dashboard right now?             |
//! | [`policy`]  | Given all that, should this person be interrupted?         |
//! | [`payload`] | What exactly does `sw.js` need to receive?                 |
//! | [`vapid`]   | Which sender is this, and how is its identity kept?        |
//! | [`sender`]  | How does an encrypted, signed push actually go out?        |
//! | [`tick`]    | What wakes any of this up?                                 |
//!
//! [`policy`] is pure. Everything impure is in [`sender`] and [`tick`], which
//! is the same split the client made between `study-nudge/index.ts` and
//! `study-nudge/service-worker.ts`, and for the same reason: the interesting
//! question — when is an interruption *welcome* — should be decided by
//! something testable at any hour of any day.
//!
//! `docs/study-nudge.md` covers setup, migrations, key custody, and what this
//! still cannot know.

pub mod clock;
pub mod constraints;
pub mod payload;
pub mod presence;
pub mod waker;

use std::time::Duration;

// The nudge's engineering values: source constants, not variables (#617).
// Each was its variable's default; changing one is a PR with its reason.

/// The VAPID `sub` claim: a `mailto:` identifying the sender. Push services
/// reject or deprioritise senders without one. One operator, one contact
/// (`VAPID_SUBJECT` was this by default).
pub const VAPID_SUBJECT: &str = "mailto:admin@maishatu.com";

/// How often the waker picks up work the engine already decided.
///
/// Not a schedule. Eligibility instants are solved when signals arrive and
/// written to an index; this is the resolution at which they are noticed,
/// so shortening it lands interventions closer to their computed instant
/// without changing which ones happen (`NUDGE_WAKER_SECONDS`).
pub const WAKER_INTERVAL: Duration = Duration::from_secs(300);

/// How long one push delivery may take before it counts as a failure (#264,
/// SLI3).
///
/// A push service that accepts answers in well under a second; one that has
/// not answered in ten is not going to, and every second past that is a
/// second the rest of the batch waits in the serial loop. A timed-out
/// delivery is recorded as a failure, never a prune, never `Accepted`, and
/// does not hand the claim back: see `waker::actuate`
/// (`PUSH_DELIVERY_TIMEOUT_MS`).
pub const DELIVERY_TIMEOUT: Duration = Duration::from_millis(10_000);

/// How long one waker pass may run before it stops taking on new subjects
/// (#264, SLI3).
///
/// Two minutes: well inside [`WAKER_INTERVAL`], so a pass that hits it still
/// leaves the next tick to start on time, and long enough for a full
/// `BATCH` of 32 subjects at typical sub-second delivery latency to finish
/// many times over. A pass that runs out starts no further subject and
/// tries no further device, but never cancels a subject part-way; subjects
/// it did not reach are due on the next pass (`WAKER_PASS_DEADLINE_MS`).
pub const PASS_DEADLINE: Duration = Duration::from_millis(120_000);

const _: () = assert!(PASS_DEADLINE.as_secs() < WAKER_INTERVAL.as_secs(), "a pass must end before the next tick");

/// How long a client-written presence lease stays fresh before a due
/// subject's notification is sent as though nobody were looking. The client
/// renews roughly every 45s while visible, so this absorbs timer throttling
/// and scheduling jitter (`NUDGE_PRESENCE_LEASE_TTL_SECONDS`).
pub const PRESENCE_LEASE_TTL: Duration = Duration::from_secs(75);

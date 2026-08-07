//! When should a system interrupt someone, and why?
//!
//! This crate answers that without knowing what the system is *about*. It has
//! no vocabulary for lessons, sessions, or notifications; it does not read a
//! database, speak HTTP, or own a runtime. What it owns is the shape of the
//! question.
//!
//! ## Warrant, admissibility, actuation
//!
//! The mistake this crate exists to avoid is fusing three different questions
//! into one predicate:
//!
//! | | Question | Lives in |
//! |---|---|---|
//! | **Warrant** | Why intervene at all? | [`Charge`] crossing a threshold |
//! | **Admissibility** | May we, *now*, on this channel? | [`Admissibility`] |
//! | **Actuation** | How does it physically go out? | Not here. `push_kit`. |
//!
//! A design with no warrant layer has to borrow one, and the usual loan is a
//! cron: something must make the call happen, so the clock does. The tell is a
//! policy whose every guard answers *may we* — quiet hours, cooldown, presence
//! — and none answers *why*. Time then becomes the cause of interventions
//! rather than a constraint on them, and the system can express "it is 19:00"
//! but not "they abandoned a session and twenty minutes have passed".
//!
//! ## The battery, and why it is not a rate limiter
//!
//! Engagement is modelled as [`Charge`]: a vector of levels, one per
//! [`Domain::Class`], that **decays with time and is restored by signals**.
//! Decay *is* inactivity — no separate "days since last seen" counter, because
//! the absence of signals is already the drain. Discrete setbacks (an abandoned
//! session, a poor score, content going stale under someone) drain further; wins
//! recharge.
//!
//! An intervention is warranted when the weighted aggregate falls to
//! [`Calibration::THRESHOLD`].
//!
//! The analogy stops at the leaky bucket. **Nothing here is polled.** Decay is
//! closed-form, so a level is computed on read, never ticked. And because the
//! aggregate is monotonically decreasing between signals, the instant it will
//! cross the threshold is *solvable* — see [`Charge::eligible_at`]. That
//! instant is stored, which turns the waker into an indexed
//! `WHERE eligible_at <= now` query that **discovers** subjects the arithmetic
//! already marked, rather than a scheduler that wakes up and decides. Work is
//! O(1) per signal and zero per idle subject.
//!
//! ## Two axes, deliberately independent
//!
//! The engine is generic over a [`Domain`] — the closed set of signal classes
//! and actions a *particular release* ships. Product vocabulary changes every
//! sprint; the structure above does not. A new user story is a new variant in
//! the domain crate, not a change here.
//!
//! There is no `dyn` anywhere in this crate, and no strings standing in for
//! types. A deployed binary picks one domain (`type Deployed = StudyV1;`),
//! which makes its world closed: matches are exhaustive, the weight table is a
//! total function the compiler checks, and unreachable arms are eliminated.
//!
//! The one place a closed world meets an open one is durable state, and it is
//! handled explicitly rather than by `dyn`: [`Enumerable::from_discriminant`]
//! is a *total* parse, so a row written by release N naming a class release
//! N+1 no longer has is quarantined at the boundary instead of panicking or
//! being silently reinterpreted.

pub mod charge;
pub mod engine;

pub use charge::{Charge, Deficit};
pub use engine::{Admissibility, Calibration, Domain, Engine, Enumerable, Selector, Signal, Verdict};

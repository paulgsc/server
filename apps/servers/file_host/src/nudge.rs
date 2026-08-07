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

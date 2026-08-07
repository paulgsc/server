//! Study sessions as server state.
//!
//! [`SessionRecord`] mirrors the TypeScript type of the same name in
//! `apps/www/src/lib/tenant/types.ts` (`paulgsc/some-ui`) field for field,
//! including its JSON casing, so a browser repository can post what it already
//! has and read back what it already understands.
//!
//! Nothing here validates `activities` or `scenes`: the client owns their
//! meaning and this crate owns their durability. The one exception is
//! [`total_duration_of`], which reads two numeric fields out of each scene
//! because the server, having taken ownership of the record, has to compute the
//! duration the client used to compute for it.
//!
//! Crate layout and `SQLite` habits follow `capture_repo`. Applying the
//! migrations is documented in `docs/study-nudge.md`.

pub mod model;
pub mod repository;

pub use model::{total_duration_of, CreateSession, LayoutMode, SessionRecord, SessionStatus, UpdateSession};
pub use repository::SessionRepository;

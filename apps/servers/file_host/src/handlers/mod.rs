//! HTTP handlers.
//!
//! A failure on a request is answered as that failure (ER1 in `crate::error`),
//! so neither a `#[must_use]` result dropped with `let _ =` nor a `_ =>` arm
//! that folds a new variant into an old answer is allowed here.
#![warn(clippy::let_underscore_must_use, clippy::wildcard_enum_match_arm)]

pub mod admin_settings;
pub mod aph;
pub mod app_events;
pub mod auth;
pub mod call_videos;
pub mod db;
pub mod feed_videos;
pub mod health;
pub mod ideas;
pub mod jobs;
pub mod korean;
pub mod leetype_files;
pub mod leetype_log;
pub mod outcomes;
pub mod pipeline;
pub mod presence;
pub mod push;
pub mod readiness;
pub mod routine;
pub mod rust_rounds;
pub mod shelf;
pub mod signals;
pub mod studio_log;
pub mod studio_videos;
pub mod study_note;
pub mod subjects;
pub mod sync;
pub mod tab_metadata;
pub mod utterance;
pub mod voice;

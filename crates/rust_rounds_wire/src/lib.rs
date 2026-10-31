//! The half of Rust Rounds' runner a client links (paulgsc/some-ui#2234,
//! RR3): what the `rust-runner` service is asked and answers over its Unix
//! socket ([`protocol`]), what a recording holds ([`recording`], keyed by
//! [`recording::key`]) and rustc's diagnostics as recorded ([`diagnostic`]).
//!
//! **It starts no process and compiles nothing.** `rust_runner` (the
//! service, and `LeetType`'s recorder through `leetype_runner`) depends on
//! this crate and re-exports it, so both sides speak one definition; a
//! client (`file_host`'s recording pass) depends on this crate alone, and
//! never reaches the code that compiles and runs. EX1 and RX1–RX4 in
//! `apps/servers/file_host/docs/leetype-execution.md` say why.

pub mod diagnostic;
pub mod protocol;
pub mod recording;

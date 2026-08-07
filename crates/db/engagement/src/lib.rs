//! Durable engagement state.
//!
//! Storage only: this crate has no opinion about what a charge means or when a
//! subject should be interrupted. It stores `(class, level, as_of)` triples and
//! a solved `eligible_at`, and it runs the one query the waker needs.
//!
//! It is deliberately generic over neither the domain nor the engine — a
//! `u16` discriminant and an `f64` are the whole vocabulary at this layer, and
//! parsing the discriminant back into a class is the caller's job precisely so
//! the quarantine arm stays in the caller's control.

pub mod repository;

pub use repository::{ChargeRow, EngagementRepository, GateRow};

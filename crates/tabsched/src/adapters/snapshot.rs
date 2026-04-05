use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::core::state::State;
use crate::domain::{
	ids::{ResourceId, SlotIndex, TrackId},
	session::{Outcome, Session},
	topology::Topology,
};

// ── Wire types ─────────────────────────────────────────────────────────────
//
// These are 1:1 with domain types but derive Serialize/Deserialize so the
// domain types stay clean. A thin `From` conversion bridges the two.

#[derive(Serialize, Deserialize)]
pub struct SessionDto {
	pub slot_index: u64,
	pub track: u32,
	pub resource: u32,
	pub outcome: OutcomeDto,
}

#[derive(Serialize, Deserialize)]
pub enum OutcomeDto {
	Unrecorded,
	Progress,
	Stuck,
	Review,
}

#[derive(Serialize, Deserialize)]
pub struct StateSnapshot {
	pub window_size: usize,
	pub history: Vec<SessionDto>,
	/// Cursor positions per leaf track (track_id -> position).
	pub cursors: HashMap<u32, u64>,
}

// ── Conversions ────────────────────────────────────────────────────────────

impl From<Outcome> for OutcomeDto {
	fn from(o: Outcome) -> Self {
		match o {
			Outcome::Unrecorded => OutcomeDto::Unrecorded,
			Outcome::Progress => OutcomeDto::Progress,
			Outcome::Stuck => OutcomeDto::Stuck,
			Outcome::Review => OutcomeDto::Review,
		}
	}
}

impl From<OutcomeDto> for Outcome {
	fn from(o: OutcomeDto) -> Self {
		match o {
			OutcomeDto::Unrecorded => Outcome::Unrecorded,
			OutcomeDto::Progress => Outcome::Progress,
			OutcomeDto::Stuck => Outcome::Stuck,
			OutcomeDto::Review => Outcome::Review,
		}
	}
}

impl From<&Session> for SessionDto {
	fn from(s: &Session) -> Self {
		SessionDto {
			slot_index: s.slot_index.as_u64(),
			track: s.track.0,
			resource: s.resource.0,
			outcome: s.outcome.into(),
		}
	}
}

impl From<SessionDto> for Session {
	fn from(d: SessionDto) -> Self {
		Session {
			slot_index: SlotIndex(d.slot_index),
			track: TrackId(d.track),
			resource: ResourceId(d.resource),
			outcome: d.outcome.into(),
		}
	}
}

// ── Persistence ────────────────────────────────────────────────────────────

/// Save a `State` snapshot to a JSON file.
pub fn save(state: &State, path: &Path) -> Result<(), SnapshotError> {
	let cursors: HashMap<u32, u64> = state.cursors_raw().map(|(id, pos)| (id.0, pos)).collect();

	let snap = StateSnapshot {
		window_size: state.window_size(),
		history: state.history.iter().map(SessionDto::from).collect(),
		cursors,
	};

	let json = serde_json::to_string_pretty(&snap)?;
	std::fs::write(path, json)?;
	Ok(())
}

/// Load a `State` from a JSON snapshot, rehydrating from history.
///
/// Cursors stored in the snapshot are used as-is if present;
/// otherwise they are recomputed from history. Recomputing from
/// history is always safe; the stored cursor is an optimisation for
/// large histories.
pub fn load(path: &Path, topology: &Topology) -> Result<State, SnapshotError> {
	let json = std::fs::read_to_string(path)?;
	let snap: StateSnapshot = serde_json::from_str(&json)?;

	let history: Vec<Session> = snap.history.into_iter().map(Session::from).collect();
	Ok(State::from_history(history, topology, snap.window_size))
}

// ── Errors ─────────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum SnapshotError {
	#[error("IO error: {0}")]
	Io(#[from] std::io::Error),
	#[error("JSON error: {0}")]
	Json(#[from] serde_json::Error),
}

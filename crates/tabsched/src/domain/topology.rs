use std::collections::{HashMap, HashSet};

use crate::domain::{
	ids::{ResourceId, TrackId},
	resource::Resource,
	track::{Track, TrackKind},
};

/// A validated, immutable tree of tracks and the resources they reference.
#[derive(Debug, Clone)]
pub struct Topology {
	tracks: HashMap<TrackId, Track>,
	resources: HashMap<ResourceId, Resource>,
	root: TrackId,
}

impl Topology {
	pub fn new(tracks: Vec<Track>, resources: Vec<Resource>) -> Result<Self, TopologyError> {
		Self::build(tracks, resources, false)
	}

	pub fn new_strict(tracks: Vec<Track>, resources: Vec<Resource>) -> Result<Self, TopologyError> {
		Self::build(tracks, resources, true)
	}

	fn build(tracks: Vec<Track>, resources: Vec<Resource>, strict_targets: bool) -> Result<Self, TopologyError> {
		if tracks.is_empty() {
			return Err(TopologyError::Empty);
		}

		// --- maps ---
		let track_map: HashMap<TrackId, Track> = tracks.into_iter().map(|t| (t.id(), t)).collect();

		let resource_map: HashMap<ResourceId, Resource> = resources.into_iter().map(|r| (r.id, r)).collect();

		// --- single root ---
		let roots: Vec<TrackId> = track_map.values().filter(|t| t.parent().is_none()).map(|t| t.id()).collect();

		if roots.len() != 1 {
			return Err(TopologyError::RootCount(roots.len()));
		}

		let root = roots[0];

		// --- parent/child consistency ---
		for track in track_map.values() {
			for &child_id in track.children() {
				let child = track_map.get(&child_id).ok_or(TopologyError::MissingTrack(child_id))?;

				if child.parent() != Some(track.id()) {
					return Err(TopologyError::ParentMismatch {
						child: child_id,
						declared_parent: child.parent(),
						actual_parent: track.id(),
					});
				}
			}
		}

		// --- no cycles ---
		detect_cycle(&track_map, root)?;

		// --- all resources exist ---
		for track in track_map.values() {
			for &rid in track.resources() {
				if !resource_map.contains_key(&rid) {
					return Err(TopologyError::MissingResource(rid));
				}
			}
		}

		// --- target sum (strict mode) ---
		if strict_targets {
			for track in track_map.values() {
				if let TrackKind::Internal { children } = track.kind() {
					let child_sum: u32 = children.iter().map(|id| track_map.get(id).expect("validated above").base_target()).sum();

					if child_sum != track.base_target() {
						return Err(TopologyError::TargetSumMismatch {
							track: track.id(),
							declared: track.base_target(),
							children_sum: child_sum,
						});
					}
				}
			}
		}

		Ok(Self {
			tracks: track_map,
			resources: resource_map,
			root,
		})
	}

	// ── accessors ─────────────────────────────────────────────────────────

	pub fn root(&self) -> TrackId {
		self.root
	}

	pub fn track(&self, id: TrackId) -> Option<&Track> {
		self.tracks.get(&id)
	}

	pub fn resource(&self, id: ResourceId) -> Option<&Resource> {
		self.resources.get(&id)
	}

	pub fn tracks(&self) -> impl Iterator<Item = &Track> {
		self.tracks.values()
	}

	pub fn leaf_tracks(&self) -> impl Iterator<Item = &Track> {
		self.tracks.values().filter(|t| t.is_leaf())
	}
}

// ── cycle detection ────────────────────────────────────────────────────────

fn detect_cycle(tracks: &HashMap<TrackId, Track>, root: TrackId) -> Result<(), TopologyError> {
	let mut visited = HashSet::new();
	let mut stack = vec![root];

	while let Some(id) = stack.pop() {
		if !visited.insert(id) {
			return Err(TopologyError::Cycle(id));
		}

		for &child in tracks.get(&id).expect("validated root exists").children() {
			stack.push(child);
		}
	}

	Ok(())
}

// ── errors ─────────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum TopologyError {
	#[error("topology must have at least one track")]
	Empty,

	#[error("expected exactly 1 root, found {0}")]
	RootCount(usize),

	#[error("track {0} referenced but not defined")]
	MissingTrack(TrackId),

	#[error("resource {0} referenced but not defined")]
	MissingResource(ResourceId),

	#[error("child {child} declares parent {declared_parent:?} but is listed under {actual_parent}")]
	ParentMismatch {
		child: TrackId,
		declared_parent: Option<TrackId>,
		actual_parent: TrackId,
	},

	#[error("cycle detected at {0}")]
	Cycle(TrackId),

	#[error("track {track}: declared target {declared} != children sum {children_sum}")]
	TargetSumMismatch { track: TrackId, declared: u32, children_sum: u32 },
}

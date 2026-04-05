//! Translate `Config` → validated `Topology`.
//!
//! Responsibilities:
//! - Assign stable numeric IDs to labels (position in config = ID).
//! - Resolve parent label strings to `TrackId`.
//! - Compute children lists from parent back-references (two-pass).
//! - Call `Topology::new` and surface any remaining invariant failures.
//!
//! The resulting `Topology` is immutable. The `LabelIndex` is kept
//! alongside it so the CLI can resolve IDs back to labels for display.

//! Translate `Config` → validated `Topology`.

use std::{collections::HashMap, num::NonZeroU32};

use anyhow::{bail, Context};
use tabsched::{Resource, ResourceId, Topology, Track, TrackId};

use super::schema::Config;

// ── Label index ─────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct LabelIndex {
	track_labels: HashMap<TrackId, String>,
	resource_labels: HashMap<ResourceId, String>,
	#[allow(dead_code)]
	track_resources: HashMap<TrackId, Vec<ResourceId>>,
}

impl LabelIndex {
	pub fn track_label(&self, id: TrackId) -> &str {
		self.track_labels.get(&id).map(String::as_str).unwrap_or("?")
	}

	pub fn resource_label(&self, id: ResourceId) -> &str {
		self.resource_labels.get(&id).map(String::as_str).unwrap_or("?")
	}

	#[allow(dead_code)]
	pub fn track_resources(&self, id: TrackId) -> &[ResourceId] {
		self.track_resources.get(&id).map(Vec::as_slice).unwrap_or(&[])
	}
}

// ── Builder ─────────────────────────────────────────────────────────────

pub fn build(config: &Config) -> anyhow::Result<(Topology, LabelIndex)> {
	// ── Pass 1: assign IDs ───────────────────────────────────────────────

	let mut label_to_tid: HashMap<&str, TrackId> = HashMap::new();
	for (i, tc) in config.tracks.iter().enumerate() {
		if label_to_tid.insert(tc.label.as_str(), TrackId(i as u32)).is_some() {
			bail!("duplicate track label: {:?}", tc.label);
		}
	}

	let mut label_to_rid: HashMap<&str, ResourceId> = HashMap::new();
	let mut next_rid = 0u32;

	for tc in &config.tracks {
		for rlabel in &tc.resources {
			label_to_rid.entry(rlabel.as_str()).or_insert_with(|| {
				let rid = ResourceId(next_rid);
				next_rid += 1;
				rid
			});
		}
	}

	// ── Pass 2: parent/child relations ───────────────────────────────────

	let mut parent_of: HashMap<TrackId, TrackId> = HashMap::new();
	let mut children_of: HashMap<TrackId, Vec<TrackId>> = HashMap::new();

	for tc in &config.tracks {
		let tid = label_to_tid[tc.label.as_str()];

		if let Some(p_label) = &tc.parent {
			let parent_tid = *label_to_tid
				.get(p_label.as_str())
				.with_context(|| format!("track {:?}: unknown parent {:?}", tc.label, p_label))?;

			parent_of.insert(tid, parent_tid);
			children_of.entry(parent_tid).or_default().push(tid);
		}
	}

	let roots: Vec<TrackId> = config
		.tracks
		.iter()
		.map(|tc| label_to_tid[tc.label.as_str()])
		.filter(|tid| !parent_of.contains_key(tid))
		.collect();

	if roots.len() != 1 {
		bail!("expected exactly 1 root track (no parent field), found {}", roots.len());
	}

	// ── Pass 3: construct domain objects ─────────────────────────────────

	let mut tracks = Vec::with_capacity(config.tracks.len());
	let mut track_labels = HashMap::new();
	let mut track_resources_map = HashMap::new();
	let mut resource_labels = HashMap::new();

	for tc in &config.tracks {
		let tid = label_to_tid[tc.label.as_str()];
		let parent = parent_of.get(&tid).copied();

		track_labels.insert(tid, tc.label.clone());

		let base_target = NonZeroU32::new(tc.target).with_context(|| format!("track {:?}: target must be > 0", tc.label))?;

		let track = if tc.resources.is_empty() {
			// Internal
			let children = children_of.get(&tid).cloned().unwrap_or_default();

			if children.is_empty() {
				bail!("track {:?} has no resources and no child tracks", tc.label);
			}

			Track::internal(tid, parent, &tc.label, base_target, children).with_context(|| format!("invalid internal track {:?}", tc.label))?
		} else {
			// Leaf
			if children_of.contains_key(&tid) {
				bail!("track {:?} has both resources and child tracks", tc.label);
			}

			let rids: Vec<ResourceId> = tc
				.resources
				.iter()
				.map(|r| {
					let rid = label_to_rid[r.as_str()];
					resource_labels.entry(rid).or_insert_with(|| r.clone());
					rid
				})
				.collect();

			track_resources_map.insert(tid, rids.clone());

			Track::leaf(tid, parent, &tc.label, base_target, rids).with_context(|| format!("invalid leaf track {:?}", tc.label))?
		};

		tracks.push(track);
	}

	let resources: Vec<Resource> = label_to_rid
		.iter()
		.map(|(&label, &rid)| {
			resource_labels.entry(rid).or_insert_with(|| label.to_owned());
			Resource::new(rid, label)
		})
		.collect();

	// ── Pass 4: topology validation ──────────────────────────────────────

	let topology = Topology::new(tracks, resources).context("topology validation failed")?;

	let index = LabelIndex {
		track_labels,
		resource_labels,
		track_resources: track_resources_map,
	};

	Ok((topology, index))
}

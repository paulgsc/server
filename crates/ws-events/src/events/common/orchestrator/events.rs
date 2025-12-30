use super::{LifetimeId, SceneId, TimeMs, UILayoutIntentData};
use serde::{Deserialize, Serialize};

/// A temporal event anchored to a specific time
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimedEvent<E> {
	pub at: TimeMs,
	pub event: E,
}

/// Core orchestrator event types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum OrchestratorEvent {
	/// Lifetime event (scenes, overlays, audio, etc.)
	Lifetime(LifetimeEvent),
	/// One-shot point event (metadata, callback, marker)
	Point(PointEvent),
}

/// Lifetime events (start/end pairs)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LifetimeEvent {
	Start { id: LifetimeId, kind: LifetimeKind },
	End { id: LifetimeId },
}

/// Different kinds of lifetimes (static sum type - no dynamic dispatch)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LifetimeKind {
	Scene(ScenePayload),
	// Future: add Overlay, Audio, Camera, etc. without changing engine
	// Overlay(OverlayPayload),
	// Audio(AudioPayload),
}

/// Scene lifetime payload
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScenePayload {
	pub scene_id: SceneId,
	pub scene_name: String,
	pub duration: TimeMs,
	pub ui: Vec<UILayoutIntentData>,
}

/// Point event payload
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PointEvent {
	pub key: String,
	pub value: serde_json::Value,
}

use crate::types::*;
use serde::{Deserialize, Serialize};

/// Events that can modify the timeline state at time t
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TimelineEvent {
	/// Start a new chapter
	StartChapter {
		uid: Uid,
		context: Context,
		start_time: Timestamp,
		payload: Payload,
	},

	/// End an active chapter
	EndChapter { uid: Uid, end_time: Timestamp, final_payload: Option<Payload> },

	/// Update payload of an existing chapter (retroactive updates)
	UpdatePayload { uid: Uid, payload: Payload },

	/// Update context/title of an existing chapter
	UpdateContext { uid: Uid, context: Context },

	/// Remove a chapter completely
	RemoveChapter { uid: Uid },

	/// Extend an active chapter's effective end time
	ExtendChapter { uid: Uid, extend_to: Timestamp },

	/// Mark a chapter as completed with final data
	CompleteChapter { uid: Uid, completion_time: Timestamp, final_payload: Payload },

	/// Clear all chapters (stream reset)
	ClearAll,
}

impl TimelineEvent {
	/// Get the UID associated with this event, if any
	pub fn uid(&self) -> Option<&str> {
		match self {
			TimelineEvent::StartChapter { uid, .. }
			| TimelineEvent::EndChapter { uid, .. }
			| TimelineEvent::UpdatePayload { uid, .. }
			| TimelineEvent::UpdateContext { uid, .. }
			| TimelineEvent::RemoveChapter { uid, .. }
			| TimelineEvent::ExtendChapter { uid, .. }
			| TimelineEvent::CompleteChapter { uid, .. } => Some(uid),
			_ => None,
		}
	}

	/// Get the primary timestamp associated with this event
	pub fn timestamp(&self) -> Option<Timestamp> {
		match self {
			TimelineEvent::StartChapter { start_time, .. } => Some(*start_time),
			TimelineEvent::EndChapter { end_time, .. } => Some(*end_time),
			TimelineEvent::ExtendChapter { extend_to, .. } => Some(*extend_to),
			TimelineEvent::CompleteChapter { completion_time, .. } => Some(*completion_time),
			_ => None,
		}
	}

	/// Check if this event creates a new chapter
	pub fn creates_chapter(&self) -> bool {
		matches!(self, TimelineEvent::StartChapter { .. })
	}

	/// Check if this event modifies existing chapter timing
	pub fn modifies_timing(&self) -> bool {
		matches!(
			self,
			TimelineEvent::EndChapter { .. } | TimelineEvent::ExtendChapter { .. } | TimelineEvent::CompleteChapter { .. }
		)
	}

	/// Check if this event is a retroactive update
	pub fn is_retroactive_update(&self) -> bool {
		matches!(self, TimelineEvent::UpdatePayload { .. } | TimelineEvent::UpdateContext { .. })
	}
}

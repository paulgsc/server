use crate::types::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TimelineEvent {
	ChapterObservation {
		uid: Uid,
		context: Context,
		time_range: TimeRange,
		payload: Payload,
	},

	CloseChapter {
		uid: Uid,
		end_time: Timestamp,
		final_payload: Option<Payload>,
	},

	RemoveChapter {
		uid: Uid,
	},

	UpdatePayload {
		uid: Uid,
		payload: Payload,
	},

	UpdateContext {
		uid: Uid,
		context: Context,
	},

	ExtendChapter {
		uid: Uid,
		extend_to: Timestamp,
	},

	ClearAll,

	AdvanceTime {
		current_time: Timestamp,
	},
}

impl TimelineEvent {
	pub fn uid(&self) -> Option<&str> {
		match self {
			Self::ChapterObservation { uid, .. }
			| Self::CloseChapter { uid, .. }
			| Self::RemoveChapter { uid, .. }
			| Self::UpdatePayload { uid, .. }
			| Self::UpdateContext { uid, .. }
			| Self::ExtendChapter { uid, .. } => Some(uid),
			_ => None,
		}
	}

	pub fn time_stamp(&self) -> Option<Timestamp> {
		match self {
			Self::ChapterObservation { time_range, .. } => Some(time_range.start),
			Self::CloseChapter { end_time, .. } => Some(*end_time),
			Self::ExtendChapter { extend_to, .. } => Some(*extend_to),
			Self::AdvanceTime { current_time, .. } => Some(*current_time),
			_ => None,
		}
	}

	pub fn modifies_timing(&self) -> bool {
		matches!(
			self,
			Self::ChapterObservation { .. } | Self::CloseChapter { .. } | Self::ExtendChapter { .. } | Self::AdvanceTime { .. }
		)
	}
}

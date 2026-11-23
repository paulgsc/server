//! What uses a take: the videos home holds whose audio plays it, for the
//! phone's badge on every take and its warning before a delete (INUSE,
//! paulgsc/server C1380).
//!
//! The type only: which videos play a take is the render passes' rule, so
//! `file_host`'s `take_uses` reads it with the very functions each pass
//! withdraws by. This crate only says how a use is written on the wire.

use serde::Serialize;

/// What kind of video uses a take. Closed: a new kind of video that plays
/// takes is a new variant, and every `match` on it says what it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum UseKind {
	/// A cards essay whose spoken cards play the take (`cards_essays`).
	Essay,
	/// A Rust Rounds or mock-interview week's essay video, the one the feed
	/// lists (`rust_essays`, `feeds`).
	FeedVideo,
	/// A Studio video queued on home whose newest DNA revision names the
	/// take (`studio_video_repo`, docs/identity.md invariant 36).
	StudioVideo,
}

impl UseKind {
	/// The wire name.
	#[must_use]
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::Essay => "essay",
			Self::FeedVideo => "feed-video",
			Self::StudioVideo => "studio-video",
		}
	}
}

/// One video that uses a take.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Use {
	/// What kind of video it is.
	pub kind: UseKind,
	/// The video's key in its own family: a week's id, an essay's key, a
	/// Studio video's id.
	pub key: String,
	/// What the phone shows for it.
	pub title: String,
	/// When home took it in, ISO-8601 UTC.
	pub at: String,
	/// Whether deleting the take makes home withdraw this video (by its
	/// pass's next tick, never inside the delete). False for a Studio video:
	/// nothing withdraws one, so it stays and loses the take.
	pub withdraws_on_delete: bool,
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn each_kind_is_written_as_its_wire_name() {
		for kind in [UseKind::Essay, UseKind::FeedVideo, UseKind::StudioVideo] {
			assert_eq!(serde_json::to_value(kind).unwrap(), serde_json::Value::from(kind.as_str()));
		}
	}

	#[test]
	fn a_use_is_camel_case() {
		let used = Use {
			kind: UseKind::StudioVideo,
			key: String::from("v1"),
			title: String::from("t"),
			at: String::from("2026-11-22T00:00:00.000Z"),
			withdraws_on_delete: false,
		};
		assert_eq!(
			serde_json::to_value(&used).unwrap(),
			serde_json::json!({ "kind": "studio-video", "key": "v1", "title": "t", "at": "2026-11-22T00:00:00.000Z", "withdrawsOnDelete": false })
		);
	}
}

#![cfg(feature = "events")]

use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Debug, Deserialize)]
pub struct NowPlaying {
	pub title: String,
	pub channel: String,
	pub video_id: String,
	pub current_time: u32,
	pub duration: u32,
	pub thumbnail: String,
}

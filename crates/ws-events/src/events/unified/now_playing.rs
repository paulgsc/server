#![cfg(feature = "events")]

use crate::events::NowPlaying;
use prost::Message;

#[derive(Clone, PartialEq, Message)]
pub struct TabMetaDataMessage {
	#[prost(string, tag = "1")]
	pub title: String,
	#[prost(string, tag = "2")]
	pub channel: String,
	#[prost(string, tag = "3")]
	pub video_id: String,
	#[prost(uint32, tag = "4")]
	pub current_time: u32,
	#[prost(uint32, tag = "5")]
	pub duration: u32,
	#[prost(string, tag = "6")]
	pub thumbnail: String,
}

impl TabMetaDataMessage {
	/// Create from NowPlaying
	pub fn from_now_playing(np: NowPlaying) -> Self {
		Self {
			title: np.title,
			channel: np.channel,
			video_id: np.video_id,
			current_time: np.current_time,
			duration: np.duration,
			thumbnail: np.thumbnail,
		}
	}

	/// Convert to NowPlaying
	pub fn to_now_playing(&self) -> NowPlaying {
		NowPlaying {
			title: self.title.clone(),
			channel: self.channel.clone(),
			video_id: self.video_id.clone(),
			current_time: self.current_time,
			duration: self.duration,
			thumbnail: self.thumbnail.clone(),
		}
	}
}

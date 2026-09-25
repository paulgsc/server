//! Periodic status snapshots, published as the `*Response` variants of
//! [`ObsEvent`].
//!
//! OBS pushes state changes as events; polling adds what events don't carry
//! (running timecodes, stats) and a full snapshot on every connect, since each
//! interval's first tick fires immediately.

use crate::types::{
	CurrentCollectionData, CurrentProfileData, CurrentSceneData, CurrentTransitionData, InputInfo, InputListData, ObsEvent, ObsStats, ProfileListData, RecordingStatusData,
	ReplayBufferStatusData, SceneCollectionListData, SceneInfo, SceneListData, StatsData, StreamStatusData, StudioModeData, TransitionInfo, TransitionListData, VersionData,
	VirtualCamStatusData,
};
use obws::Client;
use std::fmt::Write as _;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast;
use tokio::time::{interval, MissedTickBehavior};

const HIGH_FREQUENCY: Duration = Duration::from_secs(1);
const MEDIUM_FREQUENCY: Duration = Duration::from_secs(5);
const LOW_FREQUENCY: Duration = Duration::from_secs(30);

/// A status query that can be polled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PollTarget {
	StreamStatus,
	RecordStatus,
	CurrentProgramScene,
	SceneList,
	StudioMode,
	Stats,
	CurrentTransition,
	TransitionList,
	InputList,
	ProfileList,
	CurrentProfile,
	SceneCollectionList,
	CurrentSceneCollection,
	Version,
	VirtualCamStatus,
	/// OBS rejects this while the replay buffer is disabled in its settings.
	ReplayBufferStatus,
}

/// Which targets to poll at each interval: `high` every second, `medium` every
/// five, `low` every thirty.
#[derive(Debug, Clone)]
pub struct PollingConfig {
	pub high: Vec<PollTarget>,
	pub medium: Vec<PollTarget>,
	pub low: Vec<PollTarget>,
}

impl PollingConfig {
	/// Polls nothing; only OBS's pushed events are published.
	#[must_use]
	pub const fn none() -> Self {
		Self {
			high: Vec::new(),
			medium: Vec::new(),
			low: Vec::new(),
		}
	}
}

impl Default for PollingConfig {
	fn default() -> Self {
		Self {
			high: vec![PollTarget::StreamStatus, PollTarget::RecordStatus, PollTarget::CurrentProgramScene],
			medium: vec![PollTarget::SceneList, PollTarget::StudioMode, PollTarget::Stats],
			low: vec![
				PollTarget::CurrentTransition,
				PollTarget::InputList,
				PollTarget::ProfileList,
				PollTarget::CurrentProfile,
				PollTarget::SceneCollectionList,
				PollTarget::CurrentSceneCollection,
				PollTarget::TransitionList,
				PollTarget::Version,
			],
		}
	}
}

/// Polls until aborted, publishing each result. A failed query is logged and
/// skipped; the connection's own event pump owns disconnect detection.
pub async fn run(client: Arc<Client>, config: PollingConfig, events: broadcast::Sender<ObsEvent>) {
	let mut high = interval(HIGH_FREQUENCY);
	let mut medium = interval(MEDIUM_FREQUENCY);
	let mut low = interval(LOW_FREQUENCY);
	for timer in [&mut high, &mut medium, &mut low] {
		timer.set_missed_tick_behavior(MissedTickBehavior::Skip);
	}

	loop {
		let targets = tokio::select! {
			_ = high.tick() => &config.high,
			_ = medium.tick() => &config.medium,
			_ = low.tick() => &config.low,
		};
		for &target in targets {
			match poll(&client, target).await {
				// No subscribers is not an error; nobody is listening yet.
				Ok(event) => drop(events.send(event)),
				Err(e) => tracing::debug!(?target, error = %e, "poll failed"),
			}
		}
	}
}

async fn poll(client: &Client, target: PollTarget) -> Result<ObsEvent, obws::error::Error> {
	Ok(match target {
		PollTarget::StreamStatus => {
			let status = client.streaming().status().await?;
			ObsEvent::StreamStatusResponse(StreamStatusData {
				streaming: status.active,
				timecode: timecode(Duration::try_from(status.timecode).unwrap_or_default()),
			})
		}
		PollTarget::RecordStatus => {
			let status = client.recording().status().await?;
			ObsEvent::RecordingStatusResponse(RecordingStatusData {
				recording: status.active,
				timecode: timecode(Duration::try_from(status.timecode).unwrap_or_default()),
			})
		}
		PollTarget::CurrentProgramScene => ObsEvent::CurrentSceneResponse(CurrentSceneData {
			scene_name: client.scenes().current_program_scene().await?.id.name,
		}),
		PollTarget::SceneList => scene_list(client).await?,
		PollTarget::StudioMode => ObsEvent::StudioModeResponse(StudioModeData {
			enabled: client.ui().studio_mode_enabled().await?,
		}),
		PollTarget::Stats => stats(client).await?,
		PollTarget::CurrentTransition => {
			let transition = client.transitions().current().await?;
			ObsEvent::CurrentTransitionResponse(CurrentTransitionData {
				transition_name: transition.id.name,
				transition_duration: transition.duration.map_or(0, |d| u32::try_from(d.whole_milliseconds()).unwrap_or(u32::MAX)),
			})
		}
		PollTarget::TransitionList => transition_list(client).await?,
		PollTarget::InputList => input_list(client).await?,
		PollTarget::ProfileList => {
			let profiles = client.profiles().list().await?;
			ObsEvent::ProfileListResponse(ProfileListData {
				profiles: profiles.profiles,
				current_profile: profiles.current,
			})
		}
		PollTarget::CurrentProfile => ObsEvent::CurrentProfileResponse(CurrentProfileData {
			profile_name: client.profiles().current().await?,
		}),
		PollTarget::SceneCollectionList => {
			let collections = client.scene_collections().list().await?;
			ObsEvent::SceneCollectionListResponse(SceneCollectionListData {
				collections: collections.collections,
				current_collection: collections.current,
			})
		}
		PollTarget::CurrentSceneCollection => ObsEvent::CurrentCollectionResponse(CurrentCollectionData {
			collection_name: client.scene_collections().current().await?,
		}),
		PollTarget::Version => {
			let version = client.general().version().await?;
			ObsEvent::VersionResponse(VersionData {
				obs_version: version.obs_studio_version.to_string(),
				websocket_version: version.obs_web_socket_version.to_string(),
			})
		}
		PollTarget::VirtualCamStatus => ObsEvent::VirtualCamStatusResponse(VirtualCamStatusData {
			active: client.virtual_cam().status().await?,
		}),
		PollTarget::ReplayBufferStatus => ObsEvent::ReplayBufferStatusResponse(ReplayBufferStatusData {
			active: client.replay_buffer().status().await?,
		}),
	})
}

async fn scene_list(client: &Client) -> Result<ObsEvent, obws::error::Error> {
	let scenes = client.scenes().list().await?;
	Ok(ObsEvent::SceneListResponse(SceneListData {
		scenes: scenes
			.scenes
			.into_iter()
			.map(|scene| SceneInfo {
				name: scene.id.name,
				index: u32::try_from(scene.index).ok(),
			})
			.collect(),
		current_scene: scenes.current_program_scene.map(|id| id.name).unwrap_or_default(),
	}))
}

async fn stats(client: &Client) -> Result<ObsEvent, obws::error::Error> {
	let stats = client.general().stats().await?;
	Ok(ObsEvent::StatsResponse(StatsData {
		stats: ObsStats {
			cpu_usage: stats.cpu_usage,
			memory_usage: stats.memory_usage,
			available_disk_space: stats.available_disk_space,
			active_fps: stats.active_fps,
			average_frame_time: stats.average_frame_render_time,
			render_total_frames: stats.render_total_frames.into(),
			render_missed_frames: stats.render_skipped_frames.into(),
			output_total_frames: stats.output_total_frames.into(),
			output_skipped_frames: stats.output_skipped_frames.into(),
			web_socket_session_incoming_messages: stats.web_socket_session_incoming_messages,
			web_socket_session_outgoing_messages: stats.web_socket_session_outgoing_messages,
		},
	}))
}

async fn transition_list(client: &Client) -> Result<ObsEvent, obws::error::Error> {
	let transitions = client.transitions().list().await?.transitions;
	Ok(ObsEvent::TransitionListResponse(TransitionListData {
		transitions: transitions
			.into_iter()
			.map(|t| TransitionInfo {
				name: t.id.name,
				kind: t.kind,
				fixed: t.fixed,
			})
			.collect(),
	}))
}

async fn input_list(client: &Client) -> Result<ObsEvent, obws::error::Error> {
	let inputs = client.inputs().list(None).await?;
	Ok(ObsEvent::InputListResponse(InputListData {
		inputs: inputs
			.into_iter()
			.map(|input| InputInfo {
				name: input.id.name,
				kind: input.kind,
				unversioned_kind: input.unversioned_kind,
			})
			.collect(),
	}))
}

/// Formats as OBS does in `outputTimecode`: `HH:MM:SS.mmm`.
fn timecode(elapsed: Duration) -> String {
	let secs = elapsed.as_secs();
	let mut out = String::with_capacity(12);
	let _ = write!(out, "{:02}:{:02}:{:02}.{:03}", secs / 3600, secs / 60 % 60, secs % 60, elapsed.subsec_millis());
	out
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn timecode_matches_obs_format() {
		assert_eq!(timecode(Duration::ZERO), "00:00:00.000");
		assert_eq!(timecode(Duration::from_millis(3_723_045)), "01:02:03.045");
	}
}

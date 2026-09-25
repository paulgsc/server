//! Builds a [`StudioSnapshot`]: OBS's current state plus every name the
//! commands accept.

use crate::polling::timecode;
use crate::types::{AudioState, FilterState, MediaPlayback, MediaState, RecordingState, SceneSourceState, SceneState, StreamState, StudioSnapshot};
use futures_util::future::join_all;
use obws::requests::inputs::InputId;
use obws::requests::scenes::SceneId;
use obws::requests::sources::SourceId;
use obws::responses::media_inputs::MediaState as ObwsMediaState;
use obws::Client;
use std::collections::BTreeSet;
use std::time::Duration;

type Result<T> = std::result::Result<T, obws::error::Error>;

/// Input kinds OBS can play, pause and restart.
const MEDIA_KINDS: [&str; 2] = ["ffmpeg_source", "vlc_source"];

/// Queries OBS for everything in a [`StudioSnapshot`], running per-item
/// lookups concurrently.
///
/// # Errors
///
/// Fails if any query fails for a reason other than OBS saying it doesn't
/// apply (a video-only input has no volume, a disabled replay buffer has no
/// state); those are left out of the snapshot instead.
pub async fn snapshot(client: &Client) -> Result<StudioSnapshot> {
	let stream = client.streaming().status().await?;
	let recording = client.recording().status().await?;
	let replay_buffer_active = unless_unsupported(client.replay_buffer().status().await)?;
	let virtual_camera_active = client.virtual_cam().status().await?;
	let studio_mode = client.ui().studio_mode_enabled().await?;

	let scene_list = client.scenes().list().await?;
	let scene_names: Vec<String> = scene_list.scenes.into_iter().map(|scene| scene.id.name).collect();
	let scenes = collect(join_all(scene_names.iter().map(|name| scene_state(client, name))).await)?;

	let inputs = client.inputs().list(None).await?;
	let audio = collect(join_all(inputs.iter().map(|input| audio_state(client, &input.id.name))).await)?;
	let media = collect(
		join_all(
			inputs
				.iter()
				.filter(|input| MEDIA_KINDS.contains(&input.unversioned_kind.as_str()))
				.map(|input| media_state(client, &input.id.name)),
		)
		.await,
	)?;

	// Filters can sit on scenes, inputs, or sources only reachable through a scene.
	let filter_owners: BTreeSet<&str> = scene_names
		.iter()
		.map(String::as_str)
		.chain(inputs.iter().map(|input| input.id.name.as_str()))
		.chain(scenes.iter().flat_map(|scene| scene.sources.iter().map(|source| source.name.as_str())))
		.collect();
	let filters = collect(join_all(filter_owners.into_iter().map(|owner| filters_of(client, owner))).await)?;

	let transitions = client.transitions().list().await?;

	Ok(StudioSnapshot {
		stream: StreamState {
			active: stream.active,
			reconnecting: stream.reconnecting,
			timecode: timecode(Duration::try_from(stream.timecode).unwrap_or_default()),
		},
		recording: RecordingState {
			active: recording.active,
			paused: recording.paused,
			timecode: timecode(Duration::try_from(recording.timecode).unwrap_or_default()),
		},
		replay_buffer_active,
		virtual_camera_active,
		studio_mode,
		program_scene: scene_list.current_program_scene.map(|id| id.name).unwrap_or_default(),
		preview_scene: scene_list.current_preview_scene.filter(|_| studio_mode).map(|id| id.name),
		scenes,
		audio: audio.into_iter().flatten().collect(),
		media: media.into_iter().flatten().collect(),
		filters: filters.into_iter().flatten().collect(),
		transitions: transitions.transitions.into_iter().map(|t| t.id.name).collect(),
		current_transition: transitions.current_scene_transition.map(|id| id.name),
		hotkeys: client.hotkeys().list().await?,
	})
}

async fn scene_state(client: &Client, scene: &str) -> Result<SceneState> {
	let scene_items = client.scene_items();
	let items = scene_items.list(SceneId::Name(scene)).await?;
	let visibility = join_all(items.iter().map(|item| scene_items.enabled(SceneId::Name(scene), item.id))).await;
	let sources = items
		.into_iter()
		.zip(visibility)
		.map(|(item, visible)| {
			Ok(SceneSourceState {
				name: item.source_name,
				visible: visible?,
			})
		})
		.collect::<Result<_>>()?;
	Ok(SceneState { name: scene.to_owned(), sources })
}

/// `None` for inputs without audio.
async fn audio_state(client: &Client, input: &str) -> Result<Option<AudioState>> {
	let Some(muted) = unless_unsupported(client.inputs().muted(InputId::Name(input)).await)? else {
		return Ok(None);
	};
	let volume = client.inputs().volume(InputId::Name(input)).await?;
	Ok(Some(AudioState {
		input: input.to_owned(),
		muted,
		volume_db: f64::from(volume.db),
	}))
}

async fn media_state(client: &Client, input: &str) -> Result<Option<MediaState>> {
	let status = unless_unsupported(client.media_inputs().status(InputId::Name(input)).await)?;
	Ok(status.map(|status| MediaState {
		input: input.to_owned(),
		playback: playback(status.state),
	}))
}

async fn filters_of(client: &Client, source: &str) -> Result<Vec<FilterState>> {
	let filters = unless_unsupported(client.filters().list(SourceId::Name(source)).await)?.unwrap_or_default();
	Ok(
		filters
			.into_iter()
			.map(|filter| FilterState {
				source: source.to_owned(),
				filter: filter.name,
				enabled: filter.enabled,
			})
			.collect(),
	)
}

const fn playback(state: ObwsMediaState) -> MediaPlayback {
	match state {
		ObwsMediaState::Playing => MediaPlayback::Playing,
		ObwsMediaState::Opening | ObwsMediaState::Buffering => MediaPlayback::Loading,
		ObwsMediaState::Paused => MediaPlayback::Paused,
		ObwsMediaState::Stopped => MediaPlayback::Stopped,
		ObwsMediaState::Ended => MediaPlayback::Ended,
		ObwsMediaState::Error => MediaPlayback::Error,
		_ => MediaPlayback::Idle,
	}
}

/// OBS answering that a request doesn't apply to this resource becomes `None`;
/// anything else, such as a dropped connection, stays an error.
fn unless_unsupported<T>(result: Result<T>) -> Result<Option<T>> {
	match result {
		Ok(value) => Ok(Some(value)),
		Err(obws::error::Error::Api { .. }) => Ok(None),
		Err(e) => Err(e),
	}
}

fn collect<T>(results: Vec<Result<T>>) -> Result<Vec<T>> {
	results.into_iter().collect()
}

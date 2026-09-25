//! Builds and publishes [`StudioSnapshot`]s: OBS's current state plus every name the
//! commands accept.

use crate::polling::timecode;
use crate::types::{AudioState, FilterState, MediaPlayback, MediaState, ObsEvent, RecordingState, SceneSourceState, SceneState, StreamState, StudioSnapshot};
use futures_util::future::join_all;
use futures_util::FutureExt;
use obws::requests::inputs::InputId;
use obws::requests::scenes::SceneId;
use obws::requests::sources::SourceId;
use obws::responses::media_inputs::MediaState as ObwsMediaState;
use obws::Client;
use std::collections::BTreeSet;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{broadcast, Notify};

type Result<T> = std::result::Result<T, obws::error::Error>;

/// How long to gather changes after the first one before taking a snapshot,
/// so a burst (a dragged volume slider, a scene switch touching many sources)
/// yields one snapshot. While changes keep coming, snapshots follow at most
/// once per window, so a display still tracks a long drag.
const SETTLE: Duration = Duration::from_millis(250);

/// Publishes [`ObsEvent::StudioChanged`] once on start, then once per burst of
/// OBS events signalled through `changed`, until aborted with its connection.
/// A change that lands while a snapshot is being taken triggers exactly one
/// more, so the last published snapshot always reflects the last change.
pub async fn publish(client: Arc<Client>, changed: Arc<Notify>, events: broadcast::Sender<ObsEvent>) {
	publish_with(|| snapshot(&client), &changed, &events).await;
}

async fn publish_with<F, Fut>(mut take_snapshot: F, changed: &Notify, events: &broadcast::Sender<ObsEvent>)
where
	F: FnMut() -> Fut,
	Fut: Future<Output = Result<StudioSnapshot>>,
{
	loop {
		match take_snapshot().await {
			// No subscribers is not an error; nobody is listening yet.
			Ok(studio) => drop(events.send(ObsEvent::StudioChanged(Box::new(studio)))),
			Err(e) => tracing::debug!(error = %e, "studio snapshot failed"),
		}
		changed.notified().await;
		tokio::time::sleep(SETTLE).await;
		// Changes during the settle window are covered by the snapshot about to
		// be taken; drop the permit they left so they don't trigger another.
		let _: Option<()> = changed.notified().now_or_never();
	}
}

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

#[cfg(test)]
mod tests {
	use super::*;
	use std::sync::atomic::{AtomicUsize, Ordering};

	fn empty_studio() -> StudioSnapshot {
		StudioSnapshot {
			stream: StreamState {
				active: false,
				reconnecting: false,
				timecode: String::new(),
			},
			recording: RecordingState {
				active: false,
				paused: false,
				timecode: String::new(),
			},
			replay_buffer_active: None,
			virtual_camera_active: false,
			studio_mode: false,
			program_scene: String::new(),
			preview_scene: None,
			scenes: Vec::new(),
			audio: Vec::new(),
			media: Vec::new(),
			filters: Vec::new(),
			transitions: Vec::new(),
			current_transition: None,
			hotkeys: Vec::new(),
		}
	}

	/// Snapshots taken so far, after letting the publisher run `for_duration`
	/// of paused time.
	async fn run_for(taken: &Arc<AtomicUsize>, changed: &Arc<Notify>, burst: usize, for_duration: Duration) -> usize {
		let (events, _keep_open) = broadcast::channel(64);
		let counter = Arc::clone(taken);
		let notify = Arc::clone(changed);
		let publisher = tokio::spawn(async move {
			publish_with(
				|| {
					counter.fetch_add(1, Ordering::SeqCst);
					async { Ok(empty_studio()) }
				},
				&notify,
				&events,
			)
			.await;
		});

		tokio::task::yield_now().await;
		for _ in 0..burst {
			changed.notify_one();
		}
		tokio::time::sleep(for_duration).await;
		publisher.abort();
		taken.load(Ordering::SeqCst)
	}

	#[tokio::test(start_paused = true)]
	async fn publishes_once_on_start() {
		let taken = Arc::new(AtomicUsize::new(0));
		assert_eq!(run_for(&taken, &Arc::new(Notify::new()), 0, SETTLE * 4).await, 1);
	}

	#[tokio::test(start_paused = true)]
	async fn a_burst_of_changes_yields_one_more_snapshot() {
		let taken = Arc::new(AtomicUsize::new(0));
		// A dragged volume slider: many events in one instant.
		assert_eq!(run_for(&taken, &Arc::new(Notify::new()), 50, SETTLE * 4).await, 2);
	}

	#[tokio::test(start_paused = true)]
	async fn a_change_during_a_snapshot_gets_its_own_snapshot() {
		let taken = Arc::new(AtomicUsize::new(0));
		let changed = Arc::new(Notify::new());
		let (events, _keep_open) = broadcast::channel(64);
		let counter = Arc::clone(&taken);
		let notify = Arc::clone(&changed);
		let publisher = tokio::spawn(async move {
			publish_with(
				|| {
					counter.fetch_add(1, Ordering::SeqCst);
					// A snapshot takes a while: OBS answers many queries.
					async {
						tokio::time::sleep(Duration::from_millis(100)).await;
						Ok(empty_studio())
					}
				},
				&notify,
				&events,
			)
			.await;
		});

		// Land a change while the first snapshot is still being taken.
		tokio::time::sleep(Duration::from_millis(50)).await;
		changed.notify_one();
		tokio::time::sleep(SETTLE * 4).await;
		publisher.abort();

		assert_eq!(taken.load(Ordering::SeqCst), 2);
	}
}

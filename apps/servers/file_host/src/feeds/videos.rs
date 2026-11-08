//! Each feed's videos on the phone (STUDIO ST38: paulgsc/some-ui#2661, the
//! owner's decisions of 2026-11-07; Playoff Delta's beside the call's, FEED4
//! paulgsc/some-ui#2647).
//!
//! The owner watches each episode's video on the phone, shares it to the
//! `YouTube` app, records its `YouTube` link, and then lets home's copy go
//! from My data in two steps: the video, then the rest of the folder. This is
//! what `handlers::feed_videos` (and the call's older `handlers::call_videos`)
//! answers a paired phone with, for any [`Feed`]:
//!
//! - [`list`]: each episode home holds, newest first, with the bytes of its
//!   video and of the rest of its folder, its link and its receipts.
//! - [`video_file`]: where a rendered episode's video is, for the download.
//! - [`let_go`]: one step of letting an episode go, with its receipt.
//!
//! **What a step deletes, and in what order.** The video's step removes the
//! episode from the feed's input (`Feed::input`), so the render worker is
//! never given it again, then its video; the rest's step removes the
//! episode's folder under the feed's renders. What it was made from stays in
//! the feed's table (`call_week`, `pd_fill`).
//!
//! **The receipt first, then the files.** A transaction cannot take back a
//! deleted file, so each step commits its receipt before it touches the
//! drive, and answers only once the files are gone. A step whose files did
//! not all go (the drive refused, or the process stopped) is finished the
//! next time it is asked for: a step that has a receipt removes what is left
//! of it again, then answers with that receipt. So a receipt never stands
//! without its delete under way, and a delete never happens without its
//! receipt.
//!
//! **One at a time with the export passes.** [`super::FOLDERS`] is held for
//! a whole step, as each export pass holds it for a whole pass, so a pass
//! never writes an episode back between reading which were let go and
//! writing the ones that are missing.
//!
//! **Paths.** An episode's folder is joined only after the feed's table says
//! home holds it, and each table holds only keys of digits, capitals and
//! dashes, so a path never leaves the feed's renders folder.

use super::Feed;
use crate::call_weeks::render::bytes_under;
use call_week_repo::video::{self, Receipt, Recorded, Step};
use sqlx::SqlitePool;
use std::path::{Path, PathBuf};

/// An episode as the phone sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EpisodeVideo {
	/// Its key: the call's week by its first day, Playoff Delta's
	/// `<season>-<team>-<NN>`.
	pub key: String,
	/// Bytes of its video, while home has it.
	pub video_bytes: Option<u64>,
	/// Bytes of the rest of its folder, while home has it.
	pub rest_bytes: Option<u64>,
	/// Its `YouTube` link, once recorded.
	pub link: Option<video::Link>,
	/// Each step let go, video first.
	pub receipts: Vec<Receipt>,
}

/// Each episode of `feed` home holds, newest first.
///
/// # Errors
/// A storage failure.
pub async fn list(db: &SqlitePool, feed: Feed, renders: &Path) -> Result<Vec<EpisodeVideo>, sqlx::Error> {
	let (held, links, receipts) = {
		let mut tx = db.begin().await?;
		let held = feed.held(&mut tx).await?;
		let links = feed.links(&mut tx).await?;
		let receipts = feed.receipts(&mut tx).await?;
		tx.commit().await?;
		(held, links, receipts)
	};
	let mut listed = Vec::with_capacity(held.len());
	for key in held {
		let folder = renders.join(&key);
		let video_bytes = tokio::fs::metadata(folder.join(feed.video()))
			.await
			.ok()
			.filter(std::fs::Metadata::is_file)
			.map(|meta| meta.len());
		let rest_bytes = if tokio::fs::metadata(&folder).await.is_ok_and(|meta| meta.is_dir()) {
			Some(bytes_under(&folder).await.saturating_sub(video_bytes.unwrap_or(0)))
		} else {
			None
		};
		let mine: Vec<Receipt> = receipts.iter().filter(|receipt| receipt.from == key).cloned().collect();
		listed.push(EpisodeVideo {
			link: links.get(&key).cloned(),
			key,
			video_bytes,
			rest_bytes,
			receipts: mine,
		});
	}
	Ok(listed)
}

/// Where a rendered episode's video is, if home holds it and its video.
///
/// # Errors
/// A storage failure.
pub async fn video_file(db: &SqlitePool, feed: Feed, renders: &Path, key: &str) -> Result<Option<PathBuf>, sqlx::Error> {
	let holds = {
		let mut conn = db.acquire().await?;
		feed.holds(&mut conn, key).await?
	};
	if !holds {
		return Ok(None);
	}
	let file = renders.join(key).join(feed.video());
	Ok(tokio::fs::metadata(&file).await.is_ok_and(|meta| meta.is_file()).then_some(file))
}

/// What [`record_link`] did with a pasted link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkRecorded {
	/// Kept (or already held): the episode's link is now this video's.
	Kept(String),
	/// The pasted text names no `YouTube` video.
	NotAVideo,
	/// Home holds no such episode.
	NoEpisode,
}

/// Records a pasted link as the episode's `YouTube` link.
///
/// # Errors
/// A storage failure.
pub async fn record_link(db: &SqlitePool, feed: Feed, key: &str, pasted: &str, now: &str) -> Result<LinkRecorded, sqlx::Error> {
	let Some(id) = video::youtube_id(pasted) else {
		return Ok(LinkRecorded::NotAVideo);
	};
	let mut conn = db.acquire().await?;
	Ok(match feed.record_link(&mut conn, key, &id, now).await? {
		Recorded::NoWeek => LinkRecorded::NoEpisode,
		Recorded::Added | Recorded::Held => LinkRecorded::Kept(id),
	})
}

/// What [`let_go`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LetGo {
	/// The step is done: its receipt, new or as first written.
	Gone(Receipt),
	/// Home holds no such episode.
	NoEpisode,
	/// The video goes only once the episode's `YouTube` link is recorded.
	NoLink,
	/// The rest goes only once the video has.
	VideoFirst,
	/// The episode has no video to let go: it is not rendered yet.
	NotRendered,
}

/// Why a step could not be done.
#[derive(Debug, thiserror::Error)]
pub enum LetGoError {
	#[error("storage: {0}")]
	Storage(#[from] sqlx::Error),
	#[error("the drive: {0}")]
	Drive(#[from] std::io::Error),
}

/// Where an episode's files are: the feed's renders and input folders.
#[derive(Debug, Clone, Copy)]
pub struct Folders<'a> {
	pub renders: &'a Path,
	pub input: &'a Path,
}

/// Lets one step of an episode go (the module doc), under `BEGIN IMMEDIATE`
/// so two phones asking at once write one receipt.
///
/// # Errors
/// A storage failure, or a file the drive would not remove. The receipt is
/// kept then, and asking for the step again finishes it.
pub async fn let_go(db: &SqlitePool, feed: Feed, folders: Folders<'_>, key: &str, step: Step, now: &str) -> Result<LetGo, LetGoError> {
	let _folders = super::FOLDERS.lock().await;
	let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
	// A receipt names an episode home holds (its foreign key), so the path
	// below is one of the feed's renders.
	if let Some(receipt) = feed.receipt(&mut tx, key, step).await? {
		tx.commit().await?;
		finish(feed, folders, key, step).await?;
		return Ok(LetGo::Gone(receipt));
	}
	if !feed.holds(&mut tx, key).await? {
		return Ok(LetGo::NoEpisode);
	}
	let Some(link) = feed.links(&mut tx).await?.remove(key) else {
		return Ok(LetGo::NoLink);
	};
	let folder = folders.renders.join(key);
	let size = match step {
		Step::Video => {
			let Ok(meta) = tokio::fs::metadata(folder.join(feed.video())).await else {
				return Ok(LetGo::NotRendered);
			};
			meta.len()
		}
		Step::Rest => {
			if feed.receipt(&mut tx, key, Step::Video).await?.is_none() {
				return Ok(LetGo::VideoFirst);
			}
			bytes_under(&folder).await
		}
	};
	let receipt = Receipt {
		from: key.to_owned(),
		step,
		youtube_id: link.youtube_id,
		size: i64::try_from(size).unwrap_or(i64::MAX),
		deleted_at: now.to_owned(),
	};
	feed.write_receipt(&mut tx, &receipt).await?;
	tx.commit().await?;
	finish(feed, folders, key, step).await?;
	Ok(LetGo::Gone(receipt))
}

/// Removes what a step lets go; what is already gone is fine.
async fn finish(feed: Feed, folders: Folders<'_>, key: &str, step: Step) -> std::io::Result<()> {
	let folder = folders.renders.join(key);
	match step {
		Step::Video => {
			// The episode's input first: the worker renders any episode it is
			// given that has no video.
			let input = feed.input_of(folders.input, key);
			match feed {
				Feed::Call => remove_file(&input).await?,
				Feed::PlayoffDelta => remove_dir(&input).await?,
			}
			remove_file(&folder.join(feed.video())).await
		}
		Step::Rest => remove_dir(&folder).await,
	}
}

/// Removes a file; one already gone is fine.
async fn remove_file(path: &Path) -> std::io::Result<()> {
	match tokio::fs::remove_file(path).await {
		Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error),
		_ => Ok(()),
	}
}

/// Removes a folder and all in it; one already gone is fine.
pub(crate) async fn remove_dir(path: &Path) -> std::io::Result<()> {
	match tokio::fs::remove_dir_all(path).await {
		Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error),
		_ => Ok(()),
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use call_week_repo::Week;
	use sqlx::sqlite::SqlitePoolOptions;

	async fn pool() -> SqlitePool {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		crate::schema::MIGRATOR.run(&pool).await.unwrap();
		pool
	}

	const NOW: &str = "2026-11-07T12:00:00.000Z";

	async fn hold(pool: &SqlitePool, from: &str, to: &str) {
		let mut conn = pool.acquire().await.unwrap();
		call_week_repo::keep(
			&mut conn,
			&Week {
				from,
				to,
				body: b"{}",
				taken_at: "2026-11-06T08:00:00.000Z",
			},
		)
		.await
		.unwrap();
	}

	fn render(renders: &Path, from: &str) {
		let folder = renders.join(from);
		std::fs::create_dir_all(folder.join("takes")).unwrap();
		std::fs::write(folder.join(Feed::Call.video()), [0_u8; 10]).unwrap();
		std::fs::write(folder.join("takes").join("01.wav"), [0_u8; 3]).unwrap();
		std::fs::write(folder.join("cuts.json"), [0_u8; 2]).unwrap();
	}

	#[tokio::test]
	async fn a_week_is_listed_with_its_bytes_link_and_receipts() {
		let (pool, dir) = (pool().await, tempfile::tempdir().unwrap());
		let renders = dir.path().join("call");
		hold(&pool, "2026-10-30", "2026-11-05").await;
		hold(&pool, "2026-11-06", "2026-11-12").await;
		render(&renders, "2026-10-30");

		let listed = list(&pool, Feed::Call, &renders).await.unwrap();
		assert_eq!(
			listed.iter().map(|week| week.key.as_str()).collect::<Vec<_>>(),
			["2026-11-06", "2026-10-30"],
			"newest first"
		);
		assert_eq!((listed[0].video_bytes, listed[0].rest_bytes), (None, None), "not rendered yet");
		assert_eq!((listed[1].video_bytes, listed[1].rest_bytes), (Some(10), Some(5)));
		assert_eq!(listed[1].link, None);

		let pasted = "https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=1";
		assert_eq!(
			record_link(&pool, Feed::Call, "2026-10-30", pasted, NOW).await.unwrap(),
			LinkRecorded::Kept("dQw4w9WgXcQ".to_owned())
		);
		assert_eq!(
			record_link(&pool, Feed::Call, "2026-10-30", "https://youtube.com/@me", NOW).await.unwrap(),
			LinkRecorded::NotAVideo
		);
		assert_eq!(record_link(&pool, Feed::Call, "2026-01-02", pasted, NOW).await.unwrap(), LinkRecorded::NoEpisode);
		let listed = list(&pool, Feed::Call, &renders).await.unwrap();
		assert_eq!(listed[1].link.as_ref().map(|link| link.youtube_id.as_str()), Some("dQw4w9WgXcQ"));
		assert_eq!(
			video_file(&pool, Feed::Call, &renders, "2026-10-30").await.unwrap(),
			Some(renders.join("2026-10-30").join(Feed::Call.video()))
		);
		assert_eq!(video_file(&pool, Feed::Call, &renders, "2026-11-06").await.unwrap(), None, "not rendered");
		assert_eq!(video_file(&pool, Feed::Call, &renders, "2026-01-02").await.unwrap(), None, "not held");
	}

	#[tokio::test]
	async fn a_week_goes_video_first_then_the_rest_once_its_link_is_recorded() {
		let (pool, dir) = (pool().await, tempfile::tempdir().unwrap());
		let (renders, weeks) = (dir.path().join("call"), dir.path().join("call-weeks"));
		std::fs::create_dir_all(&weeks).unwrap();
		std::fs::write(weeks.join("2026-10-30.json"), b"{}").unwrap();
		hold(&pool, "2026-10-30", "2026-11-05").await;
		hold(&pool, "2026-11-06", "2026-11-12").await;
		render(&renders, "2026-10-30");

		let go = |from: &'static str, step| {
			let (pool, renders, weeks) = (pool.clone(), renders.clone(), weeks.clone());
			async move { let_go(&pool, Feed::Call, Folders { renders: &renders, input: &weeks }, from, step, NOW).await.unwrap() }
		};
		assert_eq!(go("2026-01-02", Step::Video).await, LetGo::NoEpisode);
		assert_eq!(go("2026-10-30", Step::Video).await, LetGo::NoLink, "no link, nothing goes");
		assert!(renders.join("2026-10-30").join(Feed::Call.video()).exists());
		record_link(&pool, Feed::Call, "2026-10-30", "https://youtu.be/dQw4w9WgXcQ", NOW).await.unwrap();
		record_link(&pool, Feed::Call, "2026-11-06", "https://youtu.be/aaaaaaaaaaa", NOW).await.unwrap();
		assert_eq!(go("2026-11-06", Step::Video).await, LetGo::NotRendered);
		assert_eq!(go("2026-10-30", Step::Rest).await, LetGo::VideoFirst);
		assert!(renders.join("2026-10-30").join("cuts.json").exists());

		let LetGo::Gone(video) = go("2026-10-30", Step::Video).await else {
			panic!("the video goes")
		};
		assert_eq!((video.size, video.youtube_id.as_str(), video.deleted_at.as_str()), (10, "dQw4w9WgXcQ", NOW));
		assert!(!renders.join("2026-10-30").join(Feed::Call.video()).exists());
		assert!(!weeks.join("2026-10-30.json").exists(), "the worker is not given it again");
		assert!(renders.join("2026-10-30").join("cuts.json").exists(), "the rest stays");
		assert_eq!(go("2026-10-30", Step::Video).await, LetGo::Gone(video.clone()), "a repeat answers with the same receipt");

		let LetGo::Gone(rest) = go("2026-10-30", Step::Rest).await else {
			panic!("the rest goes")
		};
		assert_eq!(rest.size, 5);
		assert!(!renders.join("2026-10-30").exists());
		let listed = list(&pool, Feed::Call, &renders).await.unwrap();
		assert_eq!(listed[1].receipts, [video, rest]);
		assert_eq!((listed[1].video_bytes, listed[1].rest_bytes), (None, None));
	}

	#[tokio::test]
	async fn a_step_the_drive_refuses_keeps_its_receipt_and_is_finished_when_asked_again() {
		let (pool, dir) = (pool().await, tempfile::tempdir().unwrap());
		let (renders, weeks) = (dir.path().join("call"), dir.path().join("call-weeks"));
		hold(&pool, "2026-10-30", "2026-11-05").await;
		record_link(&pool, Feed::Call, "2026-10-30", "https://youtu.be/dQw4w9WgXcQ", NOW).await.unwrap();
		// A folder where the week's file would be: removing it as a file fails,
		// as a drive that refuses would.
		let blocked = weeks.join("2026-10-30.json");
		std::fs::create_dir_all(blocked.join("x")).unwrap();
		render(&renders, "2026-10-30");

		assert!(
			let_go(&pool, Feed::Call, Folders { renders: &renders, input: &weeks }, "2026-10-30", Step::Video, NOW)
				.await
				.is_err(),
			"not answered as done"
		);
		let mut conn = pool.acquire().await.unwrap();
		let kept = video::receipt(&mut conn, "2026-10-30", Step::Video).await.unwrap().unwrap();
		drop(conn);
		assert_eq!(kept.size, 10, "the receipt stands, so the step is never lost");

		std::fs::remove_dir_all(&blocked).unwrap();
		let later = "2026-11-07T13:00:00.000Z";
		assert_eq!(
			let_go(&pool, Feed::Call, Folders { renders: &renders, input: &weeks }, "2026-10-30", Step::Video, later)
				.await
				.unwrap(),
			LetGo::Gone(kept)
		);
		assert!(!renders.join("2026-10-30").join(Feed::Call.video()).exists(), "asked again, it is finished");
	}

	/// Playoff Delta's episodes go by the same steps, in their own tables, and
	/// the video's step takes the fill's folder out of the worker's input.
	#[tokio::test]
	async fn a_playoff_delta_episode_goes_by_the_same_steps_and_leaves_the_workers_input() {
		let (pool, dir) = (pool().await, tempfile::tempdir().unwrap());
		let (renders, episodes) = (dir.path().join("playoff-delta"), dir.path().join("episodes"));
		let mut conn = pool.acquire().await.unwrap();
		let fill = pd_fill_repo::Fill {
			season: "2026",
			team: "SF",
			week: 9,
			version: 2,
			body: b"{}",
			season_body: b"{}",
			taken_at: "2026-11-10T20:00:00.000Z",
		};
		pd_fill_repo::keep(&mut conn, &fill).await.unwrap();
		drop(conn);
		let folder = renders.join("2026-SF-09");
		std::fs::create_dir_all(&folder).unwrap();
		std::fs::write(folder.join(Feed::PlayoffDelta.video()), [0_u8; 10]).unwrap();
		std::fs::write(folder.join("cuts.json"), [0_u8; 2]).unwrap();
		std::fs::create_dir_all(episodes.join("2026-SF-09")).unwrap();
		std::fs::write(episodes.join("2026-SF-09").join("fill.json"), b"{}").unwrap();

		let listed = list(&pool, Feed::PlayoffDelta, &renders).await.unwrap();
		assert_eq!(
			listed.iter().map(|e| (e.key.as_str(), e.video_bytes, e.rest_bytes)).collect::<Vec<_>>(),
			[("2026-SF-09", Some(10), Some(2))]
		);
		assert!(list(&pool, Feed::Call, &renders).await.unwrap().is_empty(), "each feed lists its own");
		let folders = Folders {
			renders: &renders,
			input: &episodes,
		};
		let go = |step| let_go(&pool, Feed::PlayoffDelta, folders, "2026-SF-09", step, NOW);
		assert_eq!(go(Step::Video).await.unwrap(), LetGo::NoLink);
		assert_eq!(
			record_link(&pool, Feed::PlayoffDelta, "2026-SF-09", "https://youtu.be/dQw4w9WgXcQ", NOW).await.unwrap(),
			LinkRecorded::Kept("dQw4w9WgXcQ".to_owned())
		);
		assert_eq!(
			record_link(&pool, Feed::PlayoffDelta, "2026-SF-10", "https://youtu.be/dQw4w9WgXcQ", NOW).await.unwrap(),
			LinkRecorded::NoEpisode
		);
		let LetGo::Gone(video) = go(Step::Video).await.unwrap() else { panic!("the video goes") };
		assert_eq!(video.size, 10);
		assert!(!episodes.join("2026-SF-09").exists(), "the worker is not given it again");
		assert!(folder.join("cuts.json").exists(), "the rest stays");
		let LetGo::Gone(rest) = go(Step::Rest).await.unwrap() else { panic!("the rest goes") };
		assert_eq!((rest.size, folder.exists()), (2, false));
		assert_eq!(list(&pool, Feed::PlayoffDelta, &renders).await.unwrap()[0].receipts, [video, rest]);
	}
}

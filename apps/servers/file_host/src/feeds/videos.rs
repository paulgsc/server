//! Each feed's videos on the phone (STUDIO ST38: paulgsc/some-ui#2661, the
//! owner's decisions of 2026-11-07; Playoff Delta's beside the call's, FEED4
//! paulgsc/some-ui#2647).
//!
//! The owner watches each episode's video on the phone, shares it to the
//! `YouTube` app, records its `YouTube` link, and then lets home's copy go
//! from My data in two steps: the video, then the rest of the folder. This is
//! what `handlers::feed_videos`
//! answers a paired phone with, for any [`Feed`]:
//!
//! - [`list`]: each episode home holds, newest first, with the bytes of its
//!   video and of the rest of its folder, its link and its receipts.
//! - [`video_file`]: where a rendered episode's video is, for the download.
//! - [`description`]: the chapters the recorder wrote beside it, for the
//!   phone to show and copy into `YouTube`'s upload screen (SFEED4,
//!   paulgsc/some-ui#2763).
//! - [`let_go`]: one step of letting an episode go, with its receipt.
//!
//! **What a step deletes, and in what order.** The video's step removes the
//! episode from the feed's input (`Feed::input`), so the render worker is
//! never given it again, then its video. A part of a call's week that is a
//! series (paulgsc/some-ui#2918) is marked in the input instead
//! (`call_weeks::LET_GO`) while another part still has its video, and the
//! week's file goes with the last; the rest's step removes the
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
use tokio::io::AsyncReadExt as _;

/// An episode as the phone sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EpisodeVideo {
	/// Its key: the call's week by its first day, or `<day>-p<k>` for a
	/// later part of a week that is a series; Playoff Delta's
	/// `<season>-<team>-<NN>`.
	pub key: String,
	/// What it is a part of: the call's week by its first day; every other
	/// feed's episode is its own.
	pub week: String,
	/// Which part, from 1, of how many: 1 of 1 for a week of one video, and
	/// for every other feed's episode.
	pub part: u32,
	pub of: u32,
	/// Bytes of its video, while home has it.
	pub video_bytes: Option<u64>,
	/// Bytes of the rest of its folder, while home has it.
	pub rest_bytes: Option<u64>,
	/// Its `YouTube` link, once recorded.
	pub link: Option<video::Link>,
	/// Each step let go, video first.
	pub receipts: Vec<Receipt>,
}

/// The episode `key` names, as (its week, its part, of how many), if it is
/// one: for the call, a part the worker found the week has
/// (`call_weeks::render::parts`); for every other feed, the key itself, 1
/// of 1. Whether home holds it is the feed's table's to say.
async fn part_of(feed: Feed, renders: &Path, key: &str) -> Option<(String, u32, u32)> {
	match feed {
		Feed::Call => {
			let (week, part) = video::split_key(key)?;
			let of = crate::call_weeks::render::parts(renders, week).await;
			(part <= of).then(|| (week.to_owned(), part, of))
		}
		Feed::PlayoffDelta | Feed::RustEssay => Some((key.to_owned(), 1, 1)),
	}
}

/// Each episode of `feed` home holds, newest first: a call's week as each of
/// its parts, first to last.
///
/// # Errors
/// A storage failure.
pub async fn list(db: &SqlitePool, feed: Feed, subject: &str, renders: &Path) -> Result<Vec<EpisodeVideo>, sqlx::Error> {
	let (held, links, receipts) = {
		let mut tx = db.begin().await?;
		let held = feed.held(&mut tx, subject).await?;
		let links = feed.links(&mut tx, subject).await?;
		let receipts = feed.receipts(&mut tx, subject).await?;
		tx.commit().await?;
		(held, links, receipts)
	};
	let mut listed = Vec::with_capacity(held.len());
	for week in held {
		let keys = match feed {
			Feed::Call => crate::call_weeks::render::part_keys(renders, &week).await,
			Feed::PlayoffDelta | Feed::RustEssay => vec![week.clone()],
		};
		let of = u32::try_from(keys.len()).unwrap_or(u32::MAX);
		for (part, key) in (1..).zip(keys) {
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
				week: week.clone(),
				part,
				of,
				key,
				video_bytes,
				rest_bytes,
				receipts: mine,
			});
		}
	}
	Ok(listed)
}

/// Where a rendered episode's video is, if home holds it and its video.
///
/// # Errors
/// A storage failure.
pub async fn video_file(db: &SqlitePool, feed: Feed, subject: &str, renders: &Path, key: &str) -> Result<Option<PathBuf>, sqlx::Error> {
	if part_of(feed, renders, key).await.is_none() {
		return Ok(None);
	}
	let holds = {
		let mut conn = db.acquire().await?;
		feed.holds(&mut conn, subject, key).await?
	};
	if !holds {
		return Ok(None);
	}
	let file = renders.join(key).join(feed.video());
	Ok(tokio::fs::metadata(&file).await.is_ok_and(|meta| meta.is_file()).then_some(file))
}

/// The file the recorder writes beside an episode's video: its `YouTube`
/// chapter lines (paulgsc/some-ui#2767), absent when it has fewer than three
/// chapters or was rendered before the recorder wrote one.
pub const DESCRIPTION: &str = "description.txt";

/// The most bytes [`description`] reads: `YouTube`'s own ceiling on a
/// video's description, within which the recorder writes.
pub const DESCRIPTION_CEILING: u64 = 5_000;

/// What [`description`] found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Description {
	/// Home holds no such episode, it isn't rendered, or it has no
	/// description (fewer than three chapters, rendered before the recorder
	/// wrote one, or let go with the rest of its folder).
	Absent,
	/// The file's bytes, as the recorder wrote them.
	Text(Vec<u8>),
	/// The file is over [`DESCRIPTION_CEILING`], which the recorder never
	/// writes: it is not read.
	OverCeiling,
}

/// The chapters the recorder wrote beside a rendered episode's video.
///
/// The folder is joined only after the feed's table says home holds the
/// episode (the module's "Paths"), and a file over [`DESCRIPTION_CEILING`]
/// is never read.
///
/// # Errors
/// A storage failure, or the drive refusing a read of a file that is there.
pub async fn description(db: &SqlitePool, feed: Feed, subject: &str, renders: &Path, key: &str) -> Result<Description, DescriptionError> {
	if part_of(feed, renders, key).await.is_none() {
		return Ok(Description::Absent);
	}
	let holds = {
		let mut conn = db.acquire().await?;
		feed.holds(&mut conn, subject, key).await?
	};
	if !holds {
		return Ok(Description::Absent);
	}
	let path = renders.join(key).join(DESCRIPTION);
	// The recorder writes a plain file: a link, a folder or anything else
	// under its name is not one, and is never followed.
	match tokio::fs::symlink_metadata(&path).await {
		Ok(meta) if meta.file_type().is_file() => {}
		Ok(_) => return Ok(Description::Absent),
		Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Description::Absent),
		Err(err) => return Err(DescriptionError::Drive(err)),
	}
	let file = match tokio::fs::File::open(&path).await {
		Ok(file) => file,
		// It went between the two looks: let go with the rest.
		Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Description::Absent),
		Err(err) => return Err(DescriptionError::Drive(err)),
	};
	// One byte past the ceiling at most, whatever the file holds by now.
	let mut bytes = Vec::new();
	file.take(DESCRIPTION_CEILING + 1).read_to_end(&mut bytes).await.map_err(DescriptionError::Drive)?;
	Ok(if u64::try_from(bytes.len()).is_ok_and(|read| read <= DESCRIPTION_CEILING) {
		Description::Text(bytes)
	} else {
		Description::OverCeiling
	})
}

/// Why [`description`] could not answer.
#[derive(Debug)]
pub enum DescriptionError {
	/// Reading which episodes home holds failed.
	Storage(sqlx::Error),
	/// The drive refused a read of a file that is there.
	Drive(std::io::Error),
}

impl From<sqlx::Error> for DescriptionError {
	fn from(err: sqlx::Error) -> Self {
		Self::Storage(err)
	}
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

/// Records a pasted link as the episode's `YouTube` link: a part's alone,
/// for a call's week that is a series.
///
/// # Errors
/// A storage failure.
pub async fn record_link(db: &SqlitePool, feed: Feed, subject: &str, renders: &Path, key: &str, pasted: &str, now: &str) -> Result<LinkRecorded, sqlx::Error> {
	let Some(id) = video::youtube_id(pasted) else {
		return Ok(LinkRecorded::NotAVideo);
	};
	if part_of(feed, renders, key).await.is_none() {
		return Ok(LinkRecorded::NoEpisode);
	}
	let mut conn = db.acquire().await?;
	Ok(match feed.record_link(&mut conn, subject, key, &id, now).await? {
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

/// Lets one step of an episode go (the module doc).
///
/// Under `BEGIN IMMEDIATE`, so two phones asking at once write one receipt.
/// A part of a call's week goes on its own, with its own link; the week's
/// file goes once every part's video has.
///
/// # Errors
/// A storage failure, or a file the drive would not remove. The receipt is
/// kept then, and asking for the step again finishes it.
pub async fn let_go(db: &SqlitePool, feed: Feed, subject: &str, folders: Folders<'_>, key: &str, step: Step, now: &str) -> Result<LetGo, LetGoError> {
	let Some((week, _, of)) = part_of(feed, folders.renders, key).await else {
		return Ok(LetGo::NoEpisode);
	};
	let gone = Gone { week: &week, of };
	let _folders = super::FOLDERS.lock().await;
	let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
	// A receipt names an episode home holds (its foreign key), so the path
	// below is one of the feed's renders.
	if let Some(receipt) = feed.receipt(&mut tx, subject, key, step).await? {
		let whole = whole(feed, &mut tx, &week).await?;
		tx.commit().await?;
		finish(feed, folders, key, step, gone, whole).await?;
		return Ok(LetGo::Gone(receipt));
	}
	if !feed.holds(&mut tx, subject, key).await? {
		return Ok(LetGo::NoEpisode);
	}
	let Some(link) = feed.links(&mut tx, subject).await?.remove(key) else {
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
			if feed.receipt(&mut tx, subject, key, Step::Video).await?.is_none() {
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
	feed.write_receipt(&mut tx, subject, &receipt, of).await?;
	let whole = whole(feed, &mut tx, &week).await?;
	tx.commit().await?;
	finish(feed, folders, key, step, gone, whole).await?;
	Ok(LetGo::Gone(receipt))
}

/// Which week a step is of, and how many parts it is.
#[derive(Debug, Clone, Copy)]
struct Gone<'a> {
	week: &'a str,
	of: u32,
}

/// Whether every part of `week` has let its video go: always, for a feed
/// whose episode is one.
async fn whole(feed: Feed, conn: &mut sqlx::SqliteConnection, week: &str) -> Result<bool, sqlx::Error> {
	Ok(match feed {
		Feed::Call => call_week_repo::video::weeks_let_go(conn).await?.iter().any(|gone| gone == week),
		Feed::PlayoffDelta | Feed::RustEssay => true,
	})
}

/// Removes what a step lets go; what is already gone is fine.
async fn finish(feed: Feed, folders: Folders<'_>, key: &str, step: Step, gone: Gone<'_>, whole: bool) -> std::io::Result<()> {
	let folder = folders.renders.join(key);
	match step {
		Step::Video => {
			// The episode's input first: the worker renders any episode it is
			// given that has no video. A part of a week whose other parts
			// still have theirs is marked instead, so they still render.
			let input = feed.input_of(folders.input, gone.week);
			match feed {
				Feed::Call if whole => {
					remove_file(&input).await?;
					for part in 1..=gone.of {
						let mark = String::from(&*call_week_repo::video::part_key(gone.week, part)) + crate::call_weeks::LET_GO;
						remove_file(&folders.input.join(mark)).await?;
					}
				}
				Feed::Call => mark_let_go(folders.input, key).await?,
				Feed::PlayoffDelta | Feed::RustEssay => remove_dir(&input).await?,
			}
			remove_file(&folder.join(feed.video())).await
		}
		Step::Rest => remove_dir(&folder).await,
	}
}

/// Marks a part of a call's week as let go, for the render worker
/// (`call_weeks::LET_GO`); one already marked is fine.
pub(crate) async fn mark_let_go(input: &Path, key: &str) -> std::io::Result<()> {
	let mark = input.join(String::from(key) + crate::call_weeks::LET_GO);
	if tokio::fs::try_exists(&mark).await? {
		return Ok(());
	}
	tokio::fs::write(&mark, b"").await
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

		let listed = list(&pool, Feed::Call, "", &renders).await.unwrap();
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
			record_link(&pool, Feed::Call, "", &renders, "2026-10-30", pasted, NOW).await.unwrap(),
			LinkRecorded::Kept("dQw4w9WgXcQ".to_owned())
		);
		assert_eq!(
			record_link(&pool, Feed::Call, "", &renders, "2026-10-30", "https://youtube.com/@me", NOW).await.unwrap(),
			LinkRecorded::NotAVideo
		);
		assert_eq!(
			record_link(&pool, Feed::Call, "", &renders, "2026-01-02", pasted, NOW).await.unwrap(),
			LinkRecorded::NoEpisode
		);
		let listed = list(&pool, Feed::Call, "", &renders).await.unwrap();
		assert_eq!(listed[1].link.as_ref().map(|link| link.youtube_id.as_str()), Some("dQw4w9WgXcQ"));
		assert_eq!(
			video_file(&pool, Feed::Call, "", &renders, "2026-10-30").await.unwrap(),
			Some(renders.join("2026-10-30").join(Feed::Call.video()))
		);
		assert_eq!(video_file(&pool, Feed::Call, "", &renders, "2026-11-06").await.unwrap(), None, "not rendered");
		assert_eq!(video_file(&pool, Feed::Call, "", &renders, "2026-01-02").await.unwrap(), None, "not held");
	}

	#[tokio::test]
	async fn an_episodes_description_is_read_only_when_held_rendered_and_within_the_ceiling() {
		let (pool, dir) = (pool().await, tempfile::tempdir().unwrap());
		let renders = dir.path().join("call");
		hold(&pool, "2026-10-30", "2026-11-05").await;
		hold(&pool, "2026-11-06", "2026-11-12").await;
		render(&renders, "2026-10-30");
		render(&renders, "2026-11-06");
		let chapters = "0:00 Opening\n0:20 The board\n0:53 Which music?\n";
		std::fs::write(renders.join("2026-10-30").join(DESCRIPTION), chapters).unwrap();
		let read = |key: &'static str| {
			let (pool, renders) = (&pool, &renders);
			async move { description(pool, Feed::Call, "", renders, key).await.unwrap() }
		};
		assert_eq!(read("2026-10-30").await, Description::Text(chapters.as_bytes().to_vec()));
		assert_eq!(read("2026-11-06").await, Description::Absent, "fewer than three chapters, or rendered before");
		assert_eq!(read("2026-01-02").await, Description::Absent, "not held");
		std::fs::create_dir_all(renders.join("2026-01-02")).unwrap();
		std::fs::write(renders.join("2026-01-02").join(DESCRIPTION), chapters).unwrap();
		assert_eq!(read("2026-01-02").await, Description::Absent, "a folder the table does not hold is never read");

		let ceiling = usize::try_from(DESCRIPTION_CEILING).unwrap();
		std::fs::write(renders.join("2026-11-06").join(DESCRIPTION), vec![b'a'; ceiling]).unwrap();
		assert!(
			matches!(read("2026-11-06").await, Description::Text(bytes) if bytes.len() == ceiling),
			"the ceiling itself is read"
		);
		std::fs::write(renders.join("2026-11-06").join(DESCRIPTION), vec![b'a'; ceiling + 1]).unwrap();
		assert_eq!(read("2026-11-06").await, Description::OverCeiling);

		std::fs::remove_file(renders.join("2026-11-06").join(DESCRIPTION)).unwrap();
		std::os::unix::fs::symlink(renders.join("2026-10-30").join(DESCRIPTION), renders.join("2026-11-06").join(DESCRIPTION)).unwrap();
		assert_eq!(read("2026-11-06").await, Description::Absent, "a link is never followed");
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
			async move { let_go(&pool, Feed::Call, "", Folders { renders: &renders, input: &weeks }, from, step, NOW).await.unwrap() }
		};
		assert_eq!(go("2026-01-02", Step::Video).await, LetGo::NoEpisode);
		assert_eq!(go("2026-10-30", Step::Video).await, LetGo::NoLink, "no link, nothing goes");
		assert!(renders.join("2026-10-30").join(Feed::Call.video()).exists());
		record_link(&pool, Feed::Call, "", &renders, "2026-10-30", "https://youtu.be/dQw4w9WgXcQ", NOW)
			.await
			.unwrap();
		record_link(&pool, Feed::Call, "", &renders, "2026-11-06", "https://youtu.be/aaaaaaaaaaa", NOW)
			.await
			.unwrap();
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
		let listed = list(&pool, Feed::Call, "", &renders).await.unwrap();
		assert_eq!(listed[1].receipts, [video, rest]);
		assert_eq!((listed[1].video_bytes, listed[1].rest_bytes), (None, None));
	}

	/// A week that is a series (paulgsc/some-ui#2918): each part is listed,
	/// linked and let go on its own; the week's file stays for the worker
	/// while any part has its video, each part let go is marked so it never
	/// renders again, and the last takes the file and the marks with it.
	#[tokio::test]
	async fn a_week_of_parts_lists_links_and_lets_go_each_part_on_its_own() {
		let (pool, dir) = (pool().await, tempfile::tempdir().unwrap());
		let (renders, weeks) = (dir.path().join("call"), dir.path().join("call-weeks"));
		std::fs::create_dir_all(&weeks).unwrap();
		std::fs::write(weeks.join("2026-11-06.json"), b"{}").unwrap();
		hold(&pool, "2026-10-30", "2026-11-05").await;
		hold(&pool, "2026-11-06", "2026-11-12").await;
		render(&renders, "2026-10-30");
		std::fs::create_dir_all(&renders).unwrap();
		std::fs::write(renders.join("2026-11-06.parts"), b"3\n").unwrap();
		render(&renders, "2026-11-06");
		render(&renders, "2026-11-06-p2");

		let listed = list(&pool, Feed::Call, "", &renders).await.unwrap();
		assert_eq!(
			listed.iter().map(|e| (e.key.as_str(), e.week.as_str(), e.part, e.of, e.video_bytes)).collect::<Vec<_>>(),
			[
				("2026-11-06", "2026-11-06", 1, 3, Some(10)),
				("2026-11-06-p2", "2026-11-06", 2, 3, Some(10)),
				("2026-11-06-p3", "2026-11-06", 3, 3, None),
				("2026-10-30", "2026-10-30", 1, 1, Some(10)),
			],
			"newest week first, its parts in order; a week of one video as it always was"
		);
		assert_eq!(
			video_file(&pool, Feed::Call, "", &renders, "2026-11-06-p2").await.unwrap(),
			Some(renders.join("2026-11-06-p2").join(Feed::Call.video()))
		);
		for key in ["2026-11-06-p4", "2026-11-06-p1", "2026-10-30-p2"] {
			assert_eq!(video_file(&pool, Feed::Call, "", &renders, key).await.unwrap(), None, "{key} is no part");
			assert_eq!(
				record_link(&pool, Feed::Call, "", &renders, key, "https://youtu.be/dQw4w9WgXcQ", NOW).await.unwrap(),
				LinkRecorded::NoEpisode,
				"{key}"
			);
		}

		let go = |key: &'static str, step| {
			let (pool, renders, weeks) = (pool.clone(), renders.clone(), weeks.clone());
			async move { let_go(&pool, Feed::Call, "", Folders { renders: &renders, input: &weeks }, key, step, NOW).await.unwrap() }
		};
		record_link(&pool, Feed::Call, "", &renders, "2026-11-06", "https://youtu.be/aaaaaaaaaaa", NOW)
			.await
			.unwrap();
		assert_eq!(go("2026-11-06-p2", Step::Video).await, LetGo::NoLink, "part 1's link is not part 2's");
		record_link(&pool, Feed::Call, "", &renders, "2026-11-06-p2", "https://youtu.be/bbbbbbbbbbb", NOW)
			.await
			.unwrap();
		let LetGo::Gone(two) = go("2026-11-06-p2", Step::Video).await else {
			panic!("part 2's video goes")
		};
		assert_eq!(two.youtube_id, "bbbbbbbbbbb");
		assert!(!renders.join("2026-11-06-p2").join(Feed::Call.video()).exists());
		assert!(renders.join("2026-11-06").join(Feed::Call.video()).exists(), "part 1 keeps its video");
		assert!(weeks.join("2026-11-06.json").exists(), "the worker still has parts to render");
		assert!(weeks.join("2026-11-06-p2.let-go").exists(), "but never part 2 again");
		assert!(matches!(go("2026-11-06-p2", Step::Rest).await, LetGo::Gone(_)));
		assert!(!renders.join("2026-11-06-p2").exists());
		assert!(matches!(go("2026-11-06", Step::Video).await, LetGo::Gone(_)));
		assert!(weeks.join("2026-11-06.let-go").exists(), "part 3 is still to render");

		// The last part rendered and let go: the week leaves the worker whole.
		render(&renders, "2026-11-06-p3");
		record_link(&pool, Feed::Call, "", &renders, "2026-11-06-p3", "https://youtu.be/ccccccccccc", NOW)
			.await
			.unwrap();
		assert!(matches!(go("2026-11-06-p3", Step::Video).await, LetGo::Gone(_)));
		assert!(!weeks.join("2026-11-06.json").exists(), "every part gone, the week is never rendered again");
		let left: Vec<_> = std::fs::read_dir(&weeks).unwrap().collect();
		assert!(left.is_empty(), "its marks go with it");
		let mut conn = pool.acquire().await.unwrap();
		assert_eq!(video::weeks_let_go(&mut conn).await.unwrap(), ["2026-11-06"]);
	}

	#[tokio::test]
	async fn a_step_the_drive_refuses_keeps_its_receipt_and_is_finished_when_asked_again() {
		let (pool, dir) = (pool().await, tempfile::tempdir().unwrap());
		let (renders, weeks) = (dir.path().join("call"), dir.path().join("call-weeks"));
		hold(&pool, "2026-10-30", "2026-11-05").await;
		record_link(&pool, Feed::Call, "", &renders, "2026-10-30", "https://youtu.be/dQw4w9WgXcQ", NOW)
			.await
			.unwrap();
		// A folder where the week's file would be: removing it as a file fails,
		// as a drive that refuses would.
		let blocked = weeks.join("2026-10-30.json");
		std::fs::create_dir_all(blocked.join("x")).unwrap();
		render(&renders, "2026-10-30");

		assert!(
			let_go(&pool, Feed::Call, "", Folders { renders: &renders, input: &weeks }, "2026-10-30", Step::Video, NOW)
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
			let_go(&pool, Feed::Call, "", Folders { renders: &renders, input: &weeks }, "2026-10-30", Step::Video, later)
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

		let listed = list(&pool, Feed::PlayoffDelta, "", &renders).await.unwrap();
		assert_eq!(
			listed.iter().map(|e| (e.key.as_str(), e.video_bytes, e.rest_bytes)).collect::<Vec<_>>(),
			[("2026-SF-09", Some(10), Some(2))]
		);
		assert!(list(&pool, Feed::Call, "", &renders).await.unwrap().is_empty(), "each feed lists its own");
		let folders = Folders {
			renders: &renders,
			input: &episodes,
		};
		let go = |step| let_go(&pool, Feed::PlayoffDelta, "", folders, "2026-SF-09", step, NOW);
		assert_eq!(go(Step::Video).await.unwrap(), LetGo::NoLink);
		assert_eq!(
			record_link(&pool, Feed::PlayoffDelta, "", &renders, "2026-SF-09", "https://youtu.be/dQw4w9WgXcQ", NOW)
				.await
				.unwrap(),
			LinkRecorded::Kept("dQw4w9WgXcQ".to_owned())
		);
		assert_eq!(
			record_link(&pool, Feed::PlayoffDelta, "", &renders, "2026-SF-10", "https://youtu.be/dQw4w9WgXcQ", NOW)
				.await
				.unwrap(),
			LinkRecorded::NoEpisode
		);
		let LetGo::Gone(video) = go(Step::Video).await.unwrap() else { panic!("the video goes") };
		assert_eq!(video.size, 10);
		assert!(!episodes.join("2026-SF-09").exists(), "the worker is not given it again");
		assert!(folder.join("cuts.json").exists(), "the rest stays");
		let LetGo::Gone(rest) = go(Step::Rest).await.unwrap() else { panic!("the rest goes") };
		assert_eq!((rest.size, folder.exists()), (2, false));
		assert_eq!(list(&pool, Feed::PlayoffDelta, "", &renders).await.unwrap()[0].receipts, [video, rest]);
	}

	/// An essay's video is its subject's: listed, brought, linked and let go
	/// only by them, and its input goes with its video, as every feed's.
	#[tokio::test]
	async fn an_essay_is_its_subjects_alone_and_goes_video_first_as_every_feeds() {
		let (pool, dir) = (pool().await, tempfile::tempdir().unwrap());
		let (renders, essays) = (dir.path().join("rust-essay"), dir.path().join("essays"));
		for subject in ["me", "them"] {
			sqlx::query("INSERT INTO account (subject_id, user_handle) VALUES (?1, ?1)")
				.bind(subject)
				.execute(&pool)
				.await
				.unwrap();
		}
		sqlx::query(
			"INSERT INTO mailbox_item (subject_id, folder, name, seq, sha256, size, body, pulled_at) VALUES ('me', 'brief', 'rust-essay-2026-w46.json', 1, 's', 2, x'7b7d', 'now')",
		)
		.execute(&pool)
		.await
		.unwrap();
		let folder = renders.join("2026-w46");
		std::fs::create_dir_all(&folder).unwrap();
		std::fs::write(folder.join(Feed::RustEssay.video()), [0_u8; 10]).unwrap();
		std::fs::write(folder.join("cut-list.json"), [0_u8; 2]).unwrap();
		std::fs::create_dir_all(essays.join("2026-w46")).unwrap();

		let listed = list(&pool, Feed::RustEssay, "me", &renders).await.unwrap();
		assert_eq!(listed.iter().map(|e| (e.key.as_str(), e.video_bytes)).collect::<Vec<_>>(), [("2026-w46", Some(10))]);
		assert!(list(&pool, Feed::RustEssay, "them", &renders).await.unwrap().is_empty(), "another subject's is not theirs");
		assert_eq!(video_file(&pool, Feed::RustEssay, "them", &renders, "2026-w46").await.unwrap(), None);
		assert_eq!(
			record_link(&pool, Feed::RustEssay, "them", &renders, "2026-w46", "https://youtu.be/dQw4w9WgXcQ", NOW)
				.await
				.unwrap(),
			LinkRecorded::NoEpisode
		);
		let folders = Folders {
			renders: &renders,
			input: &essays,
		};
		assert_eq!(
			let_go(&pool, Feed::RustEssay, "them", folders, "2026-w46", Step::Video, NOW).await.unwrap(),
			LetGo::NoEpisode
		);
		assert_eq!(
			record_link(&pool, Feed::RustEssay, "me", &renders, "2026-w46", "https://youtu.be/dQw4w9WgXcQ", NOW)
				.await
				.unwrap(),
			LinkRecorded::Kept("dQw4w9WgXcQ".to_owned())
		);
		let LetGo::Gone(video) = let_go(&pool, Feed::RustEssay, "me", folders, "2026-w46", Step::Video, NOW).await.unwrap() else {
			panic!("the video goes")
		};
		assert_eq!(video.size, 10);
		assert!(!essays.join("2026-w46").exists(), "the worker is not given it again");
		let LetGo::Gone(rest) = let_go(&pool, Feed::RustEssay, "me", folders, "2026-w46", Step::Rest, NOW).await.unwrap() else {
			panic!("the rest goes")
		};
		assert_eq!((rest.size, folder.exists()), (2, false));
		assert_eq!(list(&pool, Feed::RustEssay, "me", &renders).await.unwrap()[0].receipts, [video, rest]);
	}

	/// A week withdrawn because a take its essay plays was deleted from My
	/// data has no video left to list or let go: the feed leaves it out,
	/// though the subject still holds its essay in `brief`, so it is never
	/// shown as waiting for a render that will not come.
	#[tokio::test]
	async fn an_essay_withdrawn_for_a_deleted_take_is_not_listed() {
		let (pool, dir) = (pool().await, tempfile::tempdir().unwrap());
		let (renders, essays) = (dir.path().join("rust-essay"), dir.path().join("essays"));
		sqlx::query("INSERT INTO account (subject_id, user_handle) VALUES ('me', 'me')")
			.execute(&pool)
			.await
			.unwrap();
		for (seq, week) in [(1, "2026-w45"), (2, "2026-w46")] {
			sqlx::query("INSERT INTO mailbox_item (subject_id, folder, name, seq, sha256, size, body, pulled_at) VALUES ('me', 'brief', ?1, ?2, 's', 2, x'7b7d', 'now')")
				.bind(String::from("rust-essay-") + week + ".json")
				.bind(seq)
				.execute(&pool)
				.await
				.unwrap();
		}
		let listed =
			|pool: SqlitePool, renders: PathBuf| async move { list(&pool, Feed::RustEssay, "me", &renders).await.unwrap().into_iter().map(|e| e.key).collect::<Vec<_>>() };
		assert_eq!(listed(pool.clone(), renders.clone()).await, ["2026-w46", "2026-w45"]);

		let mut tx = pool.begin().await.unwrap();
		assert!(rust_round_repo::essay_video::withdraw(&mut tx, "me", "2026-w46", "note-311", 0, NOW).await.unwrap());
		tx.commit().await.unwrap();
		assert_eq!(listed(pool.clone(), renders.clone()).await, ["2026-w45"], "withdrawn: not listed, so never waiting");
		let folders = Folders {
			renders: &renders,
			input: &essays,
		};
		assert_eq!(
			let_go(&pool, Feed::RustEssay, "me", folders, "2026-w46", Step::Video, NOW).await.unwrap(),
			LetGo::NoEpisode,
			"and there is nothing of it to let go"
		);
	}
}

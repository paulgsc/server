//! The weekly call's videos on the phone (STUDIO ST38: paulgsc/some-ui#2661,
//! the owner's decisions of 2026-11-07).
//!
//! The owner watches each week's video on the phone, shares it to the
//! `YouTube` app, records the week's `YouTube` link, and then lets home's copy
//! go from My data in two steps: the video, then the rest of the folder.
//! This is what `handlers::call_videos` answers a paired phone with:
//!
//! - [`list`]: each week home holds, newest first, with the bytes of its
//!   video and of the rest of its folder, its link and its receipts.
//! - [`video_file`]: where a rendered week's `call.mp4` is, for the download.
//! - [`let_go`]: one step of letting a week go, with its receipt.
//!
//! **What a step deletes, and in what order.** The video's step removes the
//! week's file from `call_weeks::RENDER_FOLDER`, so the render worker is
//! never given it again, then its `call.mp4`; the rest's step removes the
//! week's folder under `renders/call`. The week's words stay in `call_week`.
//!
//! **The receipt first, then the files.** A transaction cannot take back a
//! deleted file, so each step commits its receipt (`call_week_repo::video`)
//! before it touches the drive, and answers only once the files are gone. A
//! step whose files did not all go (the drive refused, or the process
//! stopped) is finished the next time it is asked for: a step that has a
//! receipt removes what is left of it again, then answers with that receipt.
//! So a receipt never stands without its delete under way, and a delete
//! never happens without its receipt.
//!
//! **One at a time with the export pass.** [`super::FOLDERS`] is held for a
//! whole step, as the export pass holds it for a whole pass, so a pass never
//! writes a week's file back between reading which weeks were let go and
//! writing the ones that are missing.
//!
//! **Paths.** A week's folder is joined only after `call_week` says home
//! holds the week, and the table holds only `YYYY-MM-DD` days, so a path
//! never leaves `renders/call`.

use super::render::bytes_under;
use call_week_repo::video::{self, Receipt, Recorded, Step};
use sqlx::SqlitePool;
use std::path::{Path, PathBuf};

/// The video's file in a week's folder.
pub const VIDEO: &str = "call.mp4";

/// A week as the phone sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeekVideo {
	pub from: String,
	pub to: String,
	/// Bytes of its `call.mp4`, while home has it.
	pub video_bytes: Option<u64>,
	/// Bytes of the rest of its folder, while home has it.
	pub rest_bytes: Option<u64>,
	/// The week's `YouTube` link, once recorded.
	pub link: Option<video::Link>,
	/// Each step let go, video first.
	pub receipts: Vec<Receipt>,
}

/// Each week home holds, newest first.
///
/// # Errors
/// A storage failure.
pub async fn list(db: &SqlitePool, renders: &Path) -> Result<Vec<WeekVideo>, sqlx::Error> {
	let (weeks, links, receipts) = {
		let mut tx = db.begin().await?;
		let weeks = call_week_repo::held(&mut tx).await?;
		let links = video::links(&mut tx).await?;
		let receipts = video::receipts(&mut tx).await?;
		tx.commit().await?;
		(weeks, links, receipts)
	};
	let mut listed = Vec::with_capacity(weeks.len());
	for week in weeks {
		let folder = renders.join(&week.from);
		let video_bytes = tokio::fs::metadata(folder.join(VIDEO)).await.ok().filter(std::fs::Metadata::is_file).map(|meta| meta.len());
		let rest_bytes = if tokio::fs::metadata(&folder).await.is_ok_and(|meta| meta.is_dir()) {
			Some(bytes_under(&folder).await.saturating_sub(video_bytes.unwrap_or(0)))
		} else {
			None
		};
		let mine: Vec<Receipt> = receipts.iter().filter(|receipt| receipt.from == week.from).cloned().collect();
		listed.push(WeekVideo {
			link: links.get(&week.from).cloned(),
			from: week.from,
			to: week.to,
			video_bytes,
			rest_bytes,
			receipts: mine,
		});
	}
	Ok(listed)
}

/// Where a rendered week's video is, if home holds the week and its video.
///
/// # Errors
/// A storage failure.
pub async fn video_file(db: &SqlitePool, renders: &Path, from: &str) -> Result<Option<PathBuf>, sqlx::Error> {
	if !holds(db, from).await? {
		return Ok(None);
	}
	let file = renders.join(from).join(VIDEO);
	Ok(tokio::fs::metadata(&file).await.is_ok_and(|meta| meta.is_file()).then_some(file))
}

async fn holds(db: &SqlitePool, from: &str) -> Result<bool, sqlx::Error> {
	let mut conn = db.acquire().await?;
	Ok(call_week_repo::body(&mut conn, from).await?.is_some())
}

/// What [`record_link`] did with a pasted link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkRecorded {
	/// Kept (or already held): the week's link is now this video's.
	Kept(String),
	/// The pasted text names no `YouTube` video.
	NotAVideo,
	/// Home holds no week of that first day.
	NoWeek,
}

/// Records a pasted link as the week's `YouTube` link.
///
/// # Errors
/// A storage failure.
pub async fn record_link(db: &SqlitePool, from: &str, pasted: &str, now: &str) -> Result<LinkRecorded, sqlx::Error> {
	let Some(id) = video::youtube_id(pasted) else {
		return Ok(LinkRecorded::NotAVideo);
	};
	let mut conn = db.acquire().await?;
	Ok(match video::record_link(&mut conn, from, &id, now).await? {
		Recorded::NoWeek => LinkRecorded::NoWeek,
		Recorded::Added | Recorded::Held => LinkRecorded::Kept(id),
	})
}

/// What [`let_go`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LetGo {
	/// The step is done: its receipt, new or as first written.
	Gone(Receipt),
	/// Home holds no week of that first day.
	NoWeek,
	/// The video goes only once the week's `YouTube` link is recorded.
	NoLink,
	/// The rest goes only once the video has.
	VideoFirst,
	/// The week has no video to let go: it is not rendered yet.
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

/// Lets one step of a week go (the module doc), under `BEGIN IMMEDIATE` so
/// two phones asking at once write one receipt.
///
/// # Errors
/// A storage failure, or a file the drive would not remove. The receipt is
/// kept then, and asking for the step again finishes it.
pub async fn let_go(db: &SqlitePool, renders: &Path, weeks: &Path, from: &str, step: Step, now: &str) -> Result<LetGo, LetGoError> {
	let _folders = super::FOLDERS.lock().await;
	let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
	// A receipt names a week home holds (its foreign key), so the path below
	// is one of `renders/call`'s.
	if let Some(receipt) = video::receipt(&mut tx, from, step).await? {
		tx.commit().await?;
		finish(renders, weeks, from, step).await?;
		return Ok(LetGo::Gone(receipt));
	}
	if call_week_repo::body(&mut tx, from).await?.is_none() {
		return Ok(LetGo::NoWeek);
	}
	let Some(link) = video::links(&mut tx).await?.remove(from) else {
		return Ok(LetGo::NoLink);
	};
	let folder = renders.join(from);
	let size = match step {
		Step::Video => {
			let Ok(meta) = tokio::fs::metadata(folder.join(VIDEO)).await else {
				return Ok(LetGo::NotRendered);
			};
			meta.len()
		}
		Step::Rest => {
			if video::receipt(&mut tx, from, Step::Video).await?.is_none() {
				return Ok(LetGo::VideoFirst);
			}
			bytes_under(&folder).await
		}
	};
	let receipt = Receipt {
		from: from.to_owned(),
		step,
		youtube_id: link.youtube_id,
		size: i64::try_from(size).unwrap_or(i64::MAX),
		deleted_at: now.to_owned(),
	};
	video::write_receipt(&mut tx, &receipt).await?;
	tx.commit().await?;
	finish(renders, weeks, from, step).await?;
	Ok(LetGo::Gone(receipt))
}

/// Removes what a step lets go; what is already gone is fine.
async fn finish(renders: &Path, weeks: &Path, from: &str, step: Step) -> std::io::Result<()> {
	let folder = renders.join(from);
	match step {
		Step::Video => {
			// The week's file first: the worker renders any week it is given
			// that has no video.
			remove_file(&weeks.join(String::from(from) + ".json")).await?;
			remove_file(&folder.join(VIDEO)).await
		}
		Step::Rest => match tokio::fs::remove_dir_all(&folder).await {
			Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error),
			_ => Ok(()),
		},
	}
}

/// Removes a file; one already gone is fine.
async fn remove_file(path: &Path) -> std::io::Result<()> {
	match tokio::fs::remove_file(path).await {
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
		std::fs::write(folder.join(VIDEO), [0_u8; 10]).unwrap();
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

		let listed = list(&pool, &renders).await.unwrap();
		assert_eq!(
			listed.iter().map(|week| week.from.as_str()).collect::<Vec<_>>(),
			["2026-11-06", "2026-10-30"],
			"newest first"
		);
		assert_eq!((listed[0].video_bytes, listed[0].rest_bytes), (None, None), "not rendered yet");
		assert_eq!((listed[1].video_bytes, listed[1].rest_bytes), (Some(10), Some(5)));
		assert_eq!(listed[1].link, None);

		let pasted = "https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=1";
		assert_eq!(record_link(&pool, "2026-10-30", pasted, NOW).await.unwrap(), LinkRecorded::Kept("dQw4w9WgXcQ".to_owned()));
		assert_eq!(record_link(&pool, "2026-10-30", "https://youtube.com/@me", NOW).await.unwrap(), LinkRecorded::NotAVideo);
		assert_eq!(record_link(&pool, "2026-01-02", pasted, NOW).await.unwrap(), LinkRecorded::NoWeek);
		let listed = list(&pool, &renders).await.unwrap();
		assert_eq!(listed[1].link.as_ref().map(|link| link.youtube_id.as_str()), Some("dQw4w9WgXcQ"));
		assert_eq!(video_file(&pool, &renders, "2026-10-30").await.unwrap(), Some(renders.join("2026-10-30").join(VIDEO)));
		assert_eq!(video_file(&pool, &renders, "2026-11-06").await.unwrap(), None, "not rendered");
		assert_eq!(video_file(&pool, &renders, "2026-01-02").await.unwrap(), None, "not held");
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
			async move { let_go(&pool, &renders, &weeks, from, step, NOW).await.unwrap() }
		};
		assert_eq!(go("2026-01-02", Step::Video).await, LetGo::NoWeek);
		assert_eq!(go("2026-10-30", Step::Video).await, LetGo::NoLink, "no link, nothing goes");
		assert!(renders.join("2026-10-30").join(VIDEO).exists());
		record_link(&pool, "2026-10-30", "https://youtu.be/dQw4w9WgXcQ", NOW).await.unwrap();
		record_link(&pool, "2026-11-06", "https://youtu.be/aaaaaaaaaaa", NOW).await.unwrap();
		assert_eq!(go("2026-11-06", Step::Video).await, LetGo::NotRendered);
		assert_eq!(go("2026-10-30", Step::Rest).await, LetGo::VideoFirst);
		assert!(renders.join("2026-10-30").join("cuts.json").exists());

		let LetGo::Gone(video) = go("2026-10-30", Step::Video).await else {
			panic!("the video goes")
		};
		assert_eq!((video.size, video.youtube_id.as_str(), video.deleted_at.as_str()), (10, "dQw4w9WgXcQ", NOW));
		assert!(!renders.join("2026-10-30").join(VIDEO).exists());
		assert!(!weeks.join("2026-10-30.json").exists(), "the worker is not given it again");
		assert!(renders.join("2026-10-30").join("cuts.json").exists(), "the rest stays");
		assert_eq!(go("2026-10-30", Step::Video).await, LetGo::Gone(video.clone()), "a repeat answers with the same receipt");

		let LetGo::Gone(rest) = go("2026-10-30", Step::Rest).await else {
			panic!("the rest goes")
		};
		assert_eq!(rest.size, 5);
		assert!(!renders.join("2026-10-30").exists());
		let listed = list(&pool, &renders).await.unwrap();
		assert_eq!(listed[1].receipts, [video, rest]);
		assert_eq!((listed[1].video_bytes, listed[1].rest_bytes), (None, None));
	}

	#[tokio::test]
	async fn a_step_the_drive_refuses_keeps_its_receipt_and_is_finished_when_asked_again() {
		let (pool, dir) = (pool().await, tempfile::tempdir().unwrap());
		let (renders, weeks) = (dir.path().join("call"), dir.path().join("call-weeks"));
		hold(&pool, "2026-10-30", "2026-11-05").await;
		record_link(&pool, "2026-10-30", "https://youtu.be/dQw4w9WgXcQ", NOW).await.unwrap();
		// A folder where the week's file would be: removing it as a file fails,
		// as a drive that refuses would.
		let blocked = weeks.join("2026-10-30.json");
		std::fs::create_dir_all(blocked.join("x")).unwrap();
		render(&renders, "2026-10-30");

		assert!(let_go(&pool, &renders, &weeks, "2026-10-30", Step::Video, NOW).await.is_err(), "not answered as done");
		let mut conn = pool.acquire().await.unwrap();
		let kept = video::receipt(&mut conn, "2026-10-30", Step::Video).await.unwrap().unwrap();
		drop(conn);
		assert_eq!(kept.size, 10, "the receipt stands, so the step is never lost");

		std::fs::remove_dir_all(&blocked).unwrap();
		let later = "2026-11-07T13:00:00.000Z";
		assert_eq!(let_go(&pool, &renders, &weeks, "2026-10-30", Step::Video, later).await.unwrap(), LetGo::Gone(kept));
		assert!(!renders.join("2026-10-30").join(VIDEO).exists(), "asked again, it is finished");
	}
}

//! Home's pictures of its own dashboards, for the public site
//! (paulgsc/some-ui #3415, the owner's yes; C1071, C1073, C1074).
//!
//! Once a week, home's render worker walks home's own Grafana panels, over
//! aggregate data only, and makes the server card's clip
//! (`server.mp4`, six panels of `file_host`'s dashboard) and the cores card's
//! still (`cores.png`, the CPU panel), and home puts both in the mailbox's
//! `site-assets` folder, where the Pages build's Site Assets run reads them
//! with its own read-only token. Home holds no GitHub token for it.
//!
//! **What it asks.** For the week that ended at the last Monday 00:00 UTC
//! ([`window_end`]), unless that week was already sent or refused, this
//! writes `<day>.json` in [`SHOTS_FOLDER`]: the walk's sample (some-ui's
//! `some-ui.walk-sample/home-dashboards/1`): `file_host`'s version, Grafana as
//! the render worker reaches it ([`GRAFANA`]), the window's end and its
//! [`DAYS`]. The render worker's `site-shots` job walks it on home's lane, as
//! every batch job (`infra/render/render-calls.sh`; docs/home-compute.md),
//! and moves the finished folder into [`RENDERS_FOLDER`]. A week's request
//! is a time that has passed, never a clock that starts work: the lane does.
//!
//! **What it puts.** Each mailbox pass, a finished week's two files are
//! checked (each at most [`PICTURE_MAX_BYTES`], the mailbox's own cap, and
//! the type its name says), put as `cores-<stamp>.png` and
//! `server-<stamp>.mp4`, the stamp the window's end, and then home lets go
//! of them: the request, the render and its work (home keeps no picture
//! library, docs/snips.md invariant 1; the mailbox keeps only the newest of
//! each kind). A `<day>.sent` mark stops the week being asked again; a
//! picture the mailbox refuses, or one too big to send, leaves `<day>.refused`
//! with why, and the week is not asked again either: the next week's is. The
//! still goes first; a week whose clip is then refused leaves its still in
//! the mailbox, which is the newest of its kind there either way.
//!
//! **What the pictures hold.** Counts, rates, shares and uptime, never a
//! client, an address or a subject: the panels are named in some-ui's
//! `home-dashboards.ts`, and docs/identity.md invariant 39 holds them.
//!
//! Cost: each mailbox pass, a few `stat`s and at most one small write; once a
//! week, two reads of at most [`PICTURE_MAX_BYTES`] and two puts.

use chrono::{DateTime, Datelike, Duration, NaiveDate, Utc};
use mailbox_kit::{BodyType, Folder, Mailbox, MailboxError, MailboxTransport, Put};
use std::path::{Path, PathBuf};

/// Where each week's request is written: on the host, `${STORAGE}/site-shots`,
/// mounted read-only into the render worker as `/site-shots`.
pub const SHOTS_FOLDER: &str = "/app/dbs/site-shots";

/// Where the render worker keeps a week's finished pictures, as this server
/// sees it.
pub const RENDERS_FOLDER: &str = "/app/dbs/renders/site-shots";

/// Where the render worker says how its passes went, as this server sees it.
pub const STATUS_FOLDER: &str = "/app/dbs/renders/site-shots-worker";

/// Where the render worker keeps a waiting week's work, as this server sees it.
pub const WORK_FOLDER: &str = "/app/dbs/renders/site-shots-work";

/// Grafana, as the render worker reaches it on `site-shots-network`
/// (`infra/compose/call-render.yml`).
pub const GRAFANA: &str = "http://grafana:3000";

/// How many days a window holds, ending at its Monday.
pub const DAYS: i64 = 7;

/// The mailbox's cap on a picture (some-ui `apps/mailbox`, `PICTURE_MAX_BYTES`,
/// C1072): one past it is never sent.
pub const PICTURE_MAX_BYTES: u64 = 786_432;

/// The server card's clip, in a week's finished folder.
pub const CLIP: &str = "server.mp4";

/// The cores card's still, in a week's finished folder.
pub const STILL: &str = "cores.png";

/// The folders it reads and writes; the tests move them.
#[derive(Debug, Clone)]
pub struct Folders {
	pub shots: PathBuf,
	pub renders: PathBuf,
	pub work: PathBuf,
}

impl Folders {
	#[must_use]
	pub fn server() -> Self {
		Self {
			shots: PathBuf::from(SHOTS_FOLDER),
			renders: PathBuf::from(RENDERS_FOLDER),
			work: PathBuf::from(WORK_FOLDER),
		}
	}
}

/// Make the render worker's folders, before the server answers its health
/// check, by this server's uid, as every job's are.
pub fn start() {
	for folder in [SHOTS_FOLDER, RENDERS_FOLDER, STATUS_FOLDER, WORK_FOLDER] {
		if std::fs::create_dir_all(folder).is_err() {
			tracing::error!(error = "render folder", "site shots cannot make the render worker's folders");
		}
	}
}

/// The end of the last whole week: the latest Monday, 00:00 UTC, at or
/// before `now`.
#[must_use]
pub fn window_end(now: DateTime<Utc>) -> NaiveDate {
	let today = now.date_naive();
	today - Duration::days(i64::from(today.weekday().num_days_from_monday()))
}

/// A week's key in the folders: its end's day, `YYYY-MM-DD`.
fn key(end: NaiveDate) -> String {
	end.format("%Y-%m-%d").to_string()
}

/// A picture's name in the mailbox: its kind, then the window's end.
fn name(kind: &str, end: NaiveDate, ext: &str) -> String {
	String::from(kind) + "-" + &end.format("%Y%m%d").to_string() + "T000000000Z." + ext
}

/// A week's file in a folder: `<day><suffix>`.
fn file(day: &str, suffix: &str) -> String {
	String::from(day) + suffix
}

/// The walk's sample, as some-ui's `home-dashboards.ts` reads it.
fn sample(end: NaiveDate, version: &str) -> Vec<u8> {
	let body = serde_json::json!({
		"schema": "some-ui.walk-sample/home-dashboards/1",
		"version": version,
		"origin": GRAFANA,
		"to": key(end) + "T00:00:00Z",
		"days": DAYS,
	});
	let mut bytes = serde_json::to_vec_pretty(&body).unwrap_or_default();
	bytes.push(b'\n');
	bytes
}

/// What a pass did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Done {
	/// The week's request was written.
	pub asked: bool,
	/// The week's pictures went to the mailbox.
	pub sent: bool,
	/// Why the week's pictures were not sent and will not be asked again.
	pub refused: Option<String>,
}

/// Why a pass stopped; it is asked again next pass.
#[derive(Debug)]
pub enum Failed {
	Drive(std::io::Error),
	Mailbox(MailboxError),
}

impl Failed {
	/// A failure that will not clear by itself: the mailbox refuses home's
	/// token for the folder or does not have it (a Worker deployed before the
	/// site's pictures). Said as an error, so it reaches the owner.
	#[must_use]
	pub const fn lasting(&self) -> bool {
		matches!(self, Self::Mailbox(MailboxError::Unauthorized | MailboxError::Forbidden | MailboxError::NotFound))
	}
}

impl std::fmt::Display for Failed {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::Drive(_) => f.write_str("the drive"),
			Self::Mailbox(err) => write!(f, "the mailbox: {err}"),
		}
	}
}

impl From<std::io::Error> for Failed {
	fn from(err: std::io::Error) -> Self {
		Self::Drive(err)
	}
}

/// Why `bytes` may not go as a picture of `kind`, or `None`.
fn refusal(kind: BodyType, bytes: &[u8]) -> Option<&'static str> {
	if u64::try_from(bytes.len()).map_or(true, |len| len > PICTURE_MAX_BYTES) {
		return Some("over the mailbox's 768 KiB for a picture");
	}
	let fits = match kind {
		BodyType::Png => bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
		BodyType::Mp4 => bytes.get(4..8) == Some(b"ftyp".as_slice()),
		BodyType::Json => false,
	};
	(!fits).then_some("not the type its name says")
}

/// Written whole, so the render worker never reads half a file.
async fn write_whole(folder: &Path, file: &str, body: &[u8]) -> std::io::Result<()> {
	let partial = folder.join(String::from(".") + file);
	tokio::fs::write(&partial, body).await?;
	tokio::fs::rename(&partial, folder.join(file)).await
}

async fn remove_dir(path: &Path) -> std::io::Result<()> {
	match tokio::fs::remove_dir_all(path).await {
		Err(err) if err.kind() != std::io::ErrorKind::NotFound => Err(err),
		_ => Ok(()),
	}
}

async fn remove_file(path: &Path) -> std::io::Result<()> {
	match tokio::fs::remove_file(path).await {
		Err(err) if err.kind() != std::io::ErrorKind::NotFound => Err(err),
		_ => Ok(()),
	}
}

/// Lets go of every week but `keep`: requests, marks, renders and work. A
/// week before the last one is never asked or sent, so nothing of it stays.
async fn tidy(folders: &Folders, keep: &str) -> std::io::Result<()> {
	for folder in [&folders.shots, &folders.renders, &folders.work] {
		let mut entries = match tokio::fs::read_dir(folder).await {
			Ok(entries) => entries,
			Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
			Err(err) => return Err(err),
		};
		while let Some(entry) = entries.next_entry().await? {
			let file = entry.file_name();
			let Some(file) = file.to_str() else { continue };
			// A week's own files: `<day>`, `<day>.json`, `<day>.sent`, `<day>.refused`.
			let day = file.split('.').next().unwrap_or_default();
			if day.len() != 10 || NaiveDate::parse_from_str(day, "%Y-%m-%d").is_err() || day == keep {
				continue;
			}
			if entry.file_type().await?.is_dir() {
				remove_dir(&entry.path()).await?;
			} else {
				remove_file(&entry.path()).await?;
			}
		}
	}
	Ok(())
}

/// Puts one picture. `Ok(None)` when the mailbox holds it (added, the same
/// bytes, or a newer one of its kind), `Ok(Some(why))` when it is refused for
/// good, and `Err` for what may clear by the next pass.
async fn put_one<T: MailboxTransport>(mailbox: &Mailbox<T>, name: &str, kind: BodyType, bytes: Vec<u8>) -> Result<Option<String>, Failed> {
	if let Some(why) = refusal(kind, &bytes) {
		return Ok(Some(why.to_owned()));
	}
	match mailbox.put_as(Folder::SiteAssets, name, bytes, kind).await {
		// A conflict is other bytes under the week's name, or a newer
		// picture of its kind: either way the mailbox holds one.
		Ok(Put::Added { .. } | Put::Unchanged { .. } | Put::Conflict) => Ok(None),
		Err(MailboxError::TooBig) => Ok(Some("the mailbox found it too big".to_owned())),
		Err(MailboxError::Refused(why)) => Ok(Some(String::from("the mailbox refused it: ") + &why)),
		Err(err) => Err(Failed::Mailbox(err)),
	}
}

/// One pass: ask for the last whole week's pictures, and send them once the
/// render worker has made them. Called from the mailbox pass, last.
///
/// # Errors
/// [`Failed`] for the drive or a mailbox answer that may clear next pass.
pub async fn run_once<T: MailboxTransport>(mailbox: &Mailbox<T>, folders: &Folders, version: &str, now: DateTime<Utc>) -> Result<Done, Failed> {
	let end = window_end(now);
	let day = key(end);
	tidy(folders, &day).await?;
	let (sent, refused) = (folders.shots.join(file(&day, ".sent")), folders.shots.join(file(&day, ".refused")));
	if tokio::fs::try_exists(&sent).await? || tokio::fs::try_exists(&refused).await? {
		return Ok(Done::default());
	}
	let request = folders.shots.join(file(&day, ".json"));
	let mut done = Done::default();
	if !tokio::fs::try_exists(&request).await? {
		write_whole(&folders.shots, &file(&day, ".json"), &sample(end, version)).await?;
		done.asked = true;
	}
	let finished = folders.renders.join(&day);
	let (clip, still) = (finished.join(CLIP), finished.join(STILL));
	if !(tokio::fs::try_exists(&clip).await? && tokio::fs::try_exists(&still).await?) {
		return Ok(done);
	}
	let why = match put_one(mailbox, &name("cores", end, "png"), BodyType::Png, tokio::fs::read(&still).await?).await? {
		Some(why) => Some(why),
		None => put_one(mailbox, &name("server", end, "mp4"), BodyType::Mp4, tokio::fs::read(&clip).await?).await?,
	};
	// The request goes first, so the worker never makes the week again.
	remove_file(&request).await?;
	match &why {
		Some(why) => write_whole(&folders.shots, &file(&day, ".refused"), (why.clone() + "\n").as_bytes()).await?,
		None => write_whole(&folders.shots, &file(&day, ".sent"), b"").await?,
	}
	remove_dir(&finished).await?;
	remove_dir(&folders.work.join(&day)).await?;
	done.sent = why.is_none();
	done.refused = why;
	Ok(done)
}

#[cfg(test)]
mod tests;

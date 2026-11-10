//! Home pulls a newer image on demand, when a client needs a newer server
//! than the one it runs, and only then (the owner's ask, 2026-11-10;
//! README.md, "Deploying to home").
//!
//! A client asks `GET /compat?needs=<epoch>` with the contract epoch it was
//! built against (`crate::contract`). [`decide`] answers from that, this
//! build's epoch, and what home's deploy last recorded; when a deploy is
//! wanted it writes [`WANTED`] into [`FOLDER`], which is `${STORAGE}/deploy`
//! on the host. A systemd path unit there (`nix/modules/home-deploy.nix`)
//! runs `scripts/deploy.py --on-demand`, which pulls, deploys only an image
//! whose epoch is above this one, health-waits, rolls back, writes
//! [`STATE`] and removes the marker. Nothing polls: with no client newer
//! than home, home sends no request anywhere.
//!
//! # Bounds (no loops, no greed)
//!
//! - **One at a time.** The marker is created exclusively (`hard_link`, which
//!   fails when it exists), and while it exists every ask is answered
//!   `in_flight`. systemd runs the deploy unit once at a time.
//! - **A gap between pulls.** No marker within [`PULL_GAP`] of the last pull.
//! - **A need a pull could not meet waits.** When the last pull, within
//!   [`UNMET_RETRY`], found the newest image's epoch below `needs`, the ask is
//!   answered `unavailable` and nothing is pulled: at most four requests to
//!   Docker Hub in six hours for a client nothing published can serve.
//! - **A ceiling.** `needs` above `CONTRACT_EPOCH` plus [`CEILING`] is refused:
//!   a forged or mistyped `needs=999999` cannot start a pull. The unmet rule
//!   above bounds it too, but only after one pull.
//! - **Never down.** A client below `OLDEST_SUPPORTED` is told it is behind
//!   home (`client_behind`), and one at or below `CONTRACT_EPOCH` is served:
//!   no older or equal epoch is ever a reason to pull.

use crate::contract::{CONTRACT_EPOCH, OLDEST_SUPPORTED};
use serde::{Deserialize, Serialize};
use std::io::Write as _;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// `${STORAGE}/deploy` on the host: the marker `file_host` writes and the
/// state home's deploy writes. Beside the database, as `call_weeks`'
/// folder is.
pub const FOLDER: &str = "/app/dbs/deploy";
/// The deploy-wanted marker's file name in [`FOLDER`].
pub const WANTED: &str = "wanted";
/// The file name, in [`FOLDER`], of what home's deploy last did
/// (`scripts/deploy.py`, `--on-demand`). Read only here.
pub const STATE: &str = "state.json";

/// The shortest time between two pulls.
pub const PULL_GAP: Duration = Duration::from_secs(10 * 60);
/// How long a need the last pull could not meet waits before another.
pub const UNMET_RETRY: Duration = Duration::from_secs(6 * 60 * 60);
/// How far above this build's epoch a `needs` is believed.
pub const CEILING: u32 = 1000;

/// What home's deploy recorded after its last run (`state.json`). Every
/// field may be missing: a first deploy has written nothing.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct Recorded {
	/// When it last pulled, in seconds since the epoch.
	pub last_pull_at: Option<i64>,
	/// The contract epoch of the newest image that pull fetched, when it
	/// could read one.
	pub latest_seen_epoch: Option<u32>,
	/// What the last run did: its exit code and outcome, and when.
	pub last_run: Option<LastRun>,
	/// Running totals, by what each run did.
	#[serde(default)]
	pub counts: Counts,
}

/// The last run, as `state.json` has it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct LastRun {
	pub at: i64,
	pub exit: i32,
	pub outcome: String,
}

/// Every run home's deploy made, by outcome. Cumulative; read as counters.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct Counts {
	#[serde(default)]
	pub pulls: u64,
	#[serde(default)]
	pub pulled_bytes: u64,
	#[serde(default)]
	pub deployed: u64,
	#[serde(default)]
	pub rolled_back: u64,
	#[serde(default)]
	pub failed: u64,
	#[serde(default)]
	pub no_newer_image: u64,
	#[serde(default)]
	pub deferred: u64,
}

/// What an ask is answered with, besides whether it is supported.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Deploy {
	/// Supported: nothing to do.
	None,
	/// The client is older than home's oldest supported epoch: pulling
	/// cannot help it; the client must update.
	ClientBehind,
	/// `needs` is past the ceiling: not believed, nothing pulled.
	Refused,
	/// A deploy is already wanted or running.
	InFlight,
	/// The last pull found nothing that serves `needs`; asked again after
	/// `retry_after_s`.
	Unavailable,
	/// A pull ran within the gap; asked again after `retry_after_s`.
	Cooldown,
	/// Asked for now: the marker is written.
	Wanted,
	/// A deploy was wanted, but the marker could not be written (no
	/// `${STORAGE}/deploy`, as under `make dev`).
	CannotRequest,
}

impl Deploy {
	/// The metric label for it.
	#[must_use]
	pub const fn label(self) -> &'static str {
		match self {
			Self::None => "none",
			Self::ClientBehind => "client_behind",
			Self::Refused => "refused",
			Self::InFlight => "in_flight",
			Self::Unavailable => "unavailable",
			Self::Cooldown => "cooldown",
			Self::Wanted => "wanted",
			Self::CannotRequest => "cannot_request",
		}
	}
}

/// [`decide`]'s answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Decision {
	pub supported: bool,
	pub deploy: Deploy,
	pub retry_after: Option<Duration>,
}

/// What to answer a client that needs `needs`.
///
/// At `now` (seconds since the epoch), given what the deploy recorded and
/// whether a marker is waiting. `Wanted` means: write the marker. Pure; the bounds are the module doc's.
#[must_use]
pub fn decide(needs: u32, now: i64, recorded: &Recorded, marker_waiting: bool) -> Decision {
	let answer = |supported, deploy, retry_after| Decision { supported, deploy, retry_after };
	if needs < OLDEST_SUPPORTED {
		return answer(false, Deploy::ClientBehind, None);
	}
	if needs <= CONTRACT_EPOCH {
		return answer(true, Deploy::None, None);
	}
	if needs > CONTRACT_EPOCH.saturating_add(CEILING) {
		return answer(false, Deploy::Refused, None);
	}
	if marker_waiting {
		return answer(false, Deploy::InFlight, None);
	}
	if let Some(pulled_at) = recorded.last_pull_at {
		let since = Duration::from_secs(u64::try_from(now.saturating_sub(pulled_at)).unwrap_or(0));
		if recorded.latest_seen_epoch.is_some_and(|latest| latest < needs) && since < UNMET_RETRY {
			return answer(false, Deploy::Unavailable, Some(UNMET_RETRY.saturating_sub(since)));
		}
		if since < PULL_GAP {
			return answer(false, Deploy::Cooldown, Some(PULL_GAP.saturating_sub(since)));
		}
	}
	answer(false, Deploy::Wanted, None)
}

/// What [`FOLDER`]'s `state.json` says, or nothing recorded when it is
/// missing or unreadable.
#[must_use]
pub fn read_recorded(folder: &Path) -> Recorded {
	std::fs::read(folder.join(STATE))
		.ok()
		.and_then(|bytes| serde_json::from_slice(&bytes).ok())
		.unwrap_or_default()
}

/// Whether a marker is waiting in `folder`, and since when (seconds since
/// the epoch, from its modification time).
#[must_use]
pub fn marker_since(folder: &Path) -> Option<i64> {
	let modified = std::fs::metadata(folder.join(WANTED)).ok()?.modified().ok()?;
	let since = modified.duration_since(std::time::UNIX_EPOCH).ok()?;
	i64::try_from(since.as_secs()).ok()
}

/// The marker's contents: what was needed, what ran, and when.
#[derive(Debug, Serialize)]
struct Marker {
	needs: u32,
	running: u32,
	at: i64,
}

/// Write the marker into `folder`, exclusively.
///
/// The folder is made if missing. The marker is written to a file of this
/// process's own, then linked to [`WANTED`], which fails when another ask
/// got there first. Returns whether this call wrote it.
///
/// # Errors
///
/// The folder cannot be made, or the file written.
pub fn request(folder: &Path, needs: u32, now: i64) -> std::io::Result<bool> {
	// Each ask its own temporary file, made exclusively: two asks in the
	// same second must never share one, or the second could truncate what
	// the first had already linked as the marker.
	static NEXT: AtomicU64 = AtomicU64::new(0);
	std::fs::create_dir_all(folder)?;
	let mut temp_name = String::from(".wanted-");
	temp_name.push_str(&std::process::id().to_string());
	temp_name.push('-');
	temp_name.push_str(&NEXT.fetch_add(1, Ordering::Relaxed).to_string());
	let temp = folder.join(temp_name);
	{
		let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&temp)?;
		serde_json::to_writer(
			&mut file,
			&Marker {
				needs,
				running: CONTRACT_EPOCH,
				at: now,
			},
		)
		.map_err(std::io::Error::other)?;
		file.write_all(b"\n")?;
		file.sync_all()?;
	}
	let linked = std::fs::hard_link(&temp, folder.join(WANTED));
	let _ = std::fs::remove_file(&temp);
	match linked {
		Ok(()) => Ok(true),
		Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
		Err(err) => Err(err),
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	const NOW: i64 = 1_800_000_000;

	fn recorded(pulled_ago: Option<u64>, latest: Option<u32>) -> Recorded {
		Recorded {
			last_pull_at: pulled_ago.map(|ago| NOW - i64::try_from(ago).unwrap()),
			latest_seen_epoch: latest,
			..Recorded::default()
		}
	}

	#[test]
	fn an_epoch_home_serves_is_supported_and_pulls_nothing() {
		for needs in OLDEST_SUPPORTED..=CONTRACT_EPOCH {
			let decision = decide(needs, NOW, &Recorded::default(), false);
			assert!(decision.supported);
			assert_eq!(decision.deploy, Deploy::None);
		}
	}

	#[test]
	fn a_client_older_than_home_is_told_so_and_pulls_nothing() {
		if OLDEST_SUPPORTED > 0 {
			assert_eq!(decide(OLDEST_SUPPORTED - 1, NOW, &Recorded::default(), false).deploy, Deploy::ClientBehind);
		}
	}

	#[test]
	fn a_newer_client_wants_a_deploy() {
		let decision = decide(CONTRACT_EPOCH + 1, NOW, &Recorded::default(), false);
		assert!(!decision.supported);
		assert_eq!(decision.deploy, Deploy::Wanted);
	}

	#[test]
	fn a_forged_need_past_the_ceiling_pulls_nothing() {
		assert_eq!(decide(CONTRACT_EPOCH + CEILING, NOW, &Recorded::default(), false).deploy, Deploy::Wanted);
		assert_eq!(decide(CONTRACT_EPOCH + CEILING + 1, NOW, &Recorded::default(), false).deploy, Deploy::Refused);
		assert_eq!(decide(u32::MAX, NOW, &Recorded::default(), false).deploy, Deploy::Refused);
	}

	#[test]
	fn one_deploy_at_a_time() {
		assert_eq!(decide(CONTRACT_EPOCH + 1, NOW, &Recorded::default(), true).deploy, Deploy::InFlight);
	}

	#[test]
	fn pulls_keep_their_gap() {
		let decision = decide(CONTRACT_EPOCH + 1, NOW, &recorded(Some(60), None), false);
		assert_eq!(decision.deploy, Deploy::Cooldown);
		assert_eq!(decision.retry_after, PULL_GAP.checked_sub(Duration::from_secs(60)));
		assert_eq!(decide(CONTRACT_EPOCH + 1, NOW, &recorded(Some(PULL_GAP.as_secs()), None), false).deploy, Deploy::Wanted);
	}

	/// Done means: a client nothing published can serve costs at most one
	/// pull per `UNMET_RETRY`, however often it asks.
	#[test]
	fn a_need_the_last_pull_could_not_meet_waits_six_hours() {
		let needs = CONTRACT_EPOCH + 2;
		let unmet = recorded(Some(3600), Some(CONTRACT_EPOCH + 1));
		let decision = decide(needs, NOW, &unmet, false);
		assert_eq!(decision.deploy, Deploy::Unavailable);
		assert_eq!(decision.retry_after, UNMET_RETRY.checked_sub(Duration::from_secs(3600)));
		// Met by what was seen: only the gap applies.
		assert_eq!(decide(CONTRACT_EPOCH + 1, NOW, &unmet, false).deploy, Deploy::Wanted);
		// After six hours it is tried again.
		assert_eq!(
			decide(needs, NOW, &recorded(Some(UNMET_RETRY.as_secs()), Some(CONTRACT_EPOCH + 1)), false).deploy,
			Deploy::Wanted
		);
	}

	#[test]
	fn the_marker_is_written_once_whoever_asks() {
		let dir = tempfile::tempdir().unwrap();
		let folder = dir.path().join("deploy");
		assert!(request(&folder, CONTRACT_EPOCH + 1, NOW).unwrap());
		assert!(!request(&folder, CONTRACT_EPOCH + 2, NOW).unwrap(), "a second ask finds it there");
		let written: serde_json::Value = serde_json::from_slice(&std::fs::read(folder.join(WANTED)).unwrap()).unwrap();
		assert_eq!(written["needs"], CONTRACT_EPOCH + 1);
		assert_eq!(written["running"], CONTRACT_EPOCH);
		assert!(marker_since(&folder).is_some());
		let names: Vec<_> = std::fs::read_dir(&folder).unwrap().map(|entry| entry.unwrap().file_name()).collect();
		assert_eq!(names, [std::ffi::OsString::from(WANTED)], "no temporary file is left");
	}

	/// Two asks in the same second each write their own temporary file:
	/// the marker one of them linked keeps its whole contents.
	#[test]
	fn asks_in_the_same_second_never_share_a_file() {
		let dir = tempfile::tempdir().unwrap();
		let folder = dir.path().join("deploy");
		let wrote = std::thread::scope(|scope| {
			let asks: [_; 8] = std::array::from_fn(|_| scope.spawn(|| request(&folder, CONTRACT_EPOCH + 1, NOW).unwrap()));
			asks.into_iter().map(|ask| usize::from(ask.join().unwrap())).sum::<usize>()
		});
		assert_eq!(wrote, 1, "exactly one ask writes the marker");
		let written: serde_json::Value = serde_json::from_slice(&std::fs::read(folder.join(WANTED)).unwrap()).unwrap();
		assert_eq!(written["running"], CONTRACT_EPOCH);
		assert_eq!(std::fs::read_dir(&folder).unwrap().count(), 1, "no temporary file is left");
	}

	#[test]
	fn the_deploys_state_reads_and_tolerates_absence() {
		let dir = tempfile::tempdir().unwrap();
		assert_eq!(read_recorded(dir.path()), Recorded::default());
		std::fs::write(
			dir.path().join(STATE),
			r#"{"last_pull_at": 5, "latest_seen_epoch": 3, "last_run": {"at": 6, "exit": 2, "outcome": "rolled_back"}, "counts": {"pulls": 4, "rolled_back": 1}, "later": true}"#,
		)
		.unwrap();
		let recorded = read_recorded(dir.path());
		assert_eq!(recorded.last_pull_at, Some(5));
		assert_eq!(recorded.latest_seen_epoch, Some(3));
		assert_eq!(recorded.last_run.unwrap().outcome, "rolled_back");
		assert_eq!(recorded.counts.pulls, 4);
		assert_eq!(recorded.counts.rolled_back, 1);
	}
}

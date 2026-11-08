//! Home's batch lane, run (L4b, paulgsc/server#708).
//!
//! `crates/lane` decides whether home's batch class may run now, must pause,
//! or must stop and let go of its memory (`docs/home-compute.md`, "The
//! lane", LA1–LA4). This worker runs that decision every [`TICK`] against
//! the kernel and home's own work, and says it three ways:
//!
//! - **To the render worker:** the decision, written whole to
//!   `renders/call-worker/lane` ([`DECISION`]) as `key=value` lines: `verb`,
//!   `why`, `stops`, `stuck` and `at` (Unix seconds). A file older than
//!   [`STALE_AFTER`], or missing, means stop (L4c reads it).
//! - **To Prometheus:** the decision, every signal it read and whether each
//!   could be read, and when it last wrote a decision
//!   (`metrics::lane`), for the CALL dashboard's LANE row and the `Lane*`
//!   alerts in `infra/prometheus/rules/home.yml`.
//! - **About home at rest:** whether nothing was being worked on this tick
//!   (`lane_idle`), so WHO DUNNIT's idle footprint can say what home's
//!   services cost while they wait.
//!
//! **What it reads, each tick:**
//!
//! - `/proc/meminfo`'s `MemAvailable` and `/proc/pressure/memory`'s
//!   `some avg10`. Under Docker both are the host's: `/proc/meminfo` is not
//!   namespaced, and `/proc/pressure` is the system-wide PSI, not this
//!   container's cgroup (Docker masks neither).
//! - Interactive work: [`Interactive`], held by the transcriber while it
//!   hears a note and by Rust Rounds while the runner records a program.
//!   Work that started and ended between two ticks still counts at the
//!   next one, so no hearing is too short to be seen.
//! - The batch job and its peak, from the render worker's status folder:
//!   `job`, a decimal id the worker gives each job (never reused, kept
//!   through pauses and evacuations), and `peak`, the measured peak in bytes
//!   of its last finished run. Both are L4c's to write. Missing, there is no
//!   job and the peak is [`DECLARED_PEAK_BYTES`]; present and not a number,
//!   that signal is unreadable, which is never room (LA2).
//!
//! **Cost:** two `/proc` reads, two small file reads, one directory listing
//! of the status folder (a file or two a week) and one small write and
//! rename, every five seconds. Nothing it reads or writes names a week or a
//! person.

use crate::AppState;
use ::lane::{Decision, Policy, Signals, State, Verb, Why};
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{info, warn};

/// How often the lane decides. A pause lands within one tick of
/// interactive work starting, before the render worker's own read (L4c).
pub const TICK: Duration = Duration::from_secs(5);

/// A decision older than this is stale: the render worker treats it as
/// stop (L4c), and past twice this `LaneStale` fires.
pub const STALE_AFTER: Duration = Duration::from_secs(30);

const _: () = assert!(STALE_AFTER.as_secs() >= 6 * TICK.as_secs());

const GIB: u64 = 1024 * 1024 * 1024;
const MIB: u64 = 1024 * 1024;

/// Home's thresholds (`docs/home-compute.md`, "Home's policy", has why
/// each is what it is).
pub const POLICY: Policy = Policy {
	// Room for interactive work to arrive while a render holds its memory:
	// the rust-runner's whole 1 GiB cap, and as much again for the
	// transcriber's working memory, file_host's requests and the error in
	// `MemAvailable`'s estimate.
	floor_bytes: 2 * GIB,
	// `some avg10` past 10%: some task stalled on memory for a tenth of the
	// last ten seconds.
	pressure_limit_pct: 10.0,
	// Three of `avg10`'s windows, so the pressure that comes back is not a
	// lull inside one.
	hold_down_ms: 30_000,
	stuck_after: NonZeroU32::MIN.saturating_add(2),
};

/// The most a render may take before one is measured.
///
/// The render worker's 4 GiB cap, the speaker's room above what it holds
/// between renders (its 3 GiB cap less the 1.25 GB it keeps, #691) and the
/// stage's 256 MiB (`infra/compose/call-render.yml`). What the speaker keeps
/// between renders is already out of `MemAvailable`, so it is not counted
/// twice.
pub const DECLARED_PEAK_BYTES: u64 = 4 * GIB + (3 * GIB - 1250 * 1000 * 1000) + 256 * MIB;

/// The decision's file in the render worker's status folder.
pub const DECISION: &str = "lane";

/// The job in progress, as the render worker writes it (L4c).
pub const JOB: &str = "job";

/// The measured peak of the job's last finished run, in bytes (L4c).
pub const PEAK: &str = "peak";

/// The kernel's memory signals, host-wide.
pub const MEMINFO: &str = "/proc/meminfo";
pub const PRESSURE: &str = "/proc/pressure/memory";

/// Interactive work in progress: held while a note is heard or a program
/// recorded, read each tick by the lane (LA1). Cloned into each worker; all
/// clones count together.
#[derive(Debug, Clone, Default)]
pub struct Interactive(Arc<Counts>);

#[derive(Debug, Default)]
struct Counts {
	/// Holds not yet dropped.
	held: AtomicUsize,
	/// Holds ever taken, so the lane sees work that came and went between
	/// two ticks.
	started: AtomicU64,
}

/// One piece of interactive work, until it is dropped.
#[derive(Debug)]
#[must_use = "interactive work counts only while its hold is kept"]
pub struct Hold(Arc<Counts>);

impl Drop for Hold {
	fn drop(&mut self) {
		self.0.held.fetch_sub(1, Ordering::SeqCst);
	}
}

impl Interactive {
	/// Count interactive work from now until the hold is dropped.
	pub fn hold(&self) -> Hold {
		self.0.started.fetch_add(1, Ordering::SeqCst);
		self.0.held.fetch_add(1, Ordering::SeqCst);
		Hold(Arc::clone(&self.0))
	}

	/// Whether any interactive work is held right now.
	#[cfg(test)]
	pub(crate) fn is_held(&self) -> bool {
		self.0.held.load(Ordering::SeqCst) > 0
	}

	fn reading(&self) -> (usize, u64) {
		(self.0.held.load(Ordering::SeqCst), self.0.started.load(Ordering::SeqCst))
	}
}

/// A worker's engine or runner, holding [`Interactive`] around the work it
/// does for someone: `transcribe` implements `Hear` for it, `rust_rounds`
/// `Runner`.
pub struct Held<T> {
	pub inner: Arc<T>,
	pub interactive: Interactive,
}

/// What the lane saw of interactive work at its last tick.
#[derive(Debug, Default, Clone, Copy)]
struct Since {
	started: u64,
}

impl Since {
	/// Whether interactive work is held now, or was taken since the last
	/// tick and has already ended.
	const fn waiting(&mut self, (held, started): (usize, u64)) -> bool {
		let new = started != self.started;
		self.started = started;
		held > 0 || new
	}
}

/// A number the render worker writes in its status folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Number {
	Absent,
	Value(u64),
	Unreadable,
}

impl Number {
	async fn read(path: &Path) -> Self {
		match tokio::fs::read_to_string(path).await {
			Ok(text) => text.trim().parse().map_or(Self::Unreadable, Self::Value),
			Err(err) if err.kind() == std::io::ErrorKind::NotFound => Self::Absent,
			Err(_) => Self::Unreadable,
		}
	}
}

/// What the render worker's status folder says of batch work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Batch {
	job: Number,
	peak: Number,
	/// A week's attempt is in progress: the worker keeps its log as
	/// `.<week>.log` until the attempt ends (`infra/render/render-calls.sh`).
	/// A folder that cannot be listed counts as one, so home is never
	/// called idle on a guess.
	attempting: bool,
}

impl Batch {
	async fn read(status: &Path) -> Self {
		Self {
			job: Number::read(&status.join(JOB)).await,
			peak: Number::read(&status.join(PEAK)).await,
			attempting: attempting(status).await,
		}
	}

	/// Batch work is in progress: a job named, or a week's attempt running.
	const fn active(&self) -> bool {
		matches!(self.job, Number::Value(_)) || self.attempting
	}
}

async fn attempting(status: &Path) -> bool {
	let Ok(mut entries) = tokio::fs::read_dir(status).await else {
		return true;
	};
	loop {
		match entries.next_entry().await {
			Ok(Some(entry)) => {
				if is_attempt_log(&entry.file_name().to_string_lossy()) {
					return true;
				}
			}
			Ok(None) => return false,
			Err(_) => return true,
		}
	}
}

/// `.YYYY-MM-DD.log`, a week's attempt in progress.
fn is_attempt_log(name: &str) -> bool {
	name.strip_prefix('.').and_then(|rest| rest.strip_suffix(".log")).is_some_and(|week| {
		let bytes = week.as_bytes();
		bytes.len() == 10 && bytes.iter().enumerate().all(|(i, b)| if i == 4 || i == 7 { *b == b'-' } else { b.is_ascii_digit() })
	})
}

/// Whether each signal could be read this tick.
// One flag per signal, each read on its own and each its own series.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Readable {
	pub meminfo: bool,
	pub pressure: bool,
	pub job: bool,
	pub peak: bool,
}

/// The tick's signals, from what was read.
fn signals(now_ms: u64, meminfo: Option<&str>, pressure: Option<&str>, interactive_waiting: bool, batch: &Batch) -> (Signals, Readable) {
	let mem_available_bytes = meminfo.and_then(::lane::mem_available_bytes);
	let memory_pressure_pct = pressure.and_then(::lane::pressure_some_avg10);
	let batch_job = match batch.job {
		Number::Value(id) => Some(id),
		Number::Absent | Number::Unreadable => None,
	};
	// A peak of nothing is no measurement.
	let batch_peak_bytes = match batch.peak {
		Number::Absent => Some(DECLARED_PEAK_BYTES),
		Number::Value(peak) if peak > 0 => Some(peak),
		Number::Value(_) | Number::Unreadable => None,
	};
	let readable = Readable {
		meminfo: mem_available_bytes.is_some(),
		pressure: memory_pressure_pct.is_some(),
		job: batch.job != Number::Unreadable,
		peak: batch_peak_bytes.is_some(),
	};
	(
		Signals {
			now_ms,
			mem_available_bytes,
			memory_pressure_pct,
			interactive_waiting,
			batch_job,
			batch_peak_bytes,
		},
		readable,
	)
}

/// The decision as the render worker reads it.
fn decision_text(decision: Decision, at: i64) -> String {
	let mut text = String::with_capacity(64);
	for (key, value) in [
		("verb", decision.verb.name().to_owned()),
		("why", decision.why.name().to_owned()),
		("stops", decision.stops.to_string()),
		("stuck", decision.stuck.to_string()),
		("at", at.to_string()),
	] {
		text.push_str(key);
		text.push('=');
		text.push_str(&value);
		text.push('\n');
	}
	text
}

/// Written whole, as the worker writes its own status files: a reader
/// never sees half a decision.
async fn write_whole(status: &Path, name: &str, text: &str) -> std::io::Result<()> {
	let mut hidden = String::from(".");
	hidden.push_str(name);
	let partial = status.join(hidden);
	tokio::fs::write(&partial, text).await?;
	tokio::fs::rename(&partial, status.join(name)).await
}

/// Where the lane reads and writes; the real ones in the server, a
/// temporary folder in tests.
#[derive(Debug, Clone)]
struct Paths {
	meminfo: PathBuf,
	pressure: PathBuf,
	status: PathBuf,
}

impl Paths {
	fn server() -> Self {
		Self {
			meminfo: PathBuf::from(MEMINFO),
			pressure: PathBuf::from(PRESSURE),
			status: PathBuf::from(super::render::STATUS_FOLDER),
		}
	}
}

/// What one tick did.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Ticked {
	decision: Decision,
	signals: Signals,
	readable: Readable,
	/// Nothing was being worked on: no interactive work, and no batch work.
	idle: bool,
	batch_active: bool,
	/// When the decision was written, or `None` when it could not be.
	written_at: Option<i64>,
}

/// The lane between ticks.
struct Lane {
	state: State,
	since: Since,
	started: Instant,
}

impl Lane {
	fn new() -> Self {
		Self {
			state: State::new(),
			since: Since::default(),
			started: Instant::now(),
		}
	}

	async fn tick(&mut self, paths: &Paths, interactive: &Interactive, now: i64) -> Ticked {
		let meminfo = tokio::fs::read_to_string(&paths.meminfo).await.ok();
		let pressure = tokio::fs::read_to_string(&paths.pressure).await.ok();
		let waiting = self.since.waiting(interactive.reading());
		let batch = Batch::read(&paths.status).await;
		let now_ms = u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX);
		let (signals, readable) = signals(now_ms, meminfo.as_deref(), pressure.as_deref(), waiting, &batch);
		let (state, decision) = ::lane::step(self.state, &signals, &POLICY);
		self.state = state;
		let written_at = write_whole(&paths.status, DECISION, &decision_text(decision, now)).await.ok().map(|()| now);
		Ticked {
			decision,
			signals,
			readable,
			idle: !waiting && !batch.active(),
			batch_active: batch.active(),
			written_at,
		}
	}
}

/// Spawn the lane, cancelled through the shared token. Its first decision
/// is at once; until the hold-down has passed it is stop, so nothing runs
/// on a reading taken before the server was up.
pub fn spawn(state: &AppState) {
	let cancel = state.core.cancel_token.clone();
	let interactive = state.interactive.clone();
	tokio::spawn(async move {
		info!(tick_secs = TICK.as_secs(), "batch lane started");
		crate::metrics::lane::record_started(&POLICY, DECLARED_PEAK_BYTES);
		let paths = Paths::server();
		let mut lane = Lane::new();
		let mut said: Option<(Verb, Why, bool, bool)> = None;
		let mut ticker = tokio::time::interval(TICK);
		ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
		loop {
			tokio::select! {
				() = cancel.cancelled() => {
					info!("batch lane cancelled");
					return;
				}
				_ = ticker.tick() => {
					let done = lane.tick(&paths, &interactive, chrono::Utc::now().timestamp()).await;
					crate::metrics::lane::record(&done.decision, &done.signals, &done.readable, done.idle, done.batch_active, done.written_at);
					let now = (done.decision.verb, done.decision.why, done.decision.stuck, done.written_at.is_some());
					if said != Some(now) {
						log_change(&done);
						said = Some(now);
					}
				}
			}
		}
	});
}

/// One line when the decision, whether it is stuck, or whether it could be
/// written changes.
fn log_change(ticked: &Ticked) {
	let (verb, why) = (ticked.decision.verb.name(), ticked.decision.why.name());
	if ticked.written_at.is_none() {
		warn!(
			verb,
			why,
			error = "storage",
			"batch lane cannot write its decision; the render worker will treat it as stale"
		);
	} else if ticked.decision.stuck {
		warn!(verb, why, stops = ticked.decision.stops, "batch lane: the render in progress is stuck");
	} else {
		info!(verb, why, stops = ticked.decision.stops, "batch lane");
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	const MEMINFO_ROOM: &str = "MemTotal:       16303456 kB\nMemAvailable:   12000000 kB\n";
	const PRESSURE_CALM: &str = "some avg10=0.00 avg60=0.00 avg300=0.00 total=0\nfull avg10=0.00 avg60=0.00 avg300=0.00 total=0\n";

	fn paths(dir: &Path) -> Paths {
		let paths = Paths {
			meminfo: dir.join("meminfo"),
			pressure: dir.join("pressure"),
			status: dir.join("status"),
		};
		std::fs::create_dir_all(&paths.status).unwrap();
		std::fs::write(&paths.meminfo, MEMINFO_ROOM).unwrap();
		std::fs::write(&paths.pressure, PRESSURE_CALM).unwrap();
		paths
	}

	fn decision_file(paths: &Paths) -> String {
		std::fs::read_to_string(paths.status.join(DECISION)).unwrap()
	}

	#[test]
	fn home_policy_and_peak_are_what_the_doc_says() {
		assert_eq!(POLICY.floor_bytes, 2 * GIB);
		assert_eq!(POLICY.stuck_after.get(), 3);
		assert_eq!(DECLARED_PEAK_BYTES, 6_534_628_224, "about 6.09 GiB");
	}

	#[test]
	fn a_hold_counts_until_dropped_and_work_between_ticks_is_still_seen() {
		let interactive = Interactive::default();
		let mut since = Since::default();
		assert!(!since.waiting(interactive.reading()), "nothing held, nothing taken");
		let hold = interactive.hold();
		let clone = interactive.clone();
		assert!(since.waiting(clone.reading()), "a clone counts with the rest");
		assert!(since.waiting(interactive.reading()), "still held");
		drop(hold);
		assert!(!since.waiting(interactive.reading()), "dropped, and seen while held");
		drop(interactive.hold());
		assert!(since.waiting(interactive.reading()), "taken and dropped between two ticks");
		assert!(!since.waiting(interactive.reading()), "and only at the next one");
	}

	#[test]
	fn only_a_weeks_attempt_log_is_an_attempt() {
		assert!(is_attempt_log(".2026-10-30.log"));
		for name in [
			"2026-10-30.failed.log",
			".2026-10-30.cache.log",
			".work.cache.log",
			".lane",
			"last-pass",
			".2026-1-30.log",
			"job",
		] {
			assert!(!is_attempt_log(name), "{name}");
		}
	}

	#[test]
	fn the_peak_is_declared_until_measured_and_a_garbled_file_is_unreadable() {
		let batch = |job, peak| Batch { job, peak, attempting: false };
		let (signals, readable) = signals(0, Some(MEMINFO_ROOM), Some(PRESSURE_CALM), false, &batch(Number::Absent, Number::Absent));
		assert_eq!((signals.batch_job, signals.batch_peak_bytes), (None, Some(DECLARED_PEAK_BYTES)));
		assert_eq!(
			readable,
			Readable {
				meminfo: true,
				pressure: true,
				job: true,
				peak: true
			}
		);
		let (signals, _) = signals_of(&batch(Number::Value(7), Number::Value(3 * GIB)));
		assert_eq!((signals.batch_job, signals.batch_peak_bytes), (Some(7), Some(3 * GIB)));
		let (signals, readable) = signals_of(&batch(Number::Unreadable, Number::Unreadable));
		assert_eq!((signals.batch_job, signals.batch_peak_bytes), (None, None));
		assert!(!readable.job && !readable.peak);
		let (signals, readable) = signals_of(&batch(Number::Absent, Number::Value(0)));
		assert_eq!(signals.batch_peak_bytes, None, "a peak of nothing is no measurement");
		assert!(!readable.peak);
	}

	fn signals_of(batch: &Batch) -> (Signals, Readable) {
		signals(0, Some(MEMINFO_ROOM), Some(PRESSURE_CALM), false, batch)
	}

	#[test]
	fn no_psi_or_no_meminfo_is_unreadable() {
		let batch = Batch {
			job: Number::Absent,
			peak: Number::Absent,
			attempting: false,
		};
		let (signals, readable) = signals(0, None, None, false, &batch);
		assert_eq!((signals.mem_available_bytes, signals.memory_pressure_pct), (None, None));
		assert!(!readable.meminfo && !readable.pressure);
	}

	#[tokio::test]
	async fn each_tick_writes_the_decision_whole_and_stop_comes_first() {
		let dir = tempfile::tempdir().unwrap();
		let paths = paths(dir.path());
		let mut lane = Lane::new();
		let ticked = lane.tick(&paths, &Interactive::default(), 1_731_000_000).await;
		assert_eq!(ticked.decision.verb, Verb::Stop, "room is held for the hold-down first");
		assert_eq!(ticked.decision.why, Why::Settling);
		assert_eq!(ticked.written_at, Some(1_731_000_000));
		assert!(ticked.idle);
		assert_eq!(decision_file(&paths), "verb=stop\nwhy=settling\nstops=0\nstuck=false\nat=1731000000\n");
		assert!(!paths.status.join(".lane").exists(), "nothing half written is left behind");
		assert!(!Batch::read(&paths.status).await.attempting, "its own file is not an attempt");
	}

	#[tokio::test]
	async fn interactive_work_and_a_render_in_progress_are_not_idle() {
		let dir = tempfile::tempdir().unwrap();
		let paths = paths(dir.path());
		let interactive = Interactive::default();
		let mut lane = Lane::new();
		let hold = interactive.hold();
		let ticked = lane.tick(&paths, &interactive, 1).await;
		assert_eq!(ticked.decision.why, Why::Busy);
		assert!(!ticked.idle);
		drop(hold);
		std::fs::write(paths.status.join(".2026-10-30.log"), b"rendering").unwrap();
		let ticked = lane.tick(&paths, &interactive, 2).await;
		assert!(!ticked.signals.interactive_waiting, "seen at the last tick, and gone");
		assert!(ticked.batch_active && !ticked.idle, "a week's attempt is batch work");
		std::fs::remove_file(paths.status.join(".2026-10-30.log")).unwrap();
		std::fs::write(paths.status.join(JOB), b"41\n").unwrap();
		let ticked = lane.tick(&paths, &interactive, 3).await;
		assert_eq!(ticked.signals.batch_job, Some(41));
		assert!(ticked.batch_active && !ticked.idle);
	}

	#[tokio::test]
	async fn missing_signals_are_unknown_and_an_unwritable_folder_is_said() {
		let dir = tempfile::tempdir().unwrap();
		let paths = paths(dir.path());
		std::fs::remove_file(&paths.pressure).unwrap();
		let mut lane = Lane::new();
		let ticked = lane.tick(&paths, &Interactive::default(), 1).await;
		assert_eq!(ticked.decision.why, Why::Unknown, "a kernel without PSI never admits batch work");
		assert!(!ticked.readable.pressure && ticked.readable.meminfo);
		std::fs::remove_dir_all(&paths.status).unwrap();
		let ticked = lane.tick(&paths, &Interactive::default(), 2).await;
		assert_eq!(ticked.written_at, None);
		assert!(!ticked.idle, "a status folder that cannot be listed is never idle");
	}

	#[tokio::test]
	async fn room_held_for_the_hold_down_runs() {
		let dir = tempfile::tempdir().unwrap();
		let paths = paths(dir.path());
		let mut lane = Lane::new();
		lane.tick(&paths, &Interactive::default(), 1).await;
		lane.started = lane.started.checked_sub(Duration::from_millis(POLICY.hold_down_ms)).unwrap();
		let ticked = lane.tick(&paths, &Interactive::default(), 2).await;
		assert_eq!((ticked.decision.verb, ticked.decision.why), (Verb::Run, Why::Clear));
		assert!(decision_file(&paths).starts_with("verb=run\nwhy=clear\n"));
	}
}

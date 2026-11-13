//! The contention guard (L1, paulgsc/server#706): a busy box makes home
//! slower, never changes what home records.
//!
//! Rust Rounds keeps a program's verdict and the transcriber a note's
//! attempt. Both are measured against a wall clock: a run's 2000 ms ceiling
//! and rustc's 60 s, the runner's watchdog, a hearing's deadline. On a box
//! another service is holding, a program or a note can run past one of them
//! for no reason of its own. What separates "it was slow" from "the box was
//! busy" is evidence of contention during the measurement, so each
//! measurement is bracketed by a [`Window`]:
//!
//! - **CPU**: `/proc/pressure/cpu`'s `some` total, read before and after.
//!   The share of the window in which some task waited for a CPU is the
//!   evidence. Past [`CONTENDED_SHARE`] the window was contended.
//! - **Memory**: `/proc/vmstat`'s `oom_kill`, read before and after. A kill
//!   anywhere on the box during the window is evidence that a runner lost
//!   then may have been lost to another service's memory. The runner bounds
//!   its program's memory itself (an allocation past the ceiling aborts the
//!   program, a verdict), so a kill the program caused shows as a verdict,
//!   not a loss.
//!
//! Inside `file_host`'s container both files are the host's, as the lane's
//! signals are (`call_weeks::lane`): `/proc/pressure` is the system-wide
//! PSI and `/proc/vmstat` is not namespaced.
//!
//! What contention excuses ([`Excuse`]) is decided by each worker: a
//! verdict of time, a lost runner, a hearing's deadline. An excused outcome
//! is **nobody's**: nothing is kept, the pass stops, and the next asks
//! again. A [`Streak`] bounds that: one program or note is excused at most
//! [`NOBODYS`] times in a row, then judged as it would be on a calm box, so
//! a box that never calms down cannot hold the queue forever. That is the
//! one place a contended outcome is kept, and it is said
//! (`contention_judged_anyway_total`, a WARN).
//!
//! An unreadable signal is no evidence: the outcome is judged as before the
//! guard, and `contention_unreadable_total` says so. The lane already says
//! a kernel without PSI (`LaneSignalsUnreadable`).

use std::path::PathBuf;
use std::time::Instant;

/// The `some` CPU stall share at or past which a window was contended.
///
/// For a tenth of the window or more, a task on the box waited for a CPU.
/// A program alone on an idle box never waits for its own CPU, so its
/// windows read near 0. Set low on purpose: a false "contended" costs a
/// retry a pass later, a false "calm" blames a program or a note for good.
pub const CONTENDED_SHARE: f64 = 0.10;

/// The most times in a row one program or one note is excused before it is
/// judged anyway.
pub const NOBODYS: u32 = 3;

/// `/proc/pressure/cpu`.
pub const PSI_CPU: &str = "/proc/pressure/cpu";

/// `/proc/vmstat`.
pub const VMSTAT: &str = "/proc/vmstat";

/// One reading of the box's counters; `None` for one that could not be
/// read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Reading {
	/// `/proc/pressure/cpu`'s `some` `total=`, in microseconds.
	pub cpu_some_us: Option<u64>,
	/// `/proc/vmstat`'s `oom_kill`.
	pub oom_kills: Option<u64>,
}

/// Where a [`Window`] reads the box's counters. Read on the blocking thread
/// or the task that asks; both files are in memory and answer at once.
pub trait Gauge: Send + Sync + 'static {
	fn read(&self) -> Reading;
}

/// The host's counters, at their paths.
#[derive(Debug, Clone)]
pub struct Host {
	pub psi_cpu: PathBuf,
	pub vmstat: PathBuf,
}

impl Default for Host {
	fn default() -> Self {
		Self {
			psi_cpu: PathBuf::from(PSI_CPU),
			vmstat: PathBuf::from(VMSTAT),
		}
	}
}

impl Gauge for Host {
	fn read(&self) -> Reading {
		Reading {
			cpu_some_us: std::fs::read_to_string(&self.psi_cpu).ok().as_deref().and_then(cpu_some_total),
			oom_kills: std::fs::read_to_string(&self.vmstat).ok().as_deref().and_then(oom_kills),
		}
	}
}

/// `some`'s `total=` of a PSI file (`some avg10=0.00 avg60=0.00
/// avg300=0.00 total=12345`).
#[must_use]
pub fn cpu_some_total(text: &str) -> Option<u64> {
	text
		.lines()
		.find_map(|line| line.strip_prefix("some "))
		.and_then(|rest| rest.split_whitespace().find_map(|field| field.strip_prefix("total=")))
		.and_then(|total| total.parse().ok())
}

/// `oom_kill`'s count in `/proc/vmstat`.
#[must_use]
pub fn oom_kills(text: &str) -> Option<u64> {
	text.lines().find_map(|line| line.strip_prefix("oom_kill ")).and_then(|count| count.trim().parse().ok())
}

/// A measurement's bracket: opened before the work, closed after it.
#[derive(Debug, Clone, Copy)]
pub struct Window {
	at: Instant,
	before: Reading,
}

impl Window {
	#[must_use]
	pub fn open(gauge: &dyn Gauge) -> Self {
		Self {
			at: Instant::now(),
			before: gauge.read(),
		}
	}

	/// What the box did while the window was open.
	#[must_use]
	pub fn close(self, gauge: &dyn Gauge) -> Seen {
		let after = gauge.read();
		let micros = self.at.elapsed().as_micros();
		let cpu_share = match (self.before.cpu_some_us, after.cpu_some_us) {
			(Some(before), Some(after)) if after >= before => Some(share(after - before, micros.max(1))),
			_ => None,
		};
		let oom_killed = match (self.before.oom_kills, after.oom_kills) {
			(Some(before), Some(after)) => Some(after > before),
			_ => None,
		};
		Seen { cpu_share, oom_killed }
	}
}

#[allow(clippy::cast_precision_loss)] // a share: microseconds well inside f64's exact range
fn share(stalled_us: u64, window_us: u128) -> f64 {
	(stalled_us as f64 / window_us as f64).min(1.0)
}

/// What the box did during one [`Window`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Seen {
	/// The share of the window some task waited for a CPU, 0 to 1; `None`
	/// when either reading failed, or the counter went backwards.
	pub cpu_share: Option<f64>,
	/// Whether the kernel killed anything for memory during the window;
	/// `None` when either reading failed.
	pub oom_killed: Option<bool>,
}

impl Seen {
	/// The CPU was contended: some task waited for one for at least
	/// [`CONTENDED_SHARE`] of the window.
	#[must_use]
	pub fn cpu_contended(&self) -> bool {
		self.cpu_share.is_some_and(|share| share >= CONTENDED_SHARE)
	}

	/// The CPU was contended, or the kernel killed something for memory.
	#[must_use]
	pub fn anything(&self) -> bool {
		self.cpu_contended() || self.oom_killed == Some(true)
	}

	/// Either signal could not be read.
	#[must_use]
	pub const fn unreadable(&self) -> bool {
		self.cpu_share.is_none() || self.oom_killed.is_none()
	}
}

/// Whether an outcome contention could excuse is excused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Excuse {
	/// Nobody's: nothing is kept, and it is asked again.
	Nobodys,
	/// Excused [`NOBODYS`] times in a row already: judged as it would be on
	/// a calm box.
	JudgedAnyway,
}

/// One program's or note's run of excuses, the last one excused only: O(1)
/// state. An excused outcome stops its pass, so the next pass starts with
/// the same one, and its excuses run in a row.
#[derive(Debug, Clone)]
pub struct Streak<K> {
	last: Option<(K, u32)>,
}

impl<K> Default for Streak<K> {
	fn default() -> Self {
		Self { last: None }
	}
}

impl<K: PartialEq> Streak<K> {
	/// Excuse `key` once more, or not past [`NOBODYS`] in a row.
	pub fn excuse(&mut self, key: K) -> Excuse {
		let count = match &self.last {
			Some((last, count)) if *last == key => count + 1,
			_ => 1,
		};
		if count > NOBODYS {
			self.last = None;
			return Excuse::JudgedAnyway;
		}
		self.last = Some((key, count));
		Excuse::Nobodys
	}
}

/// A worker's guard: where it reads the box, and its streak.
pub struct Guard<K> {
	pub gauge: std::sync::Arc<dyn Gauge>,
	pub streak: Streak<K>,
}

impl<K> Guard<K> {
	#[must_use]
	pub fn new(gauge: std::sync::Arc<dyn Gauge>) -> Self {
		Self { gauge, streak: Streak::default() }
	}

	/// The host's counters.
	#[must_use]
	pub fn host() -> Self {
		Self::new(std::sync::Arc::new(Host::default()))
	}
}

/// What a test hands a [`Guard`]: a box that is calm, CPU-contended, or
/// killing for memory, at every read.
#[cfg(test)]
pub(crate) mod fake {
	use super::{Gauge, Guard, Reading};
	use std::sync::atomic::{AtomicU64, Ordering};
	use std::sync::Arc;

	#[derive(Debug, Clone, Copy, PartialEq, Eq)]
	pub(crate) enum State {
		Calm,
		Busy,
		Killing,
		Unreadable,
	}

	/// Counters that move as `state` says each time they are read.
	pub(crate) struct Fake {
		pub(crate) state: std::sync::Mutex<State>,
		reads: AtomicU64,
	}

	impl Fake {
		pub(crate) fn new(state: State) -> Arc<Self> {
			Arc::new(Self {
				state: std::sync::Mutex::new(state),
				reads: AtomicU64::new(0),
			})
		}

		pub(crate) fn set(&self, state: State) {
			*self.state.lock().unwrap() = state;
		}
	}

	impl Gauge for Fake {
		fn read(&self) -> Reading {
			let n = self.reads.fetch_add(1, Ordering::SeqCst);
			let state = *self.state.lock().unwrap();
			match state {
				State::Calm => Reading {
					cpu_some_us: Some(1_000),
					oom_kills: Some(7),
				},
				// An hour of stall between any two reads: more than any window.
				State::Busy => Reading {
					cpu_some_us: Some(n * 3_600_000_000),
					oom_kills: Some(7),
				},
				State::Killing => Reading {
					cpu_some_us: Some(1_000),
					oom_kills: Some(n),
				},
				State::Unreadable => Reading::default(),
			}
		}
	}

	pub(crate) fn guard<K>(state: State) -> (Guard<K>, Arc<Fake>) {
		let fake = Fake::new(state);
		(Guard::new(Arc::clone(&fake) as Arc<dyn Gauge>), fake)
	}
}

#[cfg(test)]
mod tests {
	use super::fake::{Fake, State};
	use super::{cpu_some_total, oom_kills, Excuse, Gauge, Reading, Seen, Streak, Window, NOBODYS};

	#[test]
	fn the_counters_read_from_their_files() {
		let psi = "some avg10=1.50 avg60=0.40 avg300=0.10 total=123456\nfull avg10=0.00 avg60=0.00 avg300=0.00 total=0\n";
		assert_eq!(cpu_some_total(psi), Some(123_456));
		assert_eq!(cpu_some_total("full avg10=0.00 total=5\n"), None);
		assert_eq!(cpu_some_total("some avg10=0.00 total=x\n"), None);
		assert_eq!(oom_kills("pgfault 9\noom_kill 3\nnr_free_pages 1\n"), Some(3));
		assert_eq!(oom_kills("oom_kill_x 3\n"), None);
	}

	struct Fixed(Reading, Reading, std::sync::atomic::AtomicBool);
	impl Gauge for Fixed {
		fn read(&self) -> Reading {
			if self.2.swap(true, std::sync::atomic::Ordering::SeqCst) {
				self.1
			} else {
				self.0
			}
		}
	}

	fn seen(before: Reading, after: Reading) -> Seen {
		let gauge = Fixed(before, after, std::sync::atomic::AtomicBool::new(false));
		let window = Window::open(&gauge);
		std::thread::sleep(std::time::Duration::from_millis(20));
		window.close(&gauge)
	}

	#[test]
	fn a_window_is_contended_only_past_the_share_and_killed_only_when_the_count_moved() {
		let at = |cpu, oom| Reading {
			cpu_some_us: Some(cpu),
			oom_kills: Some(oom),
		};
		let calm = seen(at(1_000, 4), at(1_000, 4));
		assert_eq!(calm.cpu_share, Some(0.0));
		assert!(!calm.cpu_contended() && !calm.anything() && !calm.unreadable());
		// 20 ms of window, 1 s of stall: capped at the whole window.
		let busy = seen(at(0, 4), at(1_000_000, 4));
		assert!(busy.cpu_contended() && busy.anything());
		assert_eq!(busy.cpu_share, Some(1.0));
		let killed = seen(at(0, 4), at(0, 5));
		assert!(!killed.cpu_contended() && killed.anything());
		// A counter that went backwards (a reset) is no evidence either way.
		let backwards = seen(at(5_000_000, 4), at(0, 4));
		assert!(backwards.cpu_share.is_none() && backwards.unreadable() && !backwards.anything());
		let lost = seen(Reading::default(), at(0, 4));
		assert!(lost.unreadable() && !lost.anything());
	}

	#[test]
	fn the_fakes_read_as_their_names_say() {
		for (state, contended, killed) in [(State::Calm, false, false), (State::Busy, true, false), (State::Killing, false, true)] {
			let fake = Fake::new(state);
			let seen = Window::open(fake.as_ref()).close(fake.as_ref());
			assert_eq!((seen.cpu_contended(), seen.oom_killed == Some(true)), (contended, killed), "{state:?}");
		}
		let fake = Fake::new(State::Unreadable);
		assert!(Window::open(fake.as_ref()).close(fake.as_ref()).unreadable());
	}

	#[test]
	fn one_key_is_excused_at_most_nobodys_times_in_a_row() {
		let mut streak = Streak::default();
		for _ in 0..NOBODYS {
			assert_eq!(streak.excuse(1), Excuse::Nobodys);
		}
		assert_eq!(streak.excuse(1), Excuse::JudgedAnyway);
		// Judged, so its count starts again.
		assert_eq!(streak.excuse(1), Excuse::Nobodys);
		// Another key starts its own count.
		assert_eq!(streak.excuse(2), Excuse::Nobodys);
		assert_eq!(streak.excuse(1), Excuse::Nobodys);
	}
}

//! Home's batch lane (paulgsc/server#705, L4 #708).
//!
//! Renders are home's lowest class (`docs/home-compute.md`, "Classes"). No
//! clock starts them: they run when the box has room, and give way the
//! moment anything else needs it. [`step`] is that decision, made each tick
//! from the kernel's own signals. It does no I/O, so `file_host` runs it
//! against `/proc`, and the capacity simulation (#710) can run the same code
//! against a model of another machine.
//!
//! What it promises (declared in `docs/home-compute.md`, "The lane"):
//!
//! - **LA1.** Batch work never runs while interactive work waits: it is told
//!   to pause ([`Verb::Pause`], [`Why::Busy`]), or not to start.
//! - **LA2.** Batch work never starts or resumes after a stop without room
//!   for its measured peak above the floor, held for the hold-down. A signal
//!   that could not be read is never room ([`Why::Unknown`]).
//! - **LA3.** Batch work running or paused is told to stop and let go of its
//!   memory ([`Verb::Stop`]) once free memory falls under the floor or
//!   memory pressure passes its limit, or a signal can no longer be read.
//! - **LA4.** Each stop for memory or pressure of a batch job in progress is
//!   counted until a batch job finishes, and from [`Policy::stuck_after`] in
//!   a row the decision says it is stuck, so a job that never fits is
//!   reported, never retried silently. A stop with no job in progress is
//!   never counted, so an idle lane is never called stuck.

use std::num::NonZeroU32;

/// What the kernel and home say this tick.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Signals {
	/// A monotonic clock, in milliseconds.
	pub now_ms: u64,
	/// `/proc/meminfo`'s `MemAvailable`; `None` when it could not be read.
	pub mem_available_bytes: Option<u64>,
	/// `/proc/pressure/memory`'s `some avg10`, a percentage; `None` when it
	/// could not be read.
	pub memory_pressure_pct: Option<f64>,
	/// Interactive work is waiting or running (a voice note, a Rust round).
	pub interactive_waiting: bool,
	/// A batch job is in progress, running or paused.
	pub batch_in_progress: bool,
	/// The most a batch job is expected to take, as measured on earlier runs;
	/// `None` when it is not known, which is never room (LA2).
	pub batch_peak_bytes: Option<u64>,
	/// How many batch jobs have finished so far; a change resets the stops
	/// counted for LA4.
	pub batch_finished: u64,
}

/// The lane's thresholds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Policy {
	/// Free memory that must stay free beyond what batch work takes.
	pub floor_bytes: u64,
	/// Memory pressure (`some avg10`, percent) past which batch work stops.
	pub pressure_limit_pct: f64,
	/// How long room must hold before batch work starts after a stop.
	pub hold_down_ms: u64,
	/// Stops for memory or pressure in a row before a job is called stuck.
	pub stuck_after: NonZeroU32,
}

/// What batch work is told to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verb {
	/// Run, or start.
	Run,
	/// Stay where it is without taking the CPU; its memory stays held.
	Pause,
	/// Not run: stop if running or paused, letting go of its memory, and do
	/// not start.
	Stop,
}

/// Why the lane decided what it did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Why {
	/// Nothing holds batch work back.
	Clear,
	/// Interactive work is waiting.
	Busy,
	/// Free memory is under the floor, or under the floor plus the batch
	/// peak for a start.
	Memory,
	/// Memory pressure is past its limit.
	Pressure,
	/// A signal could not be read.
	Unknown,
	/// Room has come back, and is being held for the hold-down.
	Settling,
}

impl Verb {
	/// Its name, as the state file and the metrics carry it.
	#[must_use]
	pub const fn name(self) -> &'static str {
		match self {
			Self::Run => "run",
			Self::Pause => "pause",
			Self::Stop => "stop",
		}
	}
}

impl Why {
	/// Its name, as the state file and the metrics carry it.
	#[must_use]
	pub const fn name(self) -> &'static str {
		match self {
			Self::Clear => "clear",
			Self::Busy => "busy",
			Self::Memory => "memory",
			Self::Pressure => "pressure",
			Self::Unknown => "unknown",
			Self::Settling => "settling",
		}
	}
}

/// What the lane remembers between ticks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct State {
	verb: Verb,
	room_since_ms: Option<u64>,
	stops: u32,
	finished_seen: u64,
}

impl State {
	/// The lane at start: stopped, so nothing runs before the first reading.
	#[must_use]
	pub const fn new(batch_finished: u64) -> Self {
		Self {
			verb: Verb::Stop,
			room_since_ms: None,
			stops: 0,
			finished_seen: batch_finished,
		}
	}
}

/// One tick's decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Decision {
	pub verb: Verb,
	pub why: Why,
	/// Stops for memory or pressure since a batch job last finished.
	pub stops: u32,
	/// LA4: `stops` has reached [`Policy::stuck_after`].
	pub stuck: bool,
}

/// What forces batch work to stop now, if anything (LA3).
fn shortage(signals: &Signals, policy: &Policy) -> Option<Why> {
	let (Some(available), Some(pressure)) = (signals.mem_available_bytes, signals.memory_pressure_pct) else {
		return Some(Why::Unknown);
	};
	if available < policy.floor_bytes {
		Some(Why::Memory)
	} else if pressure > policy.pressure_limit_pct {
		Some(Why::Pressure)
	} else {
		None
	}
}

/// The lane's decision this tick, and what it remembers for the next.
#[must_use]
pub fn step(state: State, signals: &Signals, policy: &Policy) -> (State, Decision) {
	let mut stops = if signals.batch_finished == state.finished_seen { state.stops } else { 0 };
	let (verb, why, room_since_ms) = match state.verb {
		Verb::Run | Verb::Pause => match shortage(signals, policy) {
			Some(why) => {
				if signals.batch_in_progress && matches!(why, Why::Memory | Why::Pressure) {
					stops = stops.saturating_add(1);
				}
				(Verb::Stop, why, None)
			}
			None if signals.interactive_waiting => (Verb::Pause, Why::Busy, None),
			None => (Verb::Run, Why::Clear, None),
		},
		Verb::Stop => {
			let room = signals
				.mem_available_bytes
				.zip(signals.batch_peak_bytes)
				.map(|(available, peak)| available >= peak.saturating_add(policy.floor_bytes));
			match (shortage(signals, policy), room) {
				_ if signals.interactive_waiting => (Verb::Stop, Why::Busy, None),
				(Some(why), _) => (Verb::Stop, why, None),
				(None, None) => (Verb::Stop, Why::Unknown, None),
				(None, Some(false)) => (Verb::Stop, Why::Memory, None),
				(None, Some(true)) => match state.room_since_ms {
					Some(since) if signals.now_ms.saturating_sub(since) >= policy.hold_down_ms => (Verb::Run, Why::Clear, None),
					Some(since) => (Verb::Stop, Why::Settling, Some(since)),
					None => (Verb::Stop, Why::Settling, Some(signals.now_ms)),
				},
			}
		}
	};
	let next = State {
		verb,
		room_since_ms,
		stops,
		finished_seen: signals.batch_finished,
	};
	(
		next,
		Decision {
			verb,
			why,
			stops,
			stuck: stops >= policy.stuck_after.get(),
		},
	)
}

/// `MemAvailable` in bytes, from the text of `/proc/meminfo`.
#[must_use]
pub fn mem_available_bytes(meminfo: &str) -> Option<u64> {
	let line = meminfo.lines().find(|line| line.starts_with("MemAvailable:"))?;
	let mut words = line.split_whitespace().skip(1);
	let value: u64 = words.next()?.parse().ok()?;
	match words.next() {
		Some("kB") => value.checked_mul(1024),
		_ => None,
	}
}

/// The `some avg10` percentage, from the text of a `/proc/pressure/*` file.
#[must_use]
pub fn pressure_some_avg10(pressure: &str) -> Option<f64> {
	let line = pressure.lines().find(|line| line.starts_with("some "))?;
	let value = line.split_whitespace().find_map(|field| field.strip_prefix("avg10="))?;
	value.parse().ok().filter(|pct: &f64| pct.is_finite() && *pct >= 0.0)
}

#[cfg(test)]
mod tests;

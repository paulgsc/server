use std::num::NonZeroU32;

use super::{mem_available_bytes, pressure_some_avg10, step, Decision, Policy, Signals, State, Verb, Why};

const GB: u64 = 1024 * 1024 * 1024;

const STUCK_AFTER: NonZeroU32 = match NonZeroU32::new(3) {
	Some(n) => n,
	None => panic!(),
};

const POLICY: Policy = Policy {
	floor_bytes: 2 * GB,
	pressure_limit_pct: 10.0,
	hold_down_ms: 60_000,
	stuck_after: STUCK_AFTER,
};

/// Room for a 4 GB batch peak above the 2 GB floor, no pressure, nobody waiting.
const fn roomy(now_ms: u64) -> Signals {
	Signals {
		now_ms,
		mem_available_bytes: Some(8 * GB),
		memory_pressure_pct: Some(0.0),
		interactive_waiting: false,
		batch_in_progress: true,
		batch_peak_bytes: Some(4 * GB),
		batch_finished: 0,
	}
}

/// Runs `ticks` from a fresh lane and gives every decision.
fn run(ticks: &[Signals]) -> Vec<Decision> {
	let mut state = State::new(0);
	ticks
		.iter()
		.map(|signals| {
			let (next, decision) = step(state, signals, &POLICY);
			state = next;
			decision
		})
		.collect()
}

fn verbs(decisions: &[Decision]) -> Vec<(Verb, Why)> {
	decisions.iter().map(|d| (d.verb, d.why)).collect()
}

/// Settled into running at 60 s, then `then` at 61 s.
fn running_then(then: Signals) -> Decision {
	let decisions = run(&[roomy(0), roomy(60_000), then]);
	assert_eq!(decisions[1].verb, Verb::Run);
	decisions[2]
}

#[test]
fn starts_stopped_and_runs_only_after_room_holds_for_the_hold_down() {
	let decisions = run(&[roomy(0), roomy(30_000), roomy(59_999), roomy(60_000)]);
	assert_eq!(
		verbs(&decisions),
		[
			(Verb::Stop, Why::Settling),
			(Verb::Stop, Why::Settling),
			(Verb::Stop, Why::Settling),
			(Verb::Run, Why::Clear)
		]
	);
}

#[test]
fn la1_never_runs_or_starts_while_interactive_work_waits() {
	let busy = |now_ms| Signals {
		interactive_waiting: true,
		..roomy(now_ms)
	};
	assert_eq!(verbs(&run(&[busy(0), busy(120_000)])), [(Verb::Stop, Why::Busy), (Verb::Stop, Why::Busy)]);
	let paused = running_then(busy(61_000));
	assert_eq!((paused.verb, paused.why), (Verb::Pause, Why::Busy));
}

#[test]
fn a_pause_resumes_as_soon_as_interactive_work_is_done() {
	let busy = |now_ms| Signals {
		interactive_waiting: true,
		..roomy(now_ms)
	};
	let decisions = run(&[roomy(0), roomy(60_000), busy(61_000), roomy(62_000)]);
	assert_eq!(verbs(&decisions[2..]), [(Verb::Pause, Why::Busy), (Verb::Run, Why::Clear)]);
}

#[test]
fn a_busy_tick_restarts_the_hold_down() {
	let busy = |now_ms| Signals {
		interactive_waiting: true,
		..roomy(now_ms)
	};
	let decisions = run(&[roomy(0), busy(50_000), roomy(60_000), roomy(119_999), roomy(120_000)]);
	assert_eq!(verbs(&decisions[2..]), [(Verb::Stop, Why::Settling), (Verb::Stop, Why::Settling), (Verb::Run, Why::Clear)]);
}

#[test]
fn la2_never_starts_without_room_for_the_peak_above_the_floor() {
	// 5.9 GB free: past the 2 GB floor, short of 4 GB + 2 GB.
	let tight = |now_ms| Signals {
		mem_available_bytes: Some(6 * GB - 1),
		..roomy(now_ms)
	};
	assert_eq!(verbs(&run(&[tight(0), tight(600_000)])), [(Verb::Stop, Why::Memory), (Verb::Stop, Why::Memory)]);
	let exact = |now_ms| Signals {
		mem_available_bytes: Some(6 * GB),
		..roomy(now_ms)
	};
	assert_eq!(run(&[exact(0), exact(60_000)])[1].verb, Verb::Run);
}

#[test]
fn la2_a_reading_that_could_not_be_made_is_never_room() {
	let no_memory = |now_ms| Signals {
		mem_available_bytes: None,
		..roomy(now_ms)
	};
	let no_pressure = |now_ms| Signals {
		memory_pressure_pct: None,
		..roomy(now_ms)
	};
	assert_eq!(verbs(&run(&[no_memory(0), no_memory(600_000)])), [(Verb::Stop, Why::Unknown), (Verb::Stop, Why::Unknown)]);
	assert_eq!(
		verbs(&run(&[no_pressure(0), no_pressure(600_000)])),
		[(Verb::Stop, Why::Unknown), (Verb::Stop, Why::Unknown)]
	);
}

#[test]
fn la3_running_or_paused_work_lets_go_when_memory_runs_short() {
	let short = |now_ms| Signals {
		mem_available_bytes: Some(2 * GB - 1),
		..roomy(now_ms)
	};
	let stopped = running_then(short(61_000));
	assert_eq!((stopped.verb, stopped.why, stopped.stops), (Verb::Stop, Why::Memory, 1));
	// Paused and short: interactive work waiting does not keep the memory held.
	let paused_short = |now_ms| Signals {
		interactive_waiting: true,
		..short(now_ms)
	};
	let busy = |now_ms| Signals {
		interactive_waiting: true,
		..roomy(now_ms)
	};
	let decisions = run(&[roomy(0), roomy(60_000), busy(61_000), paused_short(62_000)]);
	assert_eq!((decisions[3].verb, decisions[3].why), (Verb::Stop, Why::Memory));
}

#[test]
fn la3_running_work_lets_go_past_the_pressure_limit_but_not_at_it() {
	let at = running_then(Signals {
		memory_pressure_pct: Some(10.0),
		..roomy(61_000)
	});
	assert_eq!(at.verb, Verb::Run);
	let past = running_then(Signals {
		memory_pressure_pct: Some(10.01),
		..roomy(61_000)
	});
	assert_eq!((past.verb, past.why, past.stops), (Verb::Stop, Why::Pressure, 1));
}

#[test]
fn la3_running_work_stops_when_a_reading_is_lost_without_counting_it() {
	let lost = running_then(Signals {
		mem_available_bytes: None,
		..roomy(61_000)
	});
	assert_eq!((lost.verb, lost.why, lost.stops), (Verb::Stop, Why::Unknown, 0));
}

#[test]
fn la4_stops_in_a_row_are_stuck_and_a_finished_job_clears_them() {
	let short = |now_ms| Signals {
		mem_available_bytes: Some(GB),
		..roomy(now_ms)
	};
	let mut ticks = Vec::new();
	for round in 0..3 {
		let t = round * 200_000;
		ticks.extend([roomy(t), roomy(t + 60_000), short(t + 61_000)]);
	}
	let decisions = run(&ticks);
	let stops: Vec<(u32, bool)> = decisions.iter().filter(|d| d.why == Why::Memory).map(|d| (d.stops, d.stuck)).collect();
	assert_eq!(stops, [(1, false), (2, false), (3, true)]);
	// The worker finishes a job: the count starts again.
	let finished = Signals {
		batch_finished: 1,
		..roomy(700_000)
	};
	let mut state = State::new(0);
	for signals in &ticks {
		state = step(state, signals, &POLICY).0;
	}
	let (_, after) = step(state, &finished, &POLICY);
	assert_eq!((after.stops, after.stuck), (0, false));
}

#[test]
fn the_names_are_what_the_state_file_and_metrics_carry() {
	assert_eq!([Verb::Run.name(), Verb::Pause.name(), Verb::Stop.name()], ["run", "pause", "stop"]);
	assert_eq!(
		[Why::Clear, Why::Busy, Why::Memory, Why::Pressure, Why::Unknown, Why::Settling].map(Why::name),
		["clear", "busy", "memory", "pressure", "unknown", "settling"]
	);
}

#[test]
fn reads_mem_available_from_meminfo() {
	let meminfo = "MemTotal:       16303456 kB\nMemFree:          812344 kB\nMemAvailable:    9120000 kB\nBuffers:          120000 kB\n";
	assert_eq!(mem_available_bytes(meminfo), Some(9_120_000 * 1024));
	assert_eq!(mem_available_bytes("MemTotal: 1 kB\n"), None);
	assert_eq!(mem_available_bytes("MemAvailable: lots kB\n"), None);
	assert_eq!(mem_available_bytes("MemAvailable: 12\n"), None);
}

#[test]
fn reads_some_avg10_from_a_pressure_file() {
	let pressure = "some avg10=1.53 avg60=0.40 avg300=0.10 total=123456\nfull avg10=0.80 avg60=0.20 avg300=0.05 total=6543\n";
	assert_eq!(pressure_some_avg10(pressure), Some(1.53));
	assert_eq!(pressure_some_avg10("full avg10=0.80 avg60=0.20 avg300=0.05 total=1\n"), None);
	assert_eq!(pressure_some_avg10("some avg10=NaN avg60=0 avg300=0 total=0\n"), None);
	assert_eq!(pressure_some_avg10(""), None);
}

#[test]
fn la4_an_idle_lane_is_never_called_stuck() {
	let idle = |now_ms| Signals {
		batch_in_progress: false,
		..roomy(now_ms)
	};
	let idle_short = |now_ms| Signals {
		mem_available_bytes: Some(GB),
		..idle(now_ms)
	};
	let mut ticks = Vec::new();
	for round in 0..4 {
		let t = round * 200_000;
		ticks.extend([idle(t), idle(t + 60_000), idle_short(t + 61_000)]);
	}
	let decisions = run(&ticks);
	assert!(decisions.iter().all(|d| d.stops == 0 && !d.stuck));
	assert_eq!(decisions.last().map(|d| (d.verb, d.why)), Some((Verb::Stop, Why::Memory)));
}

#[test]
fn la2_an_unmeasured_peak_is_never_room() {
	let unmeasured = |now_ms| Signals {
		batch_peak_bytes: None,
		..roomy(now_ms)
	};
	assert_eq!(verbs(&run(&[unmeasured(0), unmeasured(600_000)])), [(Verb::Stop, Why::Unknown), (Verb::Stop, Why::Unknown)]);
}

#[test]
fn la3_running_work_keeps_running_at_the_floor_and_stops_one_byte_under() {
	let at = running_then(Signals {
		mem_available_bytes: Some(2 * GB),
		..roomy(61_000)
	});
	assert_eq!((at.verb, at.why), (Verb::Run, Why::Clear));
	let under = running_then(Signals {
		mem_available_bytes: Some(2 * GB - 1),
		..roomy(61_000)
	});
	assert_eq!((under.verb, under.why), (Verb::Stop, Why::Memory));
}

#[test]
fn la3_a_reading_lost_while_paused_stops_without_counting() {
	let busy = |now_ms| Signals {
		interactive_waiting: true,
		..roomy(now_ms)
	};
	let lost = Signals {
		memory_pressure_pct: None,
		..busy(62_000)
	};
	let decisions = run(&[roomy(0), roomy(60_000), busy(61_000), lost]);
	assert_eq!((decisions[3].verb, decisions[3].why, decisions[3].stops), (Verb::Stop, Why::Unknown, 0));
}

#[test]
fn la4_a_pause_is_never_counted_as_a_stop() {
	let busy = |now_ms| Signals {
		interactive_waiting: true,
		..roomy(now_ms)
	};
	let decisions = run(&[roomy(0), roomy(60_000), busy(61_000), busy(62_000), roomy(63_000)]);
	assert!(decisions.iter().all(|d| d.stops == 0));
}

#[test]
fn la4_a_job_given_up_takes_its_stops_with_it() {
	let short = |now_ms| Signals {
		mem_available_bytes: Some(GB),
		..roomy(now_ms)
	};
	let mut ticks = Vec::new();
	for round in 0..3 {
		let t = round * 200_000;
		ticks.extend([roomy(t), roomy(t + 60_000), short(t + 61_000)]);
	}
	let decisions = run(&ticks);
	assert!(decisions.last().is_some_and(|d| d.stuck));
	// The worker gives the job up without finishing it: nothing is in progress.
	let idle = |now_ms| Signals {
		batch_in_progress: false,
		..roomy(now_ms)
	};
	ticks.extend([idle(700_000), idle(760_000), idle(820_000)]);
	let after = run(&ticks);
	assert!(after[9..].iter().all(|d| d.stops == 0 && !d.stuck));
	// The next job starts its own count.
	let next_short = Signals {
		mem_available_bytes: Some(GB),
		..roomy(821_000)
	};
	ticks.push(next_short);
	let next = run(&ticks);
	assert_eq!(next.last().map(|d| (d.stops, d.stuck)), Some((1, false)));
}

#[test]
fn la4_an_evacuated_job_stays_in_progress_and_keeps_its_count() {
	// Stopped for memory, the job is still in progress (not finished, not
	// given up), so its stops add up across evacuations.
	let short = |now_ms| Signals {
		mem_available_bytes: Some(GB),
		..roomy(now_ms)
	};
	let decisions = run(&[roomy(0), roomy(60_000), short(61_000), roomy(62_000), roomy(122_000), short(123_000)]);
	assert_eq!(decisions.iter().map(|d| d.stops).collect::<Vec<_>>(), [0, 0, 1, 1, 1, 2]);
}

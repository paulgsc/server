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
		batch_job: Some(1),
		batch_peak_bytes: Some(4 * GB),
	}
}

/// Runs `ticks` from a fresh lane and gives every decision.
fn run(ticks: &[Signals]) -> Vec<Decision> {
	let mut state = State::new();
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
	// That job finishes and the next starts: its own count starts again.
	let finished = Signals {
		batch_job: Some(2),
		..roomy(700_000)
	};
	let mut state = State::new();
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
	let idle = |now_ms| Signals { batch_job: None, ..roomy(now_ms) };
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
fn la2_no_peak_at_all_is_never_room() {
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
	assert!(run(&ticks).last().is_some_and(|d| d.stuck));
	// Given up with no tick in between: the next job (id 2) is already in
	// progress at the next tick, and its first stop is its first.
	let next = |now_ms| Signals {
		batch_job: Some(2),
		..roomy(now_ms)
	};
	let next_short = |now_ms| Signals {
		mem_available_bytes: Some(GB),
		..next(now_ms)
	};
	ticks.extend([next(700_000), next(760_000), next_short(761_000)]);
	assert_eq!(run(&ticks).last().map(|d| (d.stops, d.stuck)), Some((1, false)));
	// Given up with nothing after it: an idle lane carries no count.
	let idle = Signals {
		batch_job: None,
		..roomy(900_000)
	};
	let mut idle_ticks = ticks[..9].to_vec();
	idle_ticks.push(idle);
	assert_eq!(run(&idle_ticks).last().map(|d| (d.stops, d.stuck)), Some((0, false)));
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

#[test]
fn la2_an_idle_lane_keeps_its_run_only_while_room_for_the_peak_holds() {
	let idle = |now_ms, available| Signals {
		batch_job: None,
		mem_available_bytes: Some(available),
		..roomy(now_ms)
	};
	let decisions = run(&[idle(0, 8 * GB), idle(60_000, 8 * GB), idle(61_000, 2 * GB)]);
	assert_eq!((decisions[1].verb, decisions[2].verb, decisions[2].why), (Verb::Run, Verb::Stop, Why::Memory));
	// A job seen for the first time while the lane runs needs room too.
	let new_job = Signals {
		batch_job: Some(2),
		mem_available_bytes: Some(3 * GB),
		..roomy(61_000)
	};
	let handover = run(&[roomy(0), roomy(60_000), new_job]);
	assert_eq!((handover[2].verb, handover[2].why, handover[2].stops), (Verb::Stop, Why::Memory, 0));
	// The job already running needs only the floor: its memory is in use.
	let same_job = Signals {
		mem_available_bytes: Some(3 * GB),
		..roomy(61_000)
	};
	assert_eq!(run(&[roomy(0), roomy(60_000), same_job])[2].verb, Verb::Run);
}

#[test]
fn la2_a_shortage_or_lost_reading_while_settling_restarts_the_hold_down() {
	let tight = Signals {
		mem_available_bytes: Some(6 * GB - 1),
		..roomy(30_000)
	};
	let lost = Signals {
		memory_pressure_pct: None,
		..roomy(30_000)
	};
	let pressed = Signals {
		memory_pressure_pct: Some(50.0),
		..roomy(30_000)
	};
	for interruption in [tight, lost, pressed] {
		let decisions = run(&[roomy(0), interruption, roomy(60_000), roomy(119_999), roomy(120_000)]);
		assert_eq!(verbs(&decisions[2..]), [(Verb::Stop, Why::Settling), (Verb::Stop, Why::Settling), (Verb::Run, Why::Clear)]);
	}
}

#[test]
fn la3_memory_read_under_the_floor_is_memory_even_with_pressure_lost() {
	let short_and_lost = Signals {
		mem_available_bytes: Some(GB),
		memory_pressure_pct: None,
		..roomy(61_000)
	};
	let stopped = running_then(short_and_lost);
	assert_eq!((stopped.verb, stopped.why, stopped.stops), (Verb::Stop, Why::Memory, 1));
}

#[test]
fn la3_a_pressure_that_is_not_a_number_is_never_room() {
	let nan = |now_ms| Signals {
		memory_pressure_pct: Some(f64::NAN),
		..roomy(now_ms)
	};
	assert_eq!(verbs(&run(&[nan(0), nan(60_000)])), [(Verb::Stop, Why::Unknown), (Verb::Stop, Why::Unknown)]);
	let no_limit = Policy {
		pressure_limit_pct: f64::NAN,
		..POLICY
	};
	let (_, decision) = step(State::new(), &roomy(0), &no_limit);
	assert_eq!((decision.verb, decision.why), (Verb::Stop, Why::Pressure));
}

#[test]
fn la4_a_stop_from_pause_is_counted() {
	let busy = |now_ms| Signals {
		interactive_waiting: true,
		..roomy(now_ms)
	};
	let paused_short = Signals {
		mem_available_bytes: Some(GB),
		..busy(62_000)
	};
	let decisions = run(&[roomy(0), roomy(60_000), busy(61_000), paused_short]);
	assert_eq!((decisions[3].why, decisions[3].stops), (Why::Memory, 1));
}

#[test]
fn la4_an_id_change_while_running_starts_the_count_again() {
	let short = |now_ms| Signals {
		mem_available_bytes: Some(GB),
		..roomy(now_ms)
	};
	let next = |now_ms| Signals {
		batch_job: Some(2),
		..roomy(now_ms)
	};
	let next_short = Signals {
		mem_available_bytes: Some(GB),
		..next(400_000)
	};
	// Job 1 takes two stops, the lane runs again, job 2 starts while it runs.
	let decisions = run(&[
		roomy(0),
		roomy(60_000),
		short(61_000),
		roomy(100_000),
		roomy(160_000),
		short(161_000),
		roomy(200_000),
		roomy(260_000),
		next(261_000),
		next(262_000),
		next_short,
	]);
	assert_eq!(decisions[5].stops, 2);
	assert_eq!(decisions[8].verb, Verb::Run);
	assert_eq!((decisions[10].stops, decisions[10].stuck), (1, false));
}

#[test]
fn la4_stuck_after_one_is_stuck_at_the_first_stop() {
	let one = Policy {
		stuck_after: NonZeroU32::MIN,
		..POLICY
	};
	let mut state = State::new();
	let mut last = None;
	for signals in [
		roomy(0),
		roomy(60_000),
		Signals {
			mem_available_bytes: Some(GB),
			..roomy(61_000)
		},
	] {
		let (next, decision) = step(state, &signals, &one);
		state = next;
		last = Some(decision);
	}
	assert_eq!(last.map(|d| (d.stops, d.stuck)), Some((1, true)));
}

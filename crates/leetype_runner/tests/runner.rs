//! The runner against the real `rustc`, one tiny program per outcome
//! (#381's acceptance criteria: each of the three error classes, and output
//! truncated rather than streamed).
//!
//! These compile real programs, so they need `rustc` on `PATH` — CI's
//! `rust_ci` job has it, as every machine that builds this workspace does.
//! A missing compiler **fails** these tests (`Runner::new(..).unwrap()`); it
//! never skips them, because a runner test that passes having run nothing
//! proves nothing.

use leetype_round_repo::{ErrorClass, RunResult, Variant};
use leetype_runner::round::Constraint;
use leetype_runner::{Limits, RecordError, Runner};
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// A missing `rustc` fails the calling test here, loudly.
fn runner(limits: Limits) -> Runner {
	Runner::new(limits).unwrap_or_else(|err| panic!("the runner tests need rustc on PATH: {err}"))
}

fn n(bound: u64) -> Vec<Constraint> {
	vec![
		Constraint {
			dimension: String::from("n"),
			bound,
		},
		Constraint {
			dimension: String::from("m"),
			bound: 2,
		},
	]
}

/// The error branch's class and message, or a panic naming what came back.
fn failed(result: &RunResult) -> (ErrorClass, &str) {
	match result {
		RunResult::Error { error, .. } => (error.error_class, error.message.as_str()),
		RunResult::Ok { .. } => panic!("expected an error: {result:?}"),
	}
}

const ARGS: &str = r#"
fn sizes() -> (u64, u64) {
    let mut n = 0;
    let mut m = 0;
    for arg in std::env::args().skip(1) {
        let (dimension, value) = arg.split_once('=').unwrap();
        match dimension {
            "n" => n = value.parse().unwrap(),
            "m" => m = value.parse().unwrap(),
            other => panic!("unknown dimension {other}"),
        }
    }
    (n, m)
}
"#;

fn program(main: &str) -> String {
	String::from(ARGS) + main
}

#[test]
fn a_program_that_finishes_is_ok_with_its_output_logs_and_input_size() {
	let result = runner(Limits::default())
		.run_source(&program("fn main() { let (n, m) = sizes(); eprintln!(\"a note\"); println!(\"{}\", n * m); }"), &n(21))
		.unwrap();
	let RunResult::Ok { input_size, observation } = result else {
		panic!("{result:?}");
	};
	assert_eq!(input_size, 21, "the first dimension's bound");
	assert_eq!(observation.output, "42", "every dimension reached the harness; the final newline is dropped");
	assert_eq!(observation.logs, ["a note"]);
	assert!(observation.elapsed.milliseconds < 2_000);
}

#[test]
fn a_program_that_does_not_compile_is_a_compile_error_with_rustcs_message() {
	let result = runner(Limits::default()).run_source("fn main() { let x: u8 = \"no\"; }", &n(1)).unwrap();
	let (class, message) = failed(&result);
	assert_eq!(class, ErrorClass::Compile);
	assert!(message.contains("mismatched types"), "{message}");
	assert_eq!(result.input_size(), 1, "an error still carries its input size");
}

#[test]
fn a_panic_is_a_runtime_error_with_the_tail_of_stderr() {
	let result = runner(Limits::default())
		.run_source(&program("fn main() { let (n, _) = sizes(); if n > 3 { panic!(\"too big: {n}\"); } }"), &n(4))
		.unwrap();
	let (class, message) = failed(&result);
	assert_eq!(class, ErrorClass::Runtime);
	assert!(message.starts_with("the program exited with status 101"), "{message}");
	assert!(message.contains("too big: 4"), "{message}");
}

/// Past the ceiling the run is `budget-exceeded`, and the process is gone:
/// the program writes its pid before looping, and after the runner returns
/// that pid no longer exists.
#[test]
fn a_program_past_the_ceiling_is_budget_exceeded_and_killed() {
	let scratch = tempfile::tempdir().unwrap();
	let pid_file: PathBuf = scratch.path().join("pid");
	// A temporary directory's path needs no escaping inside a string literal.
	let source = String::from("fn main() {\n    std::fs::write(r\"")
		+ &pid_file.display().to_string()
		+ "\", std::process::id().to_string()).unwrap();\n    let mut i: u64 = 0;\n    loop { i = i.wrapping_add(1); std::hint::black_box(i); }\n}\n";
	let limits = Limits {
		run_ceiling: Duration::from_millis(300),
		..Limits::default()
	};

	let started = Instant::now();
	let result = runner(limits).run_source(&source, &n(1)).unwrap();
	let (class, message) = failed(&result);
	assert_eq!(class, ErrorClass::BudgetExceeded);
	assert_eq!(message, "the program did not finish within the 300 ms wall-clock ceiling, and was killed");
	assert!(started.elapsed() < Duration::from_secs(30), "compile plus a 300 ms run");

	let pid = std::fs::read_to_string(&pid_file).unwrap();
	assert!(!PathBuf::from("/proc").join(pid.trim()).exists(), "pid {pid} is still running");
}

/// A program that exits while something it spawned still holds its stdout
/// neither outlives the ceiling nor leaves that process running: the whole
/// group is killed however the program ended (review, #399: this took the
/// descendant's full lifetime, and left it running).
#[test]
fn a_descendant_is_killed_with_the_program_and_holds_nothing_open() {
	let scratch = tempfile::tempdir().unwrap();
	let pid_file: PathBuf = scratch.path().join("pid");
	let source = String::from("fn main() {\n    let child = std::process::Command::new(\"/bin/sleep\").arg(\"30\").spawn().unwrap();\n    std::fs::write(r\"")
		+ &pid_file.display().to_string()
		+ "\", child.id().to_string()).unwrap();\n    println!(\"bye\");\n}\n";
	let limits = Limits {
		run_ceiling: Duration::from_millis(500),
		..Limits::default()
	};

	let started = Instant::now();
	let result = runner(limits).run_source(&source, &n(1)).unwrap();
	assert!(started.elapsed() < Duration::from_secs(20), "took {:?}: waited on the descendant", started.elapsed());
	let RunResult::Ok { observation, .. } = result else {
		panic!("{result:?}");
	};
	assert_eq!(observation.output, "bye");
	let pid = std::fs::read_to_string(&pid_file).unwrap();
	// Orphaned by the program's exit, the killed descendant is reaped by
	// whatever adopted it, so it may linger a moment as a zombie: dead, but
	// still listed in /proc.
	let running = || {
		std::fs::read_to_string(PathBuf::from("/proc").join(pid.trim()).join("status")).is_ok_and(|status| {
			status
				.lines()
				.any(|line| line.starts_with("State:") && !line.contains("Z (zombie)") && !line.contains("X (dead)"))
		})
	};
	let deadline = Instant::now() + Duration::from_secs(2);
	while running() && Instant::now() < deadline {
		std::thread::sleep(Duration::from_millis(20));
	}
	assert!(!running(), "the descendant {pid} is still running");
}

/// The same failure records the same bytes, and no message names the
/// runner's temporary directory (review, #399): rustc is handed paths
/// relative to it, and a panic header's thread id is dropped.
#[test]
fn a_failure_records_the_same_bytes_twice_and_names_no_temporary_directory() {
	let runner = runner(Limits::default());
	let panics = program("fn main() { let (n, _) = sizes(); if n > 3 { panic!(\"too big: {n}\"); } }");
	let first = runner.run_source(&panics, &n(4)).unwrap();
	assert_eq!(first, runner.run_source(&panics, &n(4)).unwrap());
	let (_, message) = failed(&first);
	assert!(message.contains("thread 'main' panicked at program.rs:"), "{message}");

	let broken = "fn main() { let x: u8 = \"no\"; }";
	let compile = runner.run_source(broken, &n(1)).unwrap();
	assert_eq!(compile, runner.run_source(broken, &n(1)).unwrap());
	for result in [&first, &compile] {
		let (_, message) = failed(result);
		assert!(!message.contains("leetype-run-") && !message.contains("/tmp"), "{message}");
	}
}

/// An argument the OS will not pass is this round's failure, not the
/// machine's: the recorder marks the round failed and goes on (review, #399).
#[test]
fn a_program_that_cannot_be_started_fails_its_round_not_the_recording() {
	let constraints = vec![Constraint {
		dimension: String::from("n\0"),
		bound: 1,
	}];
	let err = runner(Limits::default()).run_source("fn main() {}", &constraints).unwrap_err();
	assert!(matches!(err, RecordError::Run(_)) && !err.is_toolchain(), "{err}");
}

/// Output past the ceiling is cut to it, and the cut is noted in the logs:
/// the runner keeps the ceiling's worth and drains the rest without keeping
/// it.
#[test]
fn output_past_the_ceiling_is_truncated_not_streamed() {
	let result = runner(Limits::default())
		.run_source("fn main() { println!(\"{}\", \"x\".repeat(10_000)); }", &n(1))
		.unwrap();
	let RunResult::Ok { observation, .. } = result else {
		panic!("{result:?}");
	};
	assert_eq!(observation.output, "x".repeat(4_096));
	assert_eq!(observation.logs, ["stdout truncated: kept the first 4096 of 10001 bytes"]);
}

#[test]
fn a_missing_compiler_is_a_toolchain_error_not_a_result() {
	let err = Runner::new(Limits {
		rustc: PathBuf::from("/nonexistent/rustc"),
		..Limits::default()
	})
	.unwrap_err();
	assert!(matches!(err, RecordError::Toolchain(..)) && err.is_toolchain(), "{err}");
}

/// A round whose hunk does not apply to `A` is refused before anything is
/// compiled, naming the variant.
#[test]
fn a_hunk_that_does_not_apply_refuses_the_round_by_variant() {
	let body = serde_json::json!({
		"id": "r", "algorithm": { "language": "rust", "source": "pub fn f() -> u8 {\n    1\n}\n" },
		"constraintDiff": { "before": [{ "dimension": "n", "operator": "<=", "bound": 1 }], "after": [{ "dimension": "n", "operator": "<=", "bound": 2 }] },
		"diffOptions": [
			{ "member": { "hunk": { "oldStart": 2, "newStart": 2, "segments": [{ "kind": "deletion", "text": "    1\n" }, { "kind": "addition", "text": "    2\n" }] } } },
			{ "member": { "hunk": { "oldStart": 2, "newStart": 2, "segments": [{ "kind": "deletion", "text": "    7\n" }, { "kind": "addition", "text": "    2\n" }] } } }
		],
		"harness": { "source": "fn main() { println!(\"{}\", f()); }\n" }
	});
	let err = runner(Limits::default()).record(body.to_string().as_bytes()).unwrap_err();
	let RecordError::Program(leetype_runner::round::ProgramError::Hunk { variant, .. }) = &err else {
		panic!("{err}");
	};
	assert_eq!(*variant, Variant::Diff(1));
	assert!(!err.is_toolchain());

	let mut no_harness = body;
	no_harness.as_object_mut().unwrap().remove("harness");
	assert!(matches!(runner(Limits::default()).record(no_harness.to_string().as_bytes()), Err(RecordError::NoHarness)));
}

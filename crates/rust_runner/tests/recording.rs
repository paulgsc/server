//! The runner against the real `rustc`: the worked example
//! (paulgsc/some-ui#2231), the limits every program is held to, and the
//! service's protocol end to end.
//!
//! These compile real programs, so they need `rustc` (and `prlimit`, for the
//! memory ceiling) on `PATH`, as `leetype_runner`'s tests do. A missing one
//! **fails** them; a runner test that passes having run nothing proves
//! nothing. They assert what holds for any recent toolchain (codes, spans,
//! outcomes); the exact text rustc 1.97.0 prints is checked inside the pinned
//! container by `scripts/check_rust_runner_sandbox.py`.

use rust_runner::recording::{CompileRecord, Outcome, Recorder, Recording, RunRecord, RunSpan, RustcRun, Why, PROFILE};
use rust_runner::service::{answer, ask, Refusal, Request, Response, CLIENT_TIMEOUT, REQUEST_CEILING};
use rust_runner::{Compiler, Limits, Profile};
use std::os::unix::net::UnixListener;
use std::path::Path;
use std::time::{Duration, Instant};

fn recorder(limits: Limits, work: &Path) -> Recorder {
	Recorder::new(limits, work.to_path_buf()).unwrap_or_else(|err| panic!("these tests need rustc on PATH: {err}"))
}

/// The worked example's program with `line` as its fourth line.
fn utf8_round(line: &str) -> String {
	String::from("fn main() {\n    let s = String::from(\"aéz\");\n    let r: usize = s.len() - 1;\n    ") + line + "\n}\n"
}

fn run_of(recording: &Recording) -> &rust_runner::recording::RunRecord {
	recording.run.as_ref().unwrap_or_else(|| panic!("it did not run: {recording:?}"))
}

#[test]
fn the_worked_example_records_what_rustc_and_the_program_did() {
	let work = tempfile::tempdir().unwrap();
	let recorder = recorder(Limits::default(), work.path());

	let indexed = recorder.record(&utf8_round("let c = s[r]; println!(\"{c}\");")).unwrap();
	assert_eq!(indexed.outcome, Outcome::CompileError, "{indexed:?}");
	assert!(indexed.run.is_none());
	let error = indexed.compile.diagnostics.iter().find(|diagnostic| diagnostic.level == "error").unwrap();
	assert_eq!(error.code.as_deref(), Some("E0277"));
	assert_eq!(error.message, "the type `str` cannot be indexed by `usize`");
	let primary = error.spans.iter().find(|span| span.is_primary).unwrap();
	assert_eq!((primary.file.as_str(), primary.line_start, primary.column_start), ("main.rs", 4, 15));
	assert!(
		indexed
			.compile
			.rendered
			.starts_with("error[E0277]: the type `str` cannot be indexed by `usize`\n --> main.rs:4:15\n"),
		"{}",
		indexed.compile.rendered
	);
	assert_eq!(indexed.note.as_deref(), Some("rustc exited with status 1"));
	let RustcRun::CompileError { code, span, stderr, .. } = indexed.rustc_run() else {
		panic!("{:?}", indexed.rustc_run());
	};
	assert_eq!(code.as_deref(), Some("E0277"));
	assert_eq!(
		span,
		Some(RunSpan {
			line_start: 4,
			column_start: 15,
			line_end: 4,
			column_end: 16,
		})
	);
	assert_eq!(stderr, indexed.compile.rendered);

	let byte = recorder.record(&utf8_round("let c = s.as_bytes()[r]; println!(\"{c}\");")).unwrap();
	assert_eq!(byte.outcome, Outcome::Ok, "{byte:?}");
	assert_eq!(run_of(&byte).stdout, "122\n");
	assert_eq!(run_of(&byte).exit_code, Some(0));
	assert!(matches!(byte.rustc_run(), RustcRun::Prints { stdout, .. } if stdout == "122\n"));

	let split = recorder.record(&utf8_round("let c = &s[1..2]; println!(\"{c}\");")).unwrap();
	assert_eq!(split.outcome, Outcome::Runtime, "{split:?}");
	assert_eq!(run_of(&split).exit_code, Some(101));
	assert!(
		run_of(&split)
			.stderr
			.starts_with("\nthread 'main' panicked at main.rs:4:15:\nend byte index 2 is not a char boundary; it is inside 'é' (bytes 1..3 of string)\n"),
		"{:?}",
		run_of(&split).stderr
	);
	assert_eq!(split.note.as_deref(), Some("the program exited with status 101"));
	assert!(matches!(split.rustc_run(), RustcRun::Panics { stderr, .. } if stderr == run_of(&split).stderr));

	let whole = recorder.record(&utf8_round("let c = &s[1..3]; println!(\"{c}\");")).unwrap();
	assert_eq!(whole.outcome, Outcome::Ok);
	assert_eq!(run_of(&whole).stdout, "é\n");
	// No warning here (`r` is used by nothing, but `s` is): whatever rustc
	// printed while building is kept beside the run, apart from it.
	let RustcRun::Prints { compile_stderr, .. } = whole.rustc_run() else {
		panic!("{:?}", whole.rustc_run());
	};
	assert_eq!(compile_stderr, whole.compile.rendered);
	let warned = recorder.record("fn main() {\n    let unused = 1;\n    println!(\"hi\");\n}\n").unwrap();
	let RustcRun::Prints { compile_stderr, stdout, .. } = warned.rustc_run() else {
		panic!("{:?}", warned.rustc_run());
	};
	assert!(compile_stderr.starts_with("warning: unused variable: `unused`\n"), "{compile_stderr:?}");
	assert_eq!(stdout, "hi\n", "the compiler's text never mixes into the program's");
	assert!(warned.compile.diagnostics.iter().any(|diagnostic| diagnostic.code.as_deref() == Some("unused_variables")));
}

/// The profile `leetype_runner` compiles with, for `record-leetype-runs`.
const LEETYPE: Profile = Profile {
	edition: "2021",
	crate_name: "round",
	warnings: false,
};

/// Asking for JSON loses nothing: the rendered text is what rustc prints
/// without the flag, byte for byte, under both profiles, so the messages
/// `record-leetype-runs` recorded did not change when its runner moved here,
/// and Rust Rounds' are what the owner's own rustc prints.
#[test]
fn the_rendered_text_is_what_rustc_prints_without_json() {
	let refused = "fn main() {\n    let x: u8 = \"no\";\n    let s = \"aéz\";\n    let c = s[1];\n}\n";
	let warned = "fn main() {\n    let unused = 1;\n}\n";
	for profile in [PROFILE, LEETYPE] {
		for source in [refused, warned] {
			let work = tempfile::tempdir().unwrap();
			let compiled = Compiler::new(Limits::default()).unwrap().compile(work.path(), "main", source, &profile).unwrap();
			assert_eq!(compiled.binary.is_none(), source == refused, "{profile:?}");

			let mut plain = std::process::Command::new("rustc");
			plain.args(["--edition", profile.edition, "-C", "opt-level=0"]);
			if !profile.warnings {
				plain.args(["-A", "warnings"]);
			}
			let plain = plain
				.args(["--crate-type", "bin", "--crate-name", profile.crate_name, "-o", "plain", "main.rs"])
				.current_dir(work.path())
				.output()
				.unwrap();
			assert_eq!(plain.status.success(), source == warned);
			assert_eq!(compiled.rendered, String::from_utf8(plain.stderr).unwrap(), "{profile:?}");
			if source == refused {
				assert!(compiled.diagnostics.len() >= 3, "two errors and the summary: {:?}", compiled.diagnostics);
			} else {
				assert_eq!(
					compiled.rendered.contains("warning: unused variable"),
					profile.warnings,
					"{profile:?} keeps warnings only when it says so"
				);
			}
		}
	}
}

#[test]
fn a_run_gets_no_environment_and_no_arguments() {
	let work = tempfile::tempdir().unwrap();
	let recording = recorder(Limits::default(), work.path())
		.record("fn main() { println!(\"{} {}\", std::env::vars_os().count(), std::env::args_os().count()); }")
		.unwrap();
	assert_eq!(run_of(&recording).stdout, "0 1\n", "no variables; argv holds only the program");
}

#[test]
fn a_loop_is_budget_exceeded_and_killed() {
	let work = tempfile::tempdir().unwrap();
	let limits = Limits {
		run_ceiling: Duration::from_millis(300),
		..Limits::default()
	};
	let started = Instant::now();
	let recording = recorder(limits, work.path())
		.record("fn main() { let mut i: u64 = 0; loop { i = i.wrapping_add(1); std::hint::black_box(i); } }")
		.unwrap();
	assert_eq!(recording.outcome, Outcome::BudgetExceeded);
	assert_eq!(
		recording.note.as_deref(),
		Some("the program did not finish within the 300 ms wall-clock ceiling, and was killed")
	);
	assert_eq!(run_of(&recording).signal, Some(9));
	assert!(matches!(recording.rustc_run(), RustcRun::Stopped { why: Why::Time, .. }));
	assert!(started.elapsed() < Duration::from_secs(30));
}

#[test]
fn unbounded_allocation_ends_as_a_runtime_error_at_the_memory_ceiling() {
	let work = tempfile::tempdir().unwrap();
	let recorder = recorder(Limits::default(), work.path());
	assert!(recorder.compiler().memory_ceiling_applied(), "these tests need prlimit on PATH");
	let recording = recorder
		.record("fn main() { let mut hoard: Vec<Vec<u8>> = Vec::new(); loop { hoard.push(vec![1_u8; 64 << 20]); std::hint::black_box(&hoard); } }")
		.unwrap();
	assert_eq!(recording.outcome, Outcome::Runtime, "{recording:?}");
	assert!(run_of(&recording).stderr.contains("memory allocation of"), "{:?}", run_of(&recording).stderr);
	assert!(matches!(recording.rustc_run(), RustcRun::Stopped { why: Why::Memory, .. }));
}

#[test]
fn output_past_the_ceiling_is_kept_to_it_and_counted() {
	let work = tempfile::tempdir().unwrap();
	let recording = recorder(Limits::default(), work.path())
		.record("fn main() { println!(\"{}\", \"x\".repeat(10_000)); eprintln!(\"{}\", \"y\".repeat(10_000)); panic!(\"last\"); }")
		.unwrap();
	let run = run_of(&recording);
	assert_eq!(run.stdout, "x".repeat(4_096));
	assert_eq!(run.stdout_bytes, 10_001);
	assert!(run.stderr.len() <= 4_096 && run.stderr.contains("last"), "stderr keeps its end, where the panic is");
	assert!(run.stderr_bytes > 10_000);
	assert!(run.cut);
	assert!(
		matches!(recording.rustc_run(), RustcRun::Stopped { why: Why::Output, .. }),
		"a cut stream is not what it printed"
	);
}

#[test]
fn the_same_program_records_the_same_bytes_but_its_time() {
	let work = tempfile::tempdir().unwrap();
	let recorder = recorder(Limits::default(), work.path());
	let source = utf8_round("let c = &s[1..2]; println!(\"{c}\");");
	let mut first = recorder.record(&source).unwrap();
	let mut second = recorder.record(&source).unwrap();
	for recording in [&mut first, &mut second] {
		recording.run.as_mut().unwrap().milliseconds = 0;
	}
	assert_eq!(first, second);
	assert_eq!(first.key, rust_runner::recording::key(recorder.toolchain(), &source));
	assert!(
		recorder.toolchain().starts_with("rustc 1.") && recorder.toolchain().ends_with("; --edition 2024 -C opt-level=0"),
		"{}",
		recorder.toolchain()
	);
	assert_eq!(std::fs::read_dir(work.path()).unwrap().count(), 0, "every program's directory is removed");
}

/// The protocol end to end: the toolchain, a refusal for each reason, and a
/// recording.
#[test]
fn the_service_answers_one_request_per_connection() {
	let work = tempfile::tempdir().unwrap();
	let socket = work.path().join("runner.sock");
	let programs = tempfile::tempdir().unwrap();
	let recorder = recorder(Limits::default(), programs.path());
	let toolchain = recorder.toolchain().to_owned();
	let listener = UnixListener::bind(&socket).unwrap();
	let server = std::thread::spawn(move || {
		let mut answered = Vec::new();
		for _ in 0..5 {
			let (stream, _) = listener.accept().unwrap();
			answered.push(answer(&recorder, &stream).unwrap());
		}
		answered
	});
	let timeout = Duration::from_secs(60);

	assert_eq!(ask(&socket, &Request::Toolchain {}, timeout).unwrap(), Response::Toolchain { toolchain: toolchain.clone() });
	let wrong = Request::Record {
		toolchain: String::from("rustc 0.0.0"),
		source: String::from("fn main() {}"),
	};
	assert_eq!(
		ask(&socket, &wrong, timeout).unwrap(),
		Response::Refused {
			reason: Refusal::WrongToolchain,
			toolchain: toolchain.clone(),
		}
	);
	let large = Request::Record {
		toolchain: toolchain.clone(),
		source: "/".repeat(usize::try_from(REQUEST_CEILING).unwrap()),
	};
	assert!(matches!(ask(&socket, &large, timeout).unwrap(), Response::Refused { reason: Refusal::TooLarge, .. }));
	{
		use std::io::{BufRead, Write};
		let mut stream = std::os::unix::net::UnixStream::connect(&socket).unwrap();
		// Everything a record needs, and one field more: refused for it.
		let mut trusted = serde_json::json!({"ask": "record", "toolchain": toolchain, "source": "fn main() {}", "trusted": true}).to_string();
		trusted.push('\n');
		stream.write_all(trusted.as_bytes()).unwrap();
		let mut line = String::new();
		std::io::BufReader::new(&stream).read_line(&mut line).unwrap();
		assert!(line.contains("\"reason\":\"malformed\""), "{line}");
	}
	let source = String::from("fn main() { println!(\"hi\"); }\n");
	let Response::Recorded { recording, rustc } = ask(
		&socket,
		&Request::Record {
			toolchain: toolchain.clone(),
			source: source.clone(),
		},
		timeout,
	)
	.unwrap() else {
		panic!("not recorded");
	};
	assert_eq!(recording.outcome, Outcome::Ok);
	assert_eq!(run_of(&recording).stdout, "hi\n");
	assert_eq!(rustc, recording.rustc_run());
	assert_eq!(recording.key, rust_runner::recording::key(&toolchain, &source));
	assert_eq!(server.join().unwrap().len(), 5);
}

/// The phone's shape (paulgsc/some-ui's `RustcRunSchema`, strict objects):
/// these exact keys and values, and no others.
#[test]
fn the_phones_shape_is_rr1s_rustc_run_field_for_field() {
	let toolchain = String::from("1.97.0");
	let shapes = [
		(
			RustcRun::CompileError {
				toolchain: toolchain.clone(),
				code: Some(String::from("E0277")),
				stderr: String::from("error[E0277]"),
				span: Some(RunSpan {
					line_start: 4,
					column_start: 15,
					line_end: 4,
					column_end: 16,
				}),
			},
			r#"{"outcome":"compile-error","toolchain":"1.97.0","code":"E0277","stderr":"error[E0277]","span":{"lineStart":4,"columnStart":15,"lineEnd":4,"columnEnd":16}}"#,
		),
		(
			RustcRun::CompileError {
				toolchain: toolchain.clone(),
				code: None,
				stderr: String::from("e"),
				span: None,
			},
			r#"{"outcome":"compile-error","toolchain":"1.97.0","code":null,"stderr":"e","span":null}"#,
		),
		(
			RustcRun::Prints {
				toolchain: toolchain.clone(),
				compile_stderr: String::new(),
				stdout: String::from("122\n"),
			},
			r#"{"outcome":"prints","toolchain":"1.97.0","compileStderr":"","stdout":"122\n"}"#,
		),
		(
			RustcRun::Panics {
				toolchain: toolchain.clone(),
				compile_stderr: String::from("warning: unused variable: `r`\n"),
				stdout: String::new(),
				stderr: String::from("boom"),
			},
			r#"{"outcome":"panics","toolchain":"1.97.0","compileStderr":"warning: unused variable: `r`\n","stdout":"","stderr":"boom"}"#,
		),
		(
			RustcRun::Stopped {
				toolchain,
				compile_stderr: String::new(),
				why: Why::Memory,
				stdout: String::new(),
				stderr: String::new(),
			},
			r#"{"outcome":"stopped","toolchain":"1.97.0","compileStderr":"","why":"memory","stdout":"","stderr":""}"#,
		),
	];
	for (run, json) in shapes {
		let mut written = Vec::new();
		serde_json::to_writer(&mut written, &run).unwrap();
		assert_eq!(String::from_utf8(written).unwrap(), json);
	}
	for (why, word) in [(Why::Time, "time"), (Why::Output, "output"), (Why::Exit, "exit")] {
		assert_eq!(serde_json::to_value(why).unwrap(), word);
	}
}

#[test]
fn the_phone_reads_the_release_alone() {
	let work = tempfile::tempdir().unwrap();
	let recording = recorder(Limits::default(), work.path()).record("fn main() {}").unwrap();
	let RustcRun::Prints { toolchain, .. } = recording.rustc_run() else {
		panic!("{recording:?}");
	};
	assert!(toolchain.starts_with("1.") && toolchain.split('.').count() == 3 && !toolchain.contains(' '), "{toolchain}");
}

/// A span's columns count characters, not bytes, and its end is exclusive:
/// past an `é`, the byte column would be one more.
#[test]
fn a_spans_columns_count_characters_and_its_end_is_exclusive() {
	let work = tempfile::tempdir().unwrap();
	let line = "    let t = \"é\"; let n: u8 = t;";
	let recording = recorder(Limits::default(), work.path())
		.record(&(String::from("fn main() {\n") + line + "\n    let _ = n;\n}\n"))
		.unwrap();
	let RustcRun::CompileError { span: Some(span), .. } = recording.rustc_run() else {
		panic!("{:?}", recording.rustc_run());
	};
	// The last `t`, 1-based: the line's characters but its final `;`.
	let at = u64::try_from(line.chars().count() - 1).unwrap();
	assert_eq!(
		span,
		RunSpan {
			line_start: 2,
			column_start: at,
			line_end: 2,
			column_end: at + 1,
		}
	);
}

/// One deadline for the whole request, not one per read: a client that
/// trickles a byte at a time loses the slot at the client timeout.
#[test]
fn a_trickling_client_is_refused_at_the_client_timeout() {
	let work = tempfile::tempdir().unwrap();
	let socket = work.path().join("runner.sock");
	let programs = tempfile::tempdir().unwrap();
	let recorder = recorder(Limits::default(), programs.path());
	let listener = UnixListener::bind(&socket).unwrap();
	let server = std::thread::spawn(move || {
		let (stream, _) = listener.accept().unwrap();
		let started = Instant::now();
		(answer(&recorder, &stream).unwrap(), started.elapsed())
	});
	let stream = std::os::unix::net::UnixStream::connect(&socket).unwrap();
	let trickle = std::thread::spawn(move || {
		use std::io::Write;
		for byte in b"{\"ask\":\"toolchain\"" {
			if (&stream).write_all(&[*byte]).is_err() {
				break;
			}
			std::thread::sleep(Duration::from_millis(500));
		}
	});
	let (response, took) = server.join().unwrap();
	assert!(matches!(response, Response::Refused { reason: Refusal::Malformed, .. }), "{response:?}");
	assert!(took >= CLIENT_TIMEOUT && took < CLIENT_TIMEOUT + Duration::from_secs(2), "{took:?}");
	trickle.join().unwrap();
}

/// What the phone reads when nothing ran: only a compile rustc refused is a
/// compile error. rustc stopped at its time or memory ceiling, or a binary
/// that could not be run, is `stopped`.
#[test]
fn nothing_run_is_a_compile_error_only_when_rustc_refused_it() {
	let recording = |outcome, compile: CompileRecord, run: Option<RunRecord>| Recording {
		key: String::from("k"),
		toolchain: String::from("rustc 1.97.0 (2d8144b78 2026-07-07); --edition 2024 -C opt-level=0"),
		outcome,
		note: None,
		compile,
		run,
	};
	let compile = |rendered: &str, timed_out, exit_code, signal| CompileRecord {
		diagnostics: Vec::new(),
		rendered: String::from(rendered),
		rendered_bytes: 0,
		timed_out,
		exit_code,
		signal,
	};
	let why = |recording: Recording| match recording.rustc_run() {
		RustcRun::Stopped { why, stdout, stderr, .. } => {
			assert!(stdout.is_empty() && stderr.is_empty());
			Some(why)
		}
		RustcRun::CompileError { .. } => None,
		other => panic!("{other:?}"),
	};
	assert_eq!(why(recording(Outcome::CompileError, compile("error[E0277]", false, Some(1), None), None)), None);
	assert_eq!(why(recording(Outcome::BudgetExceeded, compile("", true, None, Some(9)), None)), Some(Why::Time));
	assert_eq!(
		why(recording(
			Outcome::BudgetExceeded,
			compile("memory allocation of 40 bytes failed\n", false, None, Some(6)),
			None
		)),
		Some(Why::Memory)
	);
	assert_eq!(
		why(recording(Outcome::BudgetExceeded, compile("rustc-LLVM ERROR: out of memory\n", false, None, Some(6)), None)),
		Some(Why::Memory)
	);
	assert_eq!(
		why(recording(Outcome::BudgetExceeded, compile("", false, None, Some(25)), None)),
		Some(Why::Exit),
		"SIGXFSZ"
	);
	assert_eq!(
		why(recording(Outcome::Runtime, compile("", false, Some(0), None), None)),
		Some(Why::Exit),
		"built, but could not be run"
	);
}

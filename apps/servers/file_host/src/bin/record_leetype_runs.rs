//! Records each `leetype` round's runs into `leetype_round_run` (#381,
//! LTY-EXEC): `A` and every `A + d`, compiled with the round's harness and
//! run at `constraintDiff.before`'s bounds and at `constraintDiff.after`'s.
//!
//! ```sh
//! DATABASE_URL=sqlite:///path/to/file_host.db \
//!   cargo run -q --bin record-leetype-runs -- [--all] [--round <id>]
//! ```
//!
//! An operator command, run offline after `import-leetype-rounds`, on a
//! machine with `rustc` — a developer's, or CI. **It is the only thing in
//! this package that compiles or executes a program**, and nothing in
//! `main.rs` calls it, waits on it, or knows it exists: the server's
//! `GET /leetype/rounds/:id/runs` and `dump-leetype-snapshot` read what it
//! stored. See `apps/servers/file_host/docs/leetype-execution.md`.
//!
//! By default it records every listed round with a harness whose current
//! version has no runs yet; `--all` re-records every one. A round without a
//! harness is skipped, not failed.
//!
//! Exits 0 when every selected round was recorded, up to date or skipped, 1
//! when any round could not be run (a hunk that does not apply) or changed
//! while it was being recorded — the rest are still recorded — and 2 when the
//! run could not start or had to stop (no database, no `rustc`, an unknown
//! `--round`).
//!
//! Deliberately does not read [`file_host::Config`], for the reason
//! `import-leetype-rounds` gives.

use chrono::Utc;
use clap::Parser;
use leetype_round_repo::{RoundRepository, RunResult};
use leetype_runner::{record_listed, Limits, RoundOutcome, Runner, Selection, DEFAULT_MEMORY_CEILING};
use sqlx::sqlite::SqlitePoolOptions;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

#[derive(Parser, Debug)]
#[command(author, version, about = "Compiles and runs each LeetType round's programs, and records the RunResults in leetype_round_run")]
struct Args {
	/// Re-record rounds whose current version already has runs.
	#[arg(long)]
	all: bool,
	/// Record only this round (listed or retired).
	#[arg(long)]
	round: Option<String>,
	/// The compiler: a path, or a name looked up on PATH.
	#[arg(long, default_value = "rustc")]
	rustc: PathBuf,
	/// Wall-clock ceiling on one run, in milliseconds; past it a run is
	/// `budget-exceeded`.
	#[arg(long, default_value_t = 2_000)]
	run_ceiling_ms: u64,
	/// How long one compile may take, in seconds.
	#[arg(long, default_value_t = 60)]
	compile_timeout_secs: u64,
	/// The most stdout one run keeps, in bytes.
	#[arg(long, default_value_t = leetype_runner::DEFAULT_OUTPUT_CEILING)]
	output_ceiling: usize,
	/// The most stderr one run (or failed compile) keeps, in bytes.
	#[arg(long, default_value_t = leetype_runner::DEFAULT_LOGS_CEILING)]
	logs_ceiling: usize,
	/// Address-space ceiling on one run, in bytes, applied through `prlimit`
	/// when it is installed; 0 for none.
	#[arg(long, default_value_t = DEFAULT_MEMORY_CEILING)]
	memory_ceiling: u64,
	/// The (already migrated, already imported) database.
	#[arg(long, env = "DATABASE_URL")]
	database_url: String,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
	let args = Args::parse();
	let stdout = io::stdout();
	let mut out = stdout.lock();
	let mut err = io::stderr();

	let runner = match Runner::new(Limits {
		rustc: args.rustc.clone(),
		compile_timeout: Duration::from_secs(args.compile_timeout_secs),
		run_ceiling: Duration::from_millis(args.run_ceiling_ms),
		output_ceiling: args.output_ceiling,
		logs_ceiling: args.logs_ceiling,
		memory_ceiling: (args.memory_ceiling > 0).then_some(args.memory_ceiling),
	}) {
		Ok(runner) => runner,
		Err(error) => {
			let _ = writeln!(err, "record-leetype-runs: {error}");
			return ExitCode::from(2);
		}
	};
	if args.memory_ceiling > 0 && !runner.memory_ceiling_applied() {
		let _ = writeln!(out, "note: prlimit is not installed, so runs have no memory ceiling");
	}

	let pool = match SqlitePoolOptions::new().max_connections(1).connect(&args.database_url).await {
		Ok(pool) => pool,
		Err(error) => {
			let _ = writeln!(err, "record-leetype-runs: could not open {}: {error}", args.database_url);
			return ExitCode::from(2);
		}
	};
	let selection = Selection { all: args.all, only: args.round };
	let report = match record_listed(&RoundRepository::new(pool), &runner, &selection, &Utc::now().to_rfc3339()).await {
		Ok(report) => report,
		Err(error) => {
			let _ = writeln!(err, "record-leetype-runs: {error}");
			return ExitCode::from(2);
		}
	};

	for (id, outcome) in &report.rounds {
		match outcome {
			RoundOutcome::Recorded(runs) => {
				let _ = writeln!(out, "{id}: recorded {} runs", runs.len());
				for run in runs {
					let _ = write!(out, "  {:<3} {:<6} ", run.variant.label(), run.bounds.as_str());
					let _ = match &run.result {
						RunResult::Ok { observation, .. } => writeln!(out, "ok     {:>5} ms  {}", observation.elapsed.milliseconds, observation.output),
						RunResult::Error { error, .. } => writeln!(out, "error  {}  {}", error.error_class.as_str(), error.message.lines().next().unwrap_or_default()),
					};
				}
			}
			RoundOutcome::UpToDate => {
				let _ = writeln!(out, "{id}: up to date");
			}
			RoundOutcome::NoHarness => {
				let _ = writeln!(out, "{id}: no harness, skipped");
			}
			RoundOutcome::Changed => {
				let _ = writeln!(out, "CHANGED {id}: its body changed while it was recorded; not stored, run again");
			}
			RoundOutcome::Failed(why) => {
				let _ = writeln!(out, "FAILED {id}: {why}");
			}
		}
	}

	if report.incomplete() {
		ExitCode::from(1)
	} else {
		ExitCode::SUCCESS
	}
}

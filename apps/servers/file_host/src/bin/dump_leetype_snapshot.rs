//! Writes the static snapshot of the `leetype` round corpus and its recorded
//! runs (#328, LTY-SRV4), for `apps/www`'s static build, which has no server.
//!
//! ```sh
//! DATABASE_URL=sqlite:///path/to/file_host.db \
//!   cargo run -q --bin dump-leetype-snapshot -- <out-dir> [--check]
//! ```
//!
//! Writes `<out-dir>/rounds/manifest.json` and `<out-dir>/rounds/<id>.json`
//! (the listed rounds, in `@some-ui/leetype`'s export layout, each body byte
//! for byte) and `<out-dir>/runs/<id>.json` (`GET /leetype/rounds/:id/runs`'s
//! body). Deterministic: regenerating from an unchanged database changes no
//! byte. See `leetype_round_repo::snapshot` and
//! `apps/servers/file_host/docs/leetype-execution.md`.
//!
//! With `--check`, writes nothing and exits 1 if `<out-dir>` is not exactly
//! what it would write — the check that keeps a checked-in snapshot from
//! drifting from the database.
//!
//! Exits 0 when the snapshot was written (or, checking, is current), 1 when
//! a check found a difference, 2 when it could not run.

use clap::Parser;
use leetype_round_repo::{dump_snapshot, RoundRepository, SnapshotMode};
use sqlx::sqlite::SqlitePoolOptions;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser, Debug)]
#[command(author, version, about = "Writes the LeetType round corpus and its recorded runs as static files")]
struct Args {
	/// The snapshot's directory; `rounds/` and `runs/` are written inside it.
	out: PathBuf,
	/// Write nothing; exit 1 if the directory is not the current snapshot.
	#[arg(long)]
	check: bool,
	/// The database to read.
	#[arg(long, env = "DATABASE_URL")]
	database_url: String,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
	let args = Args::parse();
	let stdout = io::stdout();
	let mut out = stdout.lock();

	let pool = match SqlitePoolOptions::new().max_connections(1).connect(&args.database_url).await {
		Ok(pool) => pool,
		Err(err) => {
			let _ = writeln!(io::stderr(), "dump-leetype-snapshot: could not open {}: {err}", args.database_url);
			return ExitCode::from(2);
		}
	};
	let mode = if args.check { SnapshotMode::Check } else { SnapshotMode::Write };
	let report = match dump_snapshot(&RoundRepository::new(pool), &args.out, mode).await {
		Ok(report) => report,
		Err(err) => {
			let _ = writeln!(io::stderr(), "dump-leetype-snapshot: {err}");
			return ExitCode::from(2);
		}
	};

	let (wrote, removed) = if args.check { ("differ", "would be removed") } else { ("written", "removed") };
	for file in &report.written {
		let _ = writeln!(out, "{wrote}: {file}");
	}
	for file in &report.removed {
		let _ = writeln!(out, "{removed}: {file}");
	}
	for id in &report.unrecorded {
		let _ = writeln!(out, "note: {id} has no runs recorded for its current version");
	}
	let _ = writeln!(
		out,
		"{} round(s); {} file(s) {wrote}, {} {removed}",
		report.rounds.len(),
		report.written.len(),
		report.removed.len()
	);

	if args.check && !(report.written.is_empty() && report.removed.is_empty()) {
		ExitCode::from(1)
	} else {
		ExitCode::SUCCESS
	}
}

//! Imports an existing `topiks`-shaped lesson corpus into `curriculum` (#275).
//!
//! ```sh
//! DATABASE_URL=sqlite:///path/to/file_host.db \
//!   cargo run -q --bin import-curriculum -- path/to/public/topiks [--dry-run] [--activity topik]
//! ```
//!
//! An operator command, run offline — see `curriculum_repo::importer` for why
//! it is not a startup hook or an endpoint, and `docs/study-nudge.md`
//! ("Importing lesson content") for when to run it. Nothing in `main.rs` calls
//! it, waits on it, or knows it exists.
//!
//! Exits 0 when every lesson imported (or would have), 1 when any lesson
//! failed — the rest are still imported, except on a first import, which
//! writes nothing unless every lesson succeeds — and 2 when the run could not
//! start at all (no manifest, not a manifest, no database).
//!
//! Deliberately does not read [`file_host::Config`], for the same reason
//! `dump-routes` does not: the server's config requires secrets this command
//! has no use for, and a command that is annoying to run does not get run.

use chrono::Utc;
use clap::Parser;
use curriculum_repo::import_dir;
use sqlx::sqlite::SqlitePoolOptions;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser, Debug)]
#[command(author, version, about = "Imports a topiks-shaped lesson corpus (manifest.json + <key>.json) into the curriculum table")]
struct Args {
	/// The directory holding `manifest.json` and one `<key>.json` per lesson.
	dir: PathBuf,
	/// Report what would change without writing anything.
	#[arg(long)]
	dry_run: bool,
	/// The catalogue activity these lessons are for.
	#[arg(long, default_value = "topik")]
	activity: String,
	/// The (already migrated) database to import into.
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
			let _ = writeln!(io::stderr(), "import-curriculum: could not open {}: {err}", args.database_url);
			return ExitCode::from(2);
		}
	};

	let report = match import_dir(&pool, &args.dir, &args.activity, &Utc::now().to_rfc3339(), args.dry_run).await {
		Ok(report) => report,
		Err(err) => {
			let _ = writeln!(io::stderr(), "import-curriculum: {err}");
			return ExitCode::from(2);
		}
	};

	let verb = if args.dry_run || report.rolled_back { "would be" } else { "were" };
	let _ = writeln!(
		out,
		"{} new, {} changed, {} renamed only, {} unchanged {verb} imported",
		report.inserted.len(),
		report.content_changed.len(),
		report.metadata_changed.len(),
		report.unchanged.len()
	);
	if report.rolled_back {
		let _ = writeln!(
			out,
			"first import into an empty table, and a lesson failed: nothing was written, so the re-run after fixing it is still the baseline"
		);
	} else if report.baseline && !args.dry_run {
		let _ = writeln!(
			out,
			"first import into an empty table: recorded as the baseline, so none of it will be announced as new material"
		);
	}
	for (what, why) in &report.failed {
		let _ = writeln!(out, "FAILED {what}: {why}");
	}

	if report.failed.is_empty() {
		ExitCode::SUCCESS
	} else {
		ExitCode::from(1)
	}
}

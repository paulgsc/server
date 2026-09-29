//! Imports the round corpus exported by `@some-ui/leetype` into
//! `leetype_round` (#326, LTY-SRV2).
//!
//! ```sh
//! DATABASE_URL=sqlite:///path/to/file_host.db \
//!   cargo run -q --bin import-leetype-rounds -- path/to/corpus/rounds [--dry-run]
//! ```
//!
//! An operator command, run offline, like `import-curriculum`: see
//! `leetype_round_repo::importer` for the rules, and `docs/study-nudge.md`
//! (the rounds section of "The HTTP surface") for when to run it. Nothing in
//! `main.rs` calls it, waits on it, or knows it exists.
//!
//! Exits 0 when every round imported (or would have), 1 when any round failed
//! — the rest are still imported — and 2 when the run could not start at all
//! (no manifest, not a manifest, no database).
//!
//! Deliberately does not read [`file_host::Config`], for the same reason
//! `import-curriculum` does not: the server's config requires secrets this
//! command has no use for.

use chrono::Utc;
use clap::Parser;
use leetype_round_repo::import_dir;
use sqlx::sqlite::SqlitePoolOptions;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser, Debug)]
#[command(author, version, about = "Imports a LeetType round corpus (manifest.json + <id>.json) into the leetype_round table")]
struct Args {
	/// The directory holding `manifest.json` and one `<id>.json` per round.
	dir: PathBuf,
	/// Report what would change without writing anything.
	#[arg(long)]
	dry_run: bool,
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
			let _ = writeln!(io::stderr(), "import-leetype-rounds: could not open {}: {err}", args.database_url);
			return ExitCode::from(2);
		}
	};

	let report = match import_dir(&pool, &args.dir, &Utc::now().to_rfc3339(), args.dry_run).await {
		Ok(report) => report,
		Err(err) => {
			let _ = writeln!(io::stderr(), "import-leetype-rounds: {err}");
			return ExitCode::from(2);
		}
	};

	let verb = if args.dry_run { "would be" } else { "were" };
	let _ = writeln!(
		out,
		"{} new, {} changed, {} unchanged {verb} imported",
		report.inserted.len(),
		report.content_changed.len(),
		report.unchanged.len()
	);
	for (what, why) in &report.failed {
		let _ = writeln!(out, "FAILED {what}: {why}");
	}

	if report.failed.is_empty() {
		ExitCode::SUCCESS
	} else {
		ExitCode::from(1)
	}
}

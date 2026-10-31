//! Backs up `file_host`'s database, then applies the migrations this build
//! embeds (AS16, #563).
//!
//! ```sh
//! DATABASE_URL=sqlite:///app/dbs/hopium.db migrate [--keep-backups 5] [--init]
//! ```
//!
//! Compose's one-shot `migrate` service runs it from the same image before
//! `file-host` starts (`infra/compose/file_host.yml`), and `make up` runs it
//! first, while the old container still serves. What it does, and what it
//! refuses, is `file_host::migrate`. It needs no `sqlx-cli`: the migrations
//! are the ones `file_host::schema::MIGRATOR` embeds.
//!
//! Exits 0 when the database is current (already, or now), and 1, with the
//! reason on stderr, when it refused or failed: compose then leaves the new
//! `file_host` unstarted.
//!
//! Deliberately does not read [`file_host::Config`], as `import-curriculum`
//! does not: the server's config requires settings a migration has no use
//! for, and the container runs with nothing but the database.

use clap::Parser;
use file_host::migrate::{deploy, Options, Outcome, DEFAULT_KEEP_BACKUPS};
use file_host::schema::MIGRATOR;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser, Debug)]
#[command(author, version, about = "Backs up file_host's database, then applies the migrations this build embeds")]
struct Args {
	/// The database to migrate, as `sqlite://<path>`.
	#[arg(long, env = "DATABASE_URL")]
	database_url: String,
	/// Where backups go; `backups/` beside the database by default.
	#[arg(long)]
	backup_dir: Option<PathBuf>,
	/// How many of its own backups to keep, newest first.
	#[arg(long, default_value_t = DEFAULT_KEEP_BACKUPS, value_parser = clap::value_parser!(u16).range(1..))]
	keep_backups: u16,
	/// A new host: create the database when no file is there, and migrate
	/// one that has never been migrated. Without it both are refused, since
	/// both are what a volume that is not mounted looks like.
	#[arg(long)]
	init: bool,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
	let args = Args::parse();
	let options = Options {
		database_url: args.database_url,
		backup_dir: args.backup_dir,
		keep: usize::from(args.keep_backups),
		init: args.init,
	};

	let outcome = match deploy(&options, &MIGRATOR).await {
		Ok(outcome) => outcome,
		Err(refused) => {
			let _ = writeln!(io::stderr(), "migrate: {refused}");
			return ExitCode::FAILURE;
		}
	};

	let stdout = io::stdout();
	let mut out = stdout.lock();
	let pruned = match outcome {
		Outcome::Current { pruned } => {
			let _ = writeln!(out, "migrate: current, nothing to apply");
			pruned
		}
		Outcome::Applied { backup, applied, pruned } => {
			if let Some(backup) = backup {
				let _ = writeln!(out, "migrate: backed up to {} (verified)", backup.display());
			}
			let _ = writeln!(out, "migrate: applied {} ({})", applied.len(), applied.join(", "));
			pruned
		}
	};
	for path in pruned {
		let _ = writeln!(out, "migrate: removed the old backup {}", path.display());
	}
	ExitCode::SUCCESS
}

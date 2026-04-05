//! `ts` — tabsched CLI entry point.
//!
//! # Commands
//!
//! ```text
//! ts init                    scaffold topology.toml
//! ts next                    peek at the next session (no state change)
//! ts step [--outcome <o>]    execute one slot, save state
//! ts done <outcome>          annotate last session outcome
//! ts status                  per-track deficit table
//! ts history [--last N]      recent session log
//! ```
//!
//! # Data directory
//!
//! Default: `~/.local/share/tabsched`
//! Override: `TS_DATA_DIR` env var or `--data-dir` flag.
//!
//! Files:
//! - `topology.toml` — track/resource definitions (user-edited)
//! - `state.json`    — session history (managed by ts)

mod cmd;
mod config;
mod ctx;
mod ui;

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "ts", about = "Deterministic exposure scheduler for learning domains", version)]
struct Cli {
	/// Data directory containing topology.toml and state.json.
	/// Default: $TS_DATA_DIR or ~/.local/share/tabsched
	#[arg(long, env = "TS_DATA_DIR", global = true)]
	data_dir: Option<PathBuf>,

	#[command(subcommand)]
	command: Command,
}

#[derive(Subcommand)]
enum Command {
	/// Scaffold a starter topology.toml (does not overwrite existing).
	Init,

	/// Peek at the next scheduled session without advancing state.
	Next,

	/// Execute the next slot and save state.
	Step(cmd::step::Args),

	/// Record the outcome of the most recent session.
	Done(cmd::done::Args),

	/// Show per-track deficit and session counts.
	Status,

	/// View recent session history.
	History(cmd::history::Args),
}

fn main() -> Result<()> {
	dotenvy::dotenv().ok();
	let cli = Cli::parse();
	let data_dir = resolve_data_dir(cli.data_dir)?;

	// `init` does not need a loaded Ctx (topology.toml may not exist yet).
	if let Command::Init = &cli.command {
		return cmd::init::run(&data_dir);
	}

	let mut ctx = ctx::Ctx::load(&data_dir).context("failed to load scheduler state")?;

	match &cli.command {
		Command::Init => unreachable!(),
		Command::Next => cmd::next::run(&ctx),
		Command::Step(args) => cmd::step::run(&mut ctx, args),
		Command::Done(args) => cmd::done::run(&mut ctx, args),
		Command::Status => cmd::status::run(&ctx),
		Command::History(args) => cmd::history::run(&ctx, args),
	}
}

fn resolve_data_dir(flag: Option<PathBuf>) -> Result<PathBuf> {
	if let Some(d) = flag {
		return Ok(d);
	}
	// XDG-style default: ~/.local/share/tabsched
	let base = dirs_next::data_local_dir().context("cannot determine local data directory")?;
	Ok(base.join("tabsched"))
}

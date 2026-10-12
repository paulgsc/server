//! Emits one team's Playoff Delta story for a season, as the JSON
//! `paulgsc/some-ui`'s `@some-ui/playoff-delta` stage reads, on stdout.
//!
//! ```sh
//! cargo run -q --release --bin dump-playoff-delta -- \
//!   --games games.csv --season 2026 --team SF --rivals DAL,SEA > season.json
//! ```
//!
//! `--games` is an nflverse `games.csv` holding the asked season and the one
//! before it (the benchmark); see `file_host::playoff_delta` for the format.
//! Every number comes from `playoff_delta`, which is deterministic: the same
//! file gives the same story, byte for byte.
//!
//! Like `dump-routes`, it does not read [`file_host::Config`]: it needs a file
//! and nothing else the server is configured with.

use clap::Parser;
use file_host::playoff_delta::season_from_nflverse;
use playoff_delta::{
	evaluate::{evaluate, Subject},
	league::Team,
};
use std::{
	io::{self, Write},
	path::PathBuf,
	process::ExitCode,
};

#[derive(Parser, Debug)]
#[command(author, version, about = "Emits a team's Playoff Delta story (the some-ui stage's JSON) for one season")]
struct Args {
	/// An nflverse `games.csv` with this season and the previous one.
	#[arg(long)]
	games: PathBuf,
	#[arg(long)]
	season: u16,
	/// The team the story is about, by contract code (`SF`).
	#[arg(long)]
	team: String,
	/// Named rivals, comma separated (`DAL,SEA`).
	#[arg(long, value_delimiter = ',')]
	rivals: Vec<String>,
	/// Shown on screen; defaults to the season's year.
	#[arg(long)]
	label: Option<String>,
}

fn run(args: Args) -> Result<(), Box<dyn std::error::Error>> {
	let code = |c: &str| Team::from_code(c).ok_or_else(|| io::Error::other(String::from("unknown team code: ") + c));
	let subject = Subject {
		team: code(&args.team)?,
		rivals: args.rivals.iter().map(|c| code(c)).collect::<Result<_, _>>()?,
		label: args.label.unwrap_or_else(|| args.season.to_string()),
	};
	let csv = std::fs::read_to_string(&args.games)?;
	let season = season_from_nflverse(&csv, args.season)?;
	let previous = season_from_nflverse(&csv, args.season - 1)?;
	let story = evaluate(&season, &previous, &subject)?;
	let stdout = io::stdout();
	let mut out = stdout.lock();
	serde_json::to_writer_pretty(&mut out, &story)?;
	out.write_all(b"\n")?;
	out.flush()?;
	Ok(())
}

fn main() -> ExitCode {
	match run(Args::parse()) {
		Ok(()) => ExitCode::SUCCESS,
		Err(e) => {
			let mut err = io::stderr().lock();
			// Nothing more to do if stderr itself is gone.
			let _ = writeln!(err, "dump-playoff-delta: {e}");
			ExitCode::FAILURE
		}
	}
}

//! The weekly Playoff Delta job: one team's story for a season, as the JSON
//! `paulgsc/some-ui`'s `@some-ui/playoff-delta` stage reads, on stdout.
//!
//! ```sh
//! playoff-delta --season 2026 --team SF --rivals DAL,SEA \
//!   --scoreboards espn/2026 --previous espn/2025 > SF.json
//! ```
//!
//! Each directory holds one season's ESPN scoreboard responses, one `*.json`
//! per week (see `espn.rs` for the URL). Fetching them is the caller's job,
//! so this binary does no network I/O and the same files give the same
//! story, byte for byte.

mod espn;

use clap::Parser;
use playoff_delta::{
	evaluate::{evaluate, Subject, WAYS},
	league::Team,
	season::Season,
};
use std::{
	fmt::Write as _,
	io::{self, Write},
	path::{Path, PathBuf},
	process::ExitCode,
};

#[derive(Parser, Debug)]
#[command(author, version, about = "Emits a team's Playoff Delta story (the some-ui stage's JSON) from ESPN scoreboard files")]
struct Args {
	#[arg(long)]
	season: u16,
	/// The team the story is about, by contract code (`SF`).
	#[arg(long)]
	team: String,
	/// Named rivals, comma separated (`DAL,SEA`).
	#[arg(long, value_delimiter = ',')]
	rivals: Vec<String>,
	/// This season's scoreboard responses, one `*.json` per week.
	#[arg(long)]
	scoreboards: PathBuf,
	/// Last season's, complete: the benchmark.
	#[arg(long)]
	previous: PathBuf,
	/// Shown on screen; defaults to the season's year.
	#[arg(long)]
	label: Option<String>,
}

/// Games each team plays in an NFL regular season (since 2021).
const GAMES_PER_TEAM: usize = 17;

/// One season from its directory. A missing or empty week would leave every
/// team a game short and still look like a valid season, so the count is
/// checked against the league's, not only against itself.
fn season(dir: &Path, year: u16) -> Result<Season, Box<dyn std::error::Error>> {
	let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)?.map(|e| e.map(|e| e.path())).collect::<Result<_, _>>()?;
	paths.retain(|p| p.extension().is_some_and(|x| x == "json"));
	paths.sort();
	let bodies = paths.iter().map(std::fs::read_to_string).collect::<Result<Vec<_>, _>>()?;
	let season = espn::season_from_scoreboards(bodies.iter().map(String::as_str), year)?;
	if season.length() != GAMES_PER_TEAM {
		let mut message = String::new();
		write!(
			message,
			"{}: {year} has {} games a team, not {GAMES_PER_TEAM}; is a week missing?",
			dir.display(),
			season.length()
		)?;
		return Err(message.into());
	}
	Ok(season)
}

fn run(args: Args) -> Result<(), Box<dyn std::error::Error>> {
	let code = |c: &str| Team::from_code(c).ok_or_else(|| io::Error::other(String::from("unknown team code: ") + c));
	let subject = Subject {
		team: code(&args.team)?,
		rivals: args.rivals.iter().map(|c| code(c)).collect::<Result<_, _>>()?,
		label: args.label.unwrap_or_else(|| args.season.to_string()),
		ways: WAYS,
	};
	let current = season(&args.scoreboards, args.season)?;
	let previous = season(&args.previous, args.season - 1)?;
	// The benchmark reads last season at the same week, so every game of it
	// must be final, or later weeks repeat a stale claim.
	if previous.games().iter().any(|g| g.score.is_none()) {
		let mut message = String::new();
		write!(message, "{}: {} still has games without a final score", args.previous.display(), args.season - 1)?;
		return Err(message.into());
	}
	let story = evaluate(&current, &previous, &subject)?;
	let mut out = io::stdout().lock();
	serde_json::to_writer_pretty(&mut out, &story)?;
	out.write_all(b"\n")?;
	out.flush()?;
	Ok(())
}

fn main() -> ExitCode {
	match run(Args::parse()) {
		Ok(()) => ExitCode::SUCCESS,
		Err(e) => {
			// Nothing more to do if stderr itself is gone.
			let _ = writeln!(io::stderr().lock(), "playoff-delta: {e}");
			ExitCode::FAILURE
		}
	}
}

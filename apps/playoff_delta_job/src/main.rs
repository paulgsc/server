//! The weekly Playoff Delta job: one team's story for a season, as the JSON
//! `paulgsc/some-ui`'s `@some-ui/playoff-delta` stage reads, on stdout.
//!
//! ```sh
//! playoff-delta --season 2026 --team SF --scoreboards espn/2026 > SF.json
//! ```
//!
//! The directory holds the season's ESPN scoreboard responses, one `*.json`
//! per week (see `espn.rs` for the URL). Fetching them is the caller's job,
//! so this binary does no network I/O and the same files give the same
//! story, byte for byte.

mod espn;

use clap::Parser;
use playoff_delta::{
	evaluate::{evaluate, Subject, RESTS, WAYS},
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
	/// This season's scoreboard responses, one `*.json` per week.
	#[arg(long)]
	scoreboards: PathBuf,
	/// Accepted and ignored: contract v3 dropped the rivals and the
	/// benchmark, and paulgsc/testing's `playoff-delta.yml` still passes
	/// these. Delete them once it stops.
	#[arg(long, hide = true, value_delimiter = ',')]
	rivals: Vec<String>,
	#[arg(long, hide = true)]
	previous: Option<PathBuf>,
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
		label: args.label.unwrap_or_else(|| args.season.to_string()),
		ways: WAYS,
		rests: RESTS,
	};
	let story = evaluate(&season(&args.scoreboards, args.season)?, &subject)?;
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

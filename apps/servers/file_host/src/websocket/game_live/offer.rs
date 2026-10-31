//! What home has for the phone at one moment.
//!
//! Read from the game feed's record (`game_feed_repo`) and put through the
//! scenario engine. Reads only: nothing here writes, and nothing is kept of
//! what was offered.

use super::wire::{self, Hello, State};
use chrono::{DateTime, SecondsFormat, Utc};
use nfl_feed::scoreboard;
use scenario_engine::{Reading, Situation};
use sqlx::SqlitePool;

/// The team every game is read through (the epic's "read through one team").
pub const TEAM: &str = "SF";

/// The week, and the offer for the game `hello` names, as of `now`.
///
/// The week is the newest scoreboard kept by `now`; the game's status and
/// the snap to come are its newest summary's (fetched every 15 s while it is
/// live, where the scoreboard is every ten minutes), else the scoreboard's;
/// its log is its own (`game_feed_repo::game_log`), which the engine reads
/// only as far as `now` (SE2). A game the scoreboard names without both teams
/// and a kickoff cannot be offered, so it is not listed.
///
/// # Errors
/// Any storage failure.
pub async fn state(db: &SqlitePool, hello: &Hello, now: DateTime<Utc>, following: bool) -> Result<State, sqlx::Error> {
	let as_of = now.to_rfc3339_opts(SecondsFormat::Millis, true);
	let mut conn = db.acquire().await?;
	let week = game_feed_repo::week(&mut conn, &as_of).await?.unwrap_or_default();
	let listed: Vec<(&scoreboard::Game, wire::Game)> = week.games.iter().filter_map(|game| Some((game, listing(game)?))).collect();
	let picked = hello.game_id.as_deref().and_then(|id| listed.iter().find(|(game, _)| game.id == id));
	let offer = match picked {
		None => None,
		Some((game, listed)) => {
			let summary = game_feed_repo::summary(&mut conn, &game.id, &as_of).await?;
			let log = game_feed_repo::game_log(&mut conn, &game.id).await?;
			let (status, situation, weather) = match summary {
				Some(summary) => (summary.status, summary.situation, summary.weather.or_else(|| game.weather.clone())),
				None => (game.status.clone(), game.situation.clone(), game.weather.clone()),
			};
			let engine_game = scenario_engine::Game {
				id: game.id.clone(),
				home: listed.home.clone(),
				away: listed.away.clone(),
				kickoff: game.kickoff.unwrap_or(now),
				status,
				situation: situation.as_ref().map(Situation::from),
				weather,
				reading: Reading {
					team: TEAM.to_owned(),
					want: listed.want.clone(),
				},
			};
			let offered = scenario_engine::offer(&engine_game, &log, now.with_timezone(&hello.offset), hello.mode, hello.watching(&game.id));
			Some(wire::Offer::from_engine(offered, now))
		}
	};
	Ok(State {
		following,
		games: listed.into_iter().map(|(_, listed)| listed).collect(),
		offer,
	})
}

/// A scoreboard game as the picker lists it, or `None` without both teams
/// and a kickoff.
///
/// SF wants itself to win its own game. Which side it wants in another is
/// Playoff Delta's reading (`story::NextPiece::want`), which home does not
/// make per week yet, so any other game says none, and the engine reads it
/// as one that cannot move SF.
fn listing(game: &scoreboard::Game) -> Option<wire::Game> {
	let home = game.home.team.clone()?;
	let away = game.away.team.clone()?;
	let want = [&home, &away].into_iter().find(|team| *team == TEAM).cloned();
	Some(wire::Game {
		id: game.id.clone(),
		kickoff: wire::stamp(game.kickoff?),
		home,
		away,
		want,
	})
}

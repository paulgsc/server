//! What home has for the phone at one moment.
//!
//! Read from the game feed's record (`game_feed_repo`) and put through the
//! scenario engine. Reads only: nothing here writes, and nothing is kept of
//! what was offered.

use super::wire::{self, Hello, State};
use chrono::{DateTime, SecondsFormat, Utc};
use nfl_feed::{scoreboard, summary::Summary};
use scenario_engine::{Reading, Situation};
use sqlx::{SqliteConnection, SqlitePool};

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
/// With the feed off (`following` false) it is empty, whatever was kept
/// before.
///
/// # Errors
/// Any storage failure.
pub async fn state(db: &SqlitePool, hello: &Hello, now: DateTime<Utc>, following: bool) -> Result<State, sqlx::Error> {
	// With the feed off, what it kept before is stale: offer nothing from it.
	if !following {
		return Ok(State {
			following,
			games: Vec::new(),
			offer: None,
		});
	}
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
			let engine_game = engine_game(game, listed, summary, now);
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

/// The engine's view of `game_id` as of `at`, as [`state`] makes it.
///
/// The newest scoreboard kept by then lists the game, and its newest summary
/// kept by then, if any, gives the status and the snap to come. `None` when
/// that scoreboard does not list it with both teams and a kickoff.
///
/// The reconciler (GWB7) reads a game this way at a bite's time.
///
/// # Errors
/// Any storage failure.
pub async fn game_at(conn: &mut SqliteConnection, game_id: &str, at: DateTime<Utc>) -> Result<Option<scenario_engine::Game>, sqlx::Error> {
	let as_of = at.to_rfc3339_opts(SecondsFormat::Millis, true);
	let week = game_feed_repo::week(conn, &as_of).await?.unwrap_or_default();
	let Some((game, listed)) = week.games.iter().find(|game| game.id == game_id).and_then(|game| Some((game, listing(game)?))) else {
		return Ok(None);
	};
	let summary = game_feed_repo::summary(conn, game_id, &as_of).await?;
	Ok(Some(engine_game(game, &listed, summary, at)))
}

/// The engine's view of a listed game, from its newest summary when there is
/// one, else the scoreboard's.
fn engine_game(game: &scoreboard::Game, listed: &wire::Game, summary: Option<Summary>, now: DateTime<Utc>) -> scenario_engine::Game {
	let (status, situation, weather) = match summary {
		Some(summary) => (summary.status, summary.situation, summary.weather.or_else(|| game.weather.clone())),
		None => (game.status.clone(), game.situation.clone(), game.weather.clone()),
	};
	scenario_engine::Game {
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
	}
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

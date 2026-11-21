//! The reconciler (paulgsc/some-ui#2081, GWB7): each game bite matched to the
//! play it was about, and a scenario proposed for a bite said first checked
//! before home keeps it.
//!
//! The phone keeps only what it can be sure of: the scenario, the game, when
//! the owner started talking. A time alone cannot say which of the last few
//! plays was meant, and the TV runs behind the real game; "the last third and
//! long in this game at or before that time" can. So [`reconcile`] takes each
//! bite of a game, asks the scenario engine which plays that scenario's rule
//! says were one (`scenario_engine::plays_that_were`, the rule "just
//! happened" uses, SE3), and picks its play by when it was said
//! (`scenario_engine::its_play`). A bite said on an offer of the snap to come
//! (a third and long, a fourth down, a field goal try) is about that snap: the
//! first snap home had not seen when the offer was on its screen (at the
//! bite's start, or [`LAG`] before it if home had moved on since), if the
//! rule accepts it, within [`WINDOW`] either side; for a first possession or
//! a red-zone trip, the play that ends that drive (`next`, #3040). Any other is about the nearest at
//! or before the bite's start on home's clock, within [`WINDOW`] (`nearest`).
//! What it finds is a `bite_match` row (`voice_repo::bite_match`); a bite with
//! no such play is kept, marked `unmatched` (GW15). Nothing is ever edited: a
//! later pass adds a row, and only when the match changed.
//!
//! [`propose`] is what the shot writer (GWB9) calls for a bite said first:
//! the scenario must be in home's set and the engine, replayed at the bite's
//! time, must say it could have been happening then
//! (`scenario_engine::could_be`). Only then is it a `bite_mapping` row; a
//! refused proposal adds nothing, so the bite stays unmapped, never
//! mislabelled.
//!
//! Logs carry counts and kinds only: never a bite's strip, a play's text or
//! whose bite it was (docs/identity.md).

use crate::{websocket::game_live::offer::game_at, AppState};
use chrono::{DateTime, SecondsFormat, TimeDelta, Utc};
use game_scenarios::{Scenario, CATALOG_VERSION};
use scenario_engine::{about_a_play, could_be, its_play, offered_said, on_offer, Said, Watching, ENGINE_VERSION};
use sqlx::SqlitePool;
use std::{collections::HashMap, time::Duration};
use tracing::{error, info};
use voice_repo::{
	bite_match::{self, GameBite, Mapping, Match, Method},
	VoiceError,
};

/// How far from a bite the play it was about may be.
///
/// Before it, the broadcast's delay plus the time to start talking; after
/// it, for a bite on the snap to come, the time to the snap. About 90 s, to
/// be tuned on real Sundays (#2081).
pub const WINDOW: TimeDelta = TimeDelta::seconds(90);

/// How long after home the phone may hear of a play.
///
/// So how long an offer of the snap to come can still show once that snap
/// reached home: the socket's push plus the clock offset's error (GW5's 2 s).
/// A bite whose scenario home was no longer offering at its start, but was
/// this long before, is read as taken on that earlier offer.
pub const LAG: TimeDelta = TimeDelta::seconds(5);

/// What one pass over a game did, in counts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Reconciled {
	/// The game's bites.
	pub bites: usize,
	/// Of those, matched to a play, before or after it.
	pub matched: usize,
	/// Rows added: bites whose match changed or was new.
	pub added: usize,
}

/// How often the worker looks for games gone final. A bite is matched
/// within this of its game's final, or of reaching home after it.
pub const TICK: Duration = Duration::from_secs(600);

/// Start the worker. It always runs, as the feed does.
///
/// With the feed paused (`settings::GameFeed`) no new game goes final at
/// home, so a pass finds little, but a bite of a game home already kept final
/// is still matched (GWB8 also calls [`reconcile`] itself).
///
/// The worker is its own task, apart from the game feed's, so the feed still
/// reads and writes no person's data (`docs/game-feed.md`, GF4; call C160 on
/// paulgsc/server's GWB7 PR).
pub fn start(state: &AppState) {
	let cancel = state.core.cancel_token.clone();
	let db = state.core.shared_db.clone();
	tokio::spawn(async move {
		info!(tick_secs = TICK.as_secs(), "bite reconciler started");
		let mut done = Done::default();
		let mut ticker = tokio::time::interval(TICK);
		ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
		loop {
			tokio::select! {
				() = cancel.cancelled() => {
					info!("bite reconciler cancelled");
					return;
				}
				_ = ticker.tick() => match pass(&db, &mut done, chrono::Utc::now()).await {
					Ok(Pass { games: 0, failed: 0, .. }) => {}
					Ok(passed) => info!(games = passed.games, bites = passed.bites, matched = passed.matched, added = passed.added, failed = passed.failed, "bite reconciler pass"),
					Err(_) => error!(error = "storage", "bite reconciler pass failed"),
				},
			}
		}
	});
}

/// Each game the worker reconciled since it started, with its bite count
/// then.
#[derive(Debug, Clone, Default)]
pub struct Done(HashMap<String, i64>);

/// What one worker pass did, in counts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Pass {
	/// Games reconciled.
	pub games: usize,
	pub bites: usize,
	pub matched: usize,
	pub added: usize,
	/// Games whose reconcile failed: tried again next pass.
	pub failed: usize,
}

/// One worker pass.
///
/// The games looked at are those of the newest week home kept, and any game
/// with a bite no pass has matched since it was said or mapped (a bite that
/// reached home late, a game that left the newest scoreboard first, or a
/// mapping added after). Each whose newest summary
/// says it is final is reconciled, unless it already was with the same
/// [`bite_match::bite_state`] (its bites and their mappings). A restart
/// reconciles again, which adds nothing unchanged.
///
/// A game whose reconcile fails is counted in [`Pass::failed`] and tried
/// again next pass; it does not hold up the games after it.
///
/// # Errors
/// A storage failure while choosing the games.
pub async fn pass(db: &SqlitePool, done: &mut Done, now: DateTime<Utc>) -> Result<Pass, VoiceError> {
	let as_of = now.to_rfc3339_opts(SecondsFormat::Millis, true);
	let mut due = Vec::new();
	{
		let mut conn = db.acquire().await?;
		let week = game_feed_repo::week(&mut conn, &as_of).await?.unwrap_or_default();
		let mut games: Vec<String> = week.games.into_iter().map(|game| game.id).collect();
		for game in bite_match::games_with_bites_to_match(&mut conn).await? {
			if !games.contains(&game) {
				games.push(game);
			}
		}
		for game in games {
			if !game_feed_repo::holds_final(&mut conn, &game).await? {
				continue;
			}
			let state = bite_match::bite_state(&mut conn, &game).await?;
			if done.0.get(&game) != Some(&state) {
				due.push((game, state));
			}
		}
	}
	let mut passed = Pass::default();
	for (game, state) in due {
		let Ok(reconciled) = reconcile(db, &game, now).await else {
			passed.failed += 1;
			continue;
		};
		passed.games += 1;
		passed.bites += reconciled.bites;
		passed.matched += reconciled.matched;
		passed.added += reconciled.added;
		done.0.insert(game, state);
	}
	Ok(passed)
}

/// When `bite` started, on home's clock: the phone's time plus home's clock
/// minus the phone's, when the live connection measured it.
fn started_at_home(bite: &GameBite) -> Option<DateTime<Utc>> {
	let phone = DateTime::parse_from_rfc3339(&bite.bite.started).ok()?.with_timezone(&Utc);
	Some(phone + TimeDelta::milliseconds(bite.bite.offset.unwrap_or(0)))
}

/// Match every bite of `game` to its play, as of `now`, adding a
/// `bite_match` row for each bite whose match is new or changed.
///
/// Runs after the game's final, and again before the bundle (GWB8); a pass
/// before the final matches against the plays home has kept so far.
///
/// # Errors
/// Any storage failure. A pass that fails part way has added the rows it
/// added; the next pass carries on from them.
pub async fn reconcile(db: &SqlitePool, game: &str, now: DateTime<Utc>) -> Result<Reconciled, VoiceError> {
	let mut conn = db.acquire().await?;
	let bites = bite_match::bites_of_game(&mut conn, game).await?;
	let log = game_feed_repo::game_log(&mut conn, game).await?;
	let stamp = now.to_rfc3339_opts(SecondsFormat::Millis, true);
	let mut done = Reconciled {
		bites: bites.len(),
		..Reconciled::default()
	};
	for bite in &bites {
		let matched = match_one(&mut conn, game, bite, &log, now).await?;
		done.matched += usize::from(matches!(matched.method, Method::Nearest | Method::Next));
		// Read the newest and add in one write transaction, so two passes at
		// once (the worker's and GWB8's) cannot both add the same match.
		let mut tx = sqlx::Connection::begin_with(&mut *conn, "BEGIN IMMEDIATE").await?;
		let added = bite_match::add_match(&mut tx, &bite.subject, &bite.note_id, &matched, &stamp).await?;
		tx.commit().await?;
		done.added += usize::from(added);
	}
	Ok(done)
}

async fn match_one(conn: &mut sqlx::SqliteConnection, game_id: &str, bite: &GameBite, log: &[nfl_feed::log::Entry], now: DateTime<Utc>) -> Result<Match, VoiceError> {
	let engine_version = i64::from(ENGINE_VERSION);
	let unmatched = |scenario: Option<&str>, method| Match {
		scenario: scenario.map(str::to_owned),
		play: None,
		method,
		gap_seconds: None,
		engine_version,
	};
	let Some(named) = bite.scenario() else {
		return Ok(unmatched(None, Method::NoScenario));
	};
	// A scenario home's set does not hold (said under a newer set) has no
	// rule here to match it by: kept, unmatched.
	let (Some(scenario), Some(at)) = (Scenario::from_id(named), started_at_home(bite)) else {
		return Ok(unmatched(Some(named), Method::Unmatched));
	};
	if !about_a_play(scenario) {
		return Ok(unmatched(Some(named), Method::NotAPlay));
	}
	let Some(game) = game_at(conn, game_id, at).await? else {
		return Ok(unmatched(Some(named), Method::Unmatched));
	};
	// Only an offer can be of the snap to come. The offer the bite took was
	// on home's screen at its start, or, if the phone had not yet heard that
	// home moved on, up to LAG before; the engine, replayed then, says
	// whether it was of the snap to come (BM5).
	let (said, offered) = if bite.bite.how == voice_repo::How::Offered {
		let earlier = at - LAG;
		let then = match game_at(conn, game_id, earlier).await? {
			Some(before) if !on_offer(&game, log, at, scenario) && on_offer(&before, log, earlier, scenario) => (before, earlier),
			_ => (game.clone(), at),
		};
		(offered_said(&then.0, log, then.1, scenario), then.1)
	} else {
		(Said::After, at)
	};
	Ok(match its_play(&game, log, now, scenario, said, at, offered, WINDOW) {
		Some((was, gap)) => Match {
			scenario: Some(named.to_owned()),
			play: Some(was.play_id),
			method: match said {
				Said::BeforeTheSnap | Said::BeforeTheDrive => Method::Next,
				Said::After => Method::Nearest,
			},
			gap_seconds: Some(gap),
			engine_version,
		},
		None => unmatched(Some(named), Method::Unmatched),
	})
}

/// What became of a proposed scenario.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Proposal {
	/// Accepted: the bite's newest mapping.
	Mapped,
	/// Not a scenario of home's set.
	NotInSet,
	/// The engine, replayed at the bite's time, says it could not have been
	/// happening then.
	CouldNotHaveBeen,
	/// The bite was not said first, or the owner picked its scenario after
	/// speaking: only a bite left for later is mapped.
	NotLeftForLater,
	/// No bite of that game has that id.
	NoSuchBite,
}

/// Check a scenario the shot writer (GWB9) proposes for a bite said first,
/// and keep it as a `bite_mapping` row only if it passes.
///
/// It passes when the bite was said first and left for later, the scenario
/// is in home's set, and replaying the engine at the bite's time on home's
/// clock says it could have been happening then (`could_be`). The owner's
/// UTC offset is not kept with a bite, so the replay reads the day in UTC;
/// only Full mode's 8 AM pregame window depends on it.
///
/// # Errors
/// Any storage failure.
pub async fn propose(db: &SqlitePool, game: &str, subject: &str, note_id: &str, scenario_id: &str, now: DateTime<Utc>) -> Result<Proposal, VoiceError> {
	let mut conn = db.acquire().await?;
	let bites = bite_match::bites_of_game(&mut conn, game).await?;
	let Some(bite) = bites.iter().find(|bite| bite.subject == subject && bite.note_id == note_id) else {
		return Ok(Proposal::NoSuchBite);
	};
	if bite.bite.how != voice_repo::How::SaidFirst || bite.bite.scenario.is_some() {
		return Ok(Proposal::NotLeftForLater);
	}
	let Some(scenario) = Scenario::from_id(scenario_id) else {
		return Ok(Proposal::NotInSet);
	};
	let Some(at) = started_at_home(bite) else {
		return Ok(Proposal::CouldNotHaveBeen);
	};
	let Some(engine_game) = game_at(&mut conn, game, at).await? else {
		return Ok(Proposal::CouldNotHaveBeen);
	};
	let log = game_feed_repo::game_log(&mut conn, game).await?;
	let watching = if bite.bite.watching { Watching::Watched } else { Watching::Skipped };
	if !could_be(&engine_game, &log, at.fixed_offset(), watching).contains(&scenario) {
		return Ok(Proposal::CouldNotHaveBeen);
	}
	let mapping = Mapping {
		scenario: scenario.id().to_owned(),
		proposed_by: "shot-writer",
		catalog_version: i64::from(CATALOG_VERSION),
		engine_version: i64::from(ENGINE_VERSION),
	};
	bite_match::add_mapping(&mut conn, subject, note_id, &mapping, &now.to_rfc3339_opts(SecondsFormat::Millis, true)).await?;
	Ok(Proposal::Mapped)
}

#[cfg(test)]
mod tests {
	use super::{pass, propose, reconcile, Done, Pass, Proposal, Reconciled};
	use chrono::{DateTime, SecondsFormat, TimeDelta, Utc};
	use game_feed_repo::{Fetched, Response};
	use nfl_feed::summary::Play;
	use serde_json::{json, Value};
	use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};
	use voice_repo::bite_match::{newest_match, Method};

	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");
	const FINAL: &[u8] = include_bytes!("../../../../crates/nfl_feed/testdata/summary-401873010-final.json");
	const GAME: &str = "401873010";
	const SUBJECT: &str = "s";

	async fn pool() -> SqlitePool {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();
		pool
	}

	fn at(time: &str) -> DateTime<Utc> {
		DateTime::parse_from_rfc3339(time).unwrap().with_timezone(&Utc)
	}

	fn plays() -> Vec<Play> {
		nfl_feed::summary::parse(FINAL).unwrap().plays
	}

	fn wall(play: &Play) -> DateTime<Utc> {
		at(play.wallclock.as_deref().unwrap())
	}

	/// The week, with the recorded game in it (SF reads it wanting neither).
	fn board() -> Vec<u8> {
		let side = |home_away: &str, team: &str| json!({ "homeAway": home_away, "team": { "abbreviation": team }, "score": "0" });
		json!({
			"season": { "year": 2026, "type": 2 }, "week": { "number": 8 },
			"events": [{
				"id": GAME, "date": "2026-10-23T00:15Z", "shortName": "NE @ CHI",
				"status": { "type": { "state": "pre", "completed": false } },
				"competitions": [{ "competitors": [side("home", "CHI"), side("away", "NE")] }],
			}],
		})
		.to_string()
		.into_bytes()
	}

	/// The summary as ESPN served it live once `upto` had happened.
	fn live_through(upto: &Play) -> Vec<u8> {
		let sequence = upto.sequence.unwrap();
		let mut page: Value = serde_json::from_slice(FINAL).unwrap();
		for drive in page["drives"]["previous"].as_array_mut().unwrap() {
			drive["plays"].as_array_mut().unwrap().retain(|play| {
				play["sequenceNumber"]
					.as_str()
					.and_then(|n| n.parse::<i64>().ok())
					.or_else(|| play["sequenceNumber"].as_i64())
					.is_some_and(|n| n <= sequence)
			});
		}
		// The team with the ball for the snap after `upto`, as ESPN marks it.
		let all = plays();
		let next = all.iter().skip_while(|p| p.id != upto.id).nth(1).and_then(|p| p.possession.clone());
		let competition = &mut page["header"]["competitions"][0];
		competition["status"]["type"]["state"] = json!("in");
		competition["status"]["type"]["completed"] = json!(false);
		for side in competition["competitors"].as_array_mut().unwrap() {
			let team = side["team"]["abbreviation"].as_str().map(str::to_owned);
			side["possession"] = json!(team.is_some() && team == next);
		}
		page.to_string().into_bytes()
	}

	async fn keep(db: &SqlitePool, fetched: Fetched<'_>, body: &[u8], fetched_at: DateTime<Utc>) {
		let fetched_at = fetched_at.to_rfc3339_opts(SecondsFormat::Millis, true);
		let mut tx = db.begin_with("BEGIN IMMEDIATE").await.unwrap();
		let response = Response {
			fetched,
			url: "https://site.api.espn.com/apis/site/v2/sports/football/nfl/",
			fetched_at: &fetched_at,
			status: 200,
			body,
		};
		game_feed_repo::keep(&mut tx, &response).await.unwrap();
		tx.commit().await.unwrap();
	}

	/// The recorded game kept as home would have: the week before kickoff,
	/// the summary 5 s after `third_long`, and the final.
	async fn kept_game(db: &SqlitePool, third_long: &Play) {
		let plays = plays();
		keep(db, Fetched::Scoreboard, &board(), at("2026-10-22T23:00:00Z")).await;
		keep(db, Fetched::Summary(GAME), &live_through(third_long), wall(third_long) + TimeDelta::seconds(5)).await;
		keep(db, Fetched::Summary(GAME), FINAL, wall(plays.last().unwrap()) + TimeDelta::minutes(5)).await;
	}

	/// A bite said `how` about `scenario`, started at `started` on the phone's
	/// clock, with home's clock `offset_ms` ahead of it.
	async fn bite(db: &SqlitePool, note: &str, scenario: Option<&str>, how: &str, started: DateTime<Utc>, offset_ms: Option<i64>) {
		sqlx::query(
			"INSERT INTO voice_note (subject_id, note_id, of_kind, of_ref, recorded_at, content_type, size, sha256, stored_at)
			 VALUES (?1, ?2, 'game', ?3, ?2, 'audio/webm', 1, 's', 'now')",
		)
		.bind(SUBJECT)
		.bind(note)
		.bind(GAME)
		.execute(db)
		.await
		.unwrap();
		sqlx::query(
			"INSERT INTO game_bite (subject_id, note_id, scenario_id, how, live_strip, watching, started_at, clock_offset_ms, mode, catalog_version)
			 VALUES (?1, ?2, ?3, ?4, '', 1, ?5, ?6, 'standard', 1)",
		)
		.bind(SUBJECT)
		.bind(note)
		.bind(scenario)
		.bind(how)
		.bind(started.to_rfc3339_opts(SecondsFormat::Millis, true))
		.bind(offset_ms)
		.execute(db)
		.await
		.unwrap();
	}

	fn third_long(plays: &[Play]) -> Play {
		plays
			.iter()
			.find(|p| p.down == Some(3) && p.distance.is_some_and(|d| d >= 7) && p.play_type.as_deref() == Some("Pass Incompletion"))
			.unwrap()
			.clone()
	}

	async fn matched(db: &SqlitePool, note: &str) -> (Method, Option<String>, Option<i64>) {
		let mut conn = db.acquire().await.unwrap();
		let held = newest_match(&mut conn, SUBJECT, note).await.unwrap().unwrap();
		(held.method, held.play, held.gap_seconds)
	}

	#[tokio::test]
	async fn each_bite_is_matched_to_its_play_or_marked_and_kept() {
		let db = pool().await;
		let plays = plays();
		let play = third_long(&plays);
		kept_game(&db, &play).await;
		let t = wall(&play);
		bite(&db, "on-time", Some("in.third-long"), "just-happened", t + TimeDelta::seconds(30), Some(0)).await;
		// The phone's clock a minute fast: home's is 60 s behind it.
		bite(&db, "fast-phone", Some("in.third-long"), "just-happened", t + TimeDelta::seconds(90), Some(-60_000)).await;
		bite(&db, "too-late", Some("in.third-long"), "just-happened", t + TimeDelta::minutes(30), None).await;
		bite(&db, "pregame", Some("pre.weather"), "offered", at("2026-10-22T23:30:00Z"), None).await;
		bite(&db, "left-for-later", None, "said-first", t + TimeDelta::seconds(10), Some(0)).await;
		bite(&db, "newer-set", Some("in.something-new"), "every-scenario", t, None).await;

		let now = wall(plays.last().unwrap()) + TimeDelta::minutes(10);
		let done = reconcile(&db, GAME, now).await.unwrap();
		assert_eq!(done, Reconciled { bites: 6, matched: 2, added: 6 });
		assert_eq!(matched(&db, "on-time").await, (Method::Nearest, Some(play.id.clone()), Some(30)));
		assert_eq!(matched(&db, "fast-phone").await, (Method::Nearest, Some(play.id.clone()), Some(30)));
		assert_eq!(matched(&db, "pregame").await, (Method::NotAPlay, None, None));
		assert_eq!(matched(&db, "left-for-later").await, (Method::NoScenario, None, None));
		assert_eq!(matched(&db, "newer-set").await, (Method::Unmatched, None, None));
		let (late, ..) = matched(&db, "too-late").await;
		assert_eq!(late, Method::Unmatched, "no third and long within the window: kept, marked");

		let again = reconcile(&db, GAME, now + TimeDelta::hours(1)).await.unwrap();
		assert_eq!(again.added, 0, "a pass that changes nothing adds nothing");
	}

	/// BM5 (#3040): a bite said on the offer of a third and long, before its
	/// snap, is matched to that snap, the gap negative; a bite on the same
	/// play said after it, from "just happened", still looks back to it.
	#[tokio::test]
	async fn a_bite_on_the_snap_to_come_is_matched_to_that_snap() {
		let db = pool().await;
		let plays = plays();
		let play = third_long(&plays);
		let before = &plays[plays.iter().position(|p| p.id == play.id).unwrap() - 1];
		// Home had the play before it when the owner started talking.
		keep(&db, Fetched::Summary(GAME), &live_through(before), wall(before) + TimeDelta::seconds(5)).await;
		kept_game(&db, &play).await;
		let t = wall(&play);
		let ahead = t - TimeDelta::seconds(20);
		assert!(ahead > wall(before) + TimeDelta::seconds(5), "the bite starts after home saw the play before");
		bite(&db, "before-the-snap", Some("in.third-long"), "offered", ahead, Some(0)).await;
		bite(&db, "after-it", Some("in.third-long"), "just-happened", t + TimeDelta::seconds(15), Some(0)).await;
		// Picked from the whole set before the snap: not tied to an offer, so
		// it looks back, and finds no third and long before it.
		bite(&db, "from-the-set", Some("in.third-long"), "every-scenario", ahead, Some(0)).await;

		let now = wall(plays.last().unwrap()) + TimeDelta::minutes(10);
		let done = reconcile(&db, GAME, now).await.unwrap();
		assert_eq!((done.bites, done.matched), (3, 2));
		assert_eq!(matched(&db, "before-the-snap").await, (Method::Next, Some(play.id.clone()), Some(-20)));
		assert_eq!(matched(&db, "after-it").await, (Method::Nearest, Some(play.id.clone()), Some(15)));
		assert_eq!(matched(&db, "from-the-set").await.0, Method::Unmatched);
	}

	/// BM5: the phone heard of the snap a moment after home did, so the owner
	/// took the third and long offer 2 s after home had moved on. The offer was
	/// on home's screen LAG before the bite, so the bite is about that snap.
	#[tokio::test]
	async fn a_bite_on_an_offer_home_just_moved_on_from_is_about_its_snap() {
		let db = pool().await;
		let plays = plays();
		let play = third_long(&plays);
		let before = &plays[plays.iter().position(|p| p.id == play.id).unwrap() - 1];
		keep(&db, Fetched::Summary(GAME), &live_through(before), wall(before) + TimeDelta::seconds(5)).await;
		kept_game(&db, &play).await;
		let t = wall(&play);
		// Home saw the snap at t + 5 s.
		bite(&db, "phone-behind", Some("in.third-long"), "offered", t + TimeDelta::seconds(7), Some(0)).await;
		// Long after home moved on: no offer of that snap was showing.
		bite(&db, "stale", Some("in.third-long"), "offered", t + TimeDelta::seconds(40), Some(0)).await;
		let now = wall(plays.last().unwrap()) + TimeDelta::minutes(10);
		reconcile(&db, GAME, now).await.unwrap();
		assert_eq!(matched(&db, "phone-behind").await, (Method::Next, Some(play.id.clone()), Some(7)));
		assert_eq!(matched(&db, "stale").await.0, Method::Unmatched);
	}

	#[tokio::test]
	async fn a_proposal_is_kept_only_if_it_could_have_been_happening() {
		let db = pool().await;
		let plays = plays();
		let play = third_long(&plays);
		kept_game(&db, &play).await;
		let t = wall(&play);
		bite(&db, "later", None, "said-first", t + TimeDelta::seconds(20), Some(0)).await;
		bite(&db, "picked", Some("in.sack"), "said-first", t + TimeDelta::seconds(20), Some(0)).await;
		let now = wall(plays.last().unwrap()) + TimeDelta::minutes(10);
		let ask = |note: &'static str, scenario: &'static str| propose(&db, GAME, SUBJECT, note, scenario, now);

		assert_eq!(ask("later", "in.made-up").await.unwrap(), Proposal::NotInSet);
		assert_eq!(ask("later", "post.result").await.unwrap(), Proposal::CouldNotHaveBeen);
		assert_eq!(ask("picked", "in.third-long").await.unwrap(), Proposal::NotLeftForLater);
		assert_eq!(ask("nobody", "in.third-long").await.unwrap(), Proposal::NoSuchBite);
		let mappings = || sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM bite_mapping").fetch_one(&db);
		assert_eq!(mappings().await.unwrap(), 0, "a refused proposal adds nothing");

		assert_eq!(ask("later", "in.third-long").await.unwrap(), Proposal::Mapped);
		assert_eq!(mappings().await.unwrap(), 1);
		reconcile(&db, GAME, now).await.unwrap();
		assert_eq!(matched(&db, "later").await, (Method::Nearest, Some(play.id.clone()), Some(20)), "matched under its mapping");
	}

	#[tokio::test]
	async fn the_worker_reconciles_a_final_game_again_only_when_it_has_new_bites() {
		let db = pool().await;
		let plays = plays();
		let play = third_long(&plays);
		let t = wall(&play);
		keep(&db, Fetched::Scoreboard, &board(), at("2026-10-22T23:00:00Z")).await;
		keep(&db, Fetched::Summary(GAME), &live_through(&play), t + TimeDelta::seconds(5)).await;
		bite(&db, "first", Some("in.third-long"), "just-happened", t + TimeDelta::seconds(30), Some(0)).await;
		let mut done = Done::default();
		assert_eq!(pass(&db, &mut done, t + TimeDelta::minutes(1)).await.unwrap(), Pass::default(), "not final yet");

		let end = wall(plays.last().unwrap()) + TimeDelta::minutes(5);
		keep(&db, Fetched::Summary(GAME), FINAL, end).await;
		let first = pass(&db, &mut done, end).await.unwrap();
		assert_eq!(
			first,
			Pass {
				games: 1,
				bites: 1,
				matched: 1,
				added: 1,
				failed: 0
			}
		);
		assert_eq!(pass(&db, &mut done, end + TimeDelta::minutes(10)).await.unwrap(), Pass::default(), "nothing new");

		bite(&db, "after", Some("post.result"), "offered", end + TimeDelta::minutes(2), None).await;
		let again = pass(&db, &mut done, end + TimeDelta::minutes(20)).await.unwrap();
		assert_eq!(
			again,
			Pass {
				games: 1,
				bites: 2,
				matched: 1,
				added: 1,
				failed: 0
			},
			"only the new bite adds a row"
		);
		assert_eq!(matched(&db, "after").await, (Method::NotAPlay, None, None));
	}

	#[tokio::test]
	async fn a_late_bite_or_a_new_mapping_is_matched_after_the_week_moved_on() {
		let db = pool().await;
		let plays = plays();
		let play = third_long(&plays);
		kept_game(&db, &play).await;
		let t = wall(&play);
		let end = wall(plays.last().unwrap()) + TimeDelta::minutes(5);
		let mut done = Done::default();
		assert_eq!(pass(&db, &mut done, end).await.unwrap().bites, 0, "no bites yet");

		// The next week's scoreboard no longer lists the game.
		let next_week = json!({ "season": { "year": 2026, "type": 2 }, "week": { "number": 9 }, "events": [] }).to_string();
		keep(&db, Fetched::Scoreboard, next_week.as_bytes(), end + TimeDelta::days(3)).await;
		bite(&db, "uploaded-late", None, "said-first", t + TimeDelta::seconds(20), Some(0)).await;
		let later = end + TimeDelta::days(3) + TimeDelta::minutes(1);
		let first = pass(&db, &mut done, later).await.unwrap();
		assert_eq!((first.games, first.added), (1, 1), "found by its unmatched bite");
		assert_eq!(matched(&db, "uploaded-late").await.0, Method::NoScenario);
		assert_eq!(pass(&db, &mut done, later).await.unwrap(), Pass::default());

		assert_eq!(propose(&db, GAME, SUBJECT, "uploaded-late", "in.third-long", later).await.unwrap(), Proposal::Mapped);
		// The week moved on, and the mapping still brings the game back.
		let after = pass(&db, &mut done, later).await.unwrap();
		assert_eq!((after.games, after.added), (1, 1));
		assert_eq!(pass(&db, &mut done, later).await.unwrap(), Pass::default(), "and only once");
		assert_eq!(matched(&db, "uploaded-late").await, (Method::Nearest, Some(play.id.clone()), Some(20)));
	}

	#[tokio::test]
	async fn a_new_mapping_brings_a_game_of_the_week_back() {
		let db = pool().await;
		let plays = plays();
		let play = third_long(&plays);
		kept_game(&db, &play).await;
		bite(&db, "later", None, "said-first", wall(&play) + TimeDelta::seconds(20), Some(0)).await;
		let end = wall(plays.last().unwrap()) + TimeDelta::minutes(5);
		let mut done = Done::default();
		assert_eq!(pass(&db, &mut done, end).await.unwrap().added, 1);
		assert_eq!(propose(&db, GAME, SUBJECT, "later", "in.third-long", end).await.unwrap(), Proposal::Mapped);
		let again = pass(&db, &mut done, end).await.unwrap();
		assert_eq!((again.games, again.added), (1, 1), "the mapping changed the game's state");
		assert_eq!(matched(&db, "later").await.0, Method::Nearest);
	}
}

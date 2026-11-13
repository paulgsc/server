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
//! happened" uses, SE3), and keeps the nearest at or before the bite's start
//! on home's clock, within [`WINDOW`]. What it finds is a `bite_match` row
//! (`voice_repo::bite_match`); a bite with no such play is kept, marked
//! `unmatched` (GW15). Nothing is ever edited: a later pass adds a row, and
//! only when the match changed.
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

use crate::websocket::game_live::offer::game_at;
use chrono::{DateTime, SecondsFormat, TimeDelta, Utc};
use game_scenarios::{Scenario, CATALOG_VERSION};
use scenario_engine::{about_a_play, could_be, plays_that_were, Was, Watching, ENGINE_VERSION};
use sqlx::SqlitePool;
use voice_repo::{
	bite_match::{self, GameBite, Mapping, Match, Method},
	VoiceError,
};

/// How far before a bite the play it was about may be: the broadcast's delay
/// plus the time to start talking. About 90 s, to be tuned on real Sundays
/// (#2081).
pub const WINDOW: TimeDelta = TimeDelta::seconds(90);

/// What one pass over a game did, in counts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Reconciled {
	/// The game's bites.
	pub bites: usize,
	/// Of those, matched to a play.
	pub matched: usize,
	/// Rows added: bites whose match changed or was new.
	pub added: usize,
}

/// When `bite` started, on home's clock: the phone's time plus home's clock
/// minus the phone's, when the live connection measured it.
fn started_at_home(bite: &GameBite) -> Option<DateTime<Utc>> {
	let phone = DateTime::parse_from_rfc3339(&bite.bite.started).ok()?.with_timezone(&Utc);
	Some(phone + TimeDelta::milliseconds(bite.bite.offset.unwrap_or(0)))
}

/// The nearest of `were` at or before `at`, within [`WINDOW`], and how many
/// whole seconds before.
#[must_use]
pub fn nearest(at: DateTime<Utc>, were: &[Was]) -> Option<(&Was, i64)> {
	were
		.iter()
		.filter(|was| was.at <= at && at - was.at <= WINDOW)
		.max_by_key(|was| was.at)
		.map(|was| (was, (at - was.at).num_seconds()))
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
		done.matched += usize::from(matched.method == Method::Nearest);
		done.added += usize::from(bite_match::add_match(&mut conn, &bite.subject, &bite.note_id, &matched, &stamp).await?);
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
	let were = plays_that_were(&game, log, now, scenario);
	Ok(match nearest(at, &were) {
		Some((was, gap)) => Match {
			scenario: Some(named.to_owned()),
			play: Some(was.play_id.clone()),
			method: Method::Nearest,
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
	use super::{nearest, propose, reconcile, Proposal, Reconciled, WINDOW};
	use chrono::{DateTime, SecondsFormat, TimeDelta, Utc};
	use game_feed_repo::{Fetched, Response};
	use nfl_feed::summary::Play;
	use scenario_engine::Was;
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
		let competition = &mut page["header"]["competitions"][0];
		competition["status"]["type"]["state"] = json!("in");
		competition["status"]["type"]["completed"] = json!(false);
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

	#[test]
	fn the_nearest_play_at_or_before_within_the_window() {
		let t = at("2026-10-23T01:00:00Z");
		let was = |id: &str, s: i64| Was {
			play_id: id.to_owned(),
			at: t - TimeDelta::seconds(s),
		};
		let were = [was("far", 200), was("old", 80), was("near", 20), was("after", -5)];
		let (picked, gap) = nearest(t, &were).unwrap();
		assert_eq!((picked.play_id.as_str(), gap), ("near", 20));
		assert!(nearest(t, &[was("far", 200), was("after", -5)]).is_none(), "outside the window, or after the bite");
		assert_eq!(nearest(t, &[was("edge", WINDOW.num_seconds())]).map(|(_, gap)| gap), Some(90));
	}

	#[tokio::test]
	async fn each_bite_is_matched_to_its_play_or_marked_and_kept() {
		let db = pool().await;
		let plays = plays();
		let play = third_long(&plays);
		kept_game(&db, &play).await;
		let t = wall(&play);
		bite(&db, "on-time", Some("in.third-long"), "offered", t + TimeDelta::seconds(30), Some(0)).await;
		// The phone's clock a minute fast: home's is 60 s behind it.
		bite(&db, "fast-phone", Some("in.third-long"), "offered", t + TimeDelta::seconds(90), Some(-60_000)).await;
		bite(&db, "too-late", Some("in.third-long"), "offered", t + TimeDelta::minutes(30), None).await;
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
		assert!(matches!(late, Method::Nearest | Method::Unmatched), "kept either way");

		let again = reconcile(&db, GAME, now + TimeDelta::hours(1)).await.unwrap();
		assert_eq!(again.added, 0, "a pass that changes nothing adds nothing");
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
}

//! A real game replayed through the engine, moment by moment (GWB3's "tests
//! replay recorded games"): before every snap, and 20 s after every play home
//! saw, the offer the phone would have shown in Standard mode.
//!
//! Two measures:
//!
//! - **On offer before the snap,** at least 7 times in 10 (the beta's bar).
//!   Every snap whose own fields make it a moment (3rd and 7+, 4th down,
//!   inside the 20, the last two minutes of a half) has that moment's scenario
//!   among the cards, and the most specific of them first.
//! - **Just happened,** at least 8 times in 10. Every play ESPN typed as an
//!   event (a sack, a flag, an interception, a field goal, a touchdown, a
//!   punt, an injury, a 3rd and long) shows up as its scenario.
//!
//! The whole replay is also written out, one line per moment, in
//! `testdata/replay-401873010.txt`, so a rule change shows in its PR as the
//! lines it changed on a real game. Rebuild it after a deliberate change with
//! `BLESS_REPLAY=1 cargo test -p scenario_engine --test replay`.

#[cfg(test)]
mod common;

use chrono::TimeDelta;
use common::{first_seen, game, is_snap, pacific, plays, recorded_log, status, wall};
use game_scenarios::Scenario;
use nfl_feed::{summary::Play, GameState};
use scenario_engine::{offer, Mode, Offer, Situation, Watching};

const TRANSCRIPT: &str = "testdata/replay-401873010.txt";

/// The scenarios a snap's own fields make it, most specific first.
#[cfg(test)]
fn moments(play: &Play) -> Vec<Scenario> {
	let mut out = Vec::new();
	let (down, distance) = (play.down.unwrap_or(0), play.distance.unwrap_or(0));
	if down == 4 {
		out.push(Scenario::FourthDown);
	}
	if down == 3 && distance >= 7 {
		out.push(Scenario::ThirdAndLong);
	}
	if play.yards_to_endzone.is_some_and(|y| y <= 20) {
		out.push(Scenario::RedZone);
	}
	let seconds = play
		.clock
		.as_deref()
		.and_then(|c| c.split_once(':'))
		.map(|(m, s)| m.parse::<i64>().unwrap() * 60 + s.parse::<i64>().unwrap());
	if matches!(play.period, Some(2 | 4)) && seconds.is_some_and(|s| (1..=120).contains(&s)) {
		out.push(Scenario::TwoMinute);
	}
	out
}

/// The scenario ESPN's own typing of a play says it was.
#[cfg(test)]
fn event(play: &Play) -> Option<Scenario> {
	let kind = play.play_type.as_deref().unwrap_or_default();
	let text = play.text.as_deref().unwrap_or_default();
	Some(match kind {
		"Sack" => Scenario::Sack,
		"Penalty" => Scenario::Penalty,
		"Pass Interception Return" => Scenario::Turnover,
		"Field Goal Good" | "Field Goal Missed" => Scenario::Kick,
		"Passing Touchdown" | "Rushing Touchdown" => Scenario::TwoPoint,
		"Punt" => Scenario::FourthDown,
		_ if text.contains("was injured") => Scenario::Injury,
		_ if is_snap(play) && play.down == Some(3) && play.distance.is_some_and(|d| d >= 7) => Scenario::ThirdAndLong,
		_ => return None,
	})
}

#[cfg(test)]
const fn name(scenario: Scenario) -> &'static str {
	scenario.entry().name
}

#[cfg(test)]
fn describe(offer: &Offer) -> String {
	let mut line = String::new();
	for part in [
		Some(&offer.strip.clock),
		offer.strip.possession.as_ref(),
		offer.strip.down.as_ref(),
		offer.strip.score.as_ref(),
	]
	.into_iter()
	.flatten()
	{
		line.push_str(part);
		line.push_str(" | ");
	}
	line.push_str("now:");
	for o in &offer.scenarios {
		line.push_str(&[" [", name(o.scenario), ": ", &o.line, "]"].concat());
	}
	line.push_str(" | just:");
	for h in &offer.just_happened {
		line.push_str(&[" [", name(h.scenario), ": ", &h.line, ", ", &h.ago, "]"].concat());
	}
	line
}

#[cfg(test)]
fn share(hits: usize, of: usize) -> String {
	[hits.to_string(), String::from(" of "), of.to_string()].concat()
}

#[test]
fn a_recorded_game_offers_the_moment_7_times_in_10() {
	let plays = plays();
	let log = recorded_log(&plays);
	let want = Some("CHI");
	let mut transcript = Vec::new();
	let (mut moments_seen, mut on_offer, mut first, mut just_seen, mut just_hit) = (0, 0, 0, 0, 0);
	for (k, play) in plays.iter().enumerate() {
		if is_snap(play) {
			let now = wall(play) - TimeDelta::seconds(10);
			let g = game(want, status(GameState::In, Some(play)), Some(Situation::before(play)));
			let offered = offer(&g, &log, pacific(now), Mode::Standard, Watching::Unknown);
			let expected = moments(play);
			if !expected.is_empty() {
				moments_seen += 1;
				on_offer += usize::from(expected.iter().all(|e| offered.scenarios.iter().any(|o| o.scenario == *e)));
				first += usize::from(offered.scenarios.first().map(|o| o.scenario) == Some(expected[0]));
			}
			transcript.push(["before ", &play.id, " | ", &describe(&offered)].concat());
		}
		if let Some(expected) = event(play) {
			let now = first_seen(&log, play) + TimeDelta::seconds(20);
			let next = plays[k + 1..].iter().find(|p| is_snap(p));
			let state = if plays[..=k].iter().any(|p| p.play_type.as_deref() == Some("End of Game")) {
				GameState::Post
			} else {
				GameState::In
			};
			let g = game(want, status(state, Some(play)), next.map(Situation::before));
			let offered = offer(&g, &log, pacific(now), Mode::Standard, Watching::Unknown);
			just_seen += 1;
			just_hit += usize::from(offered.just_happened.iter().any(|h| h.play_id == play.id && h.scenario == expected));
			transcript.push(["after  ", &play.id, " | ", &describe(&offered)].concat());
		}
	}
	let end = wall(plays.last().unwrap()) + TimeDelta::minutes(5);
	let g = game(want, status(GameState::Post, plays.last()), None);
	transcript.push(["final        | ", &describe(&offer(&g, &log, pacific(end), Mode::Standard, Watching::Watched))].concat());

	let header = [
		String::from("# NE at CHI, 2026-10-22 (401873010), read through SF wanting CHI, Standard mode."),
		[
			"# Moments on offer before the snap: ",
			&share(on_offer, moments_seen),
			"; the most specific first: ",
			&share(first, moments_seen),
			".",
		]
		.concat(),
		["# Events in just happened 20 s after home saw them: ", &share(just_hit, just_seen), "."].concat(),
	];
	let text = [header.join("\n"), transcript.join("\n"), String::new()].join("\n");
	let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(TRANSCRIPT);
	if option_env!("BLESS_REPLAY").is_some() {
		std::fs::write(&path, &text).unwrap();
	}
	let kept = std::fs::read_to_string(&path).unwrap();
	assert!(
		kept == text,
		"the replay differs from {TRANSCRIPT}: rebuild it with `BLESS_REPLAY=1 cargo test -p scenario_engine --test replay` and read the diff"
	);

	assert!(moments_seen >= 50, "the game has {moments_seen} moments");
	assert!(on_offer * 10 >= moments_seen * 7, "on offer {}", share(on_offer, moments_seen));
	assert!(first * 10 >= moments_seen * 7, "first {}", share(first, moments_seen));
	assert!(just_hit * 10 >= just_seen * 8, "just happened {}", share(just_hit, just_seen));
}

/// No line says what the machinery calls things.
#[test]
fn every_line_is_in_the_persons_terms() {
	let plays = plays();
	let log = recorded_log(&plays);
	for mode in [Mode::Light, Mode::Standard, Mode::Full] {
		for (k, play) in plays.iter().enumerate() {
			let next = plays.get(k + 1).map(Situation::before);
			let g = game(Some("NE"), status(GameState::In, Some(play)), next);
			let offered = offer(&g, &log, pacific(wall(play) + TimeDelta::seconds(30)), mode, Watching::Unknown);
			let lines = offered
				.scenarios
				.iter()
				.map(|o| &o.line)
				.chain(offered.just_happened.iter().flat_map(|h| [&h.line, &h.ago]));
			for line in lines.chain([&offered.strip.clock]) {
				assert!(!line.is_empty());
				for word in ["None", "Some(", "null", "_", "Pass Reception", "Rush", "yardsToEndzone", "401873010"] {
					assert!(!line.contains(word), "{line:?} says {word:?}");
				}
			}
		}
	}
}

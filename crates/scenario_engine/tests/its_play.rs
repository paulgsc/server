//! The play a bite is about, in both directions (#3040): a bite said on an
//! offer of the snap to come is about that snap, never the play before it;
//! one said after its play is about the nearest before it.
//!
//! Promise BM5's family is when a bite is said relative to its snap, by the
//! moment's rule: the test replays every snap of the recorded game, takes
//! every scenario offered before it in Full mode, and every try offered after
//! a touchdown, and checks the play each would be matched to.

#[cfg(test)]
mod common;

use chrono::{DateTime, SecondsFormat, TimeDelta, Utc};
use common::{first_seen, game, is_snap, pacific, plays, recorded_log, status, wall, CADENCE, ESPN_DELAY, GAME};
use game_scenarios::Scenario;
use nfl_feed::{
	log::{Entry, PlayLog},
	summary::Play,
	GameState,
};
use scenario_engine::{its_play, offer, offered_said, plays_that_were, Mode, Said, Situation, Was, Watching};

/// A window wide enough that only the direction decides.
const ANY: TimeDelta = TimeDelta::days(1);
/// How long after home the phone hears of a play.
const LAG: TimeDelta = TimeDelta::seconds(5);

#[cfg(test)]
fn was(id: &str, at: DateTime<Utc>, first: DateTime<Utc>) -> Was {
	Was {
		play_id: id.to_owned(),
		at,
		first,
	}
}

#[cfg(test)]
fn position(plays: &[Play], id: &str) -> usize {
	plays.iter().position(|p| p.id == id).unwrap()
}

/// BM5: before every snap, each scenario offered on it is said before its
/// play, and its play is that snap whenever the scenario's rule accepts it,
/// and never a play before the snap. The rule before #3040 (the nearest at
/// or before) matched none of those to that snap.
#[test]
fn a_bite_on_the_snap_to_come_is_about_that_snap() {
	let plays = plays();
	let log = recorded_log(&plays);
	let want = Some("CHI");
	let end = wall(plays.last().unwrap()) + TimeDelta::minutes(5);
	let over = game(want, status(GameState::Post, plays.last()), None);
	let were = |scenario: Scenario| plays_that_were(&over, &log, end, scenario);
	let (mut on_the_snap, mut before_fix) = (0, 0);
	for (k, play) in plays.iter().enumerate().filter(|(_, p)| is_snap(p)) {
		let bite = wall(play) - TimeDelta::seconds(10);
		let then = game(want, status(GameState::In, Some(play)), Some(Situation::before(play)));
		for offered in offer(&then, &log, pacific(bite), Mode::Full, Watching::Unknown).scenarios {
			let scenario = offered.scenario;
			let said = offered_said(&then, &log, bite, scenario);
			let were = were(scenario);
			let picked = its_play(said, bite, &were, ANY, LAG).map(|(w, _)| position(&plays, &w.play_id));
			match said {
				Said::Before => {
					assert!(picked.is_none_or(|i| i >= k), "{scenario:?} before {} matched a play before it", play.id);
					if were.iter().any(|w| w.play_id == play.id) {
						assert_eq!(picked, Some(k), "{scenario:?} before {}", play.id);
						on_the_snap += 1;
						before_fix += usize::from(its_play(Said::After, bite, &were, ANY, LAG).map(|(w, _)| w.play_id.as_str()) == Some(play.id.as_str()));
					}
				}
				Said::After => assert!(picked.is_none_or(|i| i < k), "{scenario:?} before {} is about a play in the log", play.id),
			}
		}
	}
	assert!(on_the_snap >= 20, "{on_the_snap} offers named a snap their rule accepts");
	assert_eq!(before_fix, 0, "looking back never finds the snap to come");
}

/// The log as [`recorded_log`] keeps it, but each touchdown served first
/// without its try, as ESPN does: its text ends at the touchdown for the
/// first 30 s, and the try is written into the same play after.
#[cfg(test)]
fn log_with_tries_to_come(plays: &[Play]) -> Vec<Entry> {
	let mut log = PlayLog::default();
	let end = wall(plays.last().unwrap()) + TimeDelta::minutes(1);
	let mut fetched = wall(&plays[0]) - TimeDelta::minutes(1);
	let mut response = 0;
	while fetched <= end {
		let held: Vec<Play> = plays
			.iter()
			.filter(|p| wall(p) + ESPN_DELAY <= fetched)
			.map(|p| {
				let mut p = p.clone();
				if fetched < wall(&p) + TimeDelta::seconds(35) {
					if let Some(text) = p.text.as_deref().and_then(|t| t.find("TOUCHDOWN.").map(|at| t[..at + "TOUCHDOWN.".len()].to_owned())) {
						p.text = Some(text);
					}
				}
				p
			})
			.collect();
		log.apply(GAME, response, &fetched.to_rfc3339_opts(SecondsFormat::Secs, true), &held);
		fetched += CADENCE;
		response += 1;
	}
	log.entries().to_vec()
}

/// BM5: after a touchdown, the try (and the kick, while the try is to come)
/// is offered on the touchdown's own play, which is in the log: said after
/// it, and the try is that touchdown, never a later play.
#[test]
fn a_bite_on_the_try_is_about_its_touchdown() {
	let plays = plays();
	let log = log_with_tries_to_come(&plays);
	let want = Some("CHI");
	let end = wall(plays.last().unwrap()) + TimeDelta::minutes(5);
	let over = game(want, status(GameState::Post, plays.last()), None);
	let (mut tries, mut kicks) = (0, 0);
	for (k, play) in plays.iter().enumerate().filter(|(_, p)| p.text.as_deref().is_some_and(|t| t.contains("TOUCHDOWN."))) {
		let bite = first_seen(&log, play) + TimeDelta::seconds(10);
		let next = plays[k + 1..].iter().find(|p| is_snap(p));
		let then = game(want, status(GameState::In, Some(play)), next.map(Situation::before));
		for offered in offer(&then, &log, pacific(bite), Mode::Full, Watching::Unknown).scenarios {
			let scenario = offered.scenario;
			if !matches!(scenario, Scenario::TwoPoint | Scenario::Kick) {
				continue;
			}
			assert_eq!(offered_said(&then, &log, bite, scenario), Said::After, "{scenario:?} after {}", play.id);
			let were = plays_that_were(&over, &log, end, scenario);
			let picked = its_play(Said::After, bite, &were, ANY, LAG).map(|(w, _)| position(&plays, &w.play_id));
			assert!(picked.is_none_or(|i| i <= k), "{scenario:?} after {} matched a later play", play.id);
			if scenario == Scenario::TwoPoint {
				assert_eq!(picked, Some(k), "the try after {} is its touchdown", play.id);
				tries += 1;
			} else {
				kicks += 1;
			}
		}
	}
	assert!(tries >= 4 && kicks >= 4, "{tries} tries and {kicks} kicks offered");
}

/// The direction alone, on plays placed by hand: after, the nearest at or
/// before within the window; before, the first home had not seen, its time
/// within the window either side, the gap signed.
#[test]
fn its_play_looks_back_after_and_ahead_before() {
	let t = DateTime::parse_from_rfc3339("2026-10-23T01:00:00Z").unwrap().with_timezone(&Utc);
	let s = |n: i64| t + TimeDelta::seconds(n);
	let window = TimeDelta::seconds(90);
	let were = [
		was("far", s(-200), s(-190)),
		was("old", s(-80), s(-70)),
		was("near", s(-20), s(-5)),
		// Happened before the bite started, but home saw it after: the snap
		// the owner spoke ahead of on a delayed broadcast.
		was("delayed", s(-3), s(4)),
		was("after", s(30), s(40)),
	];
	let pick = |said, were: &[Was]| its_play(said, t, were, window, LAG).map(|(w, gap)| (w.play_id.clone(), gap));
	assert_eq!(pick(Said::After, &were), Some(("delayed".to_owned(), 3)));
	assert_eq!(pick(Said::After, &were[..3]), Some(("near".to_owned(), 20)));
	assert_eq!(pick(Said::Before, &were), Some(("delayed".to_owned(), 3)));
	assert_eq!(pick(Said::Before, &[was("after", s(30), s(40))]), Some(("after".to_owned(), -30)));
	assert_eq!(pick(Said::Before, &were[..3]), None, "every play there home had seen: none is to come");
	assert_eq!(pick(Said::Before, &[was("much later", s(200), s(210))]), None, "outside the window");
	assert_eq!(pick(Said::After, &[was("far", s(-200), s(-190))]), None, "outside the window");
	assert_eq!(its_play(Said::After, t, &[was("edge", s(-90), s(-80))], window, LAG).map(|(_, gap)| gap), Some(90));
	assert_eq!(its_play(Said::Before, t, &[was("edge", s(90), s(95))], window, LAG).map(|(_, gap)| gap), Some(-90));
	// Home saw the snap 4 s before the bite, the phone not yet: still to come
	// on the phone. Seen 6 s before, the offer had moved on.
	assert_eq!(pick(Said::Before, &[was("just seen", s(-10), s(-4))]), Some(("just seen".to_owned(), 10)));
	assert_eq!(pick(Said::Before, &[was("long seen", s(-10), s(-6))]), None);
}

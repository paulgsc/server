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

use chrono::{SecondsFormat, TimeDelta};
use common::{first_seen, game, is_snap, pacific, plays, recorded_log, status, wall, CADENCE, ESPN_DELAY, GAME};
use game_scenarios::Scenario;
use nfl_feed::{
	log::{Entry, PlayLog},
	summary::Play,
	GameState,
};
use scenario_engine::{its_play, offer, offered_said, on_offer, plays_that_were, Mode, Said, Situation, Watching};

/// A window wide enough that only the direction decides.
const ANY: TimeDelta = TimeDelta::days(1);

#[cfg(test)]
fn position(plays: &[Play], id: &str) -> usize {
	plays.iter().position(|p| p.id == id).unwrap()
}

/// BM5: before every snap, each scenario offered on it is matched by its
/// direction. Offered on the snap itself, its play is that snap (or, after
/// a flag wiped it out, the down replayed) when the rule accepts it, and
/// none otherwise (a play call on 1st and 10 is not the
/// short-yardage call 70 s later). Offered on the drive, it is a play of the
/// same possession from that snap on. Offered on a play in the log, it is a
/// play before the snap.
#[test]
fn a_bite_on_the_snap_to_come_is_about_that_snap() {
	let plays = plays();
	let log = recorded_log(&plays);
	let want = Some("CHI");
	let end = wall(plays.last().unwrap()) + TimeDelta::minutes(5);
	let over = game(want, status(GameState::Post, plays.last()), None);
	let (mut on_the_snap, mut not_its_rule, mut on_the_drive, mut judged) = (0, 0, 0, 0);
	for (k, play) in plays.iter().enumerate().filter(|(_, p)| is_snap(p)) {
		let bite = wall(play) - TimeDelta::seconds(10);
		// Judged only where home had every play before this snap: that is
		// when the phone is offering it.
		if plays[..k].iter().any(|p| first_seen(&log, p) > bite) {
			continue;
		}
		judged += 1;
		// The snap the offer names is the first from here no flag wiped out:
		// a false start or encroachment replays the down.
		let Some(target) = (k..plays.len()).find(|j| is_snap(&plays[*j]) && !plays[*j].text.as_deref().unwrap_or_default().contains("No Play")) else {
			continue;
		};
		let then = game(want, status(GameState::In, Some(play)), Some(Situation::before(play)));
		for offered in offer(&then, &log, pacific(bite), Mode::Full, Watching::Unknown).scenarios {
			let scenario = offered.scenario;
			assert!(on_offer(&then, &log, bite, scenario));
			let said = offered_said(&then, &log, bite, scenario);
			let accepts = plays_that_were(&over, &log, end, scenario).iter().any(|w| w.play_id == plays[target].id);
			let picked = its_play(&over, &log, end, scenario, said, bite, bite, ANY).map(|(w, _)| position(&plays, &w.play_id));
			let why = [format!("{scenario:?}"), String::from(" before "), play.id.clone()].concat();
			match said {
				Said::BeforeTheSnap if accepts => {
					assert_eq!(picked, Some(target), "{why}");
					on_the_snap += 1;
				}
				Said::BeforeTheSnap => {
					assert_eq!(picked, None, "{why}: its rule does not accept the snap");
					not_its_rule += 1;
				}
				Said::BeforeTheDrive => {
					if let Some(i) = picked {
						assert!(i >= k && plays[i].possession == play.possession, "{why}");
						on_the_drive += 1;
					}
				}
				Said::After => assert!(picked.is_none_or(|i| i < k), "{why} is about a play in the log"),
			}
		}
	}
	assert!(judged >= 100, "{judged} snaps judged");
	assert!(on_the_snap >= 20, "{on_the_snap} offers matched to their snap");
	assert!(not_its_rule >= 20, "{not_its_rule} offers whose snap their rule does not accept");
	assert!(on_the_drive >= 3, "{on_the_drive} offers matched to their drive");
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
			let picked = its_play(&over, &log, end, scenario, Said::After, bite, bite, ANY).map(|(w, _)| position(&plays, &w.play_id));
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

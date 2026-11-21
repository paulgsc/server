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
			let why = [scenario.id(), " before ", &play.id].concat();
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
					// The first play the rule accepts from the snap on, before
					// the other team snaps the ball.
					let were = plays_that_were(&over, &log, end, scenario);
					let expected = plays[target..]
						.iter()
						.enumerate()
						.take_while(|(_, p)| !is_snap(p) || p.possession == plays[target].possession)
						.find(|(_, p)| were.iter().any(|w| w.play_id == p.id))
						.map(|(j, _)| target + j);
					assert_eq!(picked, expected, "{why}");
					on_the_drive += usize::from(picked.is_some());
				}
				Said::After => assert!(picked.is_none_or(|i| i < k), "{why} is about a play in the log"),
				Said::BeforeTheKickoff | Said::Either => panic!("{why}: no offer before a snap is of the kickoff, and only the reconciler says Either"),
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

/// BM5: the coin toss is offered up to a quarter hour before kickoff, and a
/// bite on it is about the opening kickoff, however long after it came.
#[test]
fn a_bite_before_kickoff_is_about_the_opening_kickoff() {
	let plays = plays();
	let log = recorded_log(&plays);
	let end = wall(plays.last().unwrap()) + TimeDelta::minutes(5);
	let over = game(Some("CHI"), status(GameState::Post, plays.last()), None);
	let kickoff = plays.iter().position(|p| p.play_type.as_deref() == Some("Kickoff")).unwrap();
	let bite = wall(&plays[kickoff]) - TimeDelta::minutes(10);
	let then = game(Some("CHI"), status(GameState::Pre, None), None);
	assert!(on_offer(&then, &log, bite, Scenario::CoinToss));
	let said = offered_said(&then, &log, bite, Scenario::CoinToss);
	assert_eq!(said, Said::BeforeTheKickoff);
	let picked = its_play(&over, &log, end, Scenario::CoinToss, said, bite, bite, TimeDelta::seconds(90));
	assert_eq!(picked.map(|(w, gap)| (w.play_id, gap)), Some((plays[kickoff].id.clone(), -600)));
}

/// BM5: a bite not tied to an offer takes the nearer in time of the play
/// before it and the play after it.
#[test]
fn a_bite_not_tied_to_an_offer_takes_the_nearer_play() {
	let plays = plays();
	let log = recorded_log(&plays);
	let end = wall(plays.last().unwrap()) + TimeDelta::minutes(5);
	let over = game(Some("CHI"), status(GameState::Post, plays.last()), None);
	let sacks: Vec<usize> = plays.iter().enumerate().filter(|(_, p)| p.play_type.as_deref() == Some("Sack")).map(|(i, _)| i).collect();
	// Two sacks in a row, with a window wide enough to reach both from
	// between them: a bite just after the first, and one just before the next.
	let (a, b) = (sacks[0], sacks[1]);
	let near = |at| its_play(&over, &log, end, Scenario::Sack, Said::Either, at, at, ANY).map(|(w, _)| w.play_id);
	assert_eq!(near(wall(&plays[b]) - TimeDelta::seconds(5)), Some(plays[b].id.clone()), "the sack 5 s after");
	assert_eq!(near(wall(&plays[a]) + TimeDelta::seconds(5)), Some(plays[a].id.clone()), "the sack 5 s before");
}

/// BM5: an opening-drive offer made before the opening kickoff is listed is
/// about the receivers' first drive: the kickoff is no snap to come, so the
/// drive is found from the first snap after it.
#[test]
fn a_bite_on_the_opening_drive_before_the_kickoff_is_about_that_drive() {
	let plays = plays();
	let log = recorded_log(&plays);
	let end = wall(plays.last().unwrap()) + TimeDelta::minutes(5);
	let over = game(Some("CHI"), status(GameState::Post, plays.last()), None);
	let kickoff = plays.iter().position(|p| p.play_type.as_deref() == Some("Kickoff")).unwrap();
	let first = plays[kickoff..].iter().position(is_snap).unwrap() + kickoff;
	let bite = wall(&plays[kickoff]) - TimeDelta::seconds(10);
	let were = plays_that_were(&over, &log, end, Scenario::OpeningDrive);
	let expected = plays[first..]
		.iter()
		.enumerate()
		.take_while(|(_, p)| !is_snap(p) || p.possession == plays[first].possession)
		.find(|(_, p)| were.iter().any(|w| w.play_id == p.id))
		.map(|(j, _)| first + j);
	assert!(expected.is_some(), "the recorded game's first drive ended in a play the rule accepts");
	// Live, with the receivers' first snap to come, and nothing in the log.
	let then = game(Some("CHI"), status(GameState::In, Some(&plays[kickoff])), Some(Situation::before(&plays[first])));
	assert!(on_offer(&then, &log, bite, Scenario::OpeningDrive), "offered before the kickoff is listed");
	let said = offered_said(&then, &log, bite, Scenario::OpeningDrive);
	assert_eq!(said, Said::BeforeTheDrive);
	let picked = its_play(&over, &log, end, Scenario::OpeningDrive, said, bite, bite, TimeDelta::seconds(90));
	assert_eq!(picked.map(|(w, _)| position(&plays, &w.play_id)), expected);
}

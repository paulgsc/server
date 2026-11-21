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

/// Whether `play` ends a possession whoever snaps next (#1077): a kickoff,
/// or the end of a quarter that ends a half.
#[cfg(test)]
fn ends_a_drive(play: &Play) -> bool {
	let kind = play.play_type.as_deref().unwrap_or_default();
	kind.contains("Kickoff") || (play.period == Some(2) && matches!(kind, "End of Half" | "End Period"))
}

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
						.take_while(|(j, p)| *j == 0 || (!ends_a_drive(p) && (!is_snap(p) || p.possession == plays[target].possession)))
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
		.take_while(|(j, p)| *j == 0 || (!ends_a_drive(p) && (!is_snap(p) || p.possession == plays[first].possession)))
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

/// A play shaped for #1077's games: the team with the ball, the period, its
/// type and text, the down and yards to the end zone, and the score after.
#[cfg(test)]
fn shaped(n: i64, period: i64, team: &str, kind: &str, text: &str, (down, to_go): (Option<i64>, i64), score: (i64, i64)) -> Play {
	let base = chrono::DateTime::parse_from_rfc3339("2026-10-23T01:00:00Z").unwrap().with_timezone(&chrono::Utc);
	// Forty seconds a play, and a quarter hour for halftime.
	let at = base + TimeDelta::seconds(40 * n) + TimeDelta::minutes(if period >= 3 { 15 } else { 0 });
	Play {
		id: ["p", &n.to_string()].concat(),
		sequence: Some(n),
		wallclock: Some(at.to_rfc3339_opts(SecondsFormat::Secs, true)),
		period: Some(period),
		clock: Some("0:10".to_owned()),
		down,
		distance: down.map(|_| 10),
		yard_line: Some(50),
		yards_to_endzone: Some(to_go),
		possession: Some(team.to_owned()),
		play_type: Some(kind.to_owned()),
		scoring: Some(false),
		away_score: Some(score.0),
		home_score: Some(score.1),
		text: Some(text.to_owned()),
	}
}

/// #1077: a possession ends at the half. NE is in the red zone as the 2nd
/// quarter runs out, the half ends with no score, and NE receives the second
/// half's kickoff and scores on that drive. A red-zone bite said before the
/// Q2 snap is about the trip that ended at the half, which had no result, so
/// it is about no play, never the 3rd quarter's touchdown.
#[test]
fn a_drive_bite_never_takes_the_next_halfs_score() {
	let plays = [
		shaped(1, 2, "NE", "Rush", "R.Stevenson up the middle to CHI 12 for 3 yards.", (Some(1), 15), (14, 20)),
		shaped(2, 2, "NE", "Pass Incompletion", "D.Maye pass incomplete.", (Some(2), 12), (14, 20)),
		shaped(3, 2, "NE", "End of Half", "END OF 2ND QUARTER", (None, 12), (14, 20)),
		shaped(4, 3, "CHI", "Kickoff", "C.Santos kicks 65 yards from CHI 35 to end zone, Touchback.", (None, 65), (14, 20)),
		shaped(5, 3, "NE", "Rush", "R.Stevenson left end to NE 40 for 5 yards.", (Some(1), 65), (14, 20)),
		shaped(
			6,
			3,
			"NE",
			"Passing Touchdown",
			"D.Maye pass short right to H.Henry for 10 yards, TOUCHDOWN.",
			(Some(1), 10),
			(20, 20),
		),
	];
	let log = recorded_log(&plays);
	let end = wall(plays.last().unwrap()) + TimeDelta::minutes(5);
	let over = game(Some("CHI"), status(GameState::Post, plays.last()), None);
	let bite = wall(&plays[0]) - TimeDelta::seconds(10);
	let then = game(Some("CHI"), status(GameState::In, Some(&plays[0])), Some(Situation::before(&plays[0])));
	assert!(on_offer(&then, &log, bite, Scenario::RedZone));
	let said = offered_said(&then, &log, bite, Scenario::RedZone);
	assert_eq!(said, Said::BeforeTheDrive);
	assert!(
		plays_that_were(&over, &log, end, Scenario::RedZone).iter().any(|w| w.play_id == "p6"),
		"the 3rd quarter's touchdown is a red-zone trip of its own"
	);
	assert_eq!(its_play(&over, &log, end, Scenario::RedZone, said, bite, bite, ANY), None);
	// Each of the three ends the drive alone: the half's end without the
	// kickoff, the kickoff without the half's end, and the quarter with
	// neither in the log.
	for gone in [&["p4"][..], &["p3"], &["p3", "p4"]] {
		let held: Vec<Play> = plays.iter().filter(|p| !gone.contains(&p.id.as_str())).cloned().collect();
		let log = recorded_log(&held);
		assert_eq!(its_play(&over, &log, end, Scenario::RedZone, said, bite, bite, ANY), None, "without {gone:?}");
	}
}

/// #1077: a possession ends at a kickoff. CHI scores on its opening drive,
/// kicks onside and gets it back: the drive after is not its first, so the
/// opening drive is not on offer, and a bite on it before the onside kick
/// is not about the next drive's score.
#[test]
fn an_onside_kick_recovered_by_the_kickers_starts_a_new_drive() {
	let plays = [
		shaped(1, 1, "NE", "Kickoff", "A.Borregales kicks 65 yards from NE 35 to end zone, Touchback.", (None, 65), (0, 0)),
		shaped(2, 1, "CHI", "Rush", "D.Swift left end to NE 15 for 50 yards.", (Some(1), 65), (0, 0)),
		shaped(
			3,
			1,
			"CHI",
			"Passing Touchdown",
			"C.Williams pass short left to R.Odunze for 15 yards, TOUCHDOWN.",
			(Some(1), 15),
			(0, 7),
		),
		shaped(
			4,
			1,
			"CHI",
			"Kickoff",
			"C.Santos kicks onside 11 yards from CHI 35 to CHI 46. RECOVERED by CHI-J.Brisker.",
			(None, 65),
			(0, 7),
		),
		shaped(5, 1, "CHI", "Rush", "D.Swift right end to NE 10 for 44 yards.", (Some(1), 54), (0, 7)),
		shaped(6, 1, "CHI", "Rushing Touchdown", "D.Swift up the middle for 10 yards, TOUCHDOWN.", (Some(1), 10), (0, 14)),
	];
	let log = recorded_log(&plays);
	let end = wall(plays.last().unwrap()) + TimeDelta::minutes(5);
	let over = game(Some("CHI"), status(GameState::Post, plays.last()), None);
	// Before the snap after the onside kick: CHI's first drive is over.
	let bite = wall(&plays[4]) - TimeDelta::seconds(10);
	let then = game(Some("CHI"), status(GameState::In, Some(&plays[3])), Some(Situation::before(&plays[4])));
	assert!(!on_offer(&then, &log, bite, Scenario::OpeningDrive), "the drive after an onside kick is not the first");
	// A red-zone bite before the first touchdown's snap is about that
	// touchdown; one on the snap after the onside kick, about the second.
	let red = |k: usize| {
		let bite = wall(&plays[k]) - TimeDelta::seconds(10);
		its_play(&over, &log, end, Scenario::RedZone, Said::BeforeTheDrive, bite, bite, ANY).map(|(w, _)| w.play_id)
	};
	assert_eq!(red(2).as_deref(), Some("p3"));
	assert_eq!(red(4).as_deref(), Some("p6"));
	// The opening drive's bite before its first snap ends with that drive.
	let first = wall(&plays[1]) - TimeDelta::seconds(10);
	let picked = its_play(&over, &log, end, Scenario::OpeningDrive, Said::BeforeTheDrive, first, first, ANY).map(|(w, _)| w.play_id);
	assert_ne!(picked.as_deref(), Some("p6"), "the opening drive is not the one after the onside kick");
	let were: Vec<String> = plays_that_were(&over, &log, end, Scenario::OpeningDrive).into_iter().map(|w| w.play_id).collect();
	assert_eq!(were, ["p3"], "only the first drive's end is an opening drive");
}

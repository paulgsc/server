//! The game so far, read from its play log as of a time, and what each play
//! says in ESPN's spelling. ESPN's play types and play text are read here and
//! nowhere else.

use crate::Game;
use chrono::{DateTime, Utc};
use game_scenarios::Kind;
use nfl_feed::{log::Entry, summary::Play, GameState};
use std::collections::HashMap;

/// One play as of `now`: its latest version home had seen by then.
pub(crate) struct Seen<'a> {
	pub(crate) play: &'a Play,
	/// When home first saw the play: what "just happened" counts from.
	pub(crate) first: DateTime<Utc>,
	/// When it happened: ESPN's wallclock, or else when home first saw it.
	pub(crate) at: DateTime<Utc>,
}

/// The game's plays as of `now`, in ESPN's order, with what the engine
/// derives from them.
pub(crate) struct SoFar<'a> {
	pub(crate) game: &'a Game,
	pub(crate) plays: Vec<Seen<'a>>,
	/// The score (away, home) after each play, a missing one carried forward.
	scores: Vec<(i64, i64)>,
	/// Each snap's possession run: a run is the snaps one team has in a row,
	/// within one half and with no kick-off between them.
	runs: Vec<Option<usize>>,
	/// The first run of each team.
	first_run: HashMap<&'a str, usize>,
	/// Whether a kick-off or the end of a half has closed the latest run.
	run_closed: bool,
}

impl<'a> SoFar<'a> {
	pub(crate) fn new(game: &'a Game, log: &'a [Entry], now: DateTime<Utc>) -> Self {
		let mut latest: HashMap<&str, Seen<'a>> = HashMap::new();
		for entry in log.iter().filter(|e| e.game_id == game.id) {
			let Some(seen) = time(&entry.seen_at).filter(|seen| *seen <= now) else {
				continue;
			};
			// The log is appended in the order home saw it, so the first entry
			// kept for a play is when home first saw it.
			let first = latest.get(entry.play.id.as_str()).map_or(seen, |held| held.first);
			let at = entry.play.wallclock.as_deref().and_then(time).unwrap_or(first);
			latest.insert(entry.play.id.as_str(), Seen { play: &entry.play, first, at });
		}
		let mut plays: Vec<Seen<'a>> = latest.into_values().collect();
		plays.sort_by(|a, b| (a.play.sequence, &a.play.id).cmp(&(b.play.sequence, &b.play.id)));
		let mut scores = Vec::with_capacity(plays.len());
		let mut score = (0, 0);
		for seen in &plays {
			score = (seen.play.away_score.unwrap_or(score.0), seen.play.home_score.unwrap_or(score.1));
			scores.push(score);
		}
		let mut runs = Vec::with_capacity(plays.len());
		let mut first_run = HashMap::new();
		// The run going: its index and the team snapping. A kick-off (an onside
		// kick the kickers recover included) or the end of a half closes it, so
		// the next snap starts a new one even by the same team; so does a snap
		// in a later half than the last snap whose half is known, in case the
		// log lacks both. Only regulation's halves count: an overtime period's
		// break carries possession over (the playoffs), and the 4th quarter's
		// end is followed by a kick-off.
		let mut run: Option<(usize, &str)> = None;
		let mut last_half: Option<i64> = None;
		let mut closed = false;
		for seen in &plays {
			let team = seen.play.possession.as_deref().filter(|_| is_snap(seen.play));
			let its_half = seen.play.period.filter(|period| *period <= 4).map(half);
			runs.push(team.map(|team| {
				let new_half = matches!((last_half, its_half), (Some(last), Some(now)) if last != now);
				let index = match run {
					Some((index, held)) if held == team && !closed && !new_half => index,
					Some((index, _)) => index + 1,
					None => 0,
				};
				run = Some((index, team));
				last_half = its_half.or(last_half);
				closed = false;
				first_run.entry(team).or_insert(index);
				index
			}));
			closed |= kind(seen.play).contains("kickoff") || ends_the_half(seen.play);
		}
		Self {
			game,
			plays,
			scores,
			runs,
			first_run,
			run_closed: closed,
		}
	}

	/// The score (away, home) now.
	pub(crate) fn score(&self) -> (i64, i64) {
		self.scores.last().copied().unwrap_or((0, 0))
	}

	/// The score (away, home) after play `i`.
	pub(crate) fn score_after(&self, i: usize) -> (i64, i64) {
		self.scores[i]
	}

	/// The score (away, home) before play `i`.
	pub(crate) fn score_before(&self, i: usize) -> (i64, i64) {
		i.checked_sub(1).map_or((0, 0), |before| self.scores[before])
	}

	/// `team`'s points minus the other side's, in `score` (away, home).
	pub(crate) fn margin(&self, team: &str, score: (i64, i64)) -> Option<i64> {
		if team == self.game.home {
			Some(score.1 - score.0)
		} else if team == self.game.away {
			Some(score.0 - score.1)
		} else {
			None
		}
	}

	/// The other side of the game from `team`.
	pub(crate) fn other(&self, team: &str) -> Option<&'a str> {
		if team == self.game.home {
			Some(&self.game.away)
		} else if team == self.game.away {
			Some(&self.game.home)
		} else {
			None
		}
	}

	/// Whether the game is over: its status says final, or the log has its
	/// last play.
	pub(crate) fn over(&self) -> bool {
		self.game.status.is_final() || self.plays.iter().any(|seen| kind(seen.play) == "end of game")
	}

	/// Whether the game ended without being played: ESPN says `post` but not
	/// completed (postponed or cancelled), and no play is in the log.
	pub(crate) fn not_played(&self) -> bool {
		self.plays.is_empty() && self.game.status.state == Some(GameState::Post) && !self.over()
	}

	/// Play `i`'s possession run, if it is a snap.
	pub(crate) fn run(&self, i: usize) -> Option<usize> {
		self.runs[i]
	}

	/// The index of the opening kickoff, the first kickoff in the log.
	pub(crate) fn opening_kickoff(&self) -> Option<usize> {
		self.plays.iter().position(|seen| kind(seen.play) == "kickoff")
	}

	/// The index of the latest snap.
	pub(crate) fn last_snap(&self) -> Option<usize> {
		self.plays.iter().rposition(|seen| is_snap(seen.play))
	}

	/// How many snaps came after play `i`.
	pub(crate) fn snaps_since(&self, i: usize) -> usize {
		self.plays[i + 1..].iter().filter(|seen| is_snap(seen.play)).count()
	}

	/// Whether play `i` is a snap in its team's first possession.
	pub(crate) fn in_first_run(&self, i: usize) -> bool {
		match (self.runs[i], self.plays[i].play.possession.as_deref()) {
			(Some(run), Some(team)) => self.first_run.get(team) == Some(&run),
			_ => false,
		}
	}

	/// Whether `team`'s first possession is still to come or still going: no
	/// kick-off or end of a half has closed it.
	pub(crate) fn first_run_open(&self, team: &str) -> bool {
		self.first_run.get(team).is_none_or(|first| {
			!self.run_closed
				&& self
					.last_snap()
					.is_some_and(|last| self.runs[last] == Some(*first) && self.plays[last].play.possession.as_deref() == Some(team))
		})
	}

	/// Timeouts `team` has left in the half of `period`: three a half, two in
	/// overtime, less each "Timeout ... by TEAM" play in that half.
	pub(crate) fn timeouts_left(&self, team: &str, period: i64) -> i64 {
		let by_team = |text: &str| text.split(" by ").skip(1).any(|rest| rest.split_once(" at ").is_some_and(|(by, _)| self::team(by) == team));
		let used = self
			.plays
			.iter()
			.filter(|seen| seen.play.period.is_some_and(|p| half(p) == half(period)))
			.filter(|seen| kind(seen.play) == "timeout" && by_team(&text(seen.play)))
			.count();
		let allowed = if period >= 5 { 2 } else { 3 };
		(allowed - i64::try_from(used).unwrap_or(allowed)).max(0)
	}
}

/// The half of `period`: 1 for the 1st and 2nd quarters, 2 for the 3rd and
/// 4th, and each overtime period its own.
pub(crate) const fn half(period: i64) -> i64 {
	if period >= 5 {
		period
	} else {
		(period + 1) / 2
	}
}

/// The teams ESPN's play text spells otherwise than its header (and so
/// `possession`, and the game's teams), as (text, header).
const SPELLED: [(&str, &str); 6] = [("ARZ", "ARI"), ("BLT", "BAL"), ("CLV", "CLE"), ("HST", "HOU"), ("LA", "LAR"), ("WAS", "WSH")];

/// A team named in ESPN's play text ("PENALTY on HST", "recovered by
/// BLT-S.Kane"), as the header names it ("HOU", "BAL"), in upper case.
pub(crate) fn team(code: &str) -> String {
	let code = code.to_ascii_uppercase();
	SPELLED.iter().find(|(text, _)| *text == code).map_or(code, |(_, header)| (*header).to_owned())
}

/// An ISO-8601 time as ESPN and the log write it.
pub(crate) fn time(at: &str) -> Option<DateTime<Utc>> {
	DateTime::parse_from_rfc3339(at).ok().map(|at| at.with_timezone(&Utc))
}

/// ESPN's play type, lower-cased.
pub(crate) fn kind(play: &Play) -> String {
	play.play_type.as_deref().unwrap_or_default().to_ascii_lowercase()
}

/// ESPN's play text, lower-cased (ASCII only, so byte offsets still match the
/// original).
pub(crate) fn text(play: &Play) -> String {
	play.text.as_deref().unwrap_or_default().to_ascii_lowercase()
}

/// A play from scrimmage, or a flag on one: it has a down and a spot, and is
/// not a timeout, a kickoff or the end of a period.
pub(crate) fn is_snap(play: &Play) -> bool {
	let kind = kind(play);
	play.down.is_some()
		&& play.yards_to_endzone.is_some_and(|y| y > 0)
		&& !["timeout", "end ", "end of", "warning", "kickoff", "coin toss"].iter().any(|word| kind.contains(word))
}

/// Words in a touchdown's text that begin its try.
const TRY_WORDS: [&str; 3] = ["extra point", "two-point", "kick is"];

/// A play a flag wiped out: "No Play" before any try, so a flag on the
/// try alone leaves its touchdown standing. Every outcome below reads a
/// wiped-out play as having none.
pub(crate) fn wiped_out(play: &Play) -> bool {
	let text = text(play);
	let end = TRY_WORDS.iter().filter_map(|word| text.find(word)).min().unwrap_or(text.len());
	text[..end].contains("no play")
}

pub(crate) fn is_touchdown(play: &Play) -> bool {
	!wiped_out(play) && (kind(play).contains("touchdown") || text(play).contains("touchdown"))
}

/// A touchdown whose try is not in the log yet.
pub(crate) fn try_pending(play: &Play) -> bool {
	let text = text(play);
	is_touchdown(play) && !TRY_WORDS.iter().any(|word| text.contains(word))
}

/// How a touchdown's try went, as far as its text says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Try {
	/// Not in the log yet.
	ToCome,
	Kicked,
	WentForTwo,
}

pub(crate) fn try_of(play: &Play) -> Try {
	if try_pending(play) {
		Try::ToCome
	} else if text(play).contains("two-point") {
		Try::WentForTwo
	} else {
		Try::Kicked
	}
}

pub(crate) fn is_run(play: &Play) -> bool {
	kind(play).contains("rush")
}

pub(crate) fn is_pass(play: &Play) -> bool {
	let kind = kind(play);
	kind.contains("pass") || kind == "sack"
}

pub(crate) fn is_punt(play: &Play) -> bool {
	kind(play).contains("punt")
}

/// Yards gained, from ESPN's "for 12 yards" / "for no gain", unless the play
/// was wiped out ("No Play"). A reviewed play is read as the review left it:
/// from the text after the last "reversed." or "upheld.", or, when an upheld
/// ruling is not restated, from the ruling before it.
pub(crate) fn gain(play: &Play) -> Option<i64> {
	let text = text(play);
	let review = ["reversed.", "upheld."]
		.into_iter()
		.filter_map(|word| text.rfind(word).map(|at| (at, word)))
		.max_by_key(|(at, _)| *at);
	match review {
		Some((at, word)) => {
			let (ruling, after) = (&text[..at], &text[at + word.len()..]);
			gain_in(after).or_else(|| (word == "upheld.").then(|| gain_in(ruling)).flatten())
		}
		None => gain_in(&text),
	}
}

fn gain_in(text: &str) -> Option<i64> {
	if text.contains("no play") {
		return None;
	}
	text.match_indices(" for ").find_map(|(at, _)| {
		let rest = &text[at + 5..];
		if rest.starts_with("no gain") {
			return Some(0);
		}
		let digits = rest.find(|c: char| !(c.is_ascii_digit() || c == '-')).map_or(rest, |end| &rest[..end]);
		rest[digits.len()..].starts_with(" yard").then(|| digits.parse().ok()).flatten()
	})
}

/// A play's gain as the big-play algebra reads it (#3048): its kind, the
/// yards, and whether the side that gained is the team ESPN gave the ball
/// (the offense on a snap, the kicking team on a kick). A return is the
/// returner's: a pick-six is the defense's interception return, never the
/// offense's pass, whose text names the target with "intended for" and the
/// return's yards after it.
pub(crate) fn gain_by(play: &Play) -> Option<(Kind, i64, bool)> {
	if wiped_out(play) || field_goal(play).is_some() || touchdown_nullified(play) {
		return None;
	}
	let text = text(play);
	let after = |word: &str| text.find(word).map(|at| &text[at + word.len()..]);
	// A return's yards are the returner's only while nothing undid them: a
	// flag on the returning side (not the team ESPN gave the ball: the kicking
	// team, or the offense that lost it) is enforced from a spot ESPN does not
	// restate as yards, and a muff or fumble on it is the kicking team's ball.
	let clean_return = !flagged_against_the_returners(play) && !text.contains("reversed.");
	let kept = !text.contains("muff") && !text.contains("fumble");
	if kind(play).contains("kickoff") {
		return (!is_onside(play) && clean_return && kept)
			.then(|| after("kicks").and_then(return_yards).map(|y| (Kind::KickReturn, y, false)))
			.flatten();
	}
	if is_punt(play) {
		return (clean_return && kept)
			.then(|| after("punts").and_then(return_yards).map(|y| (Kind::PuntReturn, y, false)))
			.flatten();
	}
	if let Some(rest) = after("intercepted by") {
		return clean_return.then(|| return_yards(rest).map(|y| (Kind::InterceptionReturn, y, false))).flatten();
	}
	if lost_fumble(play, &text) {
		return clean_return
			.then(|| after("recovered by").and_then(return_yards).map(|y| (Kind::FumbleReturn, y, false)))
			.flatten();
	}
	if !is_snap(play) {
		return None;
	}
	// ESPN types some gains by what came after them ("Fumble Recovery
	// (Own)", "Penalty" with the play standing): read the kind from the text.
	let kind = if is_run(play) {
		Kind::Run
	} else if is_pass(play) || text.contains(" pass ") {
		Kind::Pass
	} else {
		Kind::Run
	};
	gain(play).map(|y| (kind, y, true))
}

/// Whether an accepted flag on the play is on the side ESPN did not give the
/// ball (the receivers of a kick, the defense on a turnover): every "PENALTY
/// on" clause is read, and one its text declines is no flag.
fn flagged_against_the_returners(play: &Play) -> bool {
	let original = play.text.as_deref().unwrap_or_default();
	let lower = original.to_ascii_lowercase();
	let starts: Vec<usize> = lower.match_indices("penalty on ").map(|(at, _)| at).collect();
	starts.iter().enumerate().any(|(n, start)| {
		let end = starts.get(n + 1).copied().unwrap_or(lower.len());
		let clause = &lower[*start..end];
		let team = original[start + "penalty on ".len()..end].split(['-', ',', ' ']).next().unwrap_or_default();
		// A foul after the play ("enforced between downs", a celebration) does
		// not undo the return before it.
		!clause.contains("declined") && !clause.contains("between downs") && !team.is_empty() && play.possession.as_deref() != Some(self::team(team).as_str())
	})
}

/// The yards of the return a kick's or turnover's text tells after its
/// start: "C.Kiner to NE 31 for 26 yards"; none for a fair catch, a
/// touchback or a return for no gain.
fn return_yards(rest: &str) -> Option<i64> {
	if rest.contains("fair catch") {
		return None;
	}
	gain_in(rest)
}

/// A kickoff the kicking team tried to get back.
pub(crate) fn is_onside(play: &Play) -> bool {
	kind(play).contains("kickoff") && text(play).contains("onside")
}

/// The team that recovered an onside kick, from "recovered by NE-B.Bolden":
/// ESPN's team, or none when the text names none ("recovered by B.Bolden").
pub(crate) fn recovered_by(play: &Play) -> Option<String> {
	let original = play.text.as_deref()?;
	let at = original.to_ascii_lowercase().find("recovered by ")? + "recovered by ".len();
	let (team, _) = original[at..].split_once('-')?;
	(matches!(team.len(), 2 | 3) && team.bytes().all(|b| b.is_ascii_uppercase())).then(|| self::team(team))
}

/// Whether ESPN's text says a touchdown on the play was taken away.
pub(crate) fn touchdown_nullified(play: &Play) -> bool {
	text(play).contains("touchdown nullified")
}

/// Whether play `play` ends the first half: ESPN's "End of Half", or its
/// end of the 2nd quarter by another name.
pub(crate) fn ends_the_half(play: &Play) -> bool {
	let kind = kind(play);
	play.period == Some(2) && (kind == "end of half" || kind == "end period")
}

/// The team flagged and the foul, from "PENALTY on NE-W.Campbell, False
/// Start, 5 yards".
pub(crate) fn penalty(play: &Play) -> Option<(String, Option<String>)> {
	let original = play.text.as_deref().unwrap_or_default();
	let at = text(play).find("penalty on ")? + "penalty on ".len();
	let rest = &original[at..];
	let team = rest.split(['-', ',', ' ']).next().filter(|team| !team.is_empty())?;
	let foul = rest
		.split(", ")
		.nth(1)
		.map(|foul| foul.trim().to_ascii_lowercase())
		.filter(|foul| !foul.is_empty() && !foul.ends_with("yards"));
	Some((self::team(team), foul))
}

/// How a field goal went, and from how far.
pub(crate) fn field_goal(play: &Play) -> Option<(Kick, Option<i64>)> {
	if wiped_out(play) {
		return None;
	}
	let text = text(play);
	// "(Field Goal formation)" opens a fake, or a flag before the kick: the
	// kick is a later "field goal", if there is one.
	let kick = text
		.match_indices("field goal")
		.map(|(at, _)| at)
		.find(|at| !text[*at..].starts_with("field goal formation"))?;
	let at = text[..kick].strip_suffix("yard ").map_or(kick, str::len);
	let distance = text[..at].trim_end().rsplit(' ').next().and_then(|word| word.parse().ok());
	Some((kick_result(&text[at..]), distance))
}

/// An extra point that missed or was blocked, from a touchdown's text.
pub(crate) fn extra_point_missed(play: &Play) -> Option<Kick> {
	let text = text(play);
	let at = text.find("extra point")?;
	match kick_result(&text[at..]) {
		Kick::Made => None,
		other => Some(other),
	}
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kick {
	Made,
	Missed,
	Blocked,
}

fn kick_result(text: &str) -> Kick {
	if text.contains("blocked") {
		Kick::Blocked
	} else if text.contains("no good") {
		Kick::Missed
	} else {
		Kick::Made
	}
}

/// How the ball changed hands on a play, if it did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Turnover {
	Interception,
	Fumble,
	OnDowns,
}

pub(crate) fn turnover(play: &Play) -> Option<Turnover> {
	let text = text(play);
	if wiped_out(play) {
		None
	} else if text.contains("intercepted") || kind(play).contains("interception") {
		Some(Turnover::Interception)
	} else if lost_fumble(play, &text) {
		Some(Turnover::Fumble)
	} else {
		on_downs(play).then_some(Turnover::OnDowns)
	}
}

fn lost_fumble(play: &Play, text: &str) -> bool {
	let (Some(offense), Some(at)) = (play.possession.as_deref(), text.find("recovered by ")) else {
		return false;
	};
	let by = text[at + "recovered by ".len()..].split(['-', ' ']).next();
	text.contains("fumble") && by.is_some_and(|by| team(by) != offense)
}

/// A fourth-down try from scrimmage that came up short: no kick, no score,
/// and less gained than the line to gain. A wiped-out play gains nothing
/// (`gain`), so it is never one.
fn on_downs(play: &Play) -> bool {
	play.down == Some(4)
		&& is_snap(play)
		&& !is_punt(play)
		&& field_goal(play).is_none()
		&& !is_touchdown(play)
		&& play.distance.is_some_and(|distance| gain(play).is_some_and(|gained| gained < distance))
}

pub(crate) fn is_sack(play: &Play) -> bool {
	!wiped_out(play) && (kind(play) == "sack" || text(play).contains(" sacked"))
}

/// The team of the player an injury note names, from "NE-K.Byard has
/// returned to the game": the last `TEAM-Player` before "was injured" or
/// "has returned", else the first in the text; none when it names none.
pub(crate) fn injured_team(play: &Play) -> Option<String> {
	let original = play.text.as_deref()?;
	let lower = original.to_ascii_lowercase();
	let named = |text: &str| -> Vec<String> {
		text
			.split(|c: char| c.is_whitespace() || c == '(' || c == ';')
			.filter_map(|word| {
				let (team, player) = word.split_once('-')?;
				(matches!(team.len(), 2 | 3) && team.bytes().all(|b| b.is_ascii_uppercase()) && player.starts_with(|c: char| c.is_ascii_uppercase())).then(|| self::team(team))
			})
			.collect()
	};
	let note = ["was injured", "has returned"].into_iter().filter_map(|word| lower.find(word)).min();
	note.and_then(|at| named(&original[..at]).pop()).or_else(|| named(original).into_iter().next())
}

/// Whether the play's text says a player was hurt, or came back.
pub(crate) fn injury(play: &Play) -> Option<bool> {
	let text = text(play);
	if text.contains("has returned") {
		Some(false)
	} else if text.contains("was injured") || text.contains("injury") {
		Some(true)
	} else {
		None
	}
}

/// The game clock in seconds, from `1:52`.
pub(crate) fn clock_seconds(clock: &str) -> Option<i64> {
	let (minutes, seconds) = clock.split_once(':')?;
	Some(minutes.trim().parse::<i64>().ok()? * 60 + seconds.trim().parse::<i64>().ok()?)
}

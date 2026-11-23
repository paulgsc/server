//! Home's week file for the Game Week essay, in the mailbox's `index` folder.
//!
//! Each ended week's games, their bites and the plays home matched them to,
//! put after each pass (paulgsc/some-ui#2074, the essay pipeline of D1,
//! 2026-11-16; agreed with the essay's template, `some-ui.game-essay/1`).
//!
//! The essay's writer, a routine in the cloud, can't reach home, so it reads
//! the week here and writes only what a model must: each bite's stance and
//! the host's lines. Everything else is home's, and comes from home's own
//! records: the games of the week from the scoreboard home kept, each bite
//! with what home heard in it, and the play the reconciler matched it to
//! with that play's still (the field, the down, the score and the clock at
//! its snap).
//!
//! `{"schema":"some-ui.routine-index/1","routine":"game-week","asOf","weeks":[…]}`:
//! `weeks` is every week among the [`NEWEST_WEEKS`] newest that have ended
//! and are ready (below), newest first, each a [`Week`]; `asOf` is the end of
//! the newest listed, or `null` when none is.
//!
//! **A week is an NFL week, Tuesday to Monday** in [`ZONE`], so a Monday
//! night game is in the essay of its Sunday (C1241). Its `id` is the ISO week
//! that holds its Sunday (`2026-w47`), `from` its Tuesday and `to` the
//! Monday after. Its games are those of the newest scoreboard home kept by
//! the week's end whose kickoff is in the week, and only those with a bite of
//! the subject's: SF's first, then by kickoff (D3).
//!
//! **Only a week that is ready.** A week is listed once it has ended, every
//! game in it with a bite is final at home, every bite is matched under this
//! build's `ENGINE_VERSION` (the reconciler matches a final game within
//! `reconcile::TICK`), and every bite's take has an ending: heard, found
//! silent, or failed for good. Until then it is left out, so the writer never
//! writes from a week still moving. A bite that reaches home after its week
//! was listed changes the file, and the next pass puts it again.
//!
//! **Each bite** carries where the essay files it ([`Placement`]), the
//! scenario (its own or the one home mapped to it, else `null`), how it was
//! chosen, its unit, whether the owner was watching, what home made of the
//! take (`heard`: `heard`, `garbled`, `nothing_said` or `failed`), its
//! transcript exactly when it was heard, the mean of its words'
//! probabilities (`confidence`, D5: code decides whether it is read aloud;
//! `null` for words kept before probabilities were), and the play it was
//! matched to: ESPN's id and text (the essay checks its host never repeats
//! it, and never shows or says it) and its [`Still`]. A bite matched to no
//! play carries `null` (D12).
//!
//! **A view, not a record**, as every index is (the parent module): no table
//! keeps it, and what home remembers is a [`LastIndex`] in memory. The marker
//! is the SHA-256 of the bytes, so a pass puts only when they change, and
//! once after a restart.
//!
//! **What it carries, and what leaves home.** The subject's own bites: a take
//! is named by its voice note's id, never audio (docs/identity.md invariant
//! 23), and what the owner said goes as its transcript, which no game bite's
//! did before. That is the owner's decision 2 of 2026-10-26 (a weekly bundle
//! in the mailbox with each bite's transcript, no audio, paulgsc/some-ui#2082)
//! as D1 of 2026-11-16 made it the essay's week; invariant 19 says so. Which
//! folder it goes in waits on the owner (paulgsc/some-ui#3441). ESPN's play
//! text is the league's, read for the check alone. No peer address or client
//! id is in it, and nothing of it reaches a log line.
//!
//! **Bounded per pass.** Per week of the [`NEWEST_WEEKS`], the newest
//! scoreboard kept by its end (`game_feed_repo::week`, which walks the kept
//! responses back from the newest by their key: for a week three weeks old,
//! every response since, a few thousand rows); per game of it, one indexed
//! read of its bites; per game with a bite of the subject's, its newest
//! summary parsed for its final, its play log by `game_feed_play_by_game` (a
//! few hundred rows), and per bite a few reads by key. A pass writes nothing.
//! It runs with the mailbox pass, hourly while the bell is held, and the
//! computing is the same whether or not anything moved.
//!
//! **Too big:** a body over the mailbox's ceiling drops its oldest weeks until
//! it fits; one whose newest week alone is over it is not put, and is counted
//! as refused (`too_big`), as every index is.

use chrono::{DateTime, Datelike, Days, NaiveDate, SecondsFormat, TimeZone, Utc};
use game_scenarios::{Half, Phase, Scenario, Unit};
use mailbox_kit::{Mailbox, MailboxTransport};
use nfl_feed::log::Entry;
use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::{SqliteConnection, SqlitePool};
use std::collections::HashSet;
use voice_repo::{
	bite_match::{self, GameBite},
	transcript::{self, Hearing},
	verdict::{self, Kind},
	How, VoiceError,
};

use super::{index_name, put_index, stamp_of, Computed, LastIndex, INDEX_SCHEMA};
use crate::mailbox::pass::{Failure, Outcome, MAILBOX_BODY_CEILING};
use crate::websocket::game_live::offer::{listing, TEAM};

/// The routine this index is for, as its name and `routine` say.
pub const GAME_WEEK: &str = "game-week";

/// How many of the newest ended weeks a pass looks at: the week just ended,
/// and three before it, so a writer that missed a Tuesday or three still
/// finds the week it owes, as the Rust Rounds index does.
pub const NEWEST_WEEKS: u64 = 4;

/// The zone a week's days are in: the owner's, as every essay's week is.
pub const ZONE: chrono_tz::Tz = chrono_tz::America::Los_Angeles;

/// Where the essay files a bite (agreed with the template, C1242): around
/// the game's halves, or in one of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Placement {
	/// Before the game's first play.
	Before,
	First,
	Second,
	Overtime,
	/// Once ESPN's End of Game is in.
	After,
}

impl From<Half> for Placement {
	fn from(half: Half) -> Self {
		match half {
			Half::First => Self::First,
			Half::Second => Self::Second,
			Half::Overtime => Self::Overtime,
		}
	}
}

/// A score, the away side's and the home side's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Score {
	pub away: i64,
	pub home: i64,
}

/// Where the ball was snapped: the yard line and whose side of the field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Spot {
	pub side: String,
	pub yard: i64,
}

/// A play's still (D13), the template's `StillSchema`.
///
/// The quarter, the clock, the team with the ball, the down and distance and
/// the spot at the snap, and the score before it. Numbers only, never the
/// play's words.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Still {
	pub period: i64,
	pub clock: String,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub possession: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub down: Option<i64>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub distance: Option<i64>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub spot: Option<Spot>,
	pub score: Score,
}

/// The play a bite was matched to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Play {
	/// ESPN's id for the play.
	pub id: String,
	/// ESPN's text: for the essay's check alone, never drawn or said.
	pub text: String,
	pub still: Still,
}

/// What home made of a bite's take, as its verdict names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Heard {
	Heard,
	Garbled,
	NothingSaid,
	Failed,
}

impl From<Kind> for Heard {
	fn from(kind: Kind) -> Self {
		match kind {
			Kind::Heard => Self::Heard,
			Kind::Garbled => Self::Garbled,
			Kind::NothingSaid => Self::NothingSaid,
			Kind::Failed => Self::Failed,
		}
	}
}

/// One bite of a game, in the order it was said.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Bite {
	/// The voice note's id.
	pub take: String,
	/// When the owner started saying it, on the phone's clock.
	pub said_at: String,
	pub placement: Placement,
	/// Its own scenario, or the one home mapped to it; `null` for a bite said
	/// first and never mapped.
	pub scenario: Option<String>,
	pub how: How,
	pub unit: Option<Unit>,
	pub unit_side: Option<String>,
	pub watching: bool,
	pub heard: Heard,
	/// What home heard, exactly when `heard` is `heard`.
	pub transcript: Option<String>,
	/// The mean of the heard words' probabilities, 0 to 1; `null` with no
	/// transcript, or words kept before their probabilities were.
	pub confidence: Option<f64>,
	pub play: Option<Play>,
}

/// One game of the week with a bite.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Game {
	/// ESPN's event id.
	pub id: String,
	pub away: String,
	pub home: String,
	pub kickoff: String,
	/// The score after the last play home holds, the game being final; the
	/// scoreboard's when no play carries one.
	#[serde(rename = "final")]
	pub final_score: Score,
	/// The score at the half's end; `null` when home kept no play of the
	/// first half.
	pub halftime: Option<Score>,
	/// The side SF wants to win: SF in its own game, else `null` (home makes
	/// no other reading per week yet; `websocket::game_live::offer`).
	pub wants: Option<String>,
	pub bites: Vec<Bite>,
}

/// One ended week.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Week {
	pub id: String,
	pub from: String,
	pub to: String,
	/// The team every game is read through.
	#[serde(rename = "for")]
	pub for_team: String,
	pub games: Vec<Game>,
}

#[derive(Serialize)]
struct GameWeekIndex<'a> {
	schema: &'static str,
	routine: &'static str,
	#[serde(rename = "asOf")]
	as_of: Option<String>,
	weeks: &'a [Week],
}

/// A Tuesday-to-Monday week: its first and last days, in [`ZONE`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Days7 {
	pub from: NaiveDate,
	pub to: NaiveDate,
}

impl Days7 {
	/// The week `day` is in.
	#[must_use]
	pub fn of(day: NaiveDate) -> Self {
		// Tuesday is 0, Monday 6.
		let into = (day.weekday().num_days_from_monday() + 6) % 7;
		let from = day - Days::new(u64::from(into));
		Self { from, to: from + Days::new(6) }
	}

	/// The week before.
	#[must_use]
	pub fn before(self) -> Self {
		Self::of(self.from - Days::new(1))
	}

	/// The ISO week of its Sunday, as the essay names a week (`2026-w47`).
	#[must_use]
	pub fn id(self) -> String {
		let sunday = (self.from + Days::new(5)).iso_week();
		let week = sunday.week();
		[sunday.year().to_string(), String::from(if week < 10 { "-w0" } else { "-w" }), week.to_string()].concat()
	}

	/// When it starts and ends: midnight in [`ZONE`] on its Tuesday, and on
	/// the Tuesday after.
	#[must_use]
	pub fn span(self) -> Option<(DateTime<Utc>, DateTime<Utc>)> {
		let midnight = |day: NaiveDate| ZONE.from_local_datetime(&day.and_hms_opt(0, 0, 0)?).earliest().map(|at| at.with_timezone(&Utc));
		Some((midnight(self.from)?, midnight(self.to + Days::new(1))?))
	}
}

/// The [`NEWEST_WEEKS`] newest weeks that have ended by `now`, newest first.
#[must_use]
pub fn ended_weeks(now: DateTime<Utc>) -> Vec<Days7> {
	let mut week = Days7::of(now.with_timezone(&ZONE).date_naive());
	(0..NEWEST_WEEKS)
		.map(|_| {
			week = week.before();
			week
		})
		.collect()
}

/// An instant as home writes its own: UTC, to the millisecond.
fn stamp(at: DateTime<Utc>) -> String {
	at.to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn instant(at: &str) -> Option<DateTime<Utc>> {
	DateTime::parse_from_rfc3339(at).ok().map(|at| at.with_timezone(&Utc))
}

/// Each play's newest version in `log`, in the game's order: ESPN's
/// sequence (a play with none first, as `nfl_feed::summary` orders them),
/// then where home's log holds it.
fn latest(log: &[Entry]) -> Vec<&Entry> {
	let revised: HashSet<usize> = log.iter().filter_map(|entry| entry.supersedes).collect();
	let mut plays: Vec<(usize, &Entry)> = log.iter().enumerate().filter(|(at, _)| !revised.contains(at)).collect();
	plays.sort_by_key(|(at, entry)| (entry.play.sequence, *at));
	plays.into_iter().map(|(_, entry)| entry).collect()
}

/// The score after `entry`, if ESPN gave both sides'.
fn score_after(entry: &Entry) -> Option<Score> {
	Some(Score {
		away: entry.play.away_score?,
		home: entry.play.home_score?,
	})
}

/// The spot of a snap from scrimmage `to_endzone` yards from the end zone
/// `offense` attacks, `defense` the other side: on the offense's side past
/// midfield back, the defense's short of it, the offense's at the 50.
fn spot(to_endzone: i64, offense: &str, defense: &str) -> Option<Spot> {
	if !(1..=99).contains(&to_endzone) {
		return None;
	}
	Some(if to_endzone >= 50 {
		Spot {
			side: offense.to_owned(),
			yard: 100 - to_endzone,
		}
	} else {
		Spot {
			side: defense.to_owned(),
			yard: to_endzone,
		}
	})
}

/// Whether `clock` reads as the template's clock does (`4:05`, `15:00`).
fn is_clock(clock: &str) -> bool {
	clock
		.split_once(':')
		.is_some_and(|(minutes, seconds)| (1..=2).contains(&minutes.len()) && seconds.len() == 2 && minutes.bytes().chain(seconds.bytes()).all(|b| b.is_ascii_digit()))
}

/// The play `id` of `plays` (newest versions, in order), with its still;
/// `None` when it is not there, or lacks its quarter, its clock or its text.
fn matched_play(plays: &[&Entry], id: &str, away: &str, home: &str) -> Option<Play> {
	let at = plays.iter().position(|entry| entry.play.id == id)?;
	let play = &plays[at].play;
	let clock = play.clock.clone().filter(|clock| is_clock(clock))?;
	let period = play.period.filter(|period| *period >= 1)?;
	let text = play.text.clone().filter(|text| !text.trim().is_empty())?;
	let score = plays[..at].iter().rev().find_map(|entry| score_after(entry)).unwrap_or(Score { away: 0, home: 0 });
	let possession = play.possession.clone().filter(|team| team == away || team == home);
	let (down, distance, spot) = match (&possession, play.down.filter(|down| (1..=4).contains(down))) {
		(Some(offense), Some(down)) => {
			let defense = if offense == away { home } else { away };
			(
				Some(down),
				play.distance.filter(|distance| (0..=99).contains(distance)),
				play.yards_to_endzone.and_then(|to_endzone| spot(to_endzone, offense, defense)),
			)
		}
		_ => (None, None, None),
	};
	Some(Play {
		id: id.to_owned(),
		text,
		still: Still {
			period,
			clock,
			possession,
			down,
			distance,
			spot,
			score,
		},
	})
}

/// Where the essay files `bite` (C1242): a pregame bite before the game, an
/// after-the-final one after it; any other in its matched play's half, or
/// else by what home had seen when it was said: before the first play,
/// after ESPN's End of Game, otherwise the half of the latest quarter seen.
fn placement(bite: &GameBite, matched: Option<&Play>, log: &[Entry]) -> Placement {
	// By the id's own phase, so an id this build's catalog no longer holds
	// still files where it was said.
	let phase = bite.scenario().and_then(|id| {
		Scenario::from_id(id).map(|scenario| scenario.entry().phase).or_else(|| match id.split_once('.')?.0 {
			"pre" => Some(Phase::Pre),
			"in" => Some(Phase::In),
			"post" => Some(Phase::Post),
			"you" => Some(Phase::You),
			_ => None,
		})
	});
	match phase {
		Some(Phase::Pre) => return Placement::Before,
		Some(Phase::Post) => return Placement::After,
		Some(Phase::In) | None => {
			if let Some(half) = matched.and_then(|play| Half::of_period(play.still.period)) {
				return half.into();
			}
		}
		Some(Phase::You) => {}
	}
	let said = instant(&bite.bite.started).map(|at| at + chrono::TimeDelta::milliseconds(bite.bite.offset.unwrap_or(0)));
	let seen: Vec<&Entry> = log
		.iter()
		.filter(|entry| instant(&entry.seen_at).zip(said).is_some_and(|(seen, said)| seen <= said))
		.collect();
	if seen.iter().any(|entry| entry.play.play_type.as_deref() == Some("End of Game")) {
		return Placement::After;
	}
	seen
		.iter()
		.filter_map(|entry| entry.play.period)
		.max()
		.and_then(Half::of_period)
		.map_or(Placement::Before, Placement::from)
}

/// The mean of `p` over the words that have one; `None` for none.
fn confidence(words: &[transcript::Word]) -> Option<f64> {
	let known: Vec<f64> = words.iter().filter_map(|word| word.p).collect();
	#[allow(clippy::cast_precision_loss)] // A take holds a few hundred words.
	let count = known.len() as f64;
	(!known.is_empty()).then(|| known.iter().sum::<f64>() / count)
}

/// Why a week is not ready yet, or a failure to read it.
enum NotYet {
	Waiting,
	Storage(VoiceError),
}

impl From<VoiceError> for NotYet {
	fn from(err: VoiceError) -> Self {
		Self::Storage(err)
	}
}

impl From<sqlx::Error> for NotYet {
	fn from(err: sqlx::Error) -> Self {
		Self::Storage(VoiceError::from(err))
	}
}

/// `bite` as the week file carries it, or [`NotYet::Waiting`] while it is not
/// matched under this build's engine or its take has no ending.
async fn bite_of(conn: &mut SqliteConnection, bite: &GameBite, plays: &[&Entry], log: &[Entry], (away, home): (&str, &str)) -> Result<Option<Bite>, NotYet> {
	let Some(matched) = bite_match::newest_match(conn, &bite.subject, &bite.note_id).await? else {
		return Err(NotYet::Waiting);
	};
	// Matched under an older engine, or before the scenario home mapped to
	// it since: the reconciler matches it again within its tick. A newer
	// engine's match (home rolled back) stands, as the reconciler leaves it.
	if matched.engine_version < i64::from(scenario_engine::ENGINE_VERSION) || matched.scenario.as_deref() != bite.scenario() {
		return Err(NotYet::Waiting);
	}
	let verdict = verdict::of_note(conn, &bite.subject, &bite.note_id).await?;
	let (heard, transcript) = match transcript::heard(conn, &bite.subject, &bite.note_id).await? {
		// Let go of since its game was read: not in the week.
		Hearing::NotHeld => return Ok(None),
		Hearing::Waiting => match verdict.map(|verdict| verdict.kind) {
			Some(Kind::Failed) => (Heard::Failed, None),
			_ => return Err(NotYet::Waiting),
		},
		Hearing::Heard(heard) => {
			let kind = verdict.map_or_else(
				|| if heard.text.trim().is_empty() { Heard::NothingSaid } else { Heard::Heard },
				|verdict| verdict.kind.into(),
			);
			let text = (kind == Heard::Heard && !heard.text.trim().is_empty()).then_some(heard.text);
			(if kind == Heard::Heard && text.is_none() { Heard::NothingSaid } else { kind }, text)
		}
	};
	let confidence = match transcript {
		Some(_) => confidence(&transcript::words(conn, &bite.subject, &bite.note_id).await?),
		None => None,
	};
	let play = matched.play.as_deref().and_then(|id| matched_play(plays, id, away, home));
	Ok(Some(Bite {
		take: bite.note_id.clone(),
		said_at: bite.bite.started.clone(),
		placement: placement(bite, play.as_ref(), log),
		scenario: bite.scenario().map(str::to_owned),
		how: bite.bite.how,
		unit: bite.bite.unit,
		unit_side: bite.bite.unit_side.clone(),
		watching: bite.bite.watching,
		heard,
		transcript,
		confidence,
		play,
	}))
}

/// `subject`'s `week`, or `None` when it holds no bite of theirs; a week not
/// ready yet is [`NotYet::Waiting`].
async fn week_of(conn: &mut SqliteConnection, subject: &str, week: Days7) -> Result<Option<Week>, NotYet> {
	let Some((start, end)) = week.span() else {
		return Ok(None);
	};
	let Some(board) = game_feed_repo::week(conn, &stamp(end)).await? else {
		return Ok(None);
	};
	let mut games = Vec::new();
	for game in &board.games {
		let Some(kickoff) = game.kickoff.filter(|kickoff| (start..end).contains(kickoff)) else {
			continue;
		};
		let Some(listed) = listing(game) else {
			continue;
		};
		let bites: Vec<GameBite> = bite_match::bites_of_game(conn, &game.id)
			.await?
			.into_iter()
			.filter(|bite| bite.subject == subject)
			.collect();
		if bites.is_empty() {
			continue;
		}
		if !game_feed_repo::holds_final(conn, &game.id).await? {
			return Err(NotYet::Waiting);
		}
		let log = game_feed_repo::game_log(conn, &game.id).await?;
		let plays = latest(&log);
		let mut said = Vec::with_capacity(bites.len());
		for bite in &bites {
			if let Some(bite) = bite_of(conn, bite, &plays, &log, (&listed.away, &listed.home)).await? {
				said.push(bite);
			}
		}
		if said.is_empty() {
			continue;
		}
		said.sort_by(|a, b| (instant(&a.said_at), &a.take).cmp(&(instant(&b.said_at), &b.take)));
		let last = plays.iter().rev().find_map(|entry| score_after(entry));
		// The final summary's last score, which home holds final; else the
		// scoreboard's.
		let board_score = game.away.score.zip(game.home.score).map(|(away, home)| Score { away, home });
		let final_score = last.or(board_score).unwrap_or(Score { away: 0, home: 0 });
		let halftime = plays
			.iter()
			.rev()
			.filter(|entry| entry.play.period.is_some_and(|period| period <= 2))
			.find_map(|entry| score_after(entry));
		games.push((
			kickoff,
			Game {
				id: game.id.clone(),
				away: listed.away,
				home: listed.home,
				kickoff: stamp(kickoff),
				final_score,
				halftime,
				wants: listed.want,
				bites: said,
			},
		));
	}
	if games.is_empty() {
		return Ok(None);
	}
	// SF's first, then by kickoff (D3).
	games.sort_by(|(a_at, a), (b_at, b)| {
		let ours = |game: &Game| game.away != TEAM && game.home != TEAM;
		(ours(a), a_at, &a.id).cmp(&(ours(b), b_at, &b.id))
	});
	Ok(Some(Week {
		id: week.id(),
		from: week.from.to_string(),
		to: week.to.to_string(),
		for_team: TEAM.to_owned(),
		games: games.into_iter().map(|(_, game)| game).collect(),
	}))
}

/// The index's bytes for `weeks` (newest first), `ended` being the end of
/// the newest.
#[must_use]
pub fn game_week_index(weeks: &[Week], ended: Option<DateTime<Utc>>) -> Vec<u8> {
	let index = GameWeekIndex {
		schema: INDEX_SCHEMA,
		routine: GAME_WEEK,
		as_of: ended.map(stamp),
		weeks,
	};
	let mut body = Vec::new();
	// Structs of strings, numbers and booleans always serialise.
	let _ = serde_json::to_writer(&mut body, &index);
	body
}

/// The bytes to put for `weeks`: every week while they fit under the
/// mailbox's ceiling, else without the oldest until they do or one is left.
fn fitted(mut weeks: Vec<Week>, ended: Option<DateTime<Utc>>) -> Vec<u8> {
	let mut body = game_week_index(&weeks, ended);
	while body.len() > MAILBOX_BODY_CEILING && weeks.len() > 1 {
		weeks.pop();
		body = game_week_index(&weeks, ended);
	}
	body
}

/// Puts `subject`'s ready weeks when they changed since the last index the
/// mailbox answered (the module doc). Before any week is ready, it puts none.
///
/// # Errors
/// A [`Failure`] when the database fails or the mailbox cannot be reached;
/// nothing is remembered, so the next pass computes and puts it again.
pub(in crate::mailbox) async fn put_game_week_index<T: MailboxTransport>(
	db: &SqlitePool,
	mailbox: &Mailbox<T>,
	subject: &str,
	last: &LastIndex,
	now: &(impl Fn() -> String + Sync),
	outcome: &mut Outcome,
) -> Result<(), Failure> {
	let at = now();
	let (Some(stamp_now), Some(when)) = (stamp_of(&at), instant(&at)) else {
		return Ok(());
	};
	let mut conn = db.acquire().await.map_err(Failure::Storage)?;
	let mut weeks = Vec::new();
	let mut ended = None;
	for week in ended_weeks(when) {
		match week_of(&mut conn, subject, week).await {
			Ok(Some(found)) => {
				if ended.is_none() {
					ended = week.span().map(|(_, end)| end);
				}
				weeks.push(found);
			}
			Ok(None) | Err(NotYet::Waiting) => {}
			Err(NotYet::Storage(VoiceError::Storage(err))) => return Err(Failure::Storage(err)),
			// A stored row this build does not read: the week waits, said once a pass.
			Err(NotYet::Storage(_)) => tracing::error!(
				kind = "unreadable",
				"a game week holds a row that does not read back; it is left out of the game-week index"
			),
		}
	}
	drop(conn);
	// No week is ready: no index, and the writer writes none.
	if weeks.is_empty() {
		return Ok(());
	}
	let body = fitted(weeks, ended);
	let marker = hex::encode(Sha256::digest(&body));
	if last.holds(subject, Some(&marker), |_| true) {
		return Ok(());
	}
	let computed = Computed {
		subject: subject.to_owned(),
		as_of: Some(marker),
		valid: None,
	};
	put_index(mailbox, &index_name(GAME_WEEK, &stamp_now), body, (last, computed), outcome).await
}

#[cfg(test)]
mod tests {
	use super::{confidence, ended_weeks, fitted, game_week_index, is_clock, latest, spot, week_of, Days7, Heard, NotYet, Placement, Spot, Week};
	use crate::mailbox::pass::MAILBOX_BODY_CEILING;
	use chrono::{DateTime, NaiveDate, SecondsFormat, TimeDelta, Utc};
	use game_feed_repo::{Fetched, Response};
	use serde_json::{json, Value};
	use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};

	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");
	const FINAL: &[u8] = include_bytes!("../../../../../../crates/nfl_feed/testdata/summary-401873010-final.json");
	const GAME: &str = "401873010";
	const SUBJECT: &str = "s";

	fn day(text: &str) -> NaiveDate {
		NaiveDate::parse_from_str(text, "%Y-%m-%d").unwrap()
	}

	fn at(time: &str) -> DateTime<Utc> {
		DateTime::parse_from_rfc3339(time).unwrap().with_timezone(&Utc)
	}

	#[test]
	fn a_week_runs_tuesday_to_monday_and_is_named_for_its_sunday() {
		// Thursday 2026-10-22 (the recorded NE at CHI): Tuesday the 20th to Monday the 26th.
		let week = Days7::of(day("2026-10-22"));
		assert_eq!((week.from, week.to), (day("2026-10-20"), day("2026-10-26")));
		assert_eq!(Days7::of(day("2026-10-20")), week, "a Tuesday opens its week");
		assert_eq!(Days7::of(day("2026-10-26")), week, "a Monday night game is in its Sunday's week");
		assert_eq!(Days7::of(day("2026-10-27")).from, day("2026-10-27"));
		assert_eq!(week.id(), "2026-w43");
		// The ISO week of a Sunday in a new year's first days is that year's.
		assert_eq!(Days7::of(day("2027-01-03")).id(), "2026-w53");
		// Midnight in Los Angeles on each Tuesday, across the end of DST (Nov 1).
		assert_eq!(week.span().unwrap(), (at("2026-10-20T07:00:00Z"), at("2026-10-27T07:00:00Z")));
		assert_eq!(Days7::of(day("2026-10-28")).span().unwrap(), (at("2026-10-27T07:00:00Z"), at("2026-11-03T08:00:00Z")));
		assert_eq!(week.before().to, day("2026-10-19"));
	}

	#[test]
	fn the_ended_weeks_are_the_newest_four_newest_first() {
		// Monday night, Pacific: the week holding it has not ended.
		let weeks = ended_weeks(at("2026-10-27T06:59:00Z"));
		assert_eq!(weeks.iter().map(|week| week.id()).collect::<Vec<_>>(), ["2026-w42", "2026-w41", "2026-w40", "2026-w39"]);
		assert_eq!(ended_weeks(at("2026-10-27T07:00:00Z"))[0].id(), "2026-w43");
	}

	#[test]
	fn a_spot_is_on_the_side_the_ball_is() {
		let on = |side: &str, yard| Some(Spot { side: side.to_owned(), yard });
		assert_eq!(spot(55, "NE", "CHI"), on("NE", 45), "short of midfield is the offense's own side");
		assert_eq!(spot(45, "NE", "CHI"), on("CHI", 45));
		assert_eq!(spot(50, "NE", "CHI"), on("NE", 50));
		assert_eq!(spot(99, "NE", "CHI"), on("NE", 1));
		assert_eq!(spot(1, "NE", "CHI"), on("CHI", 1));
		assert_eq!([spot(0, "NE", "CHI"), spot(100, "NE", "CHI")], [None, None]);
		assert!(is_clock("4:05") && is_clock("15:00") && is_clock("0:00"));
		assert!(!is_clock("") && !is_clock("4:5") && !is_clock("123:00") && !is_clock("a:00"));
	}

	#[test]
	fn confidence_is_the_mean_of_the_words_that_have_one() {
		let word = |p: Option<f64>| voice_repo::transcript::Word {
			text: String::from("go"),
			start_ms: 0,
			end_ms: 1,
			p,
			language: None,
		};
		assert_eq!(confidence(&[word(Some(0.5)), word(Some(1.0)), word(None)]), Some(0.75));
		assert_eq!(confidence(&[word(None)]), None);
		assert_eq!(confidence(&[]), None);
	}

	async fn pool() -> SqlitePool {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();
		pool
	}

	fn stamp(at: DateTime<Utc>) -> String {
		at.to_rfc3339_opts(SecondsFormat::Millis, true)
	}

	fn board() -> Vec<u8> {
		let side = |home_away: &str, team: &str, score: &str| json!({ "homeAway": home_away, "team": { "abbreviation": team }, "score": score });
		json!({
			"season": { "year": 2026, "type": 2 }, "week": { "number": 8 },
			"events": [{
				"id": GAME, "date": "2026-10-23T00:15Z", "shortName": "NE @ CHI",
				"status": { "type": { "state": "post", "completed": true } },
				"competitions": [{ "competitors": [side("home", "CHI", "20"), side("away", "NE", "17")] }],
			}],
		})
		.to_string()
		.into_bytes()
	}

	async fn keep(db: &SqlitePool, fetched: Fetched<'_>, body: &[u8], fetched_at: &str) {
		let mut tx = db.begin_with("BEGIN IMMEDIATE").await.unwrap();
		let response = Response {
			fetched,
			url: "https://site.api.espn.com/apis/site/v2/sports/football/nfl/",
			fetched_at,
			status: 200,
			body,
		};
		game_feed_repo::keep(&mut tx, &response).await.unwrap();
		tx.commit().await.unwrap();
	}

	async fn bite(db: &SqlitePool, note: &str, scenario: Option<&str>, how: &str, started: &str) {
		sqlx::query(
			"INSERT INTO voice_note (subject_id, note_id, of_kind, of_ref, recorded_at, content_type, size, sha256, stored_at)
			 VALUES (?1, ?2, 'game', ?3, ?4, 'audio/webm', 1, 's', ?4)",
		)
		.bind(SUBJECT)
		.bind(note)
		.bind(GAME)
		.bind(started)
		.execute(db)
		.await
		.unwrap();
		sqlx::query(
			"INSERT INTO game_bite (subject_id, note_id, scenario_id, how, live_strip, watching, started_at, clock_offset_ms, mode, catalog_version, unit, unit_side)
			 VALUES (?1, ?2, ?3, ?4, '', 1, ?5, 0, 'standard', 3, 'coaching', 'NE')",
		)
		.bind(SUBJECT)
		.bind(note)
		.bind(scenario)
		.bind(how)
		.bind(started)
		.execute(db)
		.await
		.unwrap();
	}

	/// What home heard in `note`: its text and each word's probability.
	async fn heard(db: &SqlitePool, note: &str, text: &str, p: &[Option<f64>]) {
		sqlx::query(
			"INSERT INTO voice_transcript (subject_id, note_id, model, engine, attempt, outcome, language, language_p, duration_ms, text, took_ms, heard_at)
			 VALUES (?1, ?2, 'm', 'e', 1, 'heard', 'en', 0.9, 3000, ?3, 1, '2026-10-23T05:00:00.000Z')",
		)
		.bind(SUBJECT)
		.bind(note)
		.bind(text)
		.execute(db)
		.await
		.unwrap();
		for (i, p) in p.iter().enumerate() {
			sqlx::query("INSERT INTO voice_transcript_word (subject_id, note_id, model, word_index, text, start_ms, end_ms, p) VALUES (?1, ?2, 'm', ?3, 'w', 0, 1, ?4)")
				.bind(SUBJECT)
				.bind(note)
				.bind(i64::try_from(i).unwrap())
				.bind(p)
				.execute(db)
				.await
				.unwrap();
		}
	}

	async fn judged(db: &SqlitePool, note: &str, verdict: &str) {
		sqlx::query(
			"INSERT INTO voice_transcript_verdict (subject_id, note_id, model, attempt, rule, verdict, reasons, speech_ms, judged_at)
			 VALUES (?1, ?2, 'm', 1, 'heard-1', ?3, ?4, 3000, '2026-10-23T05:00:00.000Z')",
		)
		.bind(SUBJECT)
		.bind(note)
		.bind(verdict)
		.bind(if verdict == "garbled" { "low_confidence" } else { "" })
		.execute(db)
		.await
		.unwrap();
	}

	fn plays() -> Vec<nfl_feed::summary::Play> {
		nfl_feed::summary::parse(FINAL).unwrap().plays
	}

	/// The recorded game kept final, with three bites: one on a third and
	/// long in the first half, one said first and never mapped, one after
	/// the final; and the reconciler run over it.
	async fn recorded_week(db: &SqlitePool) {
		let plays = plays();
		let third = plays
			.iter()
			.find(|p| p.down == Some(3) && p.distance.is_some_and(|d| d >= 7) && p.play_type.as_deref() == Some("Pass Incompletion") && p.period == Some(1))
			.unwrap();
		let wall = |play: &nfl_feed::summary::Play| at(play.wallclock.as_deref().unwrap());
		let last = wall(plays.last().unwrap());
		keep(db, Fetched::Scoreboard, &board(), "2026-10-22T23:00:00.000Z").await;
		// Live once the third and long was in, as home kept it then.
		let mut live: Value = serde_json::from_slice(FINAL).unwrap();
		let through = third.sequence.unwrap();
		for drive in live["drives"]["previous"].as_array_mut().unwrap() {
			drive["plays"].as_array_mut().unwrap().retain(|play| {
				play["sequenceNumber"]
					.as_str()
					.and_then(|n| n.parse::<i64>().ok())
					.or_else(|| play["sequenceNumber"].as_i64())
					.is_some_and(|n| n <= through)
			});
		}
		live["header"]["competitions"][0]["status"]["type"]["state"] = json!("in");
		live["header"]["competitions"][0]["status"]["type"]["completed"] = json!(false);
		keep(db, Fetched::Summary(GAME), live.to_string().as_bytes(), &stamp(wall(third) + TimeDelta::seconds(2))).await;
		keep(db, Fetched::Summary(GAME), FINAL, &stamp(last + TimeDelta::minutes(5))).await;
		bite(db, "b1", Some("in.third-long"), "just-happened", &stamp(wall(third) + TimeDelta::seconds(4))).await;
		bite(db, "b2", None, "said-first", &stamp(wall(third) + TimeDelta::minutes(30))).await;
		bite(db, "b3", Some("post.result"), "every-scenario", &stamp(last + TimeDelta::minutes(10))).await;
		// Said first after the final summary was in: placed after the game.
		bite(db, "b5", None, "said-first", &stamp(last + TimeDelta::minutes(8))).await;
		heard(db, "b5", "What a finish.", &[Some(1.0)]).await;
		heard(db, "b1", "They have to throw it there.", &[Some(0.9), Some(0.7), Some(0.8)]).await;
		heard(db, "b2", "Something about the clock.", &[None, None]).await;
		heard(db, "b3", "uh mm", &[Some(0.2)]).await;
		judged(db, "b3", "garbled").await;
		crate::reconcile::reconcile(db, GAME, last + TimeDelta::minutes(20)).await.unwrap();
	}

	async fn week(db: &SqlitePool) -> Result<Option<Week>, NotYet> {
		let mut conn = db.acquire().await.unwrap();
		week_of(&mut conn, SUBJECT, Days7::of(day("2026-10-22"))).await
	}

	#[tokio::test]
	async fn a_ready_week_carries_each_bite_its_take_and_its_play() {
		let db = pool().await;
		recorded_week(&db).await;
		let week = week(&db).await.ok().flatten().unwrap();
		let body: Value = serde_json::from_slice(&game_week_index(std::slice::from_ref(&week), week_span_end(&week))).unwrap();
		assert_eq!(body["schema"], "some-ui.routine-index/1");
		assert_eq!(body["routine"], "game-week");
		assert_eq!(body["asOf"], "2026-10-27T07:00:00.000Z");
		let week = &body["weeks"][0];
		assert_eq!(
			(&week["id"], &week["from"], &week["to"], &week["for"]),
			(&json!("2026-w43"), &json!("2026-10-20"), &json!("2026-10-26"), &json!("SF"))
		);
		let game = &week["games"][0];
		assert_eq!((&game["away"], &game["home"], &game["wants"]), (&json!("NE"), &json!("CHI"), &Value::Null));
		let last = plays().into_iter().rev().find_map(|p| p.away_score.zip(p.home_score)).unwrap();
		assert_eq!(game["final"], json!({ "away": last.0, "home": last.1 }), "the final summary's, over the scoreboard's");
		assert_eq!(game["kickoff"], "2026-10-23T00:15:00.000Z");
		let half = plays()
			.into_iter()
			.rev()
			.filter(|p| p.period.is_some_and(|q| q <= 2))
			.find_map(|p| p.away_score.zip(p.home_score))
			.unwrap();
		assert_eq!(game["halftime"], json!({ "away": half.0, "home": half.1 }), "the score after the first half's last play");
		let bites = game["bites"].as_array().unwrap();
		assert_eq!(bites.iter().map(|b| b["take"].as_str().unwrap()).collect::<Vec<_>>(), ["b1", "b2", "b5", "b3"]);
		assert_eq!(bites[2]["placement"], "after", "said once home had seen End of Game");

		let first = &bites[0];
		assert_eq!(
			(&first["placement"], &first["scenario"], &first["how"]),
			(&json!("first"), &json!("in.third-long"), &json!("just-happened"))
		);
		assert_eq!((&first["unit"], &first["unitSide"], &first["watching"]), (&json!("coaching"), &json!("NE"), &json!(true)));
		assert_eq!((&first["heard"], &first["transcript"]), (&json!("heard"), &json!("They have to throw it there.")));
		assert!((first["confidence"].as_f64().unwrap() - 0.8).abs() < 1e-9);
		let play = &first["play"];
		assert!(!play["text"].as_str().unwrap().is_empty());
		let still = &play["still"];
		assert_eq!((&still["period"], &still["down"]), (&json!(1), &json!(3)));
		assert!(still["distance"].as_i64().unwrap() >= 7);
		let spot_side = still["spot"]["side"].as_str().unwrap();
		assert!(spot_side == "NE" || spot_side == "CHI");
		assert!(still["score"]["away"].is_i64());

		let said_first = &bites[1];
		assert_eq!(
			(&said_first["scenario"], &said_first["play"]),
			(&Value::Null, &Value::Null),
			"said first and never mapped: no scenario, no play"
		);
		assert_eq!(said_first["confidence"], Value::Null, "words kept before their probabilities were");
		assert_eq!(said_first["placement"], "first", "placed by the quarter home had seen when it was said");

		let after = &bites[3];
		assert_eq!(
			(&after["placement"], &after["heard"], &after["transcript"], &after["confidence"]),
			(&json!("after"), &json!("garbled"), &Value::Null, &Value::Null)
		);
	}

	fn week_span_end(week: &Week) -> Option<DateTime<Utc>> {
		Days7::of(day(&week.from)).span().map(|(_, end)| end)
	}

	#[tokio::test]
	async fn a_week_waits_for_every_bite_to_be_matched_and_heard() {
		let db = pool().await;
		recorded_week(&db).await;
		// A bite that reached home after the reconciler's pass: no match yet.
		bite(&db, "b4", Some("in.third-long"), "just-happened", "2026-10-23T02:00:00.000Z").await;
		assert!(matches!(week(&db).await, Err(NotYet::Waiting)));
		crate::reconcile::reconcile(&db, GAME, at("2026-10-24T00:00:00Z")).await.unwrap();
		// Matched, and not yet heard.
		assert!(matches!(week(&db).await, Err(NotYet::Waiting)));
		heard(&db, "b4", "Again.", &[Some(1.0)]).await;
		assert!(week(&db).await.ok().flatten().is_some());
		// A bite matched under an older engine waits for its re-match.
		sqlx::query(
			"INSERT INTO bite_match (subject_id, note_id, scenario_id, play_id, method, gap_seconds, engine_version, matched_at)
			 VALUES (?1, 'b4', 'in.third-long', NULL, 'unmatched', NULL, 1, '2026-10-25T00:00:00.000Z')",
		)
		.bind(SUBJECT)
		.execute(&db)
		.await
		.unwrap();
		assert!(matches!(week(&db).await, Err(NotYet::Waiting)));
		// One matched under a newer engine (home rolled back) stands.
		sqlx::query(
			"INSERT INTO bite_match (subject_id, note_id, scenario_id, play_id, method, gap_seconds, engine_version, matched_at)
			 VALUES (?1, 'b4', 'in.third-long', NULL, 'unmatched', NULL, 9999, '2026-10-25T00:01:00.000Z')",
		)
		.bind(SUBJECT)
		.execute(&db)
		.await
		.unwrap();
		assert!(week(&db).await.ok().flatten().is_some());
		// A scenario mapped to a bite said first after its match: it waits for
		// the match under that scenario.
		sqlx::query(
			"INSERT INTO bite_mapping (subject_id, note_id, scenario_id, proposed_by, catalog_version, engine_version, mapped_at)
			 VALUES (?1, 'b2', 'in.sack', 'shot-writer', 3, 1, '2026-10-25T00:02:00.000Z')",
		)
		.bind(SUBJECT)
		.execute(&db)
		.await
		.unwrap();
		assert!(matches!(week(&db).await, Err(NotYet::Waiting)));
	}

	#[tokio::test]
	async fn a_week_with_no_bite_of_the_subjects_or_no_final_is_not_listed() {
		let db = pool().await;
		keep(&db, Fetched::Scoreboard, &board(), "2026-10-22T23:00:00.000Z").await;
		assert!(matches!(week(&db).await, Ok(None)), "a scoreboard and no bite");
		sqlx::query(
			"INSERT INTO voice_note (subject_id, note_id, of_kind, of_ref, recorded_at, content_type, size, sha256, stored_at)
			 VALUES ('another', 'x1', 'game', ?1, 'then', 'audio/webm', 1, 's', 'then')",
		)
		.bind(GAME)
		.execute(&db)
		.await
		.unwrap();
		sqlx::query(
			"INSERT INTO game_bite (subject_id, note_id, scenario_id, how, live_strip, watching, started_at, mode, catalog_version)
			 VALUES ('another', 'x1', 'pre.weather', 'offered', '', 1, '2026-10-22T23:30:00.000Z', 'standard', 3)",
		)
		.execute(&db)
		.await
		.unwrap();
		assert!(matches!(week(&db).await, Ok(None)), "another subject's bite is not this subject's week, final or not");
		bite(&db, "b1", Some("pre.weather"), "offered", "2026-10-22T23:30:00.000Z").await;
		assert!(matches!(week(&db).await, Err(NotYet::Waiting)), "a bite on a game home holds no final of");
	}

	#[test]
	fn a_body_over_the_ceiling_drops_its_oldest_weeks() {
		let week = |id: &str, filler: usize| Week {
			id: id.to_owned(),
			from: String::from("2026-10-20"),
			to: String::from("2026-10-26"),
			for_team: String::from("SF"),
			games: vec![super::Game {
				id: String::from("1"),
				away: String::from("NE"),
				home: String::from("CHI"),
				kickoff: String::from("2026-10-23T00:15:00.000Z"),
				final_score: super::Score { away: 0, home: 0 },
				halftime: None,
				wants: None,
				bites: vec![super::Bite {
					take: String::from("b"),
					said_at: String::from("2026-10-23T00:20:00.000Z"),
					placement: Placement::First,
					scenario: None,
					how: voice_repo::How::SaidFirst,
					unit: None,
					unit_side: None,
					watching: true,
					heard: Heard::Heard,
					transcript: Some("x".repeat(filler)),
					confidence: None,
					play: None,
				}],
			}],
		};
		let half = MAILBOX_BODY_CEILING / 2;
		let body: Value = serde_json::from_slice(&fitted(vec![week("2026-w43", half), week("2026-w42", half)], None)).unwrap();
		assert_eq!(body["weeks"].as_array().unwrap().len(), 1);
		assert_eq!(body["weeks"][0]["id"], "2026-w43");
		assert_eq!(body["asOf"], Value::Null);
	}

	#[test]
	fn a_revision_keeps_its_plays_place() {
		let entry = |id: &str, sequence: i64, supersedes: Option<usize>| nfl_feed::log::Entry {
			game_id: String::from(GAME),
			play: nfl_feed::summary::Play {
				id: id.to_owned(),
				sequence: Some(sequence),
				wallclock: None,
				period: Some(1),
				clock: None,
				down: None,
				distance: None,
				yard_line: None,
				yards_to_endzone: None,
				possession: None,
				play_type: None,
				scoring: None,
				away_score: None,
				home_score: None,
				text: None,
			},
			response_id: 1,
			seen_at: String::from("2026-10-23T00:20:00.000Z"),
			supersedes,
		};
		let log = [entry("a", 1, None), entry("b", 2, None), entry("a", 1, Some(0))];
		let order: Vec<(&str, Option<usize>)> = latest(&log).into_iter().map(|e| (e.play.id.as_str(), e.supersedes)).collect();
		assert_eq!(order, [("a", Some(0)), ("b", None)]);
	}
}

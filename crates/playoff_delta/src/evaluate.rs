//! A season, week by week, into the story contract ([`crate::story`]).
//!
//! Every number is the solver's ([`crate::solver`]) or the count's
//! ([`crate::count`]), asked of a different table; nothing here estimates
//! beyond the count's sample. The fan's words are defined once, below, and
//! each field's doc says which.
//!
//! - **Paths** of a team at a table: for `x = 0, 1, ...`, the best seed
//!   winning exactly `x` more. Rows run to [`WINDOW`], or further when the
//!   first way in needs more, so a claim read off the rows is never "out"
//!   when the team is alive.
//! - **Share**: of the ways the rest of the season can go, the share in which
//!   the team gets in ([`count`]). It is a fair game: over the ways a week
//!   can go, the share after it averages to the share before.
//! - **Leverage** of a result: the share as it happened minus the share had
//!   it gone the other way, the rest of the week held as it happened.
//! - **Spread** of a week: the share's standard deviation over the ways the
//!   week's results can go ([`count::Slates::spread`]). The league's change
//!   over its spread is the week's luck; the stage divides, so the contract
//!   carries each number once.
//! - **Clinched**: in, whatever happens ([`solver::clinched`]).
//! - **Ways** of a week: every way its results can go, each counted against
//!   the same rests of the season, for every team ([`count::slates`], read by
//!   [`crate::ways`]): the best way and what each game is worth in it, the
//!   road from it to the way the week went, and that way's rank among them.
//! - **Podium**: every team's week ranked among its own ways, among the
//!   teams the week could move by at least [`ELIGIBLE`].

use crate::{
	count::{self, Slates, Tally},
	league::Team,
	season::{Game, Outcome, Season},
	solver::{self, Memo},
	story::{
		Collapse, Given, Kind, Left, Medal, MedalPiece, Next, NextPiece, Piece, Played, Podium, Relation, Share, Stake, StoryGame, StoryPaths, StoryResult, StorySeason,
		StoryShare, StoryWeek, WayGame, Ways, WinLoss,
	},
	table::{Pending, Table},
	ways,
};
use std::{collections::BTreeMap, fmt, num::NonZeroUsize};

/// Rows a paths grid shows at least: "never win again" through "+4 wins".
pub const WINDOW: u8 = 4;

/// The ways a share is sampled over when there are more than that to count.
///
/// A share near one half then lands within about a third of a percentage
/// point (two standard errors), and a result's leverage, compared on the
/// same sampled ways, much closer.
pub const WAYS: u32 = 100_000;

/// The most rests of the season a week's slates are counted against.
///
/// With more than that to count, these are drawn ([`count::slates`]). On the
/// 2026 season a quarter as many moved a week's spread by about a tenth of
/// itself; this many costs a few seconds a season.
pub const RESTS: u32 = 16_384;

/// How many of next week's games a story names.
pub const NEXT: usize = 3;

/// A swing smaller than this (a twentieth of a point) is not named.
const MIN_SWING: f64 = 0.0005;

/// How far a team's week must be able to move its share (its standard
/// deviation over the week's ways: a point) for the team to be ranked on
/// the week's podium.
pub const ELIGIBLE: f64 = 0.01;

/// The pieces a medal carries, dearest first.
const MEDAL_PIECES: usize = 3;

/// Who the story is about.
#[derive(Debug, Clone)]
pub struct Subject {
	pub team: Team,
	/// Shown on screen: "2025".
	pub label: String,
	/// The most ways a share is sampled over ([`WAYS`] for the story).
	pub ways: u32,
	/// The most rests a week's spread is counted against ([`RESTS`] for the
	/// story).
	pub rests: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvaluateError {
	/// No week with a final result for the subject yet.
	NothingPlayed,
	/// More games than the contract's counters hold.
	SeasonTooLong,
	/// More than [`count::SLATE_GAMES`] games in a week, which a checked
	/// season cannot have.
	WeekTooFull,
}

impl fmt::Display for EvaluateError {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.write_str(match self {
			Self::NothingPlayed => "no completed week has a game for the team",
			Self::SeasonTooLong => "more than 255 games a team",
			Self::WeekTooFull => "more than 16 games in a week",
		})
	}
}

impl std::error::Error for EvaluateError {}

/// The story of `season` so far, for `subject`: one week per completed week,
/// the subject's bye included (its `game` is `None`).
///
/// # Errors
/// [`EvaluateError`].
pub fn evaluate(season: &Season, subject: &Subject) -> Result<StorySeason, EvaluateError> {
	let season_length = u8::try_from(season.length()).map_err(|_| EvaluateError::SeasonTooLong)?;
	let through = season.complete_through();
	if !(1..=through).any(|w| season.game_of(subject.team, w).is_some()) {
		return Err(EvaluateError::NothingPlayed);
	}
	let memo = Memo::default();
	let tables: Vec<Table> = (0..=through).map(|w| Table::after(season, w)).collect();
	// Each table's next week's games, both sides counted: the subject's own
	// one splits that week's change, the rest are the ones to root for. And
	// the week's ways: the spread, the best way, the road and the podium.
	let read = parallel(&tables, |t| {
		let next: Vec<usize> = (0..t.pending().len()).filter(|&i| t.pending()[i].week.checked_sub(1) == Some(t.through())).collect();
		Ok((count::tally(t, subject.team, subject.ways, &next), ahead(season, subject, t, through)?))
	})
	.into_iter()
	.collect::<Result<Vec<_>, EvaluateError>>()?;
	let mut tallies = Vec::with_capacity(read.len());
	let mut upcoming = Vec::with_capacity(read.len());
	// One per table but the last: the week after it has been played.
	let mut played = Vec::with_capacity(read.len());
	for (tally, (up, week)) in read {
		tallies.push(tally);
		upcoming.push(up);
		played.extend(week);
	}
	let weeks = (1..=through)
		.map(|week| {
			let w = usize::from(week);
			Week {
				season,
				subject,
				memo: &memo,
				number: week,
				game: season.game_of(subject.team, week),
				tables: Pair {
					before: &tables[w - 1],
					after: &tables[w],
				},
				tallies: Pair {
					before: &tallies[w - 1],
					after: &tallies[w],
				},
				played: &played[w - 1],
				upcoming: &upcoming[w],
			}
			.story()
		})
		.collect();
	Ok(StorySeason {
		version: 4,
		label: subject.label.clone(),
		season_length,
		team: subject.team,
		names: Team::ALL.into_iter().map(|t| (t.code(), t.name())).collect::<BTreeMap<_, _>>(),
		conference: subject.team.conference().label(),
		ways: subject.ways,
		weeks,
	})
}

/// A team's paths at a table: the best seed for each count of more wins.
#[must_use]
pub fn paths(memo: &Memo, table: &Table, team: Team) -> Vec<Option<u8>> {
	let claim_at = claim_extra(memo, table, team).unwrap_or(0);
	(0..=WINDOW.max(claim_at)).map(|x| memo.best(table, team, x).map(|s| s.seed)).collect()
}

/// The fewest more wins with any way in. Ways in only open with more wins
/// (a loss handed back as a win never hurts), so this is a binary search.
fn claim_extra(memo: &Memo, table: &Table, team: Team) -> Option<u8> {
	let (mut lo, mut hi) = (0u8, table.remaining(team));
	memo.best(table, team, hi)?;
	while lo < hi {
		let mid = lo + (hi - lo) / 2;
		if memo.best(table, team, mid).is_some() {
			hi = mid;
		} else {
			lo = mid + 1;
		}
	}
	Some(lo)
}

/// Something read at the table before the week and at the table after it.
#[derive(Clone, Copy)]
struct Pair<T> {
	before: T,
	after: T,
}

struct Week<'a> {
	season: &'a Season,
	subject: &'a Subject,
	memo: &'a Memo,
	/// The week number.
	number: u8,
	/// The subject's game, `None` on its bye.
	game: Option<&'a Game>,
	tables: Pair<&'a Table>,
	tallies: Pair<&'a Tally>,
	/// The week's ways, read off the table before it.
	played: &'a WeekWays,
	/// Next week's, read off the table after it.
	upcoming: &'a Upcoming,
}

impl Week<'_> {
	fn story(&self) -> StoryWeek {
		let me = self.subject.team;
		let record = self.tables.before.record(me);
		let left = |t: &Table| u16::try_from(t.pending().len()).unwrap_or(u16::MAX);
		StoryWeek {
			week: self.number,
			record: WinLoss { w: record.w, l: record.l },
			game: self.game.map(|g| self.game(g)),
			paths: StoryPaths {
				before: paths(self.memo, self.tables.before, me),
				after: paths(self.memo, self.tables.after, me),
				wins_before: record.w,
				wins_after: self.tables.after.record(me).w,
			},
			share: self.share(),
			pending: Left {
				before: left(self.tables.before),
				after: left(self.tables.after),
			},
			results: self.results(),
			next: self.next(),
			stake: self.stake(),
			clinched: solver::clinched(self.tables.after, me),
			ways: self.played.ways.clone(),
			podium: podium(&self.played.entries, me),
		}
	}

	fn game(&self, game: &Game) -> StoryGame {
		let me = self.subject.team;
		let opponent = game.opponent(me);
		let relation = if opponent.division() == me.division() {
			Relation::Division
		} else if opponent.conference() == me.conference() {
			Relation::Conference
		} else {
			Relation::Other
		};
		StoryGame {
			opponent,
			won: game.outcome() == Some(Outcome::Won(me)),
			relation,
		}
	}

	/// The index into a table's pending games of the subject's game in
	/// `week`, when it has one.
	fn own_pending(&self, table: &Table, week: u8) -> Option<usize> {
		let me = self.subject.team;
		table.pending().iter().position(|p| p.week == week && (p.home == me || p.away == me))
	}

	/// The share, before and after, with the change split at the subject's
	/// own result (the share given that result, read off the table before),
	/// and the spread of the rest of the week, that result held.
	fn share(&self) -> StoryShare {
		let (before, after) = (self.tallies.before, self.tallies.after);
		// The subject's own result, as (its pending index before the week,
		// whether the home side won): none on a bye or a tie.
		let own = self.game.and_then(|g| {
			let Some(Outcome::Won(winner)) = g.outcome() else {
				return None;
			};
			Some((self.own_pending(self.tables.before, self.number)?, winner == g.home))
		});
		let mid = own
			.and_then(|(i, home_won)| before.given[i].map(|(home, away)| if home_won { home } else { away }))
			.unwrap_or(before.share);
		StoryShare {
			before: Share(before.share),
			after: Share(after.share),
			own: Share(mid - before.share),
			league: Share(after.share - mid),
			spread: Share(self.played.spread),
			exact: before.exact && after.exact,
		}
	}

	/// Every decided game this week with a side in the subject's conference,
	/// most leverage first.
	fn results(&self) -> Vec<StoryResult> {
		let me = self.subject.team;
		let conf = me.conference();
		let share = self.tallies.after.share;
		let mut out: Vec<StoryResult> = self
			.season
			.week(self.number)
			.filter_map(|g| {
				let Some(Outcome::Won(winner)) = g.outcome() else {
					return None;
				};
				let kind = match (g.involves(me), g.home.conference() == conf, g.away.conference() == conf) {
					(true, _, _) => Kind::Own,
					(false, true, true) => Kind::Inside,
					(false, false, false) => return None,
					_ => Kind::Exported,
				};
				let loser = g.opponent(winner);
				let flipped = count::share(&self.tables.after.decided(g, Outcome::Won(loser)), me, self.subject.ways);
				Some(StoryResult {
					winner,
					loser,
					kind,
					leverage: Share(share - flipped),
					collapse: g.collapse.map(|c| Collapse {
						peak_win_probability: c.peak_win_probability,
						led: c.led,
					}),
				})
			})
			.collect();
		out.sort_by(|a, b| b.leverage.0.abs().total_cmp(&a.leverage.0.abs()));
		out
	}

	/// Next week's games with a side in the conference, other than the
	/// subject's own, by how far the share swings on them.
	fn next(&self) -> Vec<Next> {
		let me = self.subject.team;
		let mut out: Vec<Next> = self
			.tables
			.after
			.pending()
			.iter()
			.zip(&self.tallies.after.given)
			.filter(|(p, _)| p.week.checked_sub(1) == Some(self.number) && p.home != me && p.away != me)
			.filter_map(|(p, given)| {
				let (home_won, away_won) = (*given)?;
				let swing = home_won - away_won;
				(swing.abs() >= MIN_SWING).then_some(Next {
					home: p.home,
					away: p.away,
					root_for: if swing > 0.0 { p.home } else { p.away },
					swing: Share(swing.abs()),
				})
			})
			.collect();
		out.sort_by(|a, b| b.swing.0.total_cmp(&a.swing.0));
		out.truncate(NEXT);
		out
	}

	/// What next week can do: the subject's own game, and every result
	/// together.
	fn stake(&self) -> Option<Stake> {
		let table = self.tables.after;
		if table.pending().is_empty() {
			return None;
		}
		let tally = self.tallies.after;
		let game = self.own_pending(table, self.number + 1);
		let own = game.and_then(|i| tally.given[i]).map_or(0.0, |(home, away)| (home - away).abs());
		// Each game's own effect is measured exactly (both sides on every
		// way), and the spread can be no smaller than they add up to.
		// `given` holds exactly next week's games, the ones asked about.
		let own_effects: f64 = tally.given.iter().flatten().map(|(home, away)| ((home - away) / 2.0).powi(2)).sum();
		let up = self.upcoming;
		let me = self.subject.team;
		Some(Stake {
			opponent: game.map(|i| {
				let p = table.pending()[i];
				if p.home == me {
					p.away
				} else {
					p.home
				}
			}),
			own: Share(own),
			spread: Share(up.spread.max(own_effects.sqrt())),
			exact: up.exact,
			pieces: up.pieces.clone(),
		})
	}
}

/// What one table says about the week after it, played.
struct WeekWays {
	/// The subject's.
	ways: Ways,
	/// The subject's spread, its own result held as it happened (none held
	/// on a bye or a tie).
	spread: f64,
	/// Every team's week, unplaced, for the podium.
	entries: Vec<Entry>,
}

/// One team's week, for the podium.
struct Entry {
	/// Placed 0 until the podium places it.
	medal: Medal,
	/// Its week could move its share by at least [`ELIGIBLE`].
	eligible: bool,
}

/// What one table says about the week after it, before it is played: the
/// subject's spread over all of it, and its best way.
struct Upcoming {
	spread: f64,
	exact: bool,
	pieces: Vec<NextPiece>,
}

/// The week after `table`, for `subject`: before it is played, and as it
/// went when it has been (through `through`).
fn ahead(season: &Season, subject: &Subject, table: &Table, through: u8) -> Result<(Upcoming, Option<WeekWays>), EvaluateError> {
	let me = subject.team;
	let week = table.through() + 1;
	let read = |t: &Table, teams: &[Team]| {
		let games: Vec<usize> = (0..t.pending().len()).filter(|&i| t.pending()[i].week == week).collect();
		count::slates(t, teams, subject.rests, &games).ok_or(EvaluateError::WeekTooFull)
	};
	if week > through {
		return Ok((upcoming(season, table, &read(table, &[me])?), None));
	}
	// A tied game is banked before the week: it is none of the week's ways.
	let ties: Vec<&Game> = season.week(week).filter(|g| g.outcome() == Some(Outcome::Tied)).collect();
	let banked = ties.iter().fold(table.clone(), |t, g| t.banking(g));
	// The subject first among its conference's teams.
	let ours: Vec<Team> = std::iter::once(me).chain(Team::of_conference(me.conference()).filter(|&t| t != me)).collect();
	let theirs: Vec<Team> = Team::ALL.into_iter().filter(|t| t.conference() != me.conference()).collect();
	let (ours, theirs) = (read(&banked, &ours)?, read(&banked, &theirs)?);
	let up = if ties.is_empty() {
		upcoming(season, table, &ours)
	} else {
		upcoming(season, table, &read(table, &[me])?)
	};
	Ok((up, Some(week_ways(season, &banked, week, &ours, &theirs))))
}

/// The pending game behind slate bit `j`.
fn pending_of(table: &Table, slates: &Slates, j: usize) -> Pending {
	table.pending()[slates.games[j]]
}

/// The season's game for a pending one.
fn game_of(season: &Season, p: Pending) -> Option<&Game> {
	season.week(p.week).find(|g| g.home == p.home && g.away == p.away)
}

/// The slate's bits in kickoff order: a game the feed gives no time sorts
/// last, and games at one time by their sides.
fn kickoff_order(season: &Season, table: &Table, slates: &Slates) -> Vec<usize> {
	let mut order: Vec<usize> = (0..slates.games.len()).collect();
	order.sort_by_key(|&j| {
		let p = pending_of(table, slates, j);
		(game_of(season, p).and_then(|g| g.kickoff).unwrap_or(u64::MAX), p.home, p.away)
	});
	order
}

/// The slate the week's results made: bit `j` set when the home side won.
fn actual_slate(season: &Season, table: &Table, slates: &Slates) -> usize {
	(0..slates.games.len()).fold(0, |slate, j| {
		let p = pending_of(table, slates, j);
		let home_won = game_of(season, p).and_then(Game::outcome) == Some(Outcome::Won(p.home));
		slate | usize::from(home_won) << j
	})
}

const fn piece(p: Pending, piece: ways::Piece) -> Piece {
	Piece {
		want: if piece.want_home { p.home } else { p.away },
		price: Share(piece.price),
		error: Share(piece.error),
	}
}

/// [`piece`], for `team`: a free piece of its own game wants its win, since
/// a win never hurts it (the best way is as good either way).
fn piece_for(p: Pending, wp: ways::Piece, team: Team) -> Piece {
	let mut out = piece(p, wp);
	if wp.price <= 0.0 && (p.home == team || p.away == team) {
		out.want = team;
	}
	out
}

fn upcoming(season: &Season, table: &Table, slates: &Slates) -> Upcoming {
	// The subject is the first team counted.
	let me = slates.teams[0];
	let best = slates.column(0).best(|_| true);
	Upcoming {
		spread: slates.spread(0, None),
		exact: slates.exact,
		pieces: kickoff_order(season, table, slates)
			.into_iter()
			.map(|j| {
				let p = pending_of(table, slates, j);
				NextPiece {
					home: p.home,
					away: p.away,
					piece: piece_for(p, best.pieces[j], me),
				}
			})
			.collect(),
	}
}

/// The subject's week (the first team of `ours`) and every team's entry.
fn week_ways(season: &Season, table: &Table, week: u8, ours: &Slates, theirs: &Slates) -> WeekWays {
	let me = ours.teams[0];
	let column = ours.column(0);
	let actual = actual_slate(season, table, ours);
	let order = kickoff_order(season, table, ours);
	let all = column.best(|_| true);
	let own = ours.own(0);
	let in_order = |pieces: &[ways::Piece]| order.iter().map(|&j| piece(pending_of(table, ours, j), pieces[j])).collect::<Vec<_>>();
	let all_in_order = order.iter().map(|&j| piece_for(pending_of(table, ours, j), all.pieces[j], me));
	let given = own.map(|j| {
		let held = |s: usize| s >> j & 1 == actual >> j & 1;
		let best = column.best(held);
		Given {
			best: Share(best.best),
			rank: Share(column.rank(actual, held)),
			pieces: in_order(&best.pieces),
		}
	});
	let road = column.road(actual, &order);
	let ways = Ways {
		games: order
			.iter()
			.zip(all_in_order)
			.map(|(&j, piece)| {
				let p = pending_of(table, ours, j);
				let kind = if p.home == me || p.away == me {
					Kind::Own
				} else if p.home.conference() == p.away.conference() {
					Kind::Inside
				} else {
					Kind::Exported
				};
				WayGame {
					home: p.home,
					away: p.away,
					kind,
					winner: if actual >> j & 1 == 1 { p.home } else { p.away },
					piece,
				}
			})
			.collect(),
		best: Share(all.best),
		worst: Share(column.worst()),
		actual: Share(column.share[actual]),
		rank: Share(column.rank(actual, |_| true)),
		given,
		road: road.best.into_iter().map(Share).collect(),
		mean: road.mean.into_iter().map(Share).collect(),
		strip: road.strip,
		exact: ours.exact,
	};
	let entries = [ours, theirs]
		.into_iter()
		.flat_map(|slates| (0..slates.teams.len()).map(move |t| entry(season, table, week, slates, t)))
		.collect();
	WeekWays {
		ways,
		spread: ours.spread(0, own.map(|j| actual >> j & 1 == 1)),
		entries,
	}
}

/// The `t`th team of `slates`' week, for the podium.
fn entry(season: &Season, table: &Table, week: u8, slates: &Slates, t: usize) -> Entry {
	let team = slates.teams[t];
	let column = slates.column(t);
	let actual = actual_slate(season, table, slates);
	let all = column.best(|_| true);
	let game = match season.game_of(team, week).and_then(Game::outcome) {
		None => Played::Bye,
		Some(Outcome::Won(w)) if w == team => Played::Won,
		Some(Outcome::Won(_)) => Played::Lost,
		Some(Outcome::Tied) => Played::Tie,
	};
	let mut pieces: Vec<MedalPiece> = all
		.pieces
		.iter()
		.enumerate()
		.map(|(j, wp)| {
			let p = pending_of(table, slates, j);
			let piece = piece_for(p, *wp, team);
			let winner = if actual >> j & 1 == 1 { p.home } else { p.away };
			MedalPiece {
				home: p.home,
				away: p.away,
				want: piece.want,
				price: piece.price,
				made: winner == piece.want,
			}
		})
		.collect();
	pieces.sort_by(|a, b| b.price.0.total_cmp(&a.price.0));
	pieces.truncate(MEDAL_PIECES);
	Entry {
		medal: Medal {
			team,
			place: 0,
			rank: Share(column.rank(actual, |_| true)),
			best: Share(all.best),
			worst: Share(column.worst()),
			actual: Share(column.share[actual]),
			game,
			pieces,
		},
		eligible: slates.spread(t, None) >= ELIGIBLE,
	}
}

/// The week's podium: every eligible team placed 1 to 3 (1 + the eligible
/// teams ranked strictly higher, so equal ranks share a place), and `me`'s.
fn podium(entries: &[Entry], me: Team) -> Podium {
	let eligible: Vec<&Medal> = entries.iter().filter(|e| e.eligible).map(|e| &e.medal).collect();
	let place = |m: &Medal| u8::try_from(1 + eligible.iter().filter(|o| o.rank.0 > m.rank.0).count()).unwrap_or(u8::MAX);
	let mut medals: Vec<Medal> = eligible
		.iter()
		.filter_map(|m| {
			let p = place(m);
			(p <= 3).then(|| Medal { place: p, ..(*m).clone() })
		})
		.collect();
	medals.sort_by_key(|m| (m.place, m.team));
	Podium {
		medals,
		place: eligible.iter().find(|m| m.team == me).map(|m| place(m)),
		eligible: u8::try_from(eligible.len()).unwrap_or(u8::MAX),
	}
}

/// `f` over `items` on every core, in order. Each item's result depends on
/// it alone, so the output is the same whatever the core count (PD2).
fn parallel<T: Sync, R: Send>(items: &[T], f: impl Fn(&T) -> R + Sync) -> Vec<R> {
	let workers = std::thread::available_parallelism().map_or(1, NonZeroUsize::get).clamp(1, items.len().max(1));
	let mut out: Vec<Option<R>> = std::iter::repeat_with(|| None).take(items.len()).collect();
	std::thread::scope(|scope| {
		let f = &f;
		// Round robin: early tables, with the most season left, cost most.
		let handles: Vec<_> = (0..workers)
			.map(|w| scope.spawn(move || items.iter().enumerate().skip(w).step_by(workers).map(|(i, x)| (i, f(x))).collect::<Vec<_>>()))
			.collect();
		for handle in handles {
			match handle.join() {
				Ok(done) => {
					for (i, r) in done {
						out[i] = Some(r);
					}
				}
				Err(panic) => std::panic::resume_unwind(panic),
			}
		}
	});
	out.into_iter().flatten().collect()
}

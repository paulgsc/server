//! A season, week by week, into the story contract ([`crate::story`]).
//!
//! Every number is the solver's ([`crate::solver::best`]) asked about a
//! different team or a different table; nothing here estimates. The fan's
//! words are defined once, below, and each field's doc says which.
//!
//! - **Paths** of a team at a table: for `x = 0, 1, ...`, the best seed
//!   winning exactly `x` more. Rows run to [`WINDOW`], or further when the
//!   first way in needs more, so a claim read off the rows is never "out"
//!   when the team is alive.
//! - **Claim**: the first row with a way in, and its seed.
//! - **Better for a team**: a lower [`Claim::rank`] (fewer final wins
//!   needed, then a better seed). For the subject, two equal claims are
//!   then told apart by open (more wins, seed) cells in its paths grid, which
//!   is what the stage draws.
//! - **Alive**: has a claim at the table before the week. Roles in the
//!   templates are read from that table.
//! - **Moved**: flip one of the week's results, hold the rest of the week
//!   as it happened, and compare. `helped` when what happened is better for
//!   the subject than the flip, `hurt` when worse, `null` when equal.
//! - **A rival's worst week**: every one of the week's results (its own
//!   included) whose flip alone would leave its claim worse, flipped at once.
//! - **Futility points**: the subject's competitors are its division mates
//!   and named rivals. Each game a competitor plays is possible one point,
//!   scored when the competitor lost; a game between two competitors counts
//!   once, and scores, since one of them had to lose.

use crate::{
	league::Team,
	season::{Game, Outcome, Season},
	solver::{self, Memo, Scenario},
	story::{
		Benchmark, Claim, Collapse, Collision, LowestBar, Moved, Need, Points, Relation, RivalGame, RivalWeek, Score, StoryGame, StoryPath, StoryPaths, StoryResult, StorySeason,
		StoryWeek, Templates, WinLoss,
	},
	table::Table,
};
use std::{collections::BTreeMap, fmt};

/// Rows a paths grid shows at least: "never win again" through "+4 wins".
pub const WINDOW: u8 = 4;

/// How many teams a path lists ("the Seahawks go 4 and 12, ...").
pub const NEEDS: usize = 6;

/// Who the story is about.
#[derive(Debug, Clone)]
pub struct Subject {
	pub team: Team,
	/// Named rivals, in the order the stage lists them.
	pub rivals: Vec<Team>,
	/// Shown on screen: "2025".
	pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvaluateError {
	/// The subject is listed among its own rivals.
	SelfRival,
	/// No week with a final result for the subject yet.
	NothingPlayed,
	/// More games than the contract's counters hold.
	SeasonTooLong,
}

impl fmt::Display for EvaluateError {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.write_str(match self {
			Self::SelfRival => "the team is listed as its own rival",
			Self::NothingPlayed => "no completed week has a game for the team",
			Self::SeasonTooLong => "more than 255 games a team",
		})
	}
}

impl std::error::Error for EvaluateError {}

/// The story of `season` so far, for `subject`, with `previous` (last
/// season, complete) as the benchmark.
///
/// One week per completed week the subject played. A bye has no slide in
/// contract v1 (its `game` is required), so a bye week is not emitted; its
/// results still count, and show up in the next week's "before".
///
/// # Errors
/// [`EvaluateError`].
pub fn evaluate(season: &Season, previous: &Season, subject: &Subject) -> Result<StorySeason, EvaluateError> {
	if subject.rivals.contains(&subject.team) {
		return Err(EvaluateError::SelfRival);
	}
	let season_length = u8::try_from(season.length()).map_err(|_| EvaluateError::SeasonTooLong)?;
	let (memo, last_memo) = (Memo::default(), Memo::default());
	let tables: Vec<Table> = (0..=season.complete_through()).map(|w| Table::after(season, w)).collect();
	let claims: Vec<[Claim; 32]> = tables.iter().map(|t| Team::ALL.map(|team| claim(&memo, t, team))).collect();
	let mut so_far = Score::default();
	let mut weeks = Vec::new();
	for week in 1..=season.complete_through() {
		let Some(game) = season.game_of(subject.team, week) else {
			continue;
		};
		let w = usize::from(week);
		let view = Week::new(
			Context {
				season,
				previous,
				subject,
				memo: &memo,
				last_memo: &last_memo,
			},
			week,
			game,
			[&tables[w - 1], &tables[w]],
			[&claims[w - 1], &claims[w]],
		);
		let mut story = view.story();
		so_far.scored += story.points.week.scored;
		so_far.possible += story.points.week.possible;
		story.points.season = so_far;
		weeks.push(story);
	}
	if weeks.is_empty() {
		return Err(EvaluateError::NothingPlayed);
	}
	Ok(StorySeason {
		version: 1,
		label: subject.label.clone(),
		season_length,
		team: subject.team,
		rivals: subject.rivals.clone(),
		names: Team::ALL.into_iter().map(|t| (t.code(), t.name())).collect::<BTreeMap<_, _>>(),
		conference: subject.team.conference().label(),
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

fn claim_scenario(memo: &Memo, table: &Table, team: Team) -> Option<(u8, Scenario)> {
	let x = claim_extra(memo, table, team)?;
	memo.best(table, team, x).map(|s| (x, s))
}

#[must_use]
pub fn claim(memo: &Memo, table: &Table, team: Team) -> Claim {
	let wins = table.record(team).w;
	Claim::new(wins, claim_scenario(memo, table, team).map(|(x, s)| (x, s.seed)))
}

/// Open (more wins, seed) cells in a paths grid, rows compared over the
/// longer of the two.
fn open_cells(paths: &[Option<u8>], rows: usize) -> usize {
	(0..rows)
		.map(|x| paths.get(x).copied().flatten().map_or(0, |s| usize::from(solver::LAST_SEED + 1 - s)))
		.sum()
}

/// Is `actual` (a claim's rank and its paths) better for the subject than
/// `other`: the claim first, then the open cells.
fn compare(actual: ((u16, u8), &[Option<u8>]), other: ((u16, u8), &[Option<u8>])) -> Option<Moved> {
	let rows = actual.1.len().max(other.1.len());
	let key = |(rank, paths): ((u16, u8), &[Option<u8>])| (std::cmp::Reverse(rank), open_cells(paths, rows));
	match key(actual).cmp(&key(other)) {
		std::cmp::Ordering::Greater => Some(Moved::Helped),
		std::cmp::Ordering::Less => Some(Moved::Hurt),
		std::cmp::Ordering::Equal => None,
	}
}

/// The other way a decided game could have gone (`None` for a tie).
fn flip(game: &Game) -> Option<Outcome> {
	match game.outcome()? {
		Outcome::Won(w) => Some(Outcome::Won(game.opponent(w))),
		Outcome::Tied => None,
	}
}

/// One rival's view of one result: which winner was worse for it.
struct RivalTake {
	game: usize,
	/// The winner that leaves the rival's claim worse.
	worse_winner: Team,
	/// Did it happen.
	went_against: bool,
}

#[derive(Clone, Copy)]
struct Context<'a> {
	season: &'a Season,
	previous: &'a Season,
	subject: &'a Subject,
	/// For tables of `season`, and of `previous`.
	memo: &'a Memo,
	last_memo: &'a Memo,
}

struct Week<'a> {
	cx: Context<'a>,
	/// The week number.
	number: u8,
	game: &'a Game,
	games: Vec<&'a Game>,
	before: &'a Table,
	after: &'a Table,
	/// Every team's claim at `before` and at `after`.
	claims_before: &'a [Claim; 32],
	claims_after: &'a [Claim; 32],
	after_paths: Vec<Option<u8>>,
	/// Per game this week: did it move the subject's paths.
	moved: Vec<Option<Moved>>,
}

impl<'a> Week<'a> {
	fn new(cx: Context<'a>, week: u8, game: &'a Game, [before, after]: [&'a Table; 2], [claims_before, claims_after]: [&'a [Claim; 32]; 2]) -> Self {
		let me = cx.subject.team;
		let games: Vec<&Game> = cx.season.week(week).collect();
		let after_paths = paths(cx.memo, after, me);
		let actual = (claims_after[me.index()].rank(), after_paths.as_slice());
		let moved = games
			.iter()
			.map(|g| {
				// A result outside the subject's conference cannot move its seed.
				if g.involves(me) || (g.home.conference() != me.conference() && g.away.conference() != me.conference()) {
					return None;
				}
				let other = after.decided(g, flip(g)?);
				compare(actual, (claim(cx.memo, &other, me).rank(), &paths(cx.memo, &other, me)))
			})
			.collect();
		Self {
			cx,
			number: week,
			game,
			games,
			before,
			after,
			claims_before,
			claims_after,
			after_paths,
			moved,
		}
	}

	const fn alive(&self, team: Team) -> bool {
		matches!(self.claims_before[team.index()], Claim::In { .. })
	}

	fn story(&self) -> StoryWeek {
		let me = self.cx.subject.team;
		let takes: Vec<(Team, Vec<RivalTake>)> = self.cx.subject.rivals.iter().map(|r| (*r, self.rival_takes(*r))).collect();
		let points = self.futility();
		let record = self.before.record(me);
		StoryWeek {
			week: self.number,
			record: WinLoss { w: record.w, l: record.l },
			game: self.game(),
			paths: StoryPaths {
				before: paths(self.cx.memo, self.before, me),
				after: self.after_paths.clone(),
				wins_before: record.w,
				wins_after: self.after.record(me).w,
			},
			templates: self.templates(),
			path: self.path(),
			rivals: self.rivals(&takes),
			collisions: self.collisions(&takes),
			points: Points { week: points, season: points },
			benchmark: self.benchmark(),
		}
	}

	/// Futility points this week (see the module doc).
	fn futility(&self) -> Score {
		let me = self.cx.subject.team;
		let competitor = |t: Team| t != me && (t.division() == me.division() || self.cx.subject.rivals.contains(&t));
		let mut score = Score::default();
		for g in &self.games {
			let (home, away) = (competitor(g.home), competitor(g.away));
			if !home && !away {
				continue;
			}
			score.possible += 1;
			let lost = match g.outcome() {
				Some(Outcome::Won(w)) => (home && w != g.home) || (away && w != g.away),
				_ => false,
			};
			score.scored += u16::from(lost);
		}
		score
	}

	fn game(&self) -> StoryGame {
		let me = self.cx.subject.team;
		let opponent = self.game.opponent(me);
		let relation = if opponent.division() == me.division() {
			Relation::Division
		} else if self.cx.subject.rivals.contains(&opponent) {
			Relation::Rival
		} else if opponent.conference() == me.conference() {
			Relation::Conference
		} else {
			Relation::Other
		};
		StoryGame {
			opponent,
			won: self.game.outcome() == Some(Outcome::Won(me)),
			relation,
		}
	}

	/// `team`'s result in `game`, with the fan's reading of it: the watched
	/// team losing goes your way.
	fn result(&self, index: usize, team: Team, your_way_when_won: bool) -> StoryResult {
		let g = self.games[index];
		let outcome = g.outcome();
		let won = outcome == Some(Outcome::Won(team));
		StoryResult {
			team,
			opponent: g.opponent(team),
			won,
			your_way: (outcome != Some(Outcome::Tied)).then_some(won == your_way_when_won),
			moved: self.moved[index],
			collapse: Self::collapse(g, team),
		}
	}

	/// The loser's collapse, shown on the loser's card only.
	fn collapse(g: &Game, team: Team) -> Option<Collapse> {
		let c = g.collapse?;
		(g.outcome() == Some(Outcome::Won(g.opponent(team)))).then_some(Collapse {
			peak_win_probability: c.peak_win_probability,
			led: c.led,
		})
	}

	fn index_of(&self, team: Team) -> Option<usize> {
		self.games.iter().position(|g| g.involves(team))
	}

	fn templates(&self) -> Templates {
		let me = self.cx.subject.team;
		let conf = me.conference();
		let watched = |teams: &mut dyn Iterator<Item = Team>| -> Vec<StoryResult> { teams.filter_map(|t| self.index_of(t).map(|i| self.result(i, t, false))).collect() };
		let mut above: Vec<Team> = Team::of_conference(conf)
			.filter(|t| *t != me && self.alive(*t) && self.before.points(*t) > self.before.points(me))
			.collect();
		above.sort_by_key(|t| (std::cmp::Reverse(self.before.points(*t)), t.code()));
		let mut exported = Vec::new();
		let mut spoilers = Vec::new();
		let mut forced = Vec::new();
		for (i, g) in self.games.iter().enumerate() {
			if g.involves(me) {
				continue;
			}
			match (g.home.conference() == conf, g.away.conference() == conf) {
				(true, true) => match (self.alive(g.home), self.alive(g.away)) {
					(true, true) => {
						let winner = match g.outcome() {
							Some(Outcome::Won(w)) => w,
							_ => g.home,
						};
						let mut r = self.result(i, winner, true);
						r.your_way = self.moved[i].map(|m| m == Moved::Helped);
						forced.push(r);
					}
					(true, false) => spoilers.push(self.result(i, g.away, true)),
					(false, true) => spoilers.push(self.result(i, g.home, true)),
					(false, false) => {}
				},
				(true, false) if self.alive(g.home) => exported.push(self.result(i, g.home, false)),
				(false, true) if self.alive(g.away) => exported.push(self.result(i, g.away, false)),
				_ => {}
			}
		}
		Templates {
			division: watched(&mut me.division_rivals()),
			rivals: watched(&mut self.cx.subject.rivals.iter().copied()),
			above: watched(&mut above.into_iter()),
			exported,
			spoilers,
			forced,
		}
	}

	fn path(&self) -> Option<StoryPath> {
		let me = self.cx.subject.team;
		let (extra_wins, scenario) = claim_scenario(self.cx.memo, self.after, me)?;
		let rivals = &self.cx.subject.rivals;
		let mut others: Vec<Team> = Team::of_conference(me.conference())
			.filter(|t| *t != me && !rivals.contains(t) && t.division() != me.division())
			.collect();
		// The teams that had to fall furthest first: lowest share of their
		// remaining games won (compared as cross products, no floats).
		let rest = |t: &Team| scenario.rest[t.index()];
		others.sort_by(|a, b| {
			let (ra, rb) = (rest(a), rest(b));
			(u16::from(ra.w) * u16::from(rb.w + rb.l))
				.cmp(&(u16::from(rb.w) * u16::from(ra.w + ra.l)))
				.then(a.code().cmp(b.code()))
		});
		let needs = rivals
			.iter()
			.copied()
			.chain(me.division_rivals().filter(|t| !rivals.contains(t)))
			.chain(others)
			.take(NEEDS)
			.map(|team| {
				let r = scenario.rest[team.index()];
				Need {
					team,
					rest: WinLoss { w: r.w, l: r.l },
					rival: rivals.contains(&team),
				}
			})
			.collect();
		Some(StoryPath {
			extra_wins,
			seed: scenario.seed,
			needs,
		})
	}

	/// Each of the week's decided results whose flip changes `rival`'s claim.
	fn rival_takes(&self, rival: Team) -> Vec<RivalTake> {
		let actual = self.claims_after[rival.index()].rank();
		self
			.games
			.iter()
			.enumerate()
			.filter_map(|(i, g)| {
				let Outcome::Won(winner) = g.outcome()? else {
					return None;
				};
				if g.home.conference() != rival.conference() && g.away.conference() != rival.conference() {
					return None;
				}
				let other = claim(self.cx.memo, &self.after.decided(g, flip(g)?), rival).rank();
				(other != actual).then(|| RivalTake {
					game: i,
					worse_winner: if actual > other { winner } else { g.opponent(winner) },
					went_against: actual > other,
				})
			})
			.collect()
	}

	fn rivals(&self, takes: &[(Team, Vec<RivalTake>)]) -> Vec<RivalWeek> {
		takes
			.iter()
			.filter_map(|(rival, takes)| {
				let g = self.games[self.index_of(*rival)?];
				let mut worst_table = self.after.clone();
				for t in takes.iter().filter(|t| !t.went_against) {
					worst_table = worst_table.decided(self.games[t.game], Outcome::Won(t.worse_winner));
				}
				let after = self.claims_after[rival.index()];
				let worst = claim(self.cx.memo, &worst_table, *rival);
				Some(RivalWeek {
					team: *rival,
					game: RivalGame {
						opponent: g.opponent(*rival),
						won: g.outcome() == Some(Outcome::Won(*rival)),
					},
					before: self.claims_before[rival.index()],
					after,
					// Flips that each hurt can, together, interact; the worst week
					// is never reported as better than the one that happened.
					worst: if worst.rank() >= after.rank() { worst } else { after },
				})
			})
			.collect()
	}

	fn collisions(&self, takes: &[(Team, Vec<RivalTake>)]) -> Vec<Collision> {
		let mut out = Vec::new();
		for (rival, takes) in takes {
			for t in takes {
				let g = self.games[t.game];
				// A rival's own game is the rivals slide's, not a collision.
				let Some(Outcome::Won(winner)) = g.outcome().filter(|_| !g.involves(*rival)) else {
					continue;
				};
				let best_for_you = match self.moved[t.game] {
					Some(Moved::Helped) => winner,
					Some(Moved::Hurt) => g.opponent(winner),
					None => continue,
				};
				if best_for_you != t.worse_winner {
					out.push(Collision::RivalVsYou {
						home: g.home,
						away: g.away,
						winner,
						rival: *rival,
						worst_for_rival: t.worse_winner,
						best_for_you,
					});
				}
			}
		}
		let rivals = &self.cx.subject.rivals;
		for g in &self.games {
			if let (true, true, Some(Outcome::Won(winner))) = (rivals.contains(&g.home), rivals.contains(&g.away), g.outcome()) {
				out.push(Collision::RivalVsRival {
					home: g.home,
					away: g.away,
					winner,
				});
			}
		}
		out
	}

	fn benchmark(&self) -> Benchmark {
		let last = Table::after(self.cx.previous, self.number.min(self.cx.previous.complete_through()));
		let lowest_bar = Team::ALL
			.into_iter()
			.filter_map(|team| match self.claims_after[team.index()] {
				Claim::In { wins, extra_wins, seed, .. } => Some(LowestBar {
					team,
					wins: wins + extra_wins,
					seed,
				}),
				Claim::Out { .. } => None,
			})
			.min_by_key(|b| (b.wins, b.seed, b.team.code()))
			// Every table has teams with a way in: any finish seeds fourteen.
			.unwrap_or_else(|| LowestBar {
				team: self.cx.subject.team,
				wins: u8::try_from(self.cx.season.length()).unwrap_or(u8::MAX),
				seed: solver::LAST_SEED,
			});
		Benchmark {
			last_season: claim(self.cx.last_memo, &last, self.cx.subject.team),
			lowest_bar,
		}
	}
}

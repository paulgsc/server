//! A season, week by week, into the story contract ([`crate::story`]).
//!
//! Every number is the solver's ([`crate::solver::best`]) or the count's
//! ([`crate::count`]), asked about a different team or a different table;
//! nothing here estimates beyond the count's sample. The fan's words are
//! defined once, below, and each field's doc says which.
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
//! - **Share**: of the ways the rest of the season can go, the share in which
//!   the team gets in ([`count`]).
//! - **Leverage** of a result: the share as it happened minus the share had
//!   it gone the other way, the rest of the week held as it happened.
//! - **Moved**: flip one of the week's results, hold the rest of the week
//!   as it happened, and compare paths. `helped` when what happened is
//!   better for the subject than the flip, `hurt` when worse, `null` when
//!   equal.
//! - **Futility points**: the subject's competitors are its division mates
//!   and named rivals. Each game a competitor plays against anyone else is a
//!   possible point, scored when the competitor lost. A game between two
//!   competitors is not counted: one of them had to lose.

use crate::{
	count::{self, Tally},
	league::Team,
	season::{Game, Outcome, Season},
	solver::{self, Memo, Scenario},
	story::{
		Benchmark, Boundary, Bucket, Claim, Collapse, Collision, DivisionRoute, Kind, Left, LowestBar, Moved, Need, Next, Pigeonhole, Points, Relation, RivalGame, RivalWeek,
		Score, Share, Shares, StoryGame, StoryPath, StoryPaths, StoryResult, StorySeason, StoryShare, StoryWeek, WildCardRoute, WinLoss,
	},
	table::Table,
};
use std::{collections::BTreeMap, fmt};

/// Rows a paths grid shows at least: "never win again" through "+4 wins".
pub const WINDOW: u8 = 4;

/// How many teams a path lists ("the Seahawks go 4 and 12, ...").
pub const NEEDS: usize = 6;

/// The ways a share is sampled over when there are more than that to count.
///
/// A share near one half then lands within about a third of a percentage
/// point (two standard errors), and a result's leverage, compared on the
/// same sampled ways, much closer.
pub const WAYS: u32 = 100_000;

/// How many of next week's games a story names.
pub const NEXT: usize = 3;

/// A swing smaller than this (a twentieth of a point) is not named.
const MIN_SWING: f64 = 0.0005;

/// Who the story is about.
#[derive(Debug, Clone)]
pub struct Subject {
	pub team: Team,
	/// Named rivals, in the order the stage lists them.
	pub rivals: Vec<Team>,
	/// Shown on screen: "2025".
	pub label: String,
	/// The most ways a share is sampled over ([`WAYS`] for the story).
	pub ways: u32,
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
/// season, complete) as the benchmark: one week per completed week, the
/// subject's bye included (its `game` is `None`).
///
/// # Errors
/// [`EvaluateError`].
pub fn evaluate(season: &Season, previous: &Season, subject: &Subject) -> Result<StorySeason, EvaluateError> {
	if subject.rivals.contains(&subject.team) {
		return Err(EvaluateError::SelfRival);
	}
	let season_length = u8::try_from(season.length()).map_err(|_| EvaluateError::SeasonTooLong)?;
	let through = season.complete_through();
	if !(1..=through).any(|w| season.game_of(subject.team, w).is_some()) {
		return Err(EvaluateError::NothingPlayed);
	}
	let (memo, last_memo) = (Memo::default(), Memo::default());
	let tables: Vec<Table> = (0..=through).map(|w| Table::after(season, w)).collect();
	let claims: Vec<[Claim; 32]> = tables.iter().map(|t| Team::ALL.map(|team| claim(&memo, t, team))).collect();
	let tallies: Vec<Tally> = tables.iter().map(|t| count::tally(t, subject.team, subject.ways)).collect();
	let rival_shares: Vec<Vec<f64>> = tables.iter().map(|t| subject.rivals.iter().map(|r| count::share(t, *r, subject.ways)).collect()).collect();
	let mut so_far = Score::default();
	let mut weeks = Vec::new();
	for week in 1..=through {
		let points = futility(season, subject, week);
		so_far.scored += points.scored;
		so_far.possible += points.possible;
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
			Pair {
				before: &tables[w - 1],
				after: &tables[w],
			},
			Pair {
				before: &claims[w - 1],
				after: &claims[w],
			},
			Pair {
				before: (&tallies[w - 1], &rival_shares[w - 1]),
				after: (&tallies[w], &rival_shares[w]),
			},
		);
		let mut story = view.story();
		story.points = Points { week: points, season: so_far };
		weeks.push(story);
	}
	Ok(StorySeason {
		version: 2,
		label: subject.label.clone(),
		season_length,
		team: subject.team,
		rivals: subject.rivals.clone(),
		names: Team::ALL.into_iter().map(|t| (t.code(), t.name())).collect::<BTreeMap<_, _>>(),
		conference: subject.team.conference().label(),
		ways: subject.ways,
		weeks,
	})
}

/// Futility points in `week` (see the module doc).
fn futility(season: &Season, subject: &Subject, week: u8) -> Score {
	let me = subject.team;
	let competitor = |t: Team| t != me && (t.division() == me.division() || subject.rivals.contains(&t));
	let mut score = Score::default();
	for g in season.week(week) {
		let (home, away) = (competitor(g.home), competitor(g.away));
		if home == away {
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

/// One rival's view of one result whose flip changes its claim.
struct RivalTake {
	game: usize,
	/// The winner that leaves the rival's claim worse.
	worse_winner: Team,
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

/// Something read at the table before the week and at the table after it.
#[derive(Clone, Copy)]
struct Pair<T> {
	before: T,
	after: T,
}

/// The subject's tally at a table, and each named rival's share there.
type Counted<'a> = (&'a Tally, &'a [f64]);

struct Week<'a> {
	cx: Context<'a>,
	/// The week number.
	number: u8,
	/// The subject's game, `None` on its bye.
	game: Option<&'a Game>,
	games: Vec<&'a Game>,
	tables: Pair<&'a Table>,
	/// Every team's claim at each table.
	claims: Pair<&'a [Claim; 32]>,
	counted: Pair<Counted<'a>>,
	after_paths: Vec<Option<u8>>,
	/// Per game this week: did it move the subject's paths.
	moved: Vec<Option<Moved>>,
}

impl<'a> Week<'a> {
	fn new(cx: Context<'a>, week: u8, tables: Pair<&'a Table>, claims: Pair<&'a [Claim; 32]>, counted: Pair<(&'a Tally, &'a Vec<f64>)>) -> Self {
		let me = cx.subject.team;
		let games: Vec<&Game> = cx.season.week(week).collect();
		let after_paths = paths(cx.memo, tables.after, me);
		let actual = (claims.after[me.index()].rank(), after_paths.as_slice());
		let moved = games
			.iter()
			.map(|g| {
				// A result outside the subject's conference cannot move its seed,
				// and its own game is the game slide's.
				if g.involves(me) || (g.home.conference() != me.conference() && g.away.conference() != me.conference()) {
					return None;
				}
				let other = tables.after.decided(g, flip(g)?);
				compare(actual, (claim(cx.memo, &other, me).rank(), &paths(cx.memo, &other, me)))
			})
			.collect();
		Self {
			cx,
			number: week,
			game: cx.season.game_of(me, week),
			games,
			tables,
			claims,
			counted: Pair {
				before: (counted.before.0, counted.before.1.as_slice()),
				after: (counted.after.0, counted.after.1.as_slice()),
			},
			after_paths,
			moved,
		}
	}

	fn story(&self) -> StoryWeek {
		let me = self.cx.subject.team;
		let takes: Vec<(Team, Vec<RivalTake>)> = self.cx.subject.rivals.iter().map(|r| (*r, self.rival_takes(*r))).collect();
		let record = self.tables.before.record(me);
		let left = |t: &Table| u16::try_from(t.pending().len()).unwrap_or(u16::MAX);
		StoryWeek {
			week: self.number,
			record: WinLoss { w: record.w, l: record.l },
			game: self.game.map(|g| self.game(g)),
			paths: StoryPaths {
				before: paths(self.cx.memo, self.tables.before, me),
				after: self.after_paths.clone(),
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
			path: self.path(),
			boundary: self.boundary(),
			rivals: self.rivals(),
			collisions: self.collisions(&takes),
			// Filled in by `evaluate`, which counts the season so far.
			points: Points {
				week: Score::default(),
				season: Score::default(),
			},
			benchmark: self.benchmark(),
		}
	}

	fn game(&self, game: &Game) -> StoryGame {
		let me = self.cx.subject.team;
		let opponent = game.opponent(me);
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
			won: game.outcome() == Some(Outcome::Won(me)),
			relation,
		}
	}

	/// The share, before and after, with the change split at the subject's
	/// own result: the share given that result, read off the table before.
	fn share(&self) -> StoryShare {
		let (before, after) = (self.counted.before.0, self.counted.after.0);
		let given_own = self.game.and_then(|g| {
			let Some(Outcome::Won(winner)) = g.outcome() else {
				return None;
			};
			let i = self
				.tables
				.before
				.pending()
				.iter()
				.position(|p| (p.week, p.home, p.away) == (self.number, g.home, g.away))?;
			let (home_won, away_won) = before.given[i]?;
			Some(if winner == g.home { home_won } else { away_won })
		});
		let mid = given_own.unwrap_or(before.share);
		StoryShare {
			before: Share(before.share),
			after: Share(after.share),
			own: Share(mid - before.share),
			league: Share(after.share - mid),
			exact: before.exact && after.exact,
		}
	}

	/// Every decided game this week with a side in the subject's conference,
	/// most leverage first.
	fn results(&self) -> Vec<StoryResult> {
		let me = self.cx.subject.team;
		let conf = me.conference();
		let share = self.counted.after.0.share;
		let mut out: Vec<StoryResult> = self
			.games
			.iter()
			.enumerate()
			.filter_map(|(i, g)| {
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
				let flipped = count::share(&self.tables.after.decided(g, Outcome::Won(loser)), me, self.cx.subject.ways);
				Some(StoryResult {
					winner,
					loser,
					kind,
					leverage: Share(share - flipped),
					moved: self.moved[i],
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
		let me = self.cx.subject.team;
		let tally = self.counted.after.0;
		let mut out: Vec<Next> = self
			.tables
			.after
			.pending()
			.iter()
			.zip(&tally.given)
			.filter(|(p, _)| p.week == self.number + 1 && p.home != me && p.away != me)
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

	fn index_of(&self, team: Team) -> Option<usize> {
		self.games.iter().position(|g| g.involves(team))
	}

	fn path(&self) -> Option<StoryPath> {
		let me = self.cx.subject.team;
		let (extra_wins, scenario) = claim_scenario(self.cx.memo, self.tables.after, me)?;
		let rivals = &self.cx.subject.rivals;
		let free = |t: Team| scenario.free[t.index()];
		let mut others: Vec<Team> = Team::of_conference(me.conference())
			.filter(|t| *t != me && !rivals.contains(t) && t.division() != me.division())
			.collect();
		// The teams this way holds down first, those that had to fall furthest
		// leading: lowest share of their remaining games won (compared as cross
		// products, no floats). A team it lets finish above comes last.
		let rest = |t: &Team| scenario.rest[t.index()];
		others.sort_by(|a, b| {
			let (ra, rb) = (rest(a), rest(b));
			free(*a)
				.cmp(&free(*b))
				.then((u16::from(ra.w) * u16::from(rb.w + rb.l)).cmp(&(u16::from(rb.w) * u16::from(ra.w + ra.l))))
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
					free: free(team),
				}
			})
			.collect();
		Some(StoryPath {
			extra_wins,
			seed: scenario.seed,
			needs,
		})
	}

	/// Why one fewer final win than the claim's can't get the subject in.
	fn boundary(&self) -> Option<Boundary> {
		let me = self.cx.subject.team;
		let Claim::In { wins, extra_wins, .. } = self.claims.after[me.index()] else {
			return None;
		};
		let fewer = extra_wins.checked_sub(1)?;
		let why = solver::impossible(self.tables.after, me, fewer)?;
		let pigeonhole = |p: solver::Pigeonhole| Pigeonhole {
			teams: p
				.teams
				.iter()
				.map(|b| Bucket {
					team: b.team,
					wins: b.wins,
					room: b.room,
				})
				.collect(),
			games: p.games,
		};
		let division = match why.division {
			solver::DivisionRoute::Above(team) => DivisionRoute::Above { team },
			solver::DivisionRoute::Pigeonhole(p) => {
				let p = pigeonhole(p);
				DivisionRoute::Pigeonhole { teams: p.teams, games: p.games }
			}
		};
		let w = why.wild_card;
		let (wild_pigeonhole, must_pass) = w.pigeonhole.map_or((None, 0), |(p, k)| (Some(pigeonhole(p)), k));
		Some(Boundary {
			wins: wins + fewer,
			division,
			wild_card: WildCardRoute {
				above: w.above,
				pigeonhole: wild_pigeonhole,
				must_pass,
				seed_at_best: w.seed_at_best,
				cases: w.cases,
			},
		})
	}

	/// Each of the week's decided results whose flip changes `rival`'s claim.
	fn rival_takes(&self, rival: Team) -> Vec<RivalTake> {
		let actual = self.claims.after[rival.index()].rank();
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
				let other = claim(self.cx.memo, &self.tables.after.decided(g, flip(g)?), rival).rank();
				(other != actual).then(|| RivalTake {
					game: i,
					worse_winner: if actual > other { winner } else { g.opponent(winner) },
				})
			})
			.collect()
	}

	fn rivals(&self) -> Vec<RivalWeek> {
		self
			.cx
			.subject
			.rivals
			.iter()
			.enumerate()
			.map(|(j, rival)| RivalWeek {
				team: *rival,
				game: self.index_of(*rival).map(|i| {
					let g = self.games[i];
					RivalGame {
						opponent: g.opponent(*rival),
						won: g.outcome() == Some(Outcome::Won(*rival)),
					}
				}),
				before: self.claims.before[rival.index()],
				after: self.claims.after[rival.index()],
				share: Shares {
					before: Share(self.counted.before.1[j]),
					after: Share(self.counted.after.1[j]),
				},
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
			.filter_map(|team| match self.claims.after[team.index()] {
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

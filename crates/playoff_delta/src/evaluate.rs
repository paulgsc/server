//! A season, week by week, into the story contract ([`crate::story`]).
//!
//! Every number is exact: a pole read off a table by [`crate::poles`], or a
//! seed by [`crate::solver`]. Each week carries the poles at the table before
//! it and the table after it, and what each of its results did to them, as
//! the README's field definitions say.

use crate::{
	league::Team,
	poles::{Ceiling, Field, Floor},
	season::{Game, Outcome, Season},
	solver,
	story::{
		Bloc, Ceiling as StoryCeiling, Cohort, Group, Kind, Left, Move, Poles, PolesPair, Range, Relation, Seeds, Steps, StoryGame, StoryResult, StorySeason, StoryWeek, WinLoss,
		Wins,
	},
	table::Table,
};
use std::{collections::BTreeMap, fmt};

/// Who the story is about.
#[derive(Debug, Clone)]
pub struct Subject {
	pub team: Team,
	/// Shown on screen: "2025".
	pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvaluateError {
	/// No week with a final result for the subject yet.
	NothingPlayed,
	/// More games than the contract's counters hold.
	SeasonTooLong,
}

impl fmt::Display for EvaluateError {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.write_str(match self {
			Self::NothingPlayed => "no completed week has a game for the team",
			Self::SeasonTooLong => "more than 255 games a team",
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
	let me = subject.team;
	if !(1..=through).any(|w| season.game_of(me, w).is_some()) {
		return Err(EvaluateError::NothingPlayed);
	}
	let groups = Groups::of(me);
	let tables: Vec<Table> = (0..=through).map(|w| Table::after(season, w)).collect();
	let poles: Vec<Poles> = tables.iter().map(|t| groups.poles(t)).collect();
	let weeks = (1..=through)
		.map(|week| {
			let w = usize::from(week);
			let (before, after) = (&tables[w - 1], &tables[w]);
			let record = before.record(me);
			let left = |t: &Table| u16::try_from(t.pending().len()).unwrap_or(u16::MAX);
			StoryWeek {
				week,
				record: WinLoss { w: record.w, l: record.l },
				game: season.game_of(me, week).map(|g| game(g, me)),
				pending: Left {
					before: left(before),
					after: left(after),
				},
				poles: PolesPair {
					before: poles[w - 1].clone(),
					after: poles[w].clone(),
				},
				results: groups.results(season, week, after, &poles[w]),
			}
		})
		.collect();
	Ok(StorySeason {
		version: 6,
		label: subject.label.clone(),
		season_length,
		team: me,
		names: Team::ALL.into_iter().map(|t| (t.code(), t.name())).collect::<BTreeMap<_, _>>(),
		conference: me.conference().label(),
		rivals: groups.division.iter().map(|&i| Team::ALL[i]).collect(),
		weeks,
	})
}

fn game(game: &Game, me: Team) -> StoryGame {
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

/// The subject's two groups of rivals, by [`Team::index`].
struct Groups {
	me: Team,
	division: Vec<usize>,
	/// The conference's other fifteen.
	conference: Vec<usize>,
}

/// A table as [`Field`] reads it.
struct Standing {
	points: Vec<u16>,
	games: Vec<(usize, usize)>,
}

impl Standing {
	fn of(table: &Table) -> Self {
		Self {
			points: Team::ALL.iter().map(|&t| table.points(t)).collect(),
			games: table.pending().iter().map(|p| (p.home.index(), p.away.index())).collect(),
		}
	}

	fn field(&self) -> Field<'_> {
		Field {
			points: &self.points,
			games: &self.games,
		}
	}
}

/// The parts of a group a result's move compares: its floor and ceiling.
#[derive(Clone, Copy)]
struct Bounds {
	floor: u16,
	ceiling: u16,
}

impl Groups {
	fn of(me: Team) -> Self {
		let others = |same: &dyn Fn(Team) -> bool| Team::ALL.into_iter().filter(|&t| t != me && same(t)).map(Team::index).collect();
		Self {
			me,
			division: others(&|t| t.division() == me.division()),
			conference: others(&|t| t.conference() == me.conference()),
		}
	}

	fn poles(&self, table: &Table) -> Poles {
		let standing = Standing::of(table);
		let field = standing.field();
		let me = self.me;
		let low = table.points(me);
		let group = |members: &[usize]| {
			// Both groups have members, so each has a floor and a ceiling.
			let floor = field.floor(members).unwrap_or(Floor {
				points: 0,
				bloc: Vec::new(),
				banked: 0,
				games: 0,
			});
			let ceiling = |winning_out: bool| {
				field
					.ceiling(members, winning_out.then_some(me.index()))
					.unwrap_or_else(|| Ceiling { points: 0, by: me.index() })
			};
			let lose = ceiling(false);
			Group {
				floor: Wins(floor.points),
				bloc: Bloc {
					teams: floor.bloc.iter().map(|&i| Team::ALL[i]).collect(),
					banked: Wins(floor.banked),
					games: floor.games,
				},
				ceiling: StoryCeiling {
					lose: Wins(lose.points),
					win: Wins(ceiling(true).points),
					team: Team::ALL[lose.by],
				},
			}
		};
		let division = group(&self.division);
		let held: Vec<(usize, u16)> = self.division.iter().map(|&i| (i, division.floor.0)).collect();
		let conference = group(&self.conference);
		let whole: Vec<usize> = Team::of_conference(me.conference()).map(Team::index).collect();
		Poles {
			us: Range {
				low: Wins(low),
				high: Wins(low + 2 * u16::from(table.remaining(me))),
			},
			held: Wins(field.floor_held(&self.conference, &held).unwrap_or(conference.floor.0)),
			division,
			conference,
			seeds: Seeds {
				best: solver::best(table, me, table.remaining(me)).map(|s| s.seed),
				worst: field.worst_seed(&whole, |i| Team::ALL[i].division().index(), me.index()),
			},
		}
	}

	/// A group's floor and ceiling at a table, without the bloc.
	fn bounds(field: &Field<'_>, members: &[usize]) -> Bounds {
		Bounds {
			floor: field.floor_points(members).unwrap_or(0),
			ceiling: field.ceiling(members, None).map_or(0, |c| c.points),
		}
	}

	/// The week's results with a side in the conference, in kickoff order
	/// (a game with no time last, games at one time by their sides), each
	/// against the table after the week with it the other way.
	fn results(&self, season: &Season, week: u8, after: &Table, poles: &Poles) -> Vec<StoryResult> {
		let me = self.me;
		let mut games: Vec<&Game> = season
			.week(week)
			.filter(|g| g.home.conference() == me.conference() || g.away.conference() == me.conference())
			.filter(|g| matches!(g.outcome(), Some(Outcome::Won(_))))
			.collect();
		games.sort_by_key(|g| (g.kickoff.unwrap_or(u64::MAX), g.home, g.away));
		let actual = |g: &Group| Bounds {
			floor: g.floor.0,
			ceiling: g.ceiling.lose.0,
		};
		games
			.into_iter()
			.filter_map(|g| {
				let Some(Outcome::Won(winner)) = g.outcome() else {
					return None;
				};
				let flipped = Standing::of(&after.decided(g, Outcome::Won(g.opponent(winner))));
				let field = flipped.field();
				let kind = if g.involves(me) {
					Kind::Own
				} else if g.home.conference() == g.away.conference() {
					Kind::Inside
				} else {
					Kind::Exported
				};
				let side = |members: &[usize], now: Bounds| {
					let other = Self::bounds(&field, members);
					let floor = i32::from(now.floor) - i32::from(other.floor);
					let ceiling = i32::from(now.ceiling) - i32::from(other.ceiling);
					let both_in = members.contains(&g.home.index()) && members.contains(&g.away.index());
					let cohort = if kind == Kind::Own {
						if winner == me {
							Cohort::Best
						} else {
							Cohort::Worst
						}
					} else if both_in {
						Cohort::Forced
					} else if floor > 0 || ceiling > 0 {
						Cohort::Worst
					} else if floor < 0 || ceiling < 0 {
						Cohort::Best
					} else {
						Cohort::Idle
					};
					Move {
						cohort,
						floor: Steps(floor),
						ceiling: Steps(ceiling),
					}
				};
				let touches_division = g.involves(me) || self.division.contains(&g.home.index()) || self.division.contains(&g.away.index());
				Some(StoryResult {
					home: g.home,
					away: g.away,
					winner,
					kind,
					division: touches_division.then(|| side(&self.division, actual(&poles.division))),
					conference: side(&self.conference, actual(&poles.conference)),
				})
			})
			.collect()
	}
}

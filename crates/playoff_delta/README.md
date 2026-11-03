# playoff_delta

The deterministic evaluator behind the Playoff Delta vlog: one NFL regular
season in, one team's week-by-week story out, as the JSON
`paulgsc/some-ui`'s `@some-ui/playoff-delta` stage renders
(`packages/ui/playoff-delta/src/lib/story.ts`, contract version 6).

The short asks, every week, two questions about the team's rivals, and
answers each with an extreme and its proof:

- **The best case:** if everything goes our way, the fewest wins our best
  rival can be held to. We win out, every rival loses each game it plays
  against a team outside the group, and each game between two rivals still
  hands one of them the win: that game is the forcing. The answer comes with
  its **bloc**, the rivals whose games among themselves force it on their
  own.
- **The worst case:** if everything goes against us, the most wins a rival
  can still reach.

Both are asked of our division's three rivals and of our conference's
fifteen, before each week and after it, and each of the week's results is
said to have moved us toward one pole or the other. Nothing is weighted:
no team strengths and no counting of ways, only what can still happen.

## Shape

The crate is pure: no I/O, no clock, no randomness, no feed. A caller builds a
`Season` (every game, with a score once it is final) and calls
`evaluate::evaluate(season, subject)`. Feeds are adapters
outside the crate: `apps/playoff_delta_job` reads ESPN scoreboard responses
and its `playoff-delta` binary writes the story to stdout (its README has the
commands). A scheduled workflow in the data repository runs it weekly and
commits the JSON the stage fetches.

| Module | Owns |
|---|---|
| `league` | the 32 teams, codes as the contract spells them, divisions |
| `season` | the input, checked: equal schedules, one game a week, results in week order |
| `table` | standings at a week, in points (win 2, tie 1) |
| `flow` | a small Dinic max-flow |
| `poles` | league-free: a group's floor and its bloc, its ceiling, and the worst seed |
| `solver` | `best(table, team, extra)`: the best seed, exactly, and a season that reaches it |
| `rank` | the seed one set of final standings gives (PD1), for the worst seed and the brute-force tests |
| `evaluate` | the story's fields, from the poles and the solver |
| `story` | the contract types, serialized to `story.ts` v6 |

`poles` knows no league: entities by index, their banked points, and the
games left as pairs. `evaluate` adapts a `Table` to it.

## The floor and its bloc

For a cap `c`, whether every game between two members of the group can be
decided with no member passing `c` is the classic elimination max-flow
(source → game → its two members → sink, a member's edge carrying its room
under `c`). Feasible caps only grow feasible, so the floor is a binary search
over `c`. One under the floor the flow fails, and its minimum cut names
members whose games among themselves hand out more than their caps hold: that
set forces the floor on its own (Hakimi's orientation theorem, 1965). Members
are then left out one at a time while the rest still force it, so no member
of the reported bloc is spare.

Without banked ties, the floor is the pigeonhole bound
`⌈(wins the bloc has banked + games it plays among itself) ÷ its size⌉`. A
banked tie leaves half-win room nobody can take, which the flow rounds down
per member.

`held` is the conference's floor with each division rival capped at the
division's floor: the two best cases can want opposite winners (a division
rival against the conference leader), and `held` above the conference floor
says they do.

## The worst seed

The team loses out (turning one of its wins into a loss never helps it). A
set of conference teams can all finish strictly above it (PD1) when each takes
its games against teams outside the set and the games inside the set cover
what each still needs, one flow. A set that cannot makes every larger set
impossible too, so the search grows only sets that can, and reads each one's
seed off `rank::from_above`. It stops at the first set that leaves the team
out.

`src/poles/tests.rs` checks the floor, the bloc, `held`, the ceiling and the
worst seed against brute force (every outcome of a dozen games, on random
fields with banked ties and on random NFL tables).

## The solver

For a team that finishes on `F` points, a team "capped" at `F` finishes at or
under it. Whether every remaining game can be decided with no capped team
passing its cap is the same elimination max-flow. The search is over which
teams may finish above `F`:

- **Division winner (seeds 1-4):** no division rival above, seed = 1 + other
  divisions allowed a team above. Eight subsets.
- **Wild card (seeds 5-7):** each division's winner may be above for free,
  each further team costs a seed. Branch and bound: a failed flow's minimum cut
  names capped teams whose games among themselves overflow their caps, and any
  feasible answer frees one of them, so branching on exactly those is complete.
  The same cut bounds how many more frees any answer needs, which prunes.

The story asks it of winning out, for the best seed still reachable.
`src/solver/tests.rs` checks it against brute force (every outcome of eleven
pending games, 30 random leagues), and checks that every scenario it returns
reaches the seed it claims.

A whole season evaluates in well under a second, release build.

## Field definitions

Wins are standings points over two, so a banked tie is half a win. Read from
the table before the week (`poles.before`) or after it (`poles.after`).
Every completed week is a week of the story, the subject's bye included
(`game` null).

- **rivals**: the subject's three division rivals.
- **poles.us**: `low`, the subject's wins losing every game left; `high`,
  winning every one.
- **poles.division**, **poles.conference** (the conference's other fifteen):
  - `floor`: with the subject winning out, the fewest wins the group's best
    rival can be held to;
  - `bloc`: `teams`, rivals whose games among themselves force `floor` on
    their own, none spare; `banked`, their wins; `games`, the games they have
    left against each other;
  - `ceiling`: `lose`, the most wins a rival can reach (it wins every game
    left, the subject's included); `win`, the same with the subject winning
    out; `team`, the first rival in division order that reaches `lose`.
- **poles.held**: the conference's floor with each division rival held at
  the division's floor.
- **poles.seeds**: `best`, the best seed winning out; `worst`, the worst
  losing out, with as many conference teams above as can be. Null is out.
- **results**: the week's results with a side in the conference, in kickoff
  order (a game with no time last, games at one time by their sides); a tied
  game is none of them. Each has `kind` (`own`, `exported` against the other
  conference, `inside` two conference teams), and per group a move
  (`division` null when neither side is the subject or a division rival):
  `floor` and `ceiling`, the group's floor and ceiling (`lose`) after the
  week minus the same with this one result the other way, and its `cohort`:
  - `best` for the subject's win, `worst` for its loss;
  - `forced` when both sides are in the group: one of them had to win it;
  - otherwise `worst` when it raised the floor or the ceiling, `best` when it
    lowered either, `idle` when neither moved. A result with one side in the
    group never moves the two opposite ways.

Cohorts read off these, on the stage: for the division title against
`division`, for the #1 seed against `conference`, under PD1:

- **clinched**: `us.low >= ceiling.lose`;
- **in our hands**: `us.high >= ceiling.win`;
- **needs help**: `us.high >= floor`;
- **out**: otherwise.

## Invariants

- **PD1: ties in the final standings go to the team asked about.**
  - _Claim:_ `solver::best`, `rank::seed` and `poles` apply no NFL
    tiebreaker (head to head, division record, common games, strength of
    victory...); a team capped at `F` may finish level with the asked team,
    and a team level in final standings is not above it.
  - _Falsified by_ a hunk in `src/solver.rs` that makes a cap stricter than
    `target - points` (`Ask::cap`), or a hunk in `src/solver.rs`,
    `src/rank.rs` or `src/poles.rs` that counts a level team as above
    (`Ask::new`'s `forced`, `rank::seed`'s `>`, `Field::all_above`'s
    `+ 1`), or deletes this section.
  - _Scope:_ `crates/playoff_delta`.
  - _Why not enforced:_ the brute-force tests seed by `rank::seed`, the same
    rule, so they check the search, not the rule. Whether the rule is the one
    wanted is the owner's call. Every seed and every cohort is therefore the
    most generous reading.
- **PD2: the crate does no I/O and holds no clock or randomness.**
  - _Claim:_ `evaluate` is a function of its arguments; the same season gives
    the same bytes.
  - _Falsified by_ a dependency other than `serde` under `[dependencies]` in
    this crate's `Cargo.toml`, or a `std::fs`, `std::net`, `std::time`,
    `std::env` or `rand` use under `src/`.
  - _Scope:_ `crates/playoff_delta/src` (tests excepted).
  - _Why not enforced:_ a lint for std paths is mechanical; not yet a rule.
- **PD3: `story.rs` serializes to `story.ts` v6, and only to it.**
  - _Claim:_ every key `story.rs` emits is declared by the client schema,
    and every key the schema declares is emitted.
  - _Falsified by_ a hunk that adds, renames or removes a field in
    `src/story.rs` without the same change to `CONTRACT` in
    `tests/evaluate.rs`.
  - _Scope:_ `src/story.rs`, `tests/evaluate.rs`.
  - _Why not enforced:_ the schema lives in another repository; the key test
    holds a hand copy of it. Validating real output with the client's
    `parseStorySeason` is the full check (done by hand when this landed).
- **PD4: the poles only narrow.**
  - _Claim:_ banking a result, either way, never lowers a group's floor,
    raises its ceiling (`lose`), lowers `us.low` or raises `us.high`, makes
    the best seed better or the worst seed worse.
  - _Falsified by_ a hunk in `src/poles.rs` or `src/evaluate.rs` that makes
    the floor, the ceiling, the range or the seeds read anything but the
    banked points and the games left, or counts a game left as a tie.
  - _Scope:_ `src/poles.rs`, `src/evaluate.rs`.
  - _Why not enforced:_ held by tests that sample its family (every order of
    banking every result): random fields in `src/poles/tests.rs`, an
    invented season in `tests/evaluate.rs`. They draw from it rather than
    enumerate it. `ceiling.win` is not in the claim: a loss to a rival breaks
    the "we win out" it assumes, and raises it.

## Known gaps in contract v6

- **Tied games.** `record` has no `t`. A banked tie counts as half a win
  everywhere, and the record shown drops it. Games left are decided as wins
  and losses only, and a tied game is none of the week's `results`.
- **Tiebreakers.** Every pole and seed is the most generous reading (PD1),
  paulgsc/server#496.
- **A result's move is against the week with that one result flipped.** Two
  results that only move a pole together show as idle each.
- **Postponed games.** A week with a pending game while a later week has
  finals is refused (`ResultsOutOfOrder`), not guessed around.

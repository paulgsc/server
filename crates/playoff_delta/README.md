# playoff_delta

The deterministic evaluator behind the Playoff Delta vlog: one NFL regular
season in, one team's week-by-week story out, as the JSON
`paulgsc/some-ui`'s `@some-ui/playoff-delta` stage renders
(`packages/ui/playoff-delta/src/lib/story.ts`, contract version 4).

The short asks, every week, whether following the team paid, and whether it
is still worth following. Both answers come from one count: **of the ways the
rest of the season can go, in how many does the team get in?** That share is
a fair game: over the ways a week can go, it averages to the share before the
week. So what the league's results did to it, against how far they could
have moved it (the week's spread), is the week's luck; how far next week can
move it (the stake) is what following next week is worth; and once the team
is in whatever happens (clinched), or no way in is left, there is nothing
left to follow.

Every week also keeps its **ways**: every way its results could have gone,
each counted against the same rests of the season, for every team. From
that one table come the best week the team could have had and what each
game was worth to it (its price), the road from that best week to the one
it got, one result at a time in kickoff order, and where the week it got
ranks among all of them, which ranks every team on the week's podium.

The paths keep the first question the engine was built for: if the team won
exactly `x` more games and every other remaining game went its way, what is
the best seed it could reach? The stage reads "no way in" off them.

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
| `solver` | `best(table, team, extra)`: the best seed, exactly, and a season that reaches it; `clinched`: in whatever happens, when a short proof shows it |
| `rank` | the seed one set of final standings gives (PD1), for the count and the solver's brute-force test |
| `count` | `tally(table, team, ways)`: the share of the ways the rest can go in which the team gets in; `slates`: every way a week can go, for every team of a conference, and its spread |
| `ways` | a team's column of `slates`: the best way, each game's price, the road in kickoff order, the rank |
| `evaluate` | the story's fields, from the solver and the count |
| `story` | the contract types, serialized to `story.ts` v4 |

## The solver

For a team that finishes on `F` points, a team "capped" at `F` finishes at or
under it. Whether every remaining game can be decided with no capped team
passing its cap is the classic elimination max-flow. The search is over which
teams may finish above `F`:

- **Division winner (seeds 1-4):** no division rival above, seed = 1 + other
  divisions allowed a team above. Eight subsets.
- **Wild card (seeds 5-7):** each division's winner may be above for free,
  each further team costs a seed. Branch and bound: a failed flow's minimum cut
  names capped teams whose games among themselves overflow their caps, and any
  feasible answer frees one of them, so branching on exactly those is complete.
  The same cut bounds how many more frees any answer needs, which prunes.

`src/solver/tests.rs` checks it against brute force (every outcome of eleven
pending games, 30 random leagues), and checks that every scenario it returns
reaches the seed it claims.

A real season through week 4 (2026, SF) evaluates in about 10 s on one core,
release build; early weeks dominate, since proving "no way in" with most of
the season left is the expensive case.

## The count

Every way the remaining games can go counts once: no team strengths, so it
is counting, not a forecast. Each way is seeded by `rank::seed` under PD1,
the same rule the solver uses, so a share above zero means a way in exists.
Only games with a side in the team's conference are counted, since no other
game changes its seed. When there are few enough of them that every way fits
in `ways` (`evaluate::WAYS`, 100,000), every way is counted (`exact`);
otherwise `ways` are drawn from a fixed-seed generator, so the same table
always gives the same share (PD2). A sample's draws depend only on its number
and the game's place among the conference's pending games, so a table and
itself with one banked result flipped are compared on the same ways, and so
are a pending game's two sides (each way is seeded again with that one game
flipped). That is what keeps a result's leverage, a week's split and a next
week's swing much closer than the share's own error.

`src/count/tests.rs` checks it against brute force (every outcome, exact to
the last bit), a sample against the exact share, and the sign conservation
gives a loss to the other conference.

## The spread

A week's spread is how far its results could move the share. Sampling ways
at random cannot read it (a point's spread is a variance far below a sampled
share's noise), so `count::slates` counts every slate of the week against
the same rests of the season; its module doc has the method, and
`evaluate::RESTS` the sample size and its error. `src/count/tests.rs` checks
a counted spread, plain and with one game held, against brute force, and a
sampled one against it.

## The ways

The same count, kept: `count::slates` keeps, per slate and per team of the
conference, the rests that get the team in, split between the even and the
odd rests, and `ways` reads one team's column. Both conferences are counted
for every played week, for the podium. A price is the best share with a game
won by the side the best way wants, minus the best with it won by the other;
its `error` is half the gap between the price read off each half of the
rests. Which single way is best is not reported: with rests sampled, many
ways land within the error of the best (on 2026 week 7, half of SF's 2,048
within a point), while the prices hold steady. `src/ways/tests.rs` checks
the reading against brute force; `src/count/tests.rs` that a conference
counted together is each team counted alone, and that sampled prices land
near the counted ones.

Counting every team costs what the rest of the evaluation does sixteen times
over per conference, so `evaluate` reads its tables on every core (the
output does not depend on how many). The whole 2025 season evaluates in
about 4 min 20 s on four cores, release build (32 s before the ways).

## Field definitions

Read from the table before the week (`before`) or after it (`after`). Every
completed week is a week of the story, the subject's bye included (`game`
null).

- **paths[x]**: the best seed winning exactly `x` more, `x = 0..=4`, longer
  when the first way in needs more than 4 (so a claim read off the row is
  never "out" while the team is alive). More wins never close a path, so
  "exactly" and "at most" agree. No seed in the row is no way in.
- **share**: `before` and `after`; `own` is the change the subject's own
  result made (the share had it gone as it did, read off the `before`
  count, minus `before`; 0 on a bye or a tie), `league` the rest. `spread`
  is the standard deviation of the share over the ways the week's other
  results could have gone, the subject's own result held as it happened
  (every result on a bye or a tie). `exact` when every way was counted at
  both tables.
- **pending**: games left in the league, before and after.
- **results**: every decided game of the week with a side in the subject's
  conference: `kind` `own`, `exported` (against the other conference) or
  `inside`; `leverage`, the share as it happened minus the share with that
  result flipped and the rest of the week held. Most leverage first.
- **next**: next week's games with a side in the conference, other than the
  subject's, by how far the share swings on them (`swing`: the share if
  `rootFor` wins minus the share if it loses), three at most.
- **stake**: next week, before it is played: `opponent`, the subject's
  opponent (null on its bye); `own`, the share if the subject wins its game
  minus the share if it loses, unsigned (0 on its bye, or when the game
  cannot move the share);
  `spread`, the share's standard deviation over the ways all of next week's
  results can go, the subject's own among them; `exact` when every rest was
  counted. Null once no game is left.
- **stake.pieces**: next week's best way: its games with a side in the
  conference, in kickoff order, each with `want`, `price` and `error` as in
  `ways`.
- **ways**: every way the week's results could have gone, counted against
  the same rests (`count::slates`), from the table before the week with its
  tied games banked first (a tie is none of its ways). `games`: the week's
  games with a side in the conference, in kickoff order (`Game::kickoff`;
  a game with no time sorts last, games at one time by their sides), each
  with its `winner` and its piece: `want`, the side the best way needs, and
  `price`, the best share with `want` winning minus the best with it losing,
  with its `error`. `best` and `worst`: the highest and lowest share any way
  leaves; `actual`, the share the week's results leave; `rank`, the share of
  the ways that leave the team worse than that, ties counted half. `given`:
  the same with the subject's own result held (null on a bye or a tie); its
  own game's piece is its actual winner at no price. `road`: the best share
  still reachable before any result, then after each in kickoff order, from
  `best` down to `actual`; `mean`, the mean share of the ways still possible
  at the same points; `strip`, at the same points, the ways still possible
  per bin of `ways::BINS` (20) equal bins over `[worst, best]`. These are
  read off the same rests as `share.spread`, not as `share`: `actual` and
  `share.after` differ by up to the sample's error (about half a point on
  2025). A piece at no price wants the team's own win when the game is its
  own (a win never hurts it), and otherwise its home side.
- **podium**: every team's own week ranked among its own ways (its `rank`),
  among the teams whose week could move their share by at least
  `evaluate::ELIGIBLE` (one point, its standard deviation over the week's
  ways). `medals`: each eligible team placed 1 to 3, where a place is one
  more than the eligible teams ranked strictly higher (equal ranks share
  it), with its `best`, `worst`, `actual`, own result (`game`: won, lost,
  tie or bye) and its three dearest pieces, each `made` when the week gave
  the side its best way wanted. `place`: the subject's own (null when not
  eligible); `eligible`: how many were.
- **clinched**: proven in whatever happens, under PD1, by one of two short
  proofs (`solver::clinched`): losing out, no division mate can finish
  above it, or at most two teams that can finish above it are not their
  division's winner. Both ignore the games those teams play each other, so
  `false` is "not proven", never "can miss"; a full proof is
  paulgsc/server#521.

The paths follow the prototype the stage was designed against (the deleted
`precompute.js` of #1754's session), with its heuristic search replaced by an
exact one.

## Invariants

- **PD1: ties in the final standings go to the team asked about.**
  - _Claim:_ `solver::best` and `rank::seed` apply no NFL tiebreaker (head
    to head, division record, common games, strength of victory...); a team
    capped at `F` may finish level with the asked team, and a team level in
    final standings is not above it.
  - _Falsified by_ a hunk in `src/solver.rs` that makes a cap stricter than
    `target - points` (`Ask::cap`), or a hunk in `src/solver.rs` or
    `src/rank.rs` that counts a level team as above (`Ask::new`'s `forced`,
    `rank::seed`'s `>`), or deletes this section.
  - _Scope:_ `crates/playoff_delta`.
  - _Why not enforced:_ the brute-force test seeds by `rank::seed`, the same
    rule, so it checks the search, not the rule. Whether the rule is the one
    wanted is the owner's call. Every path and every share is therefore the
    most generous reading.
- **PD2: the crate does no I/O and holds no clock or randomness.**
  - _Claim:_ `evaluate` is a function of its arguments; the same season gives
    the same bytes.
  - _Falsified by_ a dependency other than `serde` under `[dependencies]` in
    this crate's `Cargo.toml`, or a `std::fs`, `std::net`, `std::time`,
    `std::env` or `rand` use under `src/`.
  - _Scope:_ `crates/playoff_delta/src` (tests excepted).
  - _Why not enforced:_ a lint for std paths is mechanical; not yet a rule.
- **PD3: `story.rs` serializes to `story.ts` v4, and only to it.**
  - _Claim:_ every key `story.rs` emits is declared by the client schema.
  - _Falsified by_ a hunk that adds, renames or removes a field in
    `src/story.rs` without the same change to `CONTRACT` in
    `tests/evaluate.rs`.
  - _Scope:_ `src/story.rs`, `tests/evaluate.rs`.
  - _Why not enforced:_ the schema lives in another repository; the key test
    holds a hand copy of it. Validating real output with the client's
    `parseStorySeason` is the full check (done by hand when this landed).

## Known gaps in contract v4

- **Tied games.** `record` has no `t`. A banked tie counts as half a win in
  every computation, and the record shown drops it. Remaining games are
  decided as wins and losses only, in the solver and the count, and a tied
  game is not one of a week's `results`.
- **The week's spread is sampled** (`evaluate::RESTS`), and the luck read
  off it carries that error.
- **Tiebreakers in the count.** The count seeds each way under PD1, like the
  solver, so a share is the most generous reading too (paulgsc/server#496).
- **`collapse`** needs an in-game win-probability feed; the ESPN adapter
  reads scoreboards only, so it is never set.
- **Postponed games.** A week with a pending game while a later week has
  finals is refused (`ResultsOutOfOrder`), not guessed around.
- **A price is a max of sampled shares**, so it leans high: the best of
  many noisy ways beats the true best. `error` says how far the two halves
  of the rests disagree, not that bias.
- **`ways` and `share` are two samples.** `ways.actual` and `share.after`
  (and `ways.mean[0]` and `share.before`) differ by up to about half a point
  (the rests' sampling error; most 2025 weeks were within a quarter);
  neighbouring weeks' `ways` come from different rests too.

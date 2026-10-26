# playoff_delta

The deterministic evaluator behind the Playoff Delta vlog: one NFL regular
season in, one team's week-by-week story out, as the JSON
`paulgsc/some-ui`'s `@some-ui/playoff-delta` stage renders
(`packages/ui/playoff-delta/src/lib/story.ts`, contract version 2).

The question every week: **how bad can we be, and still make the playoffs?**
If the team won exactly `x` more games and every other remaining game went its
way, what is the best seed it could reach? The paths, collisions and the lowest
bar in the league are that question asked about another team, or about the
week with one result flipped.

That answer can only hold or get worse as results come in, so the story also
asks a second question: **of the ways the rest of the season can go, in how
many does the team get in?** The share, its weekly change and each result's
leverage are that count, asked of the same tables.

## Shape

The crate is pure: no I/O, no clock, no randomness, no feed. A caller builds a
`Season` (every game, with a score once it is final) and calls
`evaluate::evaluate(season, previous_season, subject)`. Feeds are adapters
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
| `solver` | `best(table, team, extra)`: the best seed, exactly, and a season that reaches it; `impossible`: when none does, the pigeonholes that prove it |
| `rank` | the seed one set of final standings gives (PD1), for the count and the solver's brute-force test |
| `count` | `tally(table, team, ways)`: the share of the ways the rest can go in which the team gets in |
| `evaluate` | the story's fields, from the solver and the count |
| `story` | the contract types, serialized to `story.ts` v2 |

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
gives a loss to the other conference. The whole 2025 season evaluates in
about 30 s, release build.

## Field definitions

Read from the table before the week (`before`) or after it (`after`). Every
completed week is a week of the story, the subject's bye included (`game`
null).

- **paths[x]**: the best seed winning exactly `x` more, `x = 0..=4`, longer
  when the first way in needs more than 4 (so a claim read off the row is
  never "out" while the team is alive). More wins never close a path, so
  "exactly" and "at most" agree.
- **claim**: the first `x` with a way in, and its seed. Found by binary search.
- **share**: `before` and `after`; `own` is the change the subject's own
  result made (the share had it gone as it did, read off the `before`
  count, minus `before`; 0 on a bye or a tie), `league` the rest. `exact` when every way
  was counted at both tables.
- **pending**: games left in the league, before and after.
- **results**: every decided game of the week with a side in the subject's
  conference: `kind` `own`, `exported` (against the other conference) or
  `inside`; `leverage`, the share as it happened minus the share with that
  result flipped and the rest of the week held; `moved`, the same flip read
  on the paths: compare the subject's claim, then its open grid cells,
  `helped` when what happened was better (`null` for the subject's own game).
  Most leverage first.
- **next**: next week's games with a side in the conference, other than the
  subject's, by how far the share swings on them (`swing`: the share if
  `rootFor` wins minus the share if it loses), three at most.
- **path**: the after claim, with the solver's scenario: named rivals, then
  division mates, then the conference teams it holds down that had to lose
  the most of what they have left, six in all. `free` marks a team the
  scenario lets finish above the subject: its record there is one choice,
  not a requirement.
- **boundary**: when the claim needs at least one more win, why one fewer
  final win (`wins`) can't get in. `division`: a division mate already past
  that line (`above`), or the pigeonhole the solver's minimum cut names:
  division teams (the subject among them or not) whose remaining games
  among themselves (`games`) are more than their room under the line adds
  up to. `wildCard`: the teams already past the line; when one group closes
  the wild card on its own, that group (the search's first minimum cut) with
  `mustPass`, how many of it besides one division winner per division must
  finish above (enough to leave none); and `cases`, the placements of who
  else finishes above the search tried, every one overflowing (0 when the
  teams already above close it).
- **rivals[]**: each named rival's claim and share, before and after, and
  its game (null on its bye).
- **collisions**: a result where the winner worst for a rival is not the one
  best for you (the rival's own game excluded), and games between two rivals.
- **points** (futility): each game a division mate or rival plays against
  anyone else is a possible point, scored when it lost. A game between two
  of them is not counted: one of them had to lose.
- **benchmark**: `lastSeason` is the subject's claim after the same week of
  the previous season; `lowestBar` the team in either conference whose claim
  needs the fewest final wins (then the better seed, then code).

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
- **PD3: `story.rs` serializes to `story.ts` v2, and only to it.**
  - _Claim:_ every key `story.rs` emits is declared by the client schema.
  - _Falsified by_ a hunk that adds, renames or removes a field in
    `src/story.rs` without the same change to `CONTRACT` in
    `tests/evaluate.rs`.
  - _Scope:_ `src/story.rs`, `tests/evaluate.rs`.
  - _Why not enforced:_ the schema lives in another repository; the key test
    holds a hand copy of it. Validating real output with the client's
    `parseStorySeason` is the full check (done by hand when this landed).

## Known gaps in contract v2

- **Tied games.** `record` has no `t`. A banked tie counts as half a win in
  every computation, and the record shown drops it. Remaining games are
  decided as wins and losses only, in the solver and the count, and a tied
  game is not one of a week's `results`.
- **Tiebreakers in the count.** The count seeds each way under PD1, like the
  solver, so a share is the most generous reading too (paulgsc/server#496).
- **`collapse`** needs an in-game win-probability feed; the ESPN adapter
  reads scoreboards only, so it is never set.
- **Postponed games.** A week with a pending game while a later week has
  finals is refused (`ResultsOutOfOrder`), not guessed around.

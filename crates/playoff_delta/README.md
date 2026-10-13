# playoff_delta

The deterministic evaluator behind the Playoff Delta vlog: one NFL regular
season in, one team's week-by-week story out, as the JSON
`paulgsc/some-ui`'s `@some-ui/playoff-delta` stage renders
(`packages/ui/playoff-delta/src/lib/story.ts`, contract version 1).

The question every week: **how bad can we be, and still make the playoffs?**
If the team won exactly `x` more games and every other remaining game went its
way, what is the best seed it could reach? Everything else in the story
(paths, rivals' worst weeks, collisions, the lowest bar in the league) is that
question asked about another team, or about the week with one result flipped.

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
| `solver` | `best(table, team, extra)`: the best seed, exactly, and a season that reaches it |
| `evaluate` | the story's fields, from the solver |
| `story` | the contract types, serialized to `story.ts` v1 |

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

## Field definitions

Read from the table before the week (`before`) or after it (`after`).

- **paths[x]**: the best seed winning exactly `x` more, `x = 0..=4`, longer
  when the first way in needs more than 4 (so a claim read off the row is
  never "out" while the team is alive). More wins never close a path, so
  "exactly" and "at most" agree.
- **claim**: the first `x` with a way in, and its seed. Found by binary search.
- **alive**: has a claim at `before`.
- **templates**, from the subject's side, roles from `before`:
  `division` and `rivals` are every game of a division mate or named rival;
  `above` is alive conference teams with more points; `exported` is alive
  conference teams in cross-conference games; `spoilers` is conference games
  with exactly one side alive (`team` = the eliminated side); `forced` is
  conference games with both alive (`team` = the winner). `yourWay` is "the
  watched team lost", except a spoiler winning, and on `forced` it is `moved`.
- **moved**: flip the result, hold the rest of the week, compare the
  subject's claim, then its open grid cells. `helped` when what happened was
  better. Results outside the conference, and the subject's own game, are
  `null`.
- **path**: the after claim, with the solver's scenario: named rivals, then
  division mates, then the conference teams that had to lose the most of what
  they have left, six in all.
- **rivals[].worst**: every result of the week whose flip alone would leave
  the rival's claim worse, flipped together; never better than `after`.
- **collisions**: a result where the winner worst for a rival is not the one
  best for you (the rival's own game excluded), and games between two rivals.
- **points** (futility): each game a division mate or rival plays is a
  possible point, scored when it lost; a game between two of them counts
  once and scores.
- **benchmark**: `lastSeason` is the subject's claim after the same week of
  the previous season; `lowestBar` the team in either conference whose claim
  needs the fewest final wins (then the better seed, then code).

These follow the prototype the stage was designed against (the deleted
`precompute.js` of #1754's session), with its heuristic search replaced by an
exact one.

## Invariants

- **PD1: ties in the final standings go to the team asked about.**
  - _Claim:_ `solver::best` applies no NFL tiebreaker (head to head, division
    record, common games, strength of victory...); a team capped at `F` may
    finish level with the asked team.
  - _Falsified by_ a hunk in `src/solver.rs` that makes a cap stricter than
    `target - points` (`Ask::cap`), counts a level team as above
    (`Ask::new`'s `forced`, `seed_of` in the tests), or deletes this section.
  - _Scope:_ `crates/playoff_delta`.
  - _Why not enforced:_ the brute-force test encodes the same rule, so it
    checks the search, not the rule. Whether the rule is the one wanted is the
    owner's call. Every path is therefore the most generous reading.
- **PD2: the crate does no I/O and holds no clock or randomness.**
  - _Claim:_ `evaluate` is a function of its arguments; the same season gives
    the same bytes.
  - _Falsified by_ a dependency other than `serde` under `[dependencies]` in
    this crate's `Cargo.toml`, or a `std::fs`, `std::net`, `std::time`,
    `std::env` or `rand` use under `src/`.
  - _Scope:_ `crates/playoff_delta/src` (tests excepted).
  - _Why not enforced:_ a lint for std paths is mechanical; not yet a rule.
- **PD3: `story.rs` serializes to `story.ts` v1, and only to it.**
  - _Claim:_ every key `story.rs` emits is declared by the client schema.
  - _Falsified by_ a hunk that adds, renames or removes a field in
    `src/story.rs` without the same change to `CONTRACT` in
    `tests/evaluate.rs`.
  - _Scope:_ `src/story.rs`, `tests/evaluate.rs`.
  - _Why not enforced:_ the schema lives in another repository; the key test
    holds a hand copy of it. Validating real output with the client's
    `parseStorySeason` is the full check (done by hand when this landed).

## Known gaps in contract v1

- **Byes.** A week's `game` is required, so the subject's bye week is not
  emitted; its results count, and show in the next week's `before`. A rival on
  its bye is left out of that week's `rivals`.
- **Tied games.** `record` has no `t`. A banked tie counts as half a win in
  every computation, and the record shown drops it. Remaining games are
  decided as wins and losses only.
- **`collapse`** needs an in-game win-probability feed; the ESPN adapter
  reads scoreboards only, so it is never set.
- **Postponed games.** A week with a pending game while a later week has
  finals is refused (`ResultsOutOfOrder`), not guessed around.

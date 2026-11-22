# game_scenarios

The fixed set of scenarios a game-week bite is said about (Game Week Bites,
paulgsc/some-ui#2074, story GWB1, requirement GW24). A bite is commentary on
one scenario from a closed set: before kickoff (`pre`), during the game (`in`),
after the final (`post`), and the owner's own watching (`you`). Every game is
read through one team, "us"; an outcome "for us" is the one that team wants.

Each entry has a dotted `id` (`in.third-long`), its `phase`, a short `name`,
what it `covers`, the `outcomes` home can later fill in (kebab-case slugs from
our side), and the `moment`: the kind of moment the scenario engine (GWB3)
writes its rule against.

Beside the scenarios, the dump lists the closed dimensions a bite is read
along (version 2, #3048): `halves` (`first`, `second`, `overtime`: the half
an in-game bite was in) and `big_play`, the big-play algebra's `sizes` (each
with the fewest yards it starts at) and `kinds` (each with the smallest size
it counts from), and `units` (#3049: `offense`, `defense`, `special-teams`,
`coaching`: what part of a team a bite is about; whose is the team's code
beside it). Each is an enum here (`Half`, `Size`, `Kind`, `Unit`), so the
engine and the phone's words agree on every value.

Version 3 (#3041, D24–D26) adds `claims`, the claim algebra a bite's claim
is folded into and home's numbers test week after week:

- `measures`: what home computes per team from the plays it keeps (`Measure`),
  each with the `unit` it is of and which way it is `better` (`higher`,
  `lower`, or null for a style: going for it on fourth down). Each is one
  home can compute from the plays it keeps (the verdict pass that does is
  #3041's server half, still to come). Eight of the prototype's nine:
  challenges won needs a reader of ESPN's challenge text, and the recorded
  replay holds one challenge to prove it on, so it waits for a later version.
- `shapes`: where a team's measure stands (`Shape`): `rank` among the 32
  (`highest` or `lowest` N), `vs-league` and `vs-team` (`above` or `below`),
  `trend` (`rising` or `falling`). A side is the direction of the value, never
  good or bad; `better` says which is good, so "worst at pass rush" is
  `lowest` on a `higher` measure, and a claim on a style reads the same way.
- `deadlines` (`Deadline`): `after-games`, `by-week`, `season-end`.
- `verdicts` (`Verdict`): `holding`, `against-so-far`, `too-early`,
  `settled-right`, `settled-wrong`, and `withdrawn` for a claim whose bite was
  deleted (never lost).

A claim's own values (whose, N, the named team or week, the owner's reason)
belong to the ledger, not the catalog.

## One set, two readers

- **The engine** compiles against `Scenario`.
- **The phone's recorder** reads paulgsc/some-ui's copy,
  `game-scenarios.snapshot.json` (in `@some-ui/game-scenarios`), which keeps
  the icons and the outcome words beside it, keyed by id.

`dump-game-scenarios` writes the set as JSON. On every PR that touches
`crates/**`, `.github/workflows/routes.yml` dumps it and
`scripts/check_client_fixtures.py` fails if some-ui's copy differs.

## Changing the set is a release

A bite records the `CATALOG_VERSION` it was said under, and its scenario is
read against that version, so a released version never changes. To add,
remove or reword a scenario, or to change a dimension (v3 changed only one,
adding `claims`):

1. Change `Scenario` and its `entry`, appending a new scenario at the end of
   both the enum and `Scenario::ALL`; or change the dimension's enum and its
   `ALL`.
2. Bump `CATALOG_VERSION` and write the new version's file beside the old ones:

   ```sh
   cargo run -q --bin dump-game-scenarios > crates/game_scenarios/testdata/catalog.v<N>.json
   ```

3. Put the case for the change in the PR (the owner decides each one), and
   update some-ui's copy, icon and words in the PR that pairs with it. Land
   that one first, or this PR's `Routes / Check some-ui against it` fails.

The tests fail on a set that differs from its version's file, on a missing
earlier file, and on a version bumped with no change.

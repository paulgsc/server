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
remove or reword a scenario:

1. Change `Scenario` and its `entry`, appending a new scenario at the end of
   both the enum and `Scenario::ALL`.
2. Bump `CATALOG_VERSION` and write the new version's file beside the old ones:

   ```sh
   cargo run -q --bin dump-game-scenarios > crates/game_scenarios/testdata/catalog.v<N>.json
   ```

3. Put the case for the change in the PR (the owner decides each one), and
   update some-ui's copy, icon and words in the PR that pairs with it. Land
   that one first, or this PR's `Routes / Check some-ui against it` fails.

The tests fail on a set that differs from its version's file, on a missing
earlier file, and on a version bumped with no change.

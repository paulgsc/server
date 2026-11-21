# scenario_engine

What can be happening now in an NFL game, and what just happened, as
scenarios from the fixed set (`crates/game_scenarios`). Game Week Bites,
paulgsc/some-ui#2074, story GWB3 (requirements GW7, GW8, GW11).

```rust
offer(&game, &log, now, Mode::Standard, Watching::Unknown) -> Offer
```

- `game`: the teams, kickoff, status as of `now`, the next snap's
  `Situation`, the forecast, and the `Reading`: the team every game is read
  through (`SF`) and the side it wants to win (itself in its own game; in
  another, the side Playoff Delta's pieces want, `want` in
  `story::NextPiece`; `None` for a game that cannot move it).
- `log`: the play log as `game_feed_repo::log` reads it back
  (`nfl_feed::log::Entry`).
- `now`: the time, with the owner's UTC offset (it places Full mode's 8 AM).
- `Mode`: Light, Standard or Full: cards 2/4/6, "just happened" 1/2/3 kept
  60 s/90 s/3 min, pregame from 1 h/3 h/8 AM on game day before kickoff.
- `Watching`: the phone's flag for the game.

The `Offer` carries the live strip (clock, possession, down and distance,
score), the scenarios that fit the moment with a line each, most specific
first, "just happened" (recent plays as scenarios, with how long ago), and
`engine_version` with `catalog_version`. It serializes to JSON for GWB4's
socket; a scenario is written as its catalog id.

## The moment rules

One rule per catalog `Moment` (`src/rules.rs`). A rule's weight, kept
there, is its specificity; the offer lists the heaviest first, ties in
catalog order.

| Moment | Scenarios | On offer now when | "Just happened" from a play that |
| --- | --- | --- | --- |
| `pregame` | `pre.stakes`, `pre.weather`, `pre.injury-report`, `pre.venue` | the mode's window is open and no play is in the log (stakes only with a side wanted) | |
| `kickoff` | `pre.coin-toss` | 15 min before kickoff until the opening kickoff is in the log | is the opening kickoff |
| `opening-drive` | `in.first-drive` | the team with the ball is on its first possession of the game | ends a team's first possession |
| `lead-change` | `in.fell-behind` | the latest score put the side wanted behind, or back ahead, and at most 3 snaps followed | is that score |
| `third-and-long` | `in.third-long` | 3rd down, 7 or more to go | was snapped on one: converted or stopped |
| `fourth-down` | `in.fourth-down` | 4th down | was snapped on one: went for it, field goal, punted |
| `try` | `in.two-point` | the latest snap is a touchdown whose try is not in the log | is a touchdown |
| `snap` | `in.play-call` | any snap; 3rd or 4th and 2 or less, or 3 yards from the goal line, weighs more | was such a short-yardage call |
| `red-zone` | `in.red-zone` | inside the 20 | ended a trip inside the 20 |
| `two-minute` | `in.two-minute` | 2nd or 4th quarter, 2:00 or less | |
| `kick` | `in.kick` | the try is to come, or 4th down within a 57-yard field goal | is a field goal, or a missed extra point |
| `turnover`, `sack`, `penalty`, `injury` | `in.turnover`, `in.sack`, `in.penalty`, `in.injury` | never ahead of time | was an interception or lost fumble, a sack, a flag, a player hurt or back |
| `big-play` | `in.big-play` | never ahead of time | gained 20+ by the big-play algebra (below) |
| `halftime` | `in.halftime` | from ESPN's end of the half until the second half's kickoff | |
| `onside-kick` | `in.onside-kick` | never ahead of time | was a kickoff ESPN calls onside, with who recovered it (ESPN's "recovered by", else the receivers) |
| `final` | `post.result`, `post.paths`, `post.pattern` | the game is final (paths only with a side wanted) | |
| `watching` | `you.*` | the game is final, a side is wanted, the result is not a tie, and the phone has said whether the owner watched | |

### The big-play algebra (#3048)

A big play in "just happened" carries a `BigPlay` beside its line: its
`size` (`big` 20+, `huge` 40+, `memorable` 60+), its `kind` (`pass`, `run`,
`kick-return`, `punt-return`, `interception-return`, `fumble-return`), `by`
(the side that gained: the returner's on a return, so a pick-six is the
defense's, never the offense's pass), `yards`, `scored`, and `good` (for the
side wanted; none without one). Sizes and kinds are `game_scenarios`'
closed enums, and the catalog dump lists them. A kick return is big from
`huge`: 20 to 39 yards is the ordinary one, out of the end zone to about
the 25 (`Kind::least`). `read::gain_by` reads which side gained and how far.

### One ranked list, and what a bite is about (#3049)

`Offer::ranked` holds every scenario of the set once, for the phone's "what
was that about" list: the cards on offer now, exactly as `scenarios` lists
them (`now`); then the always plausible, what the game means for the paths
and whether it fits a pattern (`always`); then the rest in the set's order,
least plausible last but still pickable (`rest`). Code ranks it, never a
model.

Each entry carries its default unit (`game_scenarios::Unit`) and whose, the
team's code (`unit_side`), never ours or theirs (the phone reads that from
the side wanted, and a game with none still has a defense):

- a call (4th down, the try, the two-minute drill) is the deciding team's
  `coaching`; halftime is the side wanted's, or no one team's;
- a kick is the kicking team's `special-teams`; a kick or punt return big
  play, the returners';
- anything else in the game is, with a side wanted, its `offense` when it
  had the ball and its `defense` when the other team did; with none, the
  `offense` of the team with the ball (the snap to come's, or for what just
  happened the latest play's).

None before kickoff, after the final, or about the owner. The owner can
change it with one tap; home keeps what the phone sends, once, with the bite.

A game ESPN calls over that was not completed (postponed or cancelled), with
no play in the log, offers nothing, and its strip says "Not played".

"Tuned in late" is offered when the owner started watching after kickoff
with the side wanted behind.

Every line and the strip come from `src/words.rs`, in the register of the
canvas board `Say it: what can be happening now`: "3rd and 8, down 4",
"1:52 left, 1 timeout", "NE, 4th and 2 at the CHI 43, up 6", "Flag on NE,
false start". A line about the team with the ball names it unless it is the
side wanted; a margin is that team's. Timeouts left are counted from the
log's timeout plays in the half.

## The next snap is not in the log

The log holds each play as it stood at its own snap, so the snap still to
come is not in it. `Game::situation` carries it; without one, the engine
offers nothing that needs the down (third and long, fourth down, red zone,
the opening drive, a play call, a field goal try) and the strip leaves
possession and the down out.

Live, it is ESPN's: `game_feed_repo::situation(conn, game, now)` reads it
from the newest summary home kept (`nfl_feed::situation`, paulgsc/server#570),
and `Situation::from(&live)` converts it. ESPN marks no team between a
kickoff or a score and the next snap, and then neither does the situation,
so the down rules wait for the snap after. A replay, which knows the next
play, passes `Situation::before(next)`. ESPN's timeouts left are carried by
`nfl_feed` but not read here: the two-minute line still counts them from the
log's timeout plays.

## Invariants

- **SE1: pure.** No I/O, no clock, no randomness: `now` is an argument, and
  the same arguments always give the same offer. The crate depends on
  `chrono`, `serde`, `game_scenarios` and `nfl_feed`, none of which does I/O.
- **SE2: replayable at any time.** An entry home saw after `now`, or of
  another game, is left out, and each play is read at its latest version
  seen by `now`; "just happened" counts from when home first saw a play. So
  the whole log replayed at a bite's time gives the offer the phone could
  have shown then (GWB7). Tested: `the_whole_log_at_a_time_offers_what_the_log_held_then`.
- **SE3: one rule per moment.** Whether a scenario is offered is decided only
  by the rule for its catalog `Moment`, in `src/rules.rs`; a scenario added
  to the catalog with an existing moment is offered by that moment's rule.
- **SE4: the person's terms.** Every string in an offer comes from
  `src/words.rs`, and none says a field name, an id, `None`, or ESPN's
  spelling of a play type. Tested over every play of the recorded game in
  every mode: `every_line_is_in_the_persons_terms`.
- **SE5: read through one team.** Nothing that needs a side wanted (stakes,
  fell behind, paths, `you.*`) is offered for a game with none.

## Tests replay a recorded game

`tests/replay.rs` replays NE at CHI (2026-10-22, `401873010`, GWB2's
recorded fixture in `crates/nfl_feed/testdata`) through the play log's own
rule as the poller would have kept it: a summary every 15 s, ESPN 5 s behind.
Before every snap, and 20 s after every play home saw, it takes the offer in
Standard mode, read through SF wanting CHI. It checks that every snap whose
own fields make it a moment has that moment's scenario on offer and the most
specific first, and that every play ESPN typed as an event shows up in "just
happened" as its scenario: the moments at least 7 times in 10 (the beta's
bar), the events at least 8 times in 10.

The whole replay is written to `testdata/replay-401873010.txt`, one line per
moment, so a rule change shows in its PR as the lines it changed on a real
game, and the owner can step through it. After a deliberate change, rebuild
it and read the diff:

```sh
BLESS_REPLAY=1 cargo test -p scenario_engine --test replay
```

Changing a rule or a line is a change to what a bite was offered under: bump
`ENGINE_VERSION` in the same change.

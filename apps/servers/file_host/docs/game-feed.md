# The game feed: ESPN's play-by-play, kept at home

Design note for GWB2 (paulgsc/some-ui#2076, epic paulgsc/some-ui#2074). The
owner records voice bites about NFL games on the phone. Later stories read
the live game state home keeps here: GWB3 (a pure scenario engine offering
what can be happening now) and GWB7 (a reconciler matching each bite to its
play). The owner chose ESPN's unofficial site API on 2026-10-25; it is the
same API `apps/playoff_delta_job` reads.

| Piece | What it is |
| --- | --- |
| `crates/nfl_feed` | The adapter. Parses a scoreboard and a summary into games, statuses and plays. No I/O: the same bytes always give the same output. |
| `crates/db/game_feed` (`game_feed_repo`) | The store. Keeps every response, then appends the plays it adds; replays the log from the kept responses. |
| `migrations/20261028000200_create_game_feed.up.sql` | `game_feed_response` (every raw response) and `game_feed_play` (the play log). |
| `src/game_feed.rs`, `src/game_feed/pass.rs` | The worker: configuration, the schedule, one pass, the `Fetch` port. |
| `src/metrics/game_feed.rs` | Its metrics. |

No HTTP route serves these tables yet: a route would change the client's route
snapshot, and is a later story's.

## The worker

Off unless `GAME_FEED_ENABLED=true`. It wakes every 5 s and fetches only what
its schedule says is due, so an idle wake does no I/O:

- **The scoreboard** (`/scoreboard`, the current week) every
  `GAME_FEED_SCOREBOARD_SECONDS` (600), to learn the week's games, their
  kickoffs and their states. Due at once on start, so a restart catches up.
- **A game's summary** (`/summary?event=<id>`) every `GAME_FEED_LIVE_SECONDS`
  (15) while the game is live, and from its kickoff while the scoreboard still
  calls it `pre`, since the scoreboard can be ten minutes stale at kickoff. A
  game still `pre` four hours after kickoff (delayed, postponed) waits for the
  scoreboard instead.
- **One last summary** once a game is over, then nothing more for it. A game
  first seen over (home was off during it) is fetched once, unless home
  already keeps a summary that says it is over, so a restart does not fetch
  the week's finished games again.
- **Backoff.** A 429, any other non-200, a body that does not read, or no
  answer pushes that one target out by its own cadence doubled per failure in
  a row, to at most `GAME_FEED_BACKOFF_CAP_SECONDS` (900). Its next good
  answer resets it.

GW6 asks that a play show up within a minute, 8 times in 10, while a game of
the week is live. The worker's own share is at most one cadence plus one wake,
20 s at the defaults; the rest is ESPN's delay, which its responses cache for
5 s (`cache-control: max-age=5`).

## Invariants

- **GF1: every answer is kept, raw, before it is read.** Each response, a 429
  or an HTML error page included, is a `game_feed_response` row with its URL,
  `fetched_at`, status, SHA-256 and bytes, written in the transaction that
  appends the plays read from it (`game_feed_repo::keep`). A fetch with no
  answer (timeout, connection refused) stores nothing and is logged by kind.
- **GF2: append-only.** No row of either table is updated or deleted; both
  tables' triggers abort it. A play ESPN revises is a new `game_feed_play` row
  whose `supersedes` names the row before; a play seen again unchanged adds
  nothing. A play's versions are one chain (one first row per play, each row
  superseded at most once, held by unique indexes, and a revision names a row
  of the same play, held by a trigger).
- **GF3: the log replays** (GW10). Which responses are read is one rule
  (`game_feed_repo::read`: a 200, parsed as what was fetched), and what a
  summary adds is one rule (`nfl_feed::log::changes`). `seen_at` is the
  response's `fetched_at`, so `game_feed_repo::replay` over the kept responses
  gives exactly the rows `game_feed_repo::log` reads back. Tested on the real
  fixtures, a revision included (`game_feed_repo`'s tests, and
  `game_feed::pass`'s end-to-end test).
- **GF4: public data only.** The worker reads ESPN's public API, sends no
  credential, and writes only these two tables; neither holds anything about
  a person, so neither is subject-scoped (`privacy::NOT_SUBJECT_SCOPED`).
- **GF5: logs and metrics carry counts and kinds,** never a URL, a body or a
  play's text.

## Metrics

`game_feed_enabled`, `game_feed_last_attempt_timestamp_seconds` (moves on every
wake), `game_feed_pass_failing` (a storage failure), `game_feed_games_followed`,
`game_feed_fetches_total{kind}` (what each fetch read as: `scoreboard`,
`summary`, `rate_limited`, `status`, `not_json`, `not_an_object`, or why there
was no answer: `timeout`, `connect`, `transport`) and
`game_feed_plays_added_total`. The GAME FEED dashboard (`infra/grafana/dashboards/game-feed.jsonnet`) shows them.

## What ESPN's data is like

Read from the live API on 2026-10-28; the fixtures in `crates/nfl_feed/testdata`
are those responses.

- Kickoffs are written without seconds (`2026-10-30T00:15Z`), which RFC 3339
  does not allow.
- Numbers come as numbers or as strings (`sequenceNumber: "4000"`, scores as
  strings on the scoreboard).
- `down` is 0 on a kickoff or timeout; the adapter reads it as no down.
- On a kickoff, `start.team` is the kicking team.
- Weather is a forecast: present before kickoff (`gameInfo.weather` in the
  summary, `weather` on the scoreboard event), and gone from both once the
  game is over, even for an outdoor stadium.
- Every play of a finished game carries the same `modified` stamp, so it says
  nothing about which play changed; the log compares the plays themselves.
- A live game's plays are in `drives.previous` and `drives.current`.

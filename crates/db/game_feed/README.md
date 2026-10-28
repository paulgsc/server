# game_feed_repo

Home's record of ESPN's NFL feed: every response `file_host`'s `game_feed`
worker fetched, kept raw (`game_feed_response`), and the play log read from
them (`game_feed_play`). Both are append-only: a play ESPN revises is a new row
superseding the one before, and the log can be rebuilt from the kept responses
(`replay`) to the same rows (`log`). Public game data, not subject-scoped.

The rules are in `src/lib.rs`, in
`migrations/20261028000200_create_game_feed.up.sql`, and in
`apps/servers/file_host/docs/game-feed.md`.

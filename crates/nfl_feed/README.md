# nfl_feed

ESPN's NFL site API (`site.api.espn.com/apis/site/v2/sports/football/nfl`),
parsed: the week's games from `/scoreboard`, and one game's status, weather
and plays from `/summary?event=<id>`. No I/O: the caller fetches, and the same
bytes always give the same output, which is what lets home replay a game from
the responses it kept. `log` holds the one rule for what a summary adds to the
append-only play log.

`file_host`'s `game_feed` worker is the caller, and `game_feed_repo` the
store; `apps/servers/file_host/docs/game-feed.md` describes both and what
ESPN's data is like.

`testdata/` holds real responses from 2026-10-28: the week-8 scoreboard
(every game still to play), the summary of a finished game (NE at CHI,
`401873010`) and the summary of a game before kickoff (CAR at GB,
`401873023`, with its weather forecast). Each was trimmed by removing keys the
adapter does not read (logos, links, statistics, the box score, news and the
like) and re-serialized compactly; no value was changed.

# nfl_feed

ESPN's NFL site API (`site.api.espn.com/apis/site/v2/sports/football/nfl`),
parsed: the week's games from `/scoreboard`, and one game's status, weather
and plays from `/summary?event=<id>`. No I/O: the caller fetches, and the same
bytes always give the same output, which is what lets home replay a game from
the responses it kept. `log` holds the one rule for what a summary adds to the
append-only play log. `situation` reads the snap to come while a game is live
(down, distance, spot, the team with the ball, timeouts left), from either
response, and keeps it out of the plays (paulgsc/server#570).

`file_host`'s `game_feed` worker is the caller, and `game_feed_repo` the
store; `apps/servers/file_host/docs/game-feed.md` describes both and what
ESPN's data is like.

`testdata/` holds real responses from 2026-10-28: the week-8 scoreboard
(every game still to play), the summary of a finished game (NE at CHI,
`401873010`) and the summary of a game before kickoff (CAR at GB,
`401873023`, with its weather forecast). Each was trimmed by removing keys the
adapter does not read (logos, links, statistics, the box score, news and the
like) and re-serialized compactly; no value was changed.

No NFL game was live when the situation was written, so its fixtures are
three live college games ESPN served from the same site API on 2026-10-31
(`football/college-football`, the same spelling): the scoreboard and each
game's summary, fetched together so each pair is at the same last play
(`cfb-*-live.json`; FLA with the ball as the away team, WAKE as the home team,
and NEV just after a kickoff with no team marked). They were trimmed the same
way, and each summary to its current drive and the drive before it.

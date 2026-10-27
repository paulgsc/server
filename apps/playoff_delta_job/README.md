# playoff_delta_job

The weekly job behind the Playoff Delta vlog: ESPN scoreboard files in, one
team's story JSON out (`playoff-delta` binary). The math is
`crates/playoff_delta`; this crate only adapts ESPN's responses
(`src/espn.rs`) and writes the result. It does no network I/O: the caller
fetches the scoreboards, so the same files always give the same bytes.

It depends on nothing else in the workspace, so it builds without a
database, which is what lets the data repository's scheduled workflow build
and run it on its own.

```sh
for w in $(seq 1 18); do
  curl -sf "https://site.api.espn.com/apis/site/v2/sports/football/nfl/scoreboard?dates=2026&seasontype=2&week=$w&limit=40" \
    -o "espn/2026/week-$(printf %02d "$w").json"
done
cargo run -q --release -p playoff_delta_job -- --season 2026 --team SF --scoreboards espn/2026 > SF.json
```

`--rivals` and `--previous` are still accepted, and ignored: contract v3
dropped the rivals and the benchmark they fed, and paulgsc/testing's
`playoff-delta.yml` still passes them. Delete them once that workflow stops.

# obs-websocket

Drive OBS by intent. The crate exposes everything you do to OBS by hand during
a stream as a command, so something other than a mouse (speech mapped to
commands by a rules engine or a model) can drive it. A UI only reflects state
and holds settings.

```text
speech ─▶ text ─▶ mapper (rules or model) ─▶ ObsCommand ─NATS─▶ some-obs ─▶ obws ─▶ OBS
                        ▲                                                            │
                        └──── StudioSnapshot: names + state ◀────────────────────────┤
                                  display ◀── StudioChanged, events, clocks ◀─────────┘
```

Built on [`obws`](https://crates.io/crates/obws), which needs OBS Studio ≥ 30.2
with obs-websocket ≥ 5.5, and checks this when it connects.

## Contract

- **One variant per intent.** `ObsCommand` is the whole vocabulary: stream,
  recording (pause/resume/split/chapter), replay buffer, virtual camera, scenes,
  studio mode and transitions, source visibility, audio (absolute, toggle and
  relative volume), media playback, filters, text, and any OBS hotkey.
- **Addressed by the names OBS shows.** Commands take `"Mic/Aux"`, not IDs.
  Mapping a loose spoken name onto an exact one is the mapper's job, using the
  names in `GetStudio`. An unknown name fails with OBS's own "not found"
  reason, which the mapper can pass back to the speaker.
- **Every command gets an answer.** A command sent with a NATS `reply_to` gets
  a `command_ack` (with any response data, e.g. the new state after a toggle)
  or a `command_error` with the reason. A command that fails to parse also gets
  a `command_error`, so a model generating commands learns what it got wrong.
- **State comes back whole.** `ObsEvent::StudioChanged` carries the full
  `StudioSnapshot` (same shape `GetStudio` returns), published on connect and
  once per burst of changes (at most every 250 ms while changes continue). A
  display renders it as-is; it never has to fold individual events. Polling
  adds only what events can't carry: running stream/recording clocks (1 s)
  and performance stats (5 s).
- **Effects also come back as events.** OBS pushes a change event for every command's
  effect (`StreamStateChanged`, `SceneItemEnableStateChanged`, …); events
  without a dedicated `ObsEvent` variant are forwarded as `UnknownEvent` with
  OBS's `eventType`/`eventData`.
- **Success means accepted, not finished.** `StartStream` succeeding means OBS
  started the output. Going live is a `StreamStateChanged` whose `output_state`
  is `OBS_WEBSOCKET_OUTPUT_STARTED`; a bad stream key shows up as `..._STOPPED`.

## Wire format

```json
{ "type": "setMute", "data": { "input": "Mic/Aux", "muted": true } }
{ "type": "setSourceVisible", "data": { "source": "Terminal", "visible": true } }
{ "type": "media", "data": { "input": "Intro music", "action": "play" } }
{ "type": "pauseRecording" }
```

`scene` on the source-visibility commands defaults to the live scene. A source
inside an OBS group is addressed with the group's name as `scene`; the snapshot
lists groups separately under `groups`.

## Configuration

`ObsConfig::from_env()` reads `OBS_HOST`, `OBS_PORT` and `OBS_PASSWORD`.

## Examples

- `stream`: connect, log every event, reconnect on drop.
- `youtube`: point OBS at a `YOUTUBE_STREAM_KEY`, start streaming, and report
  whether the output actually came up.

```sh
OBS_HOST=127.0.0.1 OBS_PASSWORD=... cargo run -p obs-websocket --example stream --features websocket
```

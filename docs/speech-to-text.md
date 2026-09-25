# Speech to text

> Holding a key turns speech into text on the bus. The service does not know
> who reads it, and nothing on the bus can make it listen.

This is the design for `apps/some-speech` (working name), a standalone service
that runs on the Windows machine next to the microphone and the GPU. It turns a
held push-to-talk key into one transcript per utterance and publishes it on
NATS. It is one of a swarm of disjoint services: its own binary, its own
subjects, one job. It is not a module of `file_host`, and it names no consumer.
OBS voice control, live captions and anything else are consumers that subscribe
to it; none of them is known to it.

Status: **design agreed, not implemented.** Sections marked *open* are still
undecided.

---

## Where it runs, and why the hop matters

```
Windows (daily driver)                                NixOS server
┌─────────────────────────────────────────┐           ┌──────────────────┐
│ key held ─► mic capture ─► whisper.cpp  │  outbound │ NATS :4222       │
│ (polled)    (in memory)    (CUDA)       │ ────────► │ speech.>         │──► consumers
│                               │         │  only     │                  │
│                               └─► text ─┘           └──────────────────┘
└─────────────────────────────────────────┘
```

The GPU is on Windows, so speech to text runs there and only **text** crosses
the network. Raw audio never goes on the bus.

The Windows machine is the user's daily driver, with their personal data and
sessions on it. The hard part of the hop is not reachability. It is keeping what
the network can reach on Windows, and what the service can do there, as small as
possible. Every invariant under "Scope on Windows" below comes from that.

## Decisions

| Question | Decision | Why |
|---|---|---|
| Where inference runs | The GPU on Windows | The server's Ollama is CPU-only. A GPU makes larger Whisper models viable (see `docs/models/whisper-optimization.md`, which assumes a CPU). |
| Runtime | One native Windows Rust binary: `whisper-rs` (whisper.cpp with CUDA), `cpal` capture, `some-transport` to NATS | One process and one language, following the `some-obs` precedent. WSL2 would add a NAT hop and awkward mic capture. A Python sidecar would add a second runtime to install and supervise. |
| Trigger | Push-to-talk: talk while holding one configured key, read by **polling that key's state** (about every 20 ms) | Utterance boundaries are exact. Nothing is transcribed unless the key is held. The process never sees any other key. |
| Language | English only, pinned | No per-utterance language detection: faster, and fewer misfires. |
| Bus down | **Drop.** Nothing is queued or replayed | A stale utterance delivered late is worse than one lost. |
| Bus trust | A dedicated NATS user may publish `speech.>` and subscribe to nothing. Everyone else is denied `speech.>` both ways | Nothing else on the LAN can fake a transcript or read one. Existing services are untouched. |
| Health | A `speech.status` heartbeat on the bus | Prometheus scraping would need an inbound port on Windows. |
| Partial transcripts | None | Push-to-talk gives one final transcript per utterance. If a live-captions consumer ever needs partials, add `speech.partial`. |

---

## The contract

Both messages are new `UnifiedEvent` variants in `crates/ws-events`, following
the existing envelope. Subject names describe what is published, never who
consumes it.

### `speech.transcript`: one per utterance

| Field | Type | Meaning |
|---|---|---|
| `utterance_id` | string | Random per utterance (UUID v4). It is not a counter, so it reveals nothing about how often someone speaks across restarts. |
| `text` | string | The transcript, trimmed. Empty or non-speech results (whisper's `[BLANK_AUDIO]` and the like) are **not published**. |
| `language` | string | `"en"`, pinned for now. It stays in the contract so pinning can change without a new message. |
| `captured_at_ms` | u64 | Windows wall clock (Unix ms) at key-down. For ordering and display only: see "Clocks". |
| `audio_ms` | u32 | Length of the captured audio. |
| `latency_ms` | u32 | Key-up to publish, measured on Windows's monotonic clock. |
| `confidence` | optional f32 | Mean token probability, 0–1. A heuristic, not a calibrated probability. |
| `source` | string | A **configured label** such as `"desk"`, never the OS device name. Device names carry hardware models. |
| `model` | string | Model file name, e.g. `ggml-large-v3-turbo.bin`. |

### `speech.status`: heartbeat and state

Published every 10 s and on every state change. It carries **no text, ever**.

| Field | Type | Meaning |
|---|---|---|
| `state` | enum | `idle`, `capturing`, `transcribing`, or `engine_error` |
| `run_id` | string | Random per process start, so a consumer can see a restart |
| `model`, `source` | string | As above |
| `backend` | enum | `cuda` or `cpu`, so a silent fall back to CPU is visible |
| `utterances`, `dropped` | u64 | Counts since start. `dropped` covers the bus-down case and backlog overflow |
| `last_latency_ms` | optional u32 | The last transcription's latency |

The heartbeat is how a late subscriber catches up, and it is the only state that
catches up. This is the #376 `StudioChanged` lesson (keep the latest *state* for
subscribers that arrive late), applied to state only. **Transcripts are events,
not state.** Replaying them to a late subscriber would mean storing speech, so a
subscriber that arrives late gets no transcripts from before it arrived.

### Clocks and staleness

`captured_at_ms` comes from the Windows wall clock. By default Windows keeps its
clock only loosely in sync, so comparing it with the server's clock can be off
by seconds. A consumer that **acts** on a transcript (a command mapper) must
judge staleness by the transcript's arrival time, not by `captured_at_ms`, and
owns its own staleness window. The service keeps arrival close to capture by
never publishing a transcript it held across a disconnect (invariant 9).

### What `file_host` does with it

Nothing. `file_host` relays an explicit allowlist of subjects to browsers
(`websocket/broadcast/handlers.rs`), and `speech.*` is not on it. Adding it
would be a consumer decision, so it would have to go through invariant 7.

### What it replaces

The service does not use `audio.chunk` (`AudioChunkMessage`) or `audio.subtitle`
(`SubtitleMessage`), and nothing in the repo does either. The first
implementation slice **deletes both**, along with their `EventType`, `Event` and
`UnifiedEvent` variants. They are not reused because:

- `audio.chunk` would put raw audio on the bus, about 384 KB/s at 48 kHz stereo;
- `audio.subtitle` is named for one consumer.

---

## Invariants

Each invariant names where it is enforced. "Planned" means it lands with the
implementation, and the invariant is unmet until then.

### Scope on Windows

1. **Outbound only.** The service opens no listening socket: no HTTP server, no
   metrics endpoint, no IPC endpoint. Its only connection is one outbound
   connection to NATS.
   *Enforced by (planned)* a CI check that the binary's dependency tree contains
   no server crate (`axum`, `hyper`'s server feature, `metrics-exporter-*`).

2. **Only a physical key press on Windows starts capture.** No message, command
   or subject can arm, start or configure the microphone. The service subscribes
   to nothing.
   *Enforced by* the NATS server, not by the service's own code: the speech user
   has `subscribe: { deny: [">"] }` (see "Bus permissions"), so a later code
   change cannot quietly add a control subject. Checked in `some-transport` at
   `5dd81e1`:
   - `NatsTransport::new(client)` subscribes to nothing, but
     `connect_with_receiver` subscribes to the broadcast subject, so the service
     uses the former.
   - `connect(url)` accepts no credentials, so they would have to ride in the URL,
     and `some-obs` logs its `NATS_URL` at startup. The service builds its
     `async_nats::Client` with `ConnectOptions::with_user_and_password` and never
     logs the URL.

3. **No low-level keyboard hook, ever.** The only input the service reads is the
   configured push-to-talk key, by polling that one key's state
   (`GetAsyncKeyState`). A `WH_KEYBOARD_LL` or `WH_MOUSE_LL` hook would see
   every keystroke on the machine.
   *Enforced by (planned)* a script check that `SetWindowsHookEx` appears nowhere
   in the crate, in the style of `scripts/check_privacy.py`.

4. **It runs as the logged-in user, never elevated, never as a Windows
   service.** It starts at logon (Startup folder or a per-user Task Scheduler
   task). A service would run in session 0, where it could not read the key
   anyway.

5. **Nothing is fetched at run time.** The model is a local file, downloaded
   once by hand and verified against a pinned SHA-256 (`SPEECH_MODEL_SHA256`) at
   startup. It is not loaded if the hash doesn't match. There is no auto-update.

### Privacy

6. **Audio never leaves the process and never touches disk.** The capture buffer
   lives in memory, and each utterance's buffer is dropped once it has been
   transcribed.

7. **Transcripts are published, never stored and never logged.** Text goes only
   onto core NATS `speech.transcript`:
   - No JetStream stream may include `speech.>` in its subjects. Today the only
     stream is the pipeline's `JOBS`/`DLQ` (`some-transport/src/nats/jetstream.rs`).
   - No log line or span field carries transcript text. Logs carry lengths,
     durations and counts.
   - A consumer that wants to keep transcripts must first change this invariant,
     here and in `docs/identity.md` (invariant 8), and classify its table as that
     document's invariant 1 asks.

   *Enforced by (planned)* a capturing-layer test in the service, like
   `file_host::privacy`'s, that runs an utterance through with a fake engine and
   fails if the text appears in any event or span. Also by
   `scripts/check_instrument_skip.py`, which already covers `#[instrument]`. The
   stream-subject rule belongs to review.

8. **A transcript carries no identity.** It has no `SubjectId`, no device name
   and no hardware or host identifier. `source` is a label the user chose.

### Behaviour

9. **Drop, don't buffer.** The service publishes only while the NATS client
   reports `Connected`. It discards an utterance finished while disconnected and
   counts it in `dropped`. `async-nats` itself queues publishes during a
   reconnect, so checking the state first narrows the window but cannot close
   it. The consumer staleness rule under "Clocks and staleness" covers the rest.

10. **One utterance at a time, with a bounded backlog.** Utterances are
    transcribed in order, at most two wait, and on overflow the oldest is dropped
    and counted. A single hold is capped at 30 s (Whisper's window); audio past
    the cap is discarded, not queued.

### Bus permissions

11. **Only the speech service can publish `speech.>`, and only named consumers
    can read it.**
    *Enforced by (planned)* an `authorization` block in `infra/nats/nats.conf`:

    ```
    authorization {
      users: [
        { user: speech, password: $NATS_SPEECH_PASSWORD,
          permissions: { publish: { allow: ["speech.>"] }, subscribe: { deny: [">"] } } }
        { user: anonymous,
          permissions: { publish: { deny: ["speech.>"] }, subscribe: { deny: ["speech.>"] } } }
      ]
    }
    no_auth_user: anonymous
    ```

    Clients that don't authenticate (all of today's services) are mapped to
    `anonymous` and keep every permission they have now, except on `speech.>`.
    Each future consumer gets its own user with a `subscribe` allow for the
    subjects it reads. Per-service credentials for everything else are a separate
    story.

---

## GPU, model and latency

- **Contention.** OBS encodes on NVENC, a fixed-function block separate from the
  CUDA cores, so it barely competes with inference. Games are the real
  contention. Under load, latency rises. The service does not shed load except
  through invariant 10's backlog bound.
- **Model.** *Open, because it depends on the GPU's VRAM.* The English-only
  candidates are `distil-large-v3` and `medium.en`; `large-v3-turbo` is the
  multilingual option, pinned to English. Each is around 1.5 GB of ggml weights
  at f16, plus working memory. Quantized variants are smaller.
- **Resident model.** *Open.* Keeping it loaded holds VRAM all session, while
  loading on key-down adds seconds to every utterance. The default is resident.
- **Target.** Key-up to publish under 1 s for an utterance of 10 s or less, on
  the actual GPU. This is a target to measure, not a promise.

## Testability

Linux CI cannot build a CUDA whisper.cpp for Windows, and `cpal` on Linux needs
ALSA headers the runners don't install. So:

- The pipeline (key edges, then utterance, then transcript, then publish, plus
  backlog, drop and heartbeat) is platform-independent. It works against three
  traits: `PushToTalk`, `Capture` and `Transcriber`. Linux CI runs it against
  fakes.
- The real key poller, `cpal` capture and `whisper-rs` engine live in
  `#[cfg(windows)]` modules, behind a `cuda` feature.
- *Open:* Linux CI never compiles those modules. Options are a Windows CI job,
  or `cargo check`/`clippy --target x86_64-pc-windows-msvc`. The target check
  would still need a C toolchain for `whisper-rs-sys`'s build script.

## Configuration

The service is configured by environment variables, like `some-obs`:

| Variable | Meaning |
|---|---|
| `NATS_URL` | e.g. `nats://<server>:4222` |
| `NATS_USER`, `NATS_PASSWORD` | The `speech` user. Kept in the user's environment, or in a file only that user can read. |
| `SPEECH_KEY` | The push-to-talk virtual-key code, e.g. `VK_XBUTTON2` or `VK_F13` |
| `SPEECH_SOURCE` | The `source` label |
| `SPEECH_DEVICE` | Optional input-device name. The default is the system default input. It is read locally and never published. |
| `SPEECH_MODEL_PATH`, `SPEECH_MODEL_SHA256` | The model file and its pinned hash |

## What is still exposed

- **NATS traffic is plaintext on the LAN.** Anyone who can sniff the LAN (Wi-Fi
  especially) can read transcripts, and the `speech` user's password, as they
  cross. NATS supports TLS; it is not configured today for any service.
- **Anyone at the keyboard can hold the key.** Push-to-talk proves someone
  pressed a key, not who pressed it.
- **Consumers are trusted with what they read.** Once a named consumer
  subscribes, what it does with the text is governed by that consumer's own
  doc and by invariant 7.

## Out of scope

The speech-to-OBS command mapper, the orchestrator (`apps/orchestrator`) and the
existing `utterance` subject are out of scope. `utterance` is **typed** browser
text captured by an extension, not speech. Where Ollama or an LLM runs is the
mapper's question, not this service's.

## Implementation slices (proposed)

1. **Contract.** Add the `speech.transcript` and `speech.status` types to
   `ws-events`, delete `audio.chunk` and `audio.subtitle`, and add the
   `nats.conf` authorization (invariant 11).
2. **Pipeline.** Create the `apps/some-speech` crate with the platform-independent
   pipeline, fakes and the privacy capturing-layer test (invariant 7), all
   running on Linux CI.
3. **Windows edge.** Add the key poller, `cpal` capture, the `whisper-rs` CUDA
   engine and the model hash check, and settle the Windows CI question.
4. **Operations.** Add a logon task recipe and a model download recipe with
   pinned hashes. A server-side bridge from `speech.status` to Prometheus, if
   wanted, is its own disjoint service and not part of `file_host`.

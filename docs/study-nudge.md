# The study nudge

> The session was prepared. I didn't sit down. The browser was closed. The
> desktop notified me. I clicked it and landed in the session.

That sentence is the whole feature. Everything below exists to make it true, and
— just as importantly — to make the silences correct: after studying, during
quiet hours, and while the dashboard is open.

This document covers setup, the decisions that are easy to get wrong, and, at
the end, [what this still cannot know](#known-unknowns). That last section
matters as much as the first. The premise of the feature is that the learning
process must never be load-bearing, so what is worth writing down is not that it
worked once, but what it still cannot tell you.

---

## Warrant, admissibility, actuation

The mistake this design exists to avoid is fusing three different questions into
one predicate.

| | Question | Where |
|---|---|---|
| **Warrant** | Why intervene at all? | `crates/intervention` — a charge falling to a threshold |
| **Admissibility** | May we, *now*, on this channel? | `file_host::nudge::constraints` |
| **Actuation** | How does it physically go out? | `crates/push_kit` |

A design with no warrant layer has to borrow one, and the usual loan is a cron:
something must make the call happen, so the clock does. The tell is a policy
whose every guard answers *may we* — quiet hours, cooldown, presence — and none
answers *why*. Time then becomes the cause of interventions rather than a
constraint on them, and the system can express "it is 19:00" but not "they
abandoned a session and twenty minutes have passed".

So the crates are split along that seam and not along a framework boundary:

| Crate | Knows about | Does **not** know about |
|---|---|---|
| `push_kit` | VAPID, RFC 8291/8292, a transport trait | study sessions, axum, sqlx, *when* |
| `intervention` | charge, decay, thresholds, verdicts | lessons, HTTP, databases, tokio |
| `study_domain` | lessons, sessions, scores, curriculum | how any of it is stored or sent |
| `engagement_repo` | rows | what a class means |
| `file_host` | axum | all of the above, except as dependencies |

There is no `dyn` in `push_kit` or `intervention`. Not as dogma — a binary knows
its transport and its domain at compile time, so a vtable to reach one
implementation buys nothing and costs inlining. Where the inverse would buy
something, it would be defensible; here it does not.

## The battery, and why it is not a rate limiter

Engagement is a **vector** of levels, one per class, that decays with time and is
restored by signals. Decay *is* inactivity — there is no separate "days since
last seen" counter, because the absence of signals is already the drain.
Discrete setbacks drain further; wins recharge.

An intervention is warranted when the weighted aggregate falls to the threshold.

The analogy stops at the leaky bucket, and this is the part worth understanding:

- **Nothing is polled.** Decay is closed-form, so a level is computed on read and
  never ticked. A subject nobody has seen in a year costs one exponential when
  someone finally asks.
- **The crossing instant is solved, not waited for.** Between signals the
  aggregate is strictly decreasing, so the moment it will cross the threshold can
  be found by bisection — once, when the signal arrives — and written to an
  indexed column.
- **The waker therefore discovers rather than decides.** Its entire query is
  `WHERE eligible_at <= now`. On a day when nobody has drifted it returns
  nothing.

Work is O(1) per signal and zero per idle subject. `NUDGE_WAKER_SECONDS` is not a
schedule; it is the resolution at which already-decided work is picked up.

### Why a vector and not one number

A scalar can say *whether* to intervene and can never say *what to say*.
"Abandoned twenty minutes ago" and "gone for a fortnight" reach the same
threshold and want completely different messages. The dominant deficit picks the
action:

| Most depleted | Action |
|---|---|
| Presence | `LessonReady` |
| Momentum | `ResumeAbandoned` |
| Mastery | `SuggestReview` |
| Freshness | `NewMaterial` |

## Consent is a precondition, not a preference

**The null case is silence.** There is no path through `push_repo` that creates a
subscription nobody agreed to: `upsert` takes a `Consent`, and an empty topic
list is honoured as "receives nothing" rather than read as "receives
everything". A topic list that cannot be parsed degrades to nothing, too —
consent that cannot be read is not consent.

`GET /api/v1/push/vapid-key` returns the topics on offer alongside the key, so
the permission modal renders the options the sender will actually honour rather
than a list maintained separately in the frontend.

Auth has not landed. `file_host::subject::SubjectId` is the seam: an extractor
that returns a singleton today and reads a validated token later, with every
table downstream already keyed by subject.

---

## Setting it up

### 1. Apply the migrations

**There is no in-app migration runner.** `build.rs` only declares
`rerun-if-changed=migrations`; nothing applies anything at startup. This is a
real step, not a detail, and it is the same step `capture_repo` and
`mood_event` already need.

Each crate owns its own `migrations/` directory with paired
`<timestamp>_name.up.sql` / `.down.sql` files, and they all target the one
SQLite file named by `DATABASE_URL`. Apply them with `sqlx-cli`:

```sh
cargo install sqlx-cli --no-default-features --features sqlite

# One database, several crates' migrations. Collect them by timestamp; the
# prefixes are globally ordered, which is why they are timestamps.
mkdir -p /tmp/migrations
find crates -path '*/migrations/*.up.sql' | sort | while read -r f; do
  cp "$f" "/tmp/migrations/$(basename "$f" .up.sql).sql"
done

DATABASE_URL="sqlite:///path/to/hopium.db" sqlx migrate run --source /tmp/migrations
```

This is exactly what `.github/workflows/lint.yml` does, and it is worth keeping
the two in step: if you change how migrations are applied, that workflow is the
other place that has to know.

#### Building without a database

`sqlx::query!` verifies SQL at compile time, so `cargo check` needs either a
`DATABASE_URL` pointing at a migrated database, or the offline cache:

```sh
DATABASE_URL="sqlite:///path/to/hopium.db" cargo sqlx prepare --workspace
```

`.sqlx/` is in `.gitignore` on purpose: the cache is a build artifact of a
schema the migrations already define, and committing it means a second copy of
the schema to keep in step. CI therefore builds it, the same way you just did —
`test.yml`'s `rust_ci` job creates a database, runs every crate's migrations
into it, and runs `cargo sqlx prepare` before `cargo check`. That job had been
running a bare `cargo check` with no database and failing before it compiled
anything; the setup steps are what make it a real check.

If you change how migrations are applied, `test.yml` and `lint.yml` are the two
places that have to know.

The tables added by this feature:

- `push_subscriptions` — one row per browser, with its owner and its consent.
- `engagement_charge` — `(subject, class, level, as_of)`. Undecayed; decay is a
  function of the stamp.
- `engagement_gate` — the solved `eligible_at`, and the index the waker reads.
- `intervention_log` — what actually went out, and whether it reached a push
  service.
- `sessions` — the server's copy of `SessionRecord`.

### 2. Generate a VAPID keypair

```sh
npx web-push generate-vapid-keys
```

Or, with only OpenSSL:

```sh
openssl ecparam -name prime256v1 -genkey -noout -out vapid_private.pem
openssl ec -in vapid_private.pem -outform DER | tail -c +8 | head -c 32 \
  | base64 | tr -d '=' | tr '/+' '_-'          # VAPID_PRIVATE_KEY
openssl ec -in vapid_private.pem -pubout -outform DER | tail -c 65 \
  | base64 | tr -d '=' | tr '/+' '_-'          # VAPID_PUBLIC_KEY
```

Put the two base64url strings in the environment. `.gitignore` already covers
`*.pem` and `.env`; keep the PEM out of the repository.

#### Rotation is a migration, not a config change

**The public key is baked into every subscription made with it.** Rotating it
does not re-key those subscriptions — it silently invalidates all of them. The
failure looks like "notifications just stopped", with a `403` buried in a log
that nobody is reading, because a feature that is supposed to be quiet most of
the time is exactly the kind that can be broken for a week unnoticed.

Recovery is every browser visiting the site and subscribing again. There is no
server-side fix. So:

- `file_host` derives the public key from the private one at startup and
  **refuses to boot** if the configured pair does not match. That mismatch is
  the single longest-fuse failure in this feature, and it is worth a boot
  failure to catch.
- `GET /api/v1/push/vapid-key` exists so the frontend never hardcodes the key.
  A key change should not require a frontend redeploy, and an
  `applicationServerKey` mismatch is invisible until a send fails.

There is deliberately no rotation *mechanism*. Understanding the cost is in
scope; automating it is not.

### 3. Configure the rest

See `example.env`. The two that most reward attention:

- **`NUDGE_TIMEZONE`.** Containers are UTC unless told otherwise. A UTC day
  boundary files an 18:00 local session under *tomorrow* in a UTC-07:00 zone —
  so "have I studied today" answers no, and you get nudged for a day you already
  did. The same offset makes quiet hours of 22:00–08:00 silence 15:00–01:00
  local and permit 03:00. Leaving it unset is allowed, and is logged loudly at
  startup rather than assumed.
- **`NUDGE_ENABLED`.** With this off, the `/push` routes still work and
  `POST /api/v1/push/test` still sends; only the schedule is idle. With it on
  and no usable VAPID pair, startup fails.

### 4. Serve the app over HTTPS

Service workers, `Notification`, and `PushManager` are all gated on a **secure
context**. `http://localhost` and `http://127.0.0.1` qualify by explicit
exception; `http://nixos.local` and `http://192.168.x.x` do **not**.

The symptom is confusing: the APIs are simply absent from `window`, so what you
see is "the toggle isn't there", not an error. On a LAN-only setup this is the
blocker that stops the whole feature.

The client already handles its side — `apps/www/vite.config.ts` in
`paulgsc/some-ui` picks up `certs/nixos.local+3.pem` when present, and the
settings section renders an explanation instead of the toggle when
`nudgesSupported()` is false.

#### The certificate story

- **Which cert.** A locally-issued cert for `nixos.local`, generated with
  [`mkcert`](https://github.com/FiloSottile/mkcert):
  `mkcert nixos.local localhost 127.0.0.1 ::1`, which writes
  `nixos.local+3.pem` and `nixos.local+3-key.pem`.
- **Who trusts it.** `mkcert -install` puts the local CA into the system trust
  store, and separately into Firefox's (which keeps its own). Every browser that
  is going to subscribe needs the CA trusted, or the page is not a secure
  context and there is nothing to subscribe with.
- **When it expires.** `mkcert` leaf certificates are good for a little over two
  years; the local CA for ten. Expiry presents as the app failing to load over
  HTTPS, and then — because there is no secure context — as the nudge toggle
  disappearing. The remedy is to re-run the `mkcert` command above and restart
  the dev server. If the *CA* expired, `mkcert -uninstall && mkcert -install`
  first, and every browser has to trust the new one.

Write the date somewhere you will see it. A certificate expiry that nobody
expected is indistinguishable, from the outside, from this feature being broken.

---

## The HTTP surface

All paths are under `/api/v1`.

### Push

| Method | Path | Purpose |
|---|---|---|
| `GET` | `/push/vapid-key` | The `applicationServerKey` and the topics on offer |
| `POST` | `/push/subscriptions` | The browser's `PushSubscription` **plus the topics they agreed to** |
| `DELETE` | `/push/subscriptions` | Withdrawing consent, idempotent, body `{ endpoint }` |
| `POST` | `/push/test` | Send now, without waiting for a day to pass |

### Signals

| Method | Path | Purpose |
|---|---|---|
| `POST` | `/signals` | A domain event: `session-started`, `session-abandoned`, `scored-below-target`, `curriculum-updated`, … |

**This is where work originates.** A signal folds into the subject's charge and
the crossing instant is re-solved on the spot; the response returns the new
`eligible_at`, which makes the arithmetic observable without waiting for a
notification. A cron-driven design has no endpoint like this, because nothing
needs to tell it anything — and that convenience is exactly what costs it the
ability to answer *why*.

### Sessions

One route per `SessionsRepository` method in
`apps/www/src/lib/tenant/sessions-repository.ts`, matched one-to-one so that the
client change is a swap rather than a rewrite:

| Client | Server |
|---|---|
| `list()` | `GET /sessions` |
| `get(id)` | `GET /sessions/:id` |
| `create(input)` | `POST /sessions` |
| `update(id, patch)` | `PATCH /sessions/:id` |
| `remove(id)` | `DELETE /sessions/:id` |
| `removeMany(ids)` | `DELETE /sessions` — `{ ids }` |
| `updateStatusMany(ids, s)` | `PATCH /sessions/status` — `{ ids, status }` |
| `duplicate(id)` | `POST /sessions/:id/duplicate` |

Two behaviours moved server-side with the data, because leaving them on the
client would let the two diverge silently:

- **Id generation.** `session-<uuid>`, as `generateId("session")` produced.
- **`totalDurationOf(scenes)`** — `max(start_time + duration)`. If the server
  stores `total_duration_ms` but lets the client compute it, a client that
  forgets stores a zero, and the nudge cheerfully offers you a "~1 min" session.

### Trust model, stated plainly

These routes carry **no authentication** beyond the CORS origin allowlist.
Anyone who can reach the LAN can register a push subscription — and would then
receive this person's study reminders — or read and write sessions. That is the
same trust model as the rest of `file_host`. It is written down here so it stays
a conscious acceptance rather than an oversight.

---

## The decisions worth knowing

### `201` does not mean delivered

A push service returns `201 Created` once it has **accepted** a message. Not
delivered, not displayed, and — the trap — not decryptable: a payload encrypted
with the wrong keys returns `201` and is then silently discarded by the browser,
because the push service never had the key material to notice.

Nothing in `nudge::sender` logs a `201` as though it meant delivery, and the
only real proof is a human seeing a notification. Plan the first test that way.

The failures are where the information is:

| Status | Meaning | What happens |
|---|---|---|
| `201` | Accepted for delivery | Recorded; proves nothing |
| `400` | Malformed request or bad VAPID JWT | Logged loudly; it is a bug here |
| `403` | VAPID key mismatch | The key was rotated — see above |
| `404` / `410` | Subscription is dead | The row is deleted |
| `413` | Payload too large | Refused before sending too |
| `429` | Rate limited | `Retry-After` surfaced; no retry queue |

A `403` deliberately does **not** prune. It means *this server* signed with the
wrong key, and pruning on it would delete every subscription over one bad
environment variable.

### Presence is an input, not a veto

The client checks `document.visibilityState` and knows whether its tab is
*visible*. The server can only see whether a WebSocket is *connected* — and a
pinned background tab holds its socket open all day.

Treating connection as presence would suppress every nudge for exactly the
person this feature was built for: someone who keeps the dashboard open on a
second monitor and forgets about it. The failure mode is total silence that
looks like a broken feature.

So the signal is bounded twice:

1. A connection counts only if the actor reports it active, not stale, and last
   active within `NUDGE_PRESENCE_FRESHNESS_SECONDS`. A zombie half-open socket
   cannot silence the nudge indefinitely.
2. Every tick logs what presence concluded, so a missing nudge is diagnosable.

The stronger signal — the client reporting genuine visibility, which it already
knows — is the escape hatch if that is not enough. It is a `some-ui` change and
is deliberately not built speculatively.

### One intervention at a time, across restarts

The claim is taken **before** the send, in a single conditional `UPDATE ... WHERE
eligible_at <= now` that also moves `eligible_at` forward. Two waker passes — or
one pass and a just-restarted process — cannot both conclude a subject is due. A
crash between claiming and sending therefore costs that intervention rather than
duplicating it, which is the right way round for a feature whose whole value is
not being annoying. If every device fails, the claim is released.

Two further brakes: an intervention **recharges** the classes it addresses, so
the next pass finds nothing to do; and `REFRACTORY` is a hard floor whatever the
arithmetic says.

### One policy, one language

An earlier draft kept a JSON fixture so a TypeScript copy of the policy and a
Rust copy could be checked against each other. That is gone. The typestate is
Rust: signal classes, weights, half-lives, and the decision are all types the
compiler checks, and anything user-specific is an instantiation persisted per
subject. There is no second implementation to keep honest, and a fixture file
would only have been a weaker restatement of the enums.

The client keeps `decideNudge` for the GitHub Pages build, which has no backend
at all — but that is a *fallback*, not a mirror, and `paulgsc/some-ui#924` is
where the two are prevented from both firing.

---

## Known unknowns

What this feature cannot tell you, written down so the next piece of work starts
from a stated boundary rather than rediscovering it.

**Connected is not focused.** The server sees WebSocket connections, not tab
visibility. A dashboard open on a second monitor and ignored for six hours looks
exactly like one being read. The freshness bound narrows this but does not close
it; only a client-side visibility report would.

**There is no delivery confirmation.** The Web Push protocol does not offer
one. `last_success_at` records that a push service accepted a message, and
nothing downstream of that is observable: not whether the browser decrypted it,
not whether the OS displayed it, not whether anyone saw it. A subscription can
therefore look healthy for weeks while delivering nothing — the `403` case is
the one where that is silent by construction.

**Multi-device is untested.** The subscription table admits several rows and the
sender fans out to all of them, but this has only ever run against one browser.
Two devices would raise questions this has no answer for: whether dismissing on
the laptop should silence the phone, and whether "already nudged today" should
be per-device or global. It is currently global.

**DST is accepted, not solved.** Local midnight does not exist on spring-forward
day and exists twice on fall-back day. `local_day_bounds` takes the earliest
valid instant in both cases, so the range is always well-formed, but a nudge on
those two days a year may land up to an hour off. The consequence is bounded and
the alternative is a lot of machinery.

**The calibration is a guess.** Every weight, half-life, ceiling, and the
threshold itself were chosen by argument rather than by evidence. They are
plausible and they are not tuned; the first week of real `intervention_log` rows
is what should revise them.

**A class removed in a future release strands rows.** `from_discriminant` is a
total parse and unknown discriminants are quarantined, so nothing panics or is
silently reinterpreted — but the stored level is dropped on the floor. A release
that retires a class needs a migration, and there is no mechanism that would
notice if it forgot.

**One subject.** `SubjectId` is a singleton until auth lands. Everything is keyed
by it, so nothing needs restructuring, but nothing has been exercised with two.

**The tag constant lives in three places.** `NUDGE_TAG` (`some-ui.study-nudge`)
is hand-maintained in `file_host::nudge::payload`, in `public/sw.js`, and in
`src/lib/study-nudge/service-worker.ts` — the last two cannot import from each
other because one is a plain `public/` asset. A mismatch shows up as
notifications stacking instead of replacing. Fixing it properly is a `some-ui`
build change.

**A worker update lands one visit late.** A service worker update takes effect
on the visit *after* the one that fetched it. So any change to the payload
contract has to ship in `sw.js` first, and the server may only start relying on
it a deployment later.

---

## Verifying it end to end

Unit tests do not catch the interesting failures here. Several of them look like
success. This is the list worth actually walking, once, on the real machine:

- [ ] Subscribe from the real study browser over HTTPS; confirm the
      `push_subscriptions` row has both keys.
- [ ] `POST /api/v1/push/test` with the browser **closed**; confirm an OS
      notification arrives.
- [ ] Click it; confirm the deep link lands in the session, not the dashboard.
- [ ] `POST /api/v1/signals` with a `session-abandoned` and confirm the returned
      `eligible_at` moves closer; then with a `session-completed` and confirm it
      moves further out. This is the whole engine, observable in two requests.
- [ ] Let engagement actually decay with no signals; confirm the intervention
      arrives near its solved instant and that it is a `LessonReady` rather than
      a coaching message.
- [ ] Verify each silence separately — after completing a session, inside quiet
      hours, after a dismissal, and while the dashboard is connected. Each
      should appear in the logs with its own reason.
- [ ] Revoke the subscription in browser settings; confirm the next send prunes
      the row on `410`.
- [ ] Restart `file_host` immediately after an intervention; confirm no second
      one, and that `engagement_gate.eligible_at` sits in the future.
- [ ] `Ctrl-C`; confirm shutdown does not hang on the tick.
- [ ] Run seven consecutive days and count the notifications against
      `intervention_log`: the refractory floor should be visible in the gaps.

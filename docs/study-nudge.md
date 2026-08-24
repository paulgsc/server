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

### Cold start: the one deficit with a sessionless answer

Three of the four actions above need a prepared session — you cannot resume a
session that was never started, review material that was never studied, or
announce new material to someone who has seen none. Before the recommender
exists (`#279`–`#284`), nothing is ever prepared, so those three stay silent
exactly as the table implies: `StudySelector::select` returns `None` and the
engine reaches `Verdict::NothingToSay`.

Plain absence is different: it has an honest sessionless answer. When
`prepared_session` is `None` and the dominant deficit is `Presence`, the
selector returns `StudyAction::GetStarted` instead of staying silent — an
invitation, deep-linking to the app base rather than a session that does not
exist. This is a deliberately **interim** answer, not the recommender: it
invites, it does not propose, and it does not provision anything (the
mistake PR #251 made). `#279` starts replacing it with real proposals, and
once `#285` lands, `GetStarted` either retires or becomes the documented
fallback for an empty catalogue.

### First contact: how a subject enters the gate at all

Until `#278`, nothing did. The waker's entire query is `WHERE eligible_at <=
now`, and `engagement_gate` rows were created in exactly one place —
`waker::observe`, called from `POST /signals` — whose only caller is the
client's session-mutation layer, fired when a session is *created*. The chain
was: no session ⇒ no signal ⇒ no gate row ⇒ never in `due` ⇒ never nudged.
Not late. Never. This is silence #1 in `#257`'s cold-start argument, and it
was silent for exactly the person who most needed the nudge: someone who had
just installed the app and done nothing else yet.

`POST /push/subscriptions` closes it. `waker::first_contact` runs on every
call, after the subscription itself is stored: it is a person explicitly
saying "you may interrupt me" — the strongest statement of intent this
deployment has — and, unlike a page load (too broad to count as consent) or a
dedicated "hello" route (a new endpoint restating what this one already
implies), it happens exactly once per device before any study behaviour
exists at all. Seeding lazily when the waker runs was never on the table: the
waker can only see rows that already exist, which is the circularity this
closes.

The row is written **full**, by the same `Charge::full`/`eligible_at`
arithmetic `observe` already falls back to for an unseen subject — so a
first-contact subject and a subject discovered through a stray signal land on
identical footing, and `eligible_at` comes out on the order of a week later
rather than five minutes. Idempotent by construction: the insert is guarded
by `engagement_gate`'s own primary key (`INSERT OR IGNORE`), not a
read-then-write, so a second device subscribing — or the same one
resubscribing after its keys rotate — cannot reset an already-drifting
subject back to full, and two devices racing to be the first one in cannot
both win.

The edge case worth naming: someone subscribes and then unsubscribes every
device before ever becoming due. They keep their gate row and have no way to
be reached. This degrades correctly rather than looping loudly — `Presence`
(fastest half-life, highest weight) stays the dominant deficit for a subject
who has never received a single signal, so `GetStarted` keeps firing when
they become due, and admission fails on `NotConsented` at `info!` rather than
falling through to `Verdict::NothingToSay`'s `warn!`.

Landing this closes **P0** (`#295`'s first deployable increment): a fresh
database, one subscription, no sessions, no signals, and the clock advanced
is now enough to produce one notification that opens the app.

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

The workspace owns one migration history in the repository-root `migrations/`
directory. Migrations are paired `<timestamp>_name.up.sql` / `.down.sql` files
and target the SQLite file named by `DATABASE_URL`. `sqlx-cli` expects the
forward migrations in a custom source to end in `.sql`, so stage the `.up.sql`
half of each pair before applying it:

```sh
cargo install sqlx-cli --no-default-features --features sqlite

# One database, one workspace migration history.
rm -rf /tmp/hopium-migrations && mkdir -p /tmp/hopium-migrations
find migrations -maxdepth 1 -name '*.up.sql' -type f | sort | while read -r f; do
  cp "$f" "/tmp/hopium-migrations/$(basename "$f" .up.sql).sql"
done

DATABASE_URL="sqlite:///path/to/hopium.db" \
  sqlx migrate run --source /tmp/hopium-migrations
```

This is also how `.github/actions/prepare-sqlx/action.yml` prepares image builds.
The test and lint workflows apply the same root migration history before
compiling SQLx queries. Keep all three in step if the migration layout or
staging convention changes.

#### Building without a database

`sqlx::query!` verifies SQL at compile time, so `cargo check` needs either a
`DATABASE_URL` pointing at a migrated database, or the offline cache:

```sh
DATABASE_URL="sqlite:///path/to/hopium.db" cargo sqlx prepare --workspace
```

`.sqlx/` is in `.gitignore` on purpose: the cache is a build artifact of a
schema the migrations already define, and committing it means a second copy of
the schema to keep in step. CI therefore builds it, the same way you just did —
`test.yml`'s `rust_ci` job creates a database, runs the workspace migrations
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
- `activities` — the server's copy of `ActivityDefinition` (#269). Deliberately
  the odd one out: every table above carries a `subject_id` as of #259, and
  this one does not, because an activity is catalogue-wide — a fact about what
  exists to play, not about who has played it. Per-subject state (played,
  dismissed) is #258's table, not a column here.

  Seeded with the four activities `paulgsc/some-ui@packages/activity-catalog`
  bundles (#270, `20260823000700_seed_activities.up.sql`), transcribed field
  for field. The parity test guarding that transcription lives in
  `crates/db/activity/tests/catalog_parity.rs`; its own module doc records
  which of #270's three proposed mechanisms this took and why the other two
  were deferred. What this table deliberately has no column for —
  `toSceneProps` — and what a server-composed session writes instead, is
  [its own decision below](#the-server-writes-data-the-client-writes-behaviour-272)
  (#272).

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
| `POST` | `/push/subscriptions` | The browser's `PushSubscription` **plus the topics they agreed to**, and first contact — see below |
| `DELETE` | `/push/subscriptions` | Withdrawing consent, idempotent, body `{ endpoint }` |
| `POST` | `/push/test` | Send now, without waiting for a day to pass |

### Signals

| Method | Path | Purpose |
|---|---|---|
| `POST` | `/signals` | A domain event: `session-started`, `session-abandoned`, `scored-below-target`, `curriculum-updated`, … |

**This is where a charge is updated.** A signal folds into the subject's
existing charge and the crossing instant is re-solved on the spot; the
response returns the new `eligible_at`, which makes the arithmetic observable
without waiting for a notification. A cron-driven design has no endpoint like
this, because nothing needs to tell it anything — and that convenience is
exactly what costs it the ability to answer *why*.

It is not, since `#278`, the only place a subject *begins* to exist:
[first contact](#first-contact-how-a-subject-enters-the-gate-at-all)
(`POST /push/subscriptions`) writes the initial gate row, seeded full, before
any signal has ever arrived.

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

**`sessions` has an owner now.** #259 added `subject_id`, backfilled to
`SINGLETON_SUBJECT` for every row that existed before the migration, and
rebuilt `idx_sessions_status` as `(subject_id, status, updated_at DESC)` for
the per-subject scan #260 (SUB2) then added: every non-admin method on
`SessionRepository` (`list`, `get`, `upsert`, `delete`, `delete_many`,
`set_status_many`, `touched_between`) now takes a `subject_id` and is scoped
by it — `get` returns `None` for a foreign id rather than the row (an id is
not a capability), `delete`/`delete_many` treat a foreign id as a no-op, and
`upsert` refuses outright, as `SessionRepoError::SubjectMismatch`, to move an
existing row to a different subject. The route layer does not extract a real
subject yet — `handlers/db/session.rs` passes `SINGLETON_SUBJECT` explicitly,
same as every other unauthenticated caller — that thread-through is #261
(SUB3), whose own acceptance criterion (`grep -rn "SINGLETON_SUBJECT"` outside
`subject.rs` finds nothing) is what retires those call sites.

### Activities

| Method | Path | Purpose |
|---|---|---|
| `GET` | `/activities` | The full catalogue, bounded (`activity_repo::CATALOG_CEILING`) |
| `GET` | `/activities/:id` | One activity, or `404` |

Read-only. #271's own out-of-scope note is explicit: the catalogue is seeded
and migrated, not written through this surface, so there is no `POST
/activities`.

No `SubjectId` scoping either, unlike Sessions above: an activity is
catalogue-wide, not owned by whoever plays it — `activity_repo`'s own crate
doc makes the same point about per-subject state (played, dismissed) living
in a different table, not a column here.

**`ETag`, and the one thing it has to agree with #273 about.** Both routes
answer `If-None-Match` with `304` and no body. On `GET /activities` the tag
is `ActivityRepository::fingerprint()` — a hash over every row's `(id,
version)`, so a publish, an edit, or a removal all invalidate it, not just an
increasing `version` somewhere in the set. On `GET /activities/:id` the tag
is `id@version`, scoped to the one row that response actually returned,
rather than the whole-catalogue fingerprint — an unrelated activity's edit
should not invalidate a client's cached copy of this one. `fingerprint()` is
also the value #273 (CAT5)'s `CurriculumUpdated` producer reads "the
catalogue changed" from; that story calls this method directly rather than
inventing a second notion of catalogue version.

**The bound is a refusal, not a truncation.** `GET /activities` counts the
table before querying it; over `CATALOG_CEILING` rows and the whole request
is refused (`FileHostError::MaxRecordLimitExceeded`) rather than answered
with a silently short prefix. The same bounded-or-refused-never-silently-partial
invariant `#253` argues for elsewhere applies to this new surface too.

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

### A read reachable from the waker declares its own bound

**There is no caller to paginate it.** `GET /sessions` can leave `list()`
unpaginated because the client is the one caller, and the client already
paginates the result in memory. `nudge::waker::consider` broke that: nothing
sits above the waker to page through what a query on this path hands back —
the tick fires, the pass runs, and the read happens in full. A query on this
path that assumes some caller will bound it is assuming a caller that does
not exist.

So the invariant is stated the other way round: **a read reachable from the
waker declares its own bound**, in the query itself (a `LIMIT`, an indexed
`WHERE`, or a narrower question than "everything") rather than in a caller
that isn't there to enforce one. `list()` narrows *which subject's rows* it
can see (#260/SUB2 — a query can no longer return another subject's sessions
at all) but does not bound *how many*; `#262` (SLI1) measured that gap, a
characterisation test in `nudge::waker`'s test module pinning down
`list()`'s `O(BATCH × sessions this subject owns)` cost. `#263` (SLI2) closes
it: the waker no longer calls `list()` at all. `SessionRepository::first_prepared(subject)`
replaces the list-then-find with a purpose-built, `LIMIT 1` query backed by
`idx_sessions_status` (`(subject_id, status, updated_at DESC)`) — a `paused`
session outranks a `scheduled` one, which outranks an untouched `draft`, with
`updated_at DESC` breaking ties within one status (see `first_prepared`'s own
doc comment for why). #262's characterisation test now asserts this bound
directly: at most one row read per due subject, independent of how many
sessions it owns.

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

### The server writes data; the client writes behaviour (#272)

Every `ActivityDefinition` (`packages/activity-catalog/src/lib/types.ts`,
`paulgsc/some-ui`) carries `toSceneProps`, a closure mapping a friendly config
onto a scene's `props`. There is no JSON encoding of a function, so a
catalogue this server serves arrives without one — and a session this server
*composes* (`#279`–`#285`) hits the same wall from the other side: it cannot
write scene `props` it has no way to compute.

Two ways of closing that gap were considered and are recorded here as
**rejected**, so neither gets re-proposed:

- **Ship the source and `eval` it.** Remote code execution as a product
  feature. Not an option, for any reason.
- **Ship a template language and interpret it here.** This server would need
  a mini-interpreter, and the boundary the client's own catalogue already
  draws would sit underneath it unenforced. `interview`'s entry is explicit
  about why it passes an identifier rather than resolved content: *"importing
  anything from `@some-ui/interview` for a value puts that package in this
  app's eager bundle — undoing the lazy import the content registry exists
  for."* A server-side renderer has no way to know that, and would happily
  let someone violate it.

**What this server actually writes** is `activities: [{ activityId, config
}]` — already what the client's `SessionActivity`
(`packages/activity-catalog/src/lib/to-scene-config/index.ts`) is — and
nothing else. The client's existing `toSceneConfig`/`sequenceScenes` do the
rest, the same way they already do for a Basic-composer session where the
user made zero arrangement decisions: a server-composed session is not a new
code path on the client, it is the existing one fed data from a different
source. `paulgsc/some-ui#1036`'s ACT2 is where the closures move into a
resolver keyed on `registryKey` instead of living on each definition, but
that move is independent of this conclusion — the server was never going to
call them either way.

**The catalogue schema already honours this.** `ActivityRecord`
(`crates/db/activity/src/model.rs`) has no `toSceneProps` field at all —
`fields`, `default_config`, and `audio` are the only JSON-blob columns, and
each is opaque *data* this crate persists without interpreting, never a
column that only makes sense as executable code. Nothing about landing this
story required a schema change.

**The `scenes` consequence, faced honestly.** If the server does not write
`props`, it cannot write playable `scenes` either — and `scenes` is `NOT
NULL`. A session this server provisions therefore writes `scenes` as `'[]'`:
a real, valid empty JSON array, not a guess dressed up as a placeholder and
not the string `'null'`. That is a session state nothing before this story
produced — `activities` non-empty, `scenes` empty — and it is deliberate, not
an oversight: turning `activities` into playable `scenes` is the client's
job, using the same `sequenceScenes` pipeline a Basic-composer session
already runs, at whatever point the client chooses to materialise them
(closing that loop is `paulgsc/some-ui#1038`'s PRO1, not this story).
**RCM5 (#282)** is where a provisioned row actually gets written, and its own
"the hard one" section already arrives at the same three options this
paragraph does; #282 is expected to cite this decision rather than re-argue
it. The combination consistent with the conclusion above is #282's option 1
(`scenes: []`) for what the server writes, materialised later by #282's
option 2 (the client computing real scenes and writing them back) rather
than option 3's half-measure of structurally-complete scenes with empty
`props` — a shape that would look valid and would not be.

The round trip this implies — a catalogue row's `default_config` is
sufficient, as-is, to become a `SessionActivity.config` the client's
`toSceneConfig` accepts — is checked on both sides of the boundary:
`crates/db/activity/tests/session_activity_shape.rs` here, asserting every
seeded `default_config` is shaped like `ActivityConfigValues` (a flat object
of strings and numbers, nothing nested); `paulgsc/some-ui`'s
`packages/activity-catalog/src/lib/to-scene-config/session-activity-round-trip.test.ts`,
asserting a `SessionActivity` built the same way survives an actual JSON
round trip into a playable scene.

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

**One subject.** `SubjectId` is a singleton until auth lands. Every table the
study policy reads carries a `subject_id` column as of #259, and every
repository that reads or writes one — including `SessionRepository`, as of
#260 — filters and writes it. What's still singleton is the *value*: every
session route extracts its `SubjectId` via `SubjectId::from_request_parts`
(#261), rather than a handler passing a raw constant, but that extractor still
returns `SubjectId::singleton()` until real auth lands, so nothing has been
exercised with two genuinely different subjects regardless.

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

# Identity and privacy

> An account here is pseudonymous, not anonymous. The server can tell that the
> same subject came back, and it never asks who that subject is. It cannot
> promise that nobody could work that out: what a subject studies, when, and
> what they choose to keep is stored under that id, and the machinery in front
> of the server sees network addresses.

The design goal is that the server never needs to know who a subject is. The
client holds the secret: a passkey, whose private key never leaves the person's
authenticator. The server asks for nothing that names a person (no name, email,
phone number or biometric), and holds what it needs to recognise a returning
subject, to keep one client from starving another, and to run the study
features that subject turns on. The code is built so that nothing it stores or
logs names a person.

That is a statement about what the code asks for. It is not a promise about
what people choose to send it (a kept shelf item or a captured page can say
anything), and it says nothing about what sits outside this process: the
reverse proxy, the container log driver, the push services and the passkey
provider. "What is still exposed" lists each, stated rather than assumed.

This document says who owns identity in `file_host`, lists the invariants that
hold, names where each one is enforced and what it does *not* prove, and ends
with what is still exposed.

---

## Who owns "who is calling"

There are four handles on a request. Exactly one of them is an identity.

| Handle | What it is | Identity? |
|---|---|---|
| `subject::SubjectId` | The subject a request acts for. Decided in one place, the extractor in `subject.rs`, from the request's session: a passkey sign-in's cookie, or a paired phone's bearer token. | **Yes, the only one.** |
| `net::PeerKey` | A keyed, daily-rotating hash of the peer's address. Used by the rate limiter for fairness, and named in log lines. | No |
| `net::AdmissionKey` | The same keyed hash over a secret that lives as long as the process. Used for WebSocket admission accounting. **Never logged.** | No |
| `ws_connection::ClientId` | `probe:` / `proxy:<PeerKey>` / `direct:<PeerKey>`. Groups WebSocket connections for metric labels. | No |

Until #372 there were three identities and none had an owner. The rate limiter
keyed on the raw IP. The WebSocket layer built its own `ClientId` from IP plus a
user-agent hash, and believed a client-supplied `X-Client-ID` header, which it
labelled `auth`. Nothing authenticated that header, and no client sent it.

### The `PeerKey`

`net.rs` is the only code that sees a peer address:

- **Keyed.** The digest is `SHA-256(secret || address)`, where `secret` is 32
  random bytes that exist only in this process's memory. The IPv4 space is small
  enough to hash exhaustively, so an unkeyed hash would just be the address with
  extra steps.
- **Rotated daily, to a fresh secret.** At each UTC day boundary the secret is
  replaced with new random bytes, not derived from the old ones. Once a day's
  secret is dropped, nothing can recompute that day's keys, including a later
  memory dump of the process. A key in yesterday's logs can no longer be tied to
  an address.
- **A second, process-stable key for accounting that outlives a day.** A
  WebSocket held open across midnight keeps its admission permit. If the next
  connection from the same address were counted under the new day's
  `PeerKey`, it would get a fresh per-client allowance, and a client could
  stack connections up to the global limit by outlasting rotations. So
  `ConnectionGuard` counts under an `AdmissionKey` instead. Because it doesn't
  rotate, it would link a client across days if it ever reached a log line,
  so it can't: it has no `Display`, its `Debug` is redacted, and
  `ConnectionGuard` logs no client key at all. It lives only in memory, and is
  gone with the process.
- **Not an identity.** The first `X-Forwarded-For` hop is client-controlled.
  Forging it buys a caller a different fairness bucket and nothing else, which is
  only acceptable because nothing treats a `PeerKey` as "who".

### The seam

`SubjectId::from_request_parts` is where a request's session becomes a subject,
whether the session arrived as the cookie or as a paired phone's
`Authorization: Bearer`, and a request without a live session is refused with
`401`. There is no fallback subject. Passkey auth changed that one function body; no subject-scoped
handler, repository or route path changed with it.

---

## Passkey auth

A passkey is the only way in. There is no email, password, magic link, social
login or recovery code, so there is no contact detail or shared secret to store,
leak or recover with. Losing every copy of every passkey loses the account; that
is the cost of holding nothing that could recover it.

Passkey sign-in is on unless `AUTH_ENABLED=false`, and while it is on, the
server refuses to start without `WEBAUTHN_RP_ID` and `WEBAUTHN_ORIGINS`. It
used to boot without them and log a warning, so the only symptom was every
sign-in failing with "Passkey sign-in isn't set up on this server yet".
`AUTH_ENABLED=false` runs with nobody able to sign in: every ceremony answers
`503` and every subject-scoped route `401`.

### What an account is

| Stored | Where | What it is |
|---|---|---|
| Subject id | `account.subject_id` | `subject-` and 32 random hex digits, minted by `subject.rs`. The key of every per-person row. |
| User handle | `account.user_handle` | The WebAuthn `user.id`: 16 random bytes. The only thing an authenticator returns on a sign-in that names no account. |
| Passkeys | `passkey` | Credential id, public key, signature counter and backup flags. At most 16 per account. |
| Sessions | `auth_session` | SHA-256 of each session token (a paired phone's over a fixed prefix too), its subject, and when it expires. At most 32 per account, paired phones included. |

Nothing records when an account or a passkey was made or last used. The
per-day cap on new accounts is counted in memory for the same reason.

Every new account gets a random subject id, the first one included. The rows
written before auth existed carry the placeholder subject `subject-local`, and
an account takes them over only with the operator's one-time
`AUTH_LEGACY_CLAIM_TOKEN`: set it on the server, open the app at
`/auth#claim=<token>`, create a passkey there, then unset it. The token travels
in the registration body (a URL fragment never reaches a server, so no access
log sees it), is compared as a digest, and works once. "Whoever registers
first" is not an owner: on a reachable server, first is whoever gets there.
Deleting that account deletes those rows like any other account's.

### The ceremonies

Every ceremony is `…/start` then `…/finish`, with the challenge held in memory
between them (`auth::ceremony`: at most 10,000 open, five minutes each, taken
once).

- **Create an account** (`/auth/register/*`). Options ask for
  `attestation: "none"`, a discoverable credential (`residentKey: "required"`)
  and user verification. `user.name` and `user.displayName` are the fixed
  string `Some UI`, so no name travels to the authenticator or anything that
  syncs it.
- **Sign in** (`/auth/sign-in/*`). The options name no account
  (`allowCredentials` is empty). The browser lists the passkeys it holds for the
  site; the chosen one returns its user handle, which finds the account.
- **Add a passkey** (`/auth/passkeys/*`), signed in, for a device outside the
  first passkey's sync ecosystem.
- **Leave.** `/auth/sign-out` ends this session, `/auth/sign-out-everywhere`
  ends all of them, and `DELETE /auth/account` deletes the subject's rows from
  every table in `subject::SUBJECT_SCOPED_TABLES`, in one transaction. The
  deletion first waits for every write already in flight for a subject, and
  holds new ones off until it commits (`auth::DeletionLock`, held by
  `SubjectId` for the whole request and by the waker around each write). So a write resolved
  before the deletion is deleted with the rest, and one after it finds no
  session. Nothing in the subject tables references `account`, so the lock is
  what stops a write outliving its account.

### What deleting an account erases

`DELETE /auth/account` deletes the subject's rows from every table in
`subject::SUBJECT_SCOPED_TABLES`. Said to the person, those tables fall into
these kinds, and the Account screen (paulgsc/some-ui,
`apps/www/src/lib/auth/account-erases.ts`) names each one, and what stays,
above the button that deletes (invariant 22):

| Kind | Tables |
|---|---|
| Sessions | `sessions` |
| Saved shelf items | `learner_shelf` |
| Reminders and progress | `activity_outcome`, `engagement_charge`, `engagement_gate`, `intervention_log`, `presence_leases`, `push_subscriptions` |
| aph | `aph_entry`, `aph_settings` |
| The Jobs record | `jobs_event` |
| Telling the phone a morning run is in | `jobs_notice_device`, `jobs_notice_sent` |
| Jobs mailbox files home holds | `mailbox_item`, `mailbox_archive` |
| Filed with home for the mailbox | `mailbox_outgoing`, `mailbox_handed` |
| LeetType sessions | `leetype_session_event` |
| Voice notes | `voice_note`, `voice_note_audio`, `voice_transcript` |
| What a phone shares | `sharing_choice` |
| Sign-in | `account`, `passkey`, `auth_session` |

A file home archived out of the mailbox goes with the account, and the person
is told so: once the mailbox has let a file go, `mailbox_archive` holds the
only copy of its bytes (invariant 21). What deleting does not reach is under
"What is still exposed", in "What deleting an account does not reach".

### The session

A session is 32 random bytes, base64url in a `__Host-session` cookie:
`Path=/`, `HttpOnly`, `Secure`, `SameSite=Strict`. Only its SHA-256 is stored,
so a copy of the database opens no session. It lasts `AUTH_SESSION_DAYS` (30)
and slides: `GET /auth/session` past the halfway point renews it to a full term.
Renewal changes state, so it is held to the origin check below even though it
is a GET. A sibling page embedding the URL as an image learns nothing and
extends nothing.

`SameSite=Strict` keeps the cookie off cross-*site* requests, but a page on a
sibling subdomain is same-site, and its form posts carry the cookie. So every
state-changing, cookie-authenticated request is also held to an origin check
(`auth::csrf`, applied in `SubjectId`'s extractor and by `/auth/sign-out`):
`Sec-Fetch-Site: same-origin` passes, and otherwise the `Origin` must be in
`ALLOWED_ORIGINS` or `WEBAUTHN_ORIGINS`, or the request is refused with `403`.

The app reaches `file_host` through its own origin's `/api/file-host` proxy,
or in development on its published port: same-site but cross-origin, so the
app's transport sends `credentials: "include"` and the modules it calls use
the credentialed CORS allowlist. A session is created only if its account
still exists, in the same statement, so a sign-in racing an account deletion
gets no session rather than an orphan one.

### Pairing a phone

The Android app serves its pages from `https://localhost`, another site, so it
can neither hold the `SameSite=Strict` cookie nor run a passkey ceremony for
this relying party. It gets a session another way (`auth::pairing`):

1. A signed-in browser calls `POST /auth/pairing/start` and is shown a code:
   8 characters of Crockford base32 (40 bits), open for ten minutes. Asking
   again replaces the account's open code.
2. The person types it on the phone, which sends it to
   `POST /auth/pairing/finish` and is answered `{ token, expiresAt }`.
3. The phone sends that token as `Authorization: Bearer`. A browser never
   attaches that header on its own, so it is no ambient credential.

A code is held in memory only, found by its SHA-256 and taken once, like a
ceremony; a restart drops the open ones, and so do account deletion and
sign-out everywhere. Both run under the exclusive hold against deletion,
and redeeming a code holds it from taking the code to making the session, so
a pairing in flight lands wholly before them (and is ended) or after them
(and finds no code). The token is an ordinary
`auth_session` row, so no column is added: it expires and slides like any
session (renewing it sets no cookie), and it ends with `/auth/sign-out` sent
with the token (unpairing), with `/auth/sign-out-everywhere`, or with the
account. No column says a session is a phone's; its digest is taken over a
fixed prefix and the token (`SessionToken::hash`), so a phone's token sent as
a cookie, or a browser's sent as a bearer, finds no session. How a session
arrives is therefore what kind it is. What the phone's person chose to keep
sending home names the pairing by a digest of that session's digest, in
`sharing_choice` (invariant 17), not in `auth_session`.

A phone's token acts on the account's study data and nothing that would
outlive or widen it: adding a passkey, pairing another phone, deleting the
account and the operator routes take a browser's passkey session only
(`subject::BrowserSubject`), and answer a bearer token `403`. The token sits
in the phone's app storage, easier to lift than an `HttpOnly` cookie, so a
stolen one must not be able to plant a passkey that outlives sign-out
everywhere.

The origin check (`auth::csrf`) runs on every state-changing request before
its session is read, the phone's included. The phone passes it by sending
none of `Origin` and `Sec-Fetch-Site`: it calls these routes over native
HTTP, never a WebView `fetch` from `https://localhost`. Redeeming a code goes
further and refuses any request that carries either header, so a code is
redeemed by a native client, never by the page that asked for it.

That is not a defence against script running on the app's own origin (an
XSS). Such script can start a pairing with the cookie and hand the code to a
native client of its own, which redeems it. It gains nothing it lacked:
with the cookie it can already start a passkey registration and answer it
with a key pair it made itself (registration asks for no attestation), a
passkey that outlives every session. Script on the app's origin is account
takeover, pairing or not; keeping it off that origin is the app's job (its
content security policy), not this route's.

### Who can make an account

Anyone who can reach the server. The per-client rate limiter on `/api/v1`
bounds one client, and `AUTH_NEW_ACCOUNTS_PER_DAY` (100) bounds the server,
which also covers a client forging `X-Forwarded-For` to get fresh rate-limit
buckets. An account with nothing in it costs a few hundred bytes.

### Who is an operator

A subject listed in `OPERATOR_SUBJECTS`, and nobody else. The operator routes
(`/curriculum/operator/*`, `/leetype/operator/*`) rewrite what everyone is
served, and since anyone can make an account, a session alone cannot be what
gates them. `auth::operator::Operator` extracts a `SubjectId` first, so
the `401`, the origin check and the hold against deletion are the same as on any
subject-scoped route, then answers `403` unless the subject is on the list.
Unset, nobody is an operator.

The subject id is never returned to the browser. `GET /auth/session` answers
only `{ expiresAt }`: page script has never needed a stable id that follows the
account across devices, and handing it one for a one-time setup step would be a
new exposure. Instead a refused subject is logged by id at `info`, and the
operator, who configures the server and reads its log, copies it from there.
Subject ids are random and name nobody; the nudge waker logs them already.

### Conventions

- **Auth is its own context.** Its state is an `AuthContext` whose extractor is
  bounded on `AuthContext: FromRef<S>` only, and the auth routes' builder asks
  for nothing else. It does not ask for the whole `AppState`, because
  `AppState::build` connects to NATS and a handler that takes the whole state
  cannot be tested through the router. The auth tests drive the real routes with
  a pool and this context. Existing handlers are not migrated.
- **Secrets are wrapped.** A session token is held in `redacted::Redacted`,
  whose `Debug` prints `[redacted]` and which has no `Display`. A stored
  passkey's `Debug` prints no credential id.
- **The WebAuthn library logs only warnings.** It records whole credentials in
  its own debug- and trace-level spans, so `auth::loggable` caps it at `WARN`
  in `main`'s subscriber, whatever `RUST_LOG` says.

---

## Privacy invariants

Each invariant names where it is enforced. An invariant that is enforced
nowhere is marked as such and belongs to review.

1. **No column is built to hold who a person is, and every table has an
   owner.** No column is named for an address, a contact detail or a
   fingerprint, and every table is classified, in writing, as subject-scoped or
   not. A new table fails the test until someone decides which it is and says
   why.
   *Enforced by* `file_host::privacy`'s
   `the_migrated_schema_has_no_identifying_column_names_and_every_table_is_classified`,
   against the schema the migrations actually produce.
   *What it proves:* names and ownership, nothing about values. It accepts a
   `body` that holds an email address and a `tab_title` that holds a person's
   name, and it says nothing about what several columns reveal together. What a
   column's contents can say is under "What is still exposed".

2. **The peer's address never leaves `net.rs`, and never reaches a log line
   `file_host` writes.**
   *Enforced by* `clippy.toml`'s `disallowed-types` (`axum::extract::ConnectInfo`),
   through `lint.yml`'s clippy ratchet: `net::Peer` is the one `#[allow]`. Also by
   `file_host::privacy`'s capturing-layer tests. They record every field of every
   event and span on the rate limiter's rejection path and on WebSocket
   admission, the two paths that logged an address before #372, and on
   `ConnectionGuard`'s permit accounting, and fail if an address, or the
   process-stable admission key, appears anywhere in them.
   *What it proves:* those three paths, under the test's own subscriber. It does
   not cover the reverse proxy in front of `file_host`, Docker's log driver, or
   any collector: see "What is still exposed".

3. **Fingerprinting headers are named only in `net.rs`,** other than as a
   write that puts one on a request, which is how tests prove they're ignored.
   These are `User-Agent`, `X-Forwarded-For`, `X-Real-IP`, `Forwarded` and the
   CDN variants. The check matches any mention rather than a list of read
   methods, because every such list turned out to have another way through.
   *Enforced by* `scripts/check_privacy.py`, whose `--self-test` also runs in CI.
   Clippy cannot express this one, because its `disallowed-*` lints match paths,
   never arguments.

4. **Nothing a client asserts about itself becomes an identity.**
   *Enforced by* the same script, which puts `X-Client-ID` on its list, and by a
   test in `websocket::connection`.

5. **Only `subject.rs` can mint a `SubjectId`.**
   *Enforced by* `compile_fail` doctests on the type, next to one doctest that
   compiles, so a `compile_fail` can only be failing on the constructor and not
   on a wrong path.

6. **Every `#[instrument]` names what it skips, and a handler never records
   request content.** Without `skip`/`skip_all`, tracing records every
   argument as a span field, at the default `info` level, and the
   OpenTelemetry layer exports those fields too. `skip(state)` alone is not
   enough on a handler that takes a body: `Json(payload)` is recorded. So an
   extractor that carries request content (`Json`, `Form`, `Bytes`,
   `Multipart`, `Query`, `Path`) must be skipped itself, with `skip_all` or by
   its binding, and a value worth recording is named in `fields(...)`.
   *Enforced by* `scripts/check_instrument_skip.py`. It reads the attribute
   and the signature that follows it. It does not read what `fields(...)`
   records or what a handler passes to `info!`, which belongs to review.

7. **Passkeys reveal no person and no device.** WebAuthn `attestation: "none"`,
   since anything else reveals the authenticator model, and whatever attestation
   and transports a client sends anyway are dropped before storage. The WebAuthn
   `user.id` is 16 random bytes, and `user.name` is a fixed string. No email or
   other contact detail is required to hold an account; none can be given.
   *Enforced by* `handlers::auth`'s tests
   `registration_options_ask_for_nothing_identifying` and
   `a_stored_passkey_keeps_no_attestation_and_no_transports`, which run a real
   ceremony with a software authenticator that sends `packed` attestation.

8. **No secret is stored or logged in the clear.** The database holds a
   session token's SHA-256 (a phone's, over a fixed prefix too), never the
   token, and a pairing code is never
   stored at all: it lives in memory under its SHA-256 until it is taken or
   expires. No session token, pairing code, credential id or user handle
   reaches a log line or span field, at any level the production subscriber
   lets through.
   *Enforced by* the `auth_session.token_hash` column (invariant 10's column
   test) and `handlers::auth`'s `no_secret_reaches_a_log_line`, which captures
   every field of a sign-up, a sign-in, a refused sign-in and a phone's
   pairing through `auth::loggable`. A companion test proves that filter is
   needed.

9. **An account takes all its rows with it.** Deleting an account deletes the
   subject's rows from every subject-scoped table, including a row a request
   that was already running when the deletion began writes.
   *Enforced by* `subject::SUBJECT_SCOPED_TABLES` being both the list
   `DELETE /auth/account` walks and the list invariant 1's schema test holds
   every table with a `subject_id` to, and by
   `deleting_an_account_removes_its_rows_from_every_subject_scoped_table_and_nobody_elses`.
   In-flight writes are covered by one rule: **every writer of subject-scoped
   rows holds `auth::DeletionLock` across its check that the subject still
   exists and its write.** A deletion, or a sign-out everywhere, holds it
   exclusively for its one transaction. There are four such writers today:
   - requests, through `SubjectId`, which takes the lock before looking up
     the session and holds it for the whole request
     (`a_write_already_in_flight_does_not_outlive_the_account_it_was_for`);
   - the nudge waker, through `waker::unless_deleted`, which takes the lock
     around one re-check of the subject's `engagement_gate` row and one write
     (`a_write_for_a_deleted_subject_is_skipped_and_waits_out_a_deletion`).
     It wraps the writes that check nothing a deletion removes: the
     provisioned `sessions` draft, and a charge save. It never holds the lock
     across a push delivery, so a deletion waits for writes, not for a pass.
     The claim and its log entry need no wrapper: they share a transaction
     with an `UPDATE engagement_gate … WHERE subject_id = ?`.
   - redeeming a pairing code (`/auth/pairing/finish`), which acts for no
     `SubjectId` and so takes the hold itself, from taking the code to
     making the phone's session; sign-out everywhere takes the exclusive
     hold too, so neither leaves a session or a code behind it.
   - the Jobs mailbox poller (`mailbox::pass`), which takes the lock around
     each file it keeps and each answer it records, and writes each in a
     statement that checks the
     subject's `account` row (`nothing_is_written_for_a_deleted_subject`,
     `a_write_waits_out_a_deletion_in_flight`). It never holds the lock
     across a request to the mailbox (invariant 19).

   A new writer of subject-scoped rows outside a request must do the same,
   or its rows can outlive a deleted account. Every hold is short, so a
   deletion stays well inside the HTTP timeout.

10. **The auth tables keep no timeline.** `account`, `passkey` and
    `auth_session` hold exactly the columns listed under "What an account is":
    no creation, last-seen or last-used time. A session's expiry is the one
    instant stored.
    *Enforced by* `file_host::privacy`'s
    `the_auth_tables_hold_exactly_their_listed_columns`. A new column there
    fails it until someone decides what it tells the server and says why.

11. **The learner shelf holds content only** (#387). `learner_shelf` is one
    of the four subject-scoped places that store something the learner wrote
    rather than something the server observed (the aph and Jobs records and
    the LeetType session log, invariants 15, 16 and 18, are the others): items
    they generated and chose to keep (`PUT /shelf/:activity/:key`,
    docs/study-nudge.md, "Learner shelf").
    A row is the subject, the activity, the item's key, its content hash, when
    it was kept, and the body verbatim: what storing and listing it needs, and
    no survey, flag, outcome, title or anything derived from play. It is
    capped at 20 items per activity, readable only by its own subject, written
    only when the learner asks, and deleted with the account like every other
    subject-scoped table.
    *Enforced by* `learner_shelf_repo`'s
    `the_table_holds_exactly_its_described_columns`, which fails on a new
    column until someone decides what it tells the server and says why, and
    by `handlers::shelf`'s `one_subject_can_neither_list_read_nor_delete_anothers_items`.

12. **Account deletion holds across a rollback.** `schema::drift` accepts a
    database ahead of the binary, so an image rollback runs a binary whose
    `SUBJECT_SCOPED_TABLES` predates newer tables. Every binary that serves
    `DELETE /auth/account` deletes the `account` row, so every subject-scoped
    table created after account deletion shipped (#395) also carries a trigger
    `AFTER DELETE ON account` that deletes the subject's rows. `learner_shelf`
    is the first (#398). A new subject-scoped table adds its own trigger, in
    the migration that creates it, and drops it first in the down migration.
    *Enforced by* `file_host::privacy`'s
    `every_table_newer_than_account_deletion_leaves_with_the_account`, which
    fails on a subject-scoped table outside the #395 list with no such
    trigger.

13. **HTTP spans name a route, not a URL.** tower-http's default span records
    the whole URI at DEBUG, query string included. `TraceLayer` is built with
    `trace::RouteSpan`, which records the method and the matched route
    template (`/api/v1/tabs/:tab_id`), or one fixed word for a request no route
    matched, and nothing else from the request. A query string or a free-text
    path segment never reaches a span, and so never reaches the log or the
    OpenTelemetry exporter.
    *Enforced by* `file_host::trace`'s
    `a_request_span_names_the_route_and_never_the_query_or_a_path_segment`, with
    `the_default_span_this_replaces_records_the_whole_uri` as the control that
    keeps it from passing vacuously. *It proves* the span this layer builds. It
    does not cover a handler that logs its own arguments (invariant 6).

14. **A push endpoint is never logged; the log names a recognised push
    service, or `other`.** An endpoint is a per-browser address, and
    `validate()` accepts any `https://` host, so even its host can be the
    client's own choice (a device-specific name, a token in a subdomain).
    `PushSubscription::service()` therefore returns one of a fixed set of
    labels for the browsers' own providers (`fcm.googleapis.com`,
    `updates.push.services.mozilla.com`, `web.push.apple.com`,
    `notify.windows.com`) and the word `other` for everything else, lookalikes
    included. That says which provider answered when it is one of those, which
    is what a log line and a failure need, and nothing the client chose. The
    HTTP client's own connection logs name the host and port of every outbound
    connection at debug and trace level, a push endpoint's included, so
    `auth::loggable` keeps `hyper_util::client` and `reqwest` below `warn` out
    of the log whatever `RUST_LOG` asks for, as it already does for
    `webauthn_rs`. `ReqwestTransport` strips the URL from its errors with
    `reqwest::Error::without_url`, because a transport error's text names the
    URL it failed on and `SendOutcome::Transport` is logged. A push service's
    own words are not logged either: the waker logs `SendOutcome::log_detail()`
    (a status, a number of seconds, or text this process produced), never the
    outcome's `Debug`, because `Rejected`, `ServiceError` and `RateLimited`
    hold a response body and a `Retry-After` that a provider may fill with the
    URI or token it was sent.
    *Enforced by* `waker`'s
    `no_push_endpoint_reaches_a_log_line_whatever_the_delivery_outcome`, which
    makes one device time out, one refuse the connection, one accept and three
    answer 400, 503 and 429 with the request line quoted in the body and the
    `Retry-After`, and fails if any endpoint, its host or its per-browser path
    appears in anything captured; and by `push_kit`'s tests of
    `PushSubscription::service` (a device-specific host and lookalikes are
    `other`) and of `SendOutcome::log_detail`; the same test runs under the
    production filter, and a control test shows the HTTP client's connection
    log does name the host without it. *It proves* the waker's delivery paths. The subscribe route's
    single log line names the service by the same method, and is covered by
    review.

15. **The aph record is append-only, and holds what the owner logged.**
    `aph_entry` and `aph_settings` keep the owner's aph log once the phone is
    opted in to keeping it on home (paulgsc/some-ui `packages/ui/aph`): every
    revision of every entry and of the settings, never updated (a trigger
    aborts an `UPDATE`) and never deleted but with the account. A revision is
    the entry as the phone wrote it, checked and re-serialised (the figures,
    labels and note), the day and checkpoint and whether it is plain (the
    columns the one-plain-figure check reads), and when it was written. It is
    written only by `PUT /aph/entries/:id`, `PUT /aph/settings` and
    `POST /sync/aph/batch` (entries, each as its own route), readable
    only by its own subject, and deleted with the account by both the sweep
    and the migration's triggers (invariant 12).
    *Enforced by* `aph_repo`'s `the_tables_hold_exactly_their_described_columns`
    and `every_change_is_a_new_revision_and_every_old_one_stays` (an
    `UPDATE` is refused), `the_record_leaves_with_the_account`, and
    `handlers::aph`'s `refusals_name_their_fields_and_records_stay_apart`.

16. **The Jobs record is append-only, and holds what the owner recorded.**
    `jobs_event` keeps the owner's Jobs log once the phone is opted in to
    keeping it on home (paulgsc/some-ui `apps/www/src/lib/jobs`): every event,
    as the phone recorded it, checked and re-serialised (picks taken, passed
    or archived and why, postings read, why the owner fits one taken, how
    applying went, outcomes, postings found by hand), when the phone recorded
    it and when it reached home. An event is never updated
    (a trigger aborts an `UPDATE`) or replaced (a different body under a held
    id is a `409`), and never deleted but with the account. It names voice
    notes by the phone's ids only, and holds no audio: a note's audio, once
    the phone sends it home, is `voice_note_audio`'s (invariant 23). It is
    written only by
    `PUT /jobs/events/:id` and `POST /sync/jobs/batch` (each event as that
    route), readable only by its own subject, and deleted with
    the account by both the sweep and the migration's trigger (invariant 12).
    *Enforced by* `jobs_repo`'s `the_table_holds_exactly_its_described_columns`,
    `an_event_is_stored_once_and_never_replaced` and
    `the_record_leaves_with_the_account`, and `handlers::jobs`'s
    `refusals_name_their_fields_and_records_stay_apart`.

17. **What a phone shares is a choice home keeps a time for at each change,
    never one per item.** `sharing_choice` keeps what each paired phone's
    person chose to keep sending home (paulgsc/some-ui
    `apps/www/src/lib/home-link`, its consent log): one row each time sharing
    a kind of record (`sessions`, `shelf`, `aph`, `jobs`, `voice`) began or
    ended under a pairing, with when home heard it, and nothing about when
    anything was sent under it. Those times are kept so home can refuse the
    aph, Jobs and voice-note writes a phone sends after its person stopped
    them (`PUT /aph/*`, `PUT /jobs/events/:id`, `POST /sync/:collection/batch`
    and `PUT /voice-notes/:id` answer `409`, code `sharing_stopped`; sessions, the shelf and the LeetType session log
    also go home on a tap no choice covers, and are not refused),
    and say when each choice started and stopped; a timestamp on each synced
    row stays out. A choice is made only by a paired phone
    (`PUT /sharing/:scope`, a bearer session: a browser's is `403`), counts
    for that pairing alone, and is never updated (a trigger aborts an
    `UPDATE`) or deleted but with the account. The pairing is named by
    `pairing_id`, a digest of its session's stored digest
    (`auth::cookie::pairing_of`): it opens nothing, is never sent by the
    phone (invariant 4), and is no identity, since it is only ever read under
    the subject it came with.
    *Enforced by* `sharing_repo`'s `the_table_holds_exactly_its_described_columns`,
    `a_change_is_one_row_and_no_row_changes` and
    `the_choices_leave_with_the_account`, and `handlers::sharing`'s
    `only_a_paired_phone_makes_or_reads_a_choice` and
    `home_refuses_only_what_a_phone_stopped`, `handlers::sync`'s
    `a_batch_is_refused_whole_once_its_phone_stopped_sending_it`, and
    `handlers::voice`'s `home_refuses_a_note_once_its_phone_stopped_sending_them`.
    A new scope widens the table's CHECK by rebuilding it with every row
    copied (`20261025000300_add_voice_to_sharing_choice`, held by
    `sharing_repo`'s `adding_voice_keeps_every_change_and_both_triggers`), and
    rolling that back once a pairing has chosen the scope fails rather than
    discarding the choice
    (`rolling_back_past_voice_refuses_rather_than_discards`).

18. **The LeetType session log is append-only, holds what the owner played,
    and holds no soundbite.** `leetype_session_event` keeps the phone's
    LeetType session log once the phone sends a finished session home
    (paulgsc/some-ui `packages/ui/leetype/src/lib/leetype/session-log`): every
    event of the session, as the phone recorded it, checked and re-serialised
    (the session file started and in which form, each pick and how many
    milliseconds it took, the readings placed, opened, saved or read, the
    rounds parked, returned and confirmed, how each spoken answer was given,
    for how many seconds and whether it was strong, and how the session
    ended), its type, when the phone recorded it and when it reached home.
    What was said or typed in a spoken answer, and what it was marked against
    (`ticks`, `prompt`, `rubric`, `typed`), goes to the owner's Drive and
    never here: an event carrying any of them is a `422` naming the field, and
    nothing of it is stored. An event is never updated (a trigger aborts an
    `UPDATE`) or replaced (a different body under an id its session holds is
    a `409`), and never deleted but with the account. It is written only by
    `PUT /leetype/sessions/:session/events/:id`, readable only by its own
    subject (`GET /leetype/sessions` and `GET /leetype/sessions/:session/events`),
    and deleted with the account by both the sweep and the migration's
    trigger (invariant 12). The corpus-wide `GET /leetype/rounds*` and
    `leetype_round` hold nothing of it.
    *Enforced by* `leetype_log_repo`'s `the_table_holds_exactly_its_described_columns`,
    `an_event_is_stored_once_and_never_replaced`,
    `the_record_leaves_with_the_account` and, for the soundbites,
    `a_soundbite_is_refused_by_name` and `a_refused_event_writes_nothing`, and
    `handlers::leetype_log`'s `refusals_name_their_fields_and_records_stay_apart`.

19. **Home's copy of the Jobs mailbox is append-only, each table has one
    writer, and it holds JSON files, never audio.** The mailbox (paulgsc/some-ui
    `apps/mailbox`) is a Cloudflare Worker where the owner's cloud routine
    and home leave each other files. For the one subject `MAILBOX_SUBJECT`
    names, `file_host`'s `mailbox` poller reads only its `brief` folder (the
    routine's daily job digests), its `scene` and `drama` folders (the
    reflections routine's word sets and dramas) and its `run` folder (a
    record of what each routine's runs found and added, which the phone
    shows as when the routine last ran and, for the jobs routine, what this
    morning's run couldn't do; owner, 2026-10-24),
    `mailbox::pass::READS`, and puts only to `decisions`, `scout` and
    `opened`.
    - `mailbox_item`, written only by the poller: each file it kept from
      those four folders, its bytes as the mailbox held them (they must
      match the listing's SHA-256 and parse as JSON), its hash, size and
      sequence number, and when home read it. Read only by
      `GET /jobs/digests` (`brief`), `GET /jobs/runs` (the jobs routine's
      `run-jobs-*` records in `run`) and `GET /routine/files` (`scene`,
      `drama` and the other `run` records), each for its own subject, and by
      the arrival notice's pass (`jobs_notice`), which reads the same
      `run-jobs-*` records for the poller's subject (invariant 24).
    - `mailbox_outgoing`, written only by `PUT /jobs/files/:name` and
      `PUT /routine/opened`: a decisions or scouts file the phone filed, or
      which of the routine's lessons it opened, its bytes, hash and when.
      The jobs route refuses a file that names a voice note anywhere (any
      `voice` key that is not an empty list), so no file that points at
      audio is handed on; those go by the phone's share sheet, which carries
      the audio to Drive. The routine route builds its file itself, as
      `opened-<stamp>.json`: `<folder>/<name>` of each lesson (`scene` or
      `drama` and a mailbox name), at most 100, and home's time, nothing
      else (owner, 2026-10-24: the routine keeps three lessons not opened).
    - `mailbox_handed`, written only by the poller: the mailbox's answer to
      each filed file it handed over, once, final.
    - `mailbox_archive`, written only by the poller with the archive token
      (invariant 21): each file it archived from any folder, its bytes as the
      mailbox held them (they must match the listing's SHA-256), its hash,
      size, sequence number, when it was first put and when home stored it.
      Read only by the poller, to put a file back when the routine asks.

    No row of the four is updated (each table's trigger aborts an
    `UPDATE`), replaced (other bytes under a held or filed name are refused,
    and nothing writes `INSERT OR REPLACE`, which SQLite runs no `UPDATE`
    trigger for) or deleted but with the account, by the sweep and the
    migration's triggers (invariant 12). Every write holds
    `auth::DeletionLock` (invariant 9): the route through `SubjectId`, the
    poller around each write. The poller's token (`MAILBOX_TOKEN`) is held in a
    `mailbox_kit::Token`, which prints `[redacted]` and has no `Display`; it
    reaches no log line, and neither does the mailbox's URL or a file's body
    (errors are logged by kind, and the transport strips the URL from its
    errors). No audio is stored in them: home holds audio only in
    `voice_note_audio` (invariant 23), and the mailbox never carries it.
    *Enforced by* `mailbox_repo`'s `the_tables_hold_exactly_their_described_columns`,
    `a_held_name_is_not_kept_twice`, `a_filed_name_is_not_filed_twice`,
    `no_row_is_ever_updated`, `nothing_is_kept_for_a_subject_with_no_account`
    and `the_copy_leaves_with_the_account`; `mailbox::pass`'s
    `a_sha_mismatch_is_not_kept`, `a_non_json_body_is_not_kept`,
    `nothing_is_written_for_a_deleted_subject` and
    `a_conflict_or_a_refusal_is_final_and_not_handed_again`, and its fake
    transport, which fails a `GET` of anything but those three and a `PUT` of
    anything but `decisions` or `scout`; `mailbox_kit`'s
    `the_token_never_appears_in_a_debug_or_an_error`; `handlers::routine`'s
    `files_answer_the_callers_own_scenes_and_dramas_newest_first`; and
    `handlers::jobs`'s `digests_answers_only_the_callers_own` and
    `files_refuse_a_file_that_names_a_voice_note_wherever_it_hangs`.
    *Not enforced:* that nothing else writes these tables, which a
    `git grep -n 'mailbox_item\|mailbox_outgoing\|mailbox_handed\|mailbox_archive' -- '*.rs'`
    shows today and review keeps; and what a digest says, which is the
    routine's.

20. *Retired (2026-10-23).* It held home's D1 usage collector to one
    Cloudflare analytics read that kept nothing but gauges. The collector was
    removed: Cloudflare's own D1 Metrics page shows the same numbers, and
    home no longer asks Cloudflare anything.

21. **Home archives a mailbox file only once it holds the bytes, and puts
    back only those bytes.** The mailbox is where the routine and home meet,
    not the record (owner, 2026-10-23). With `MAILBOX_ARCHIVE_TOKEN` (the
    Worker's `archive` role, a token apart from `MAILBOX_TOKEN`), each pass
    of `file_host`'s `mailbox` poller takes the files live in any folder for
    `MAILBOX_ARCHIVE_DAYS` (at least 30, the mailbox's own floor), at most
    `ITEMS_PER_PASS` of them. For each it reads the bytes, checks them
    against the listing's size and SHA-256, and stores them in
    `mailbox_archive`, committed, under the deletion hold. Only then does it
    ask the mailbox to archive the file, naming that SHA-256 in `If-Match`,
    so the mailbox drops nothing but the bytes home holds. A file home could
    not store (other bytes already under its name, no account) is never asked
    for, and neither is a file from a folder home reads (`brief`, `scene`,
    `drama`) that `mailbox_item` does not hold, so the phone never loses one
    to the archive. When the routine asks for an archived file again, the poller puts
    back the bytes `mailbox_archive` holds for it, and the mailbox takes only
    the bytes it archived. Nothing here deletes: an archived file's bytes
    are home's, and the mailbox keeps a row about it for good. Deleting the
    account deletes `mailbox_archive` with the rest (invariant 12), and an
    archived file then exists nowhere: the mailbox kept only its row.
    *Enforced by* `mailbox::pass`'s
    `an_old_file_is_stored_home_before_the_mailbox_drops_it_and_a_young_one_stays`,
    `a_file_home_cannot_store_stays_live`,
    `a_digest_the_feed_does_not_hold_is_not_archived`,
    `the_routines_word_sets_and_dramas_are_kept_like_digests`,
    `an_ask_is_answered_from_home_and_one_home_lacks_stays_open` and
    `a_pass_archives_at_most_twenty`; `mailbox_repo`'s
    `an_archived_name_holds_one_body_and_hands_it_back`; `MailboxContext`'s
    refusal of an archive token equal to `MAILBOX_TOKEN` or of fewer than 30
    days (`a_disabled_mailbox_spawns_nothing`); and, in the mailbox, MB1's
    trigger that lets an item go only once its `archived` row is written.
    *Not enforced:* that `archive_old` stores before it asks, beyond those
    tests, which review keeps by reading the function's order.

22. **Every subject-scoped table is named in what deleting an account
    erases.** "What deleting an account erases" gives each table in
    `subject::SUBJECT_SCOPED_TABLES` a kind, the unit the Account screen
    tells the person, so a table that deletion erases cannot go unsaid by being
    left off the list (#452: archived mailbox files went with an account
    before the screen said so). A table is listed once.
    *Enforced by* `file_host::privacy`'s
    `every_subject_scoped_table_is_named_in_what_deleting_erases`, which fails
    on a subject-scoped table the list omits, a name it lists that is not one,
    and a table listed twice.
    *Not enforced:* that the screen says each kind. The words live in another
    repository (paulgsc/some-ui, `apps/www/src/lib/auth/account-erases.ts`),
    and nothing there can read this list; a kind added here is added there by
    review.

23. **Home holds audio only as voice notes, append-only, and keeps them.**
    `voice_note` describes each voice note the phone sends home
    (paulgsc/server#460): what it was said for (`reflection`, `jobs` or
    `scene`) and which one, when the phone recorded it and when it reached
    home, its container (`audio/webm`, `audio/ogg` or `audio/mp4`), its size
    and SHA-256. `voice_note_audio` holds its bytes as the phone sent them,
    written in the same transaction. These two tables are the only place home
    holds a recording (the catalogue's `activities.audio` is an activity's
    disclosure copy, text), and the Jobs mailbox never carries one
    (invariant 19). A note
    is never updated (each table's trigger aborts an `UPDATE`) or replaced
    (other bytes or another description under a held id is a `409`), and
    never deleted but with the account, after it is transcribed as before
    (owner, 2026-10-25: the audio is worth more than its transcript). It is
    written only by `PUT /voice-notes/:id`, whose span skips its arguments,
    so neither the audio nor its hash reaches a log line, and which refuses a
    paired phone whose person stopped sending voice notes (invariant 17,
    scope `voice`); it is readable by
    no route yet; and it is deleted with the account by both the sweep and
    the migration's trigger (invariant 12).

    **What home heard in a note is `voice_transcript`, append-only too**
    (paulgsc/server#462). `file_host`'s `transcribe` worker, on only when
    `TRANSCRIBE_ENABLED`, reads each note's audio and hears it on home's CPU
    with whisper.cpp: oldest first, one at a time, on a blocking thread by a
    deadline, never on a request's path. Each attempt is a row: the model
    file and engine, the attempt's number, its outcome (`heard`, with the
    text, the language and its probability; `silent`, when the VAD found no
    speech; or why not: `unreadable`, `too_long`, `deadline`, `failed`), the
    audio's length, how long it took and when. A model that has heard a note
    or found it silent never hears it again (a partial unique index), and
    one that fails on a note stops after three attempts; a better model adds
    rows of its own and replaces none, and a reader takes the newest that
    heard something. No row is updated (the trigger aborts an `UPDATE`), and
    one is written only for a note the subject still holds, in the
    transaction that writes, under the deletion hold (invariant 9). The
    worker reads `voice_note_audio` and never writes it. To decode, it writes
    the note to a fresh `0700` temporary directory inside the container
    (ffmpeg cannot read the phone's `m4a` from a pipe) and removes it when
    the attempt ends; that file is the one place audio is held outside these
    tables, for the length of one decode. Neither the text nor the audio
    reaches a log line: whisper.cpp's log is dropped, ffmpeg's output is
    discarded, and a pass logs only how many attempts of each kind it kept.
    *Enforced by* `voice_repo`'s `the_tables_hold_exactly_their_described_columns`,
    `a_note_is_stored_once_and_never_replaced`,
    `the_audio_is_kept_byte_for_byte_beside_its_note`,
    `a_note_whose_audio_fails_leaves_no_description` and
    `the_notes_leave_with_the_account`; `voice_repo::transcript`'s
    `the_table_holds_exactly_its_described_columns`,
    `notes_wait_oldest_first_until_heard`,
    `a_failing_note_stops_waiting_and_an_ended_one_takes_nothing_more`,
    `the_schema_refuses_a_rewrite_a_second_ending_or_a_heard_row_without_text`
    and `the_transcripts_leave_with_the_account`; `transcribe::pass`'s
    `every_note_gets_a_row_and_a_failure_is_retried_a_few_times`,
    `a_shutdown_keeps_nothing_for_the_note_it_interrupts`,
    `nothing_is_kept_for_a_note_that_left_with_its_account` and
    `no_transcript_or_audio_reaches_a_log_line`; `transcribe::engine`'s
    `a_note_decodes_in_a_directory_only_its_owner_can_enter`; and `handlers::voice`'s
    `refusals_name_their_fields_and_notes_stay_apart` and
    `home_refuses_a_note_once_its_phone_stopped_sending_them`.
    *Not enforced:* that nothing else writes these tables or holds audio,
    which `git grep -n 'voice_note\|voice_transcript' -- '*.rs'` shows today
    and review keeps; and that the temporary directory is removed when a
    decode is killed by the process dying, which only a restart's fresh
    `/tmp` covers.

24. **A phone's FCM token is never logged and never an identity, and what
    home tells it is fixed text.** `jobs_notice_device` holds the FCM
    registration token of each phone that asked (`PUT /jobs/notice`) to be
    told when a morning run's record arrives (paulgsc/server#470). The token
    names that install of the app to Google, as a push endpoint names a
    browser (invariant 14). It is filed under the caller's `SubjectId` and
    found only by it; nothing reads one as who is calling (invariant 4).
    `fcm_kit::DeviceToken` and the access token print as `[redacted]`, the
    token travels only in a message's body (never a URL), and `jobs_notice`
    logs counts, statuses and the transport's words (`ReqwestTransport`
    strips URLs, as `push_kit`'s does), never a token or FCM's own words.
    What it sends is one of three fixed notices (`jobs_notice::PICKS`,
    `PARTLY`, `STOPPED`): whether the picks are at home or the run couldn't
    finish, and nothing of the picks, counts or reasons, since FCM carries
    the text through Google. `jobs_notice_sent` is append-only (a trigger
    aborts an `UPDATE`), one row per run record noticed, and records what was
    said and to how many phones, never which. A token FCM calls
    `UNREGISTERED`, or one the phone withdraws (`DELETE /jobs/notice`), is
    deleted. Both tables leave with the account (invariant 12).
    *Enforced by* `jobs_notice`'s
    `no_device_token_reaches_a_log_line_whatever_fcm_answers` (a refusal that
    quotes the token, no answer, a gone phone and a delivery, under the
    production filter); `fcm_kit`'s `nothing_secret_prints`; and
    `notice_repo`'s `a_run_is_noticed_once_and_never_rewritten` and
    `both_leave_with_the_account`.
    *Not enforced:* that the three notices stay free of anything personal,
    which review keeps; and that nothing else writes these tables, which
    `git grep -n 'jobs_notice_' -- '*.rs'` shows today.

Whether separately harmless fields combine into a fingerprint, and whether
timing correlates requests, cannot be linted. They belong to review. Raise them
the way `CLAUDE.md`'s "Drift is loud" asks.

---

## What is still exposed

- **Push endpoints.** A `push_subscriptions.endpoint` is a URL that is stable
  per browser, and the push service that issued it (Google, Mozilla, Apple) can
  tie it to a browser install. Payloads are encrypted end to end (RFC 8291), but
  the push service sees delivery timing.
- **FCM tokens and the arrival notice.** A `jobs_notice_device.token` names
  one install of the app to Google, which issued it. Unlike a web push, an FCM
  notice is not end-to-end encrypted: Google sees its fixed text ("This
  morning's picks are at home", "This morning's run couldn't finish"), the
  Firebase project and service account it came from, and when, so it learns
  when the owner's morning run finished, day by day (invariant 24).
- **`tabs`.** Browser-extension page captures: URL, title and extracted content,
  keyed by URL hash with no subject column. The content itself can identify
  whoever captured it. It is classified as not subject-scoped, which is accurate,
  but that does not make it harmless. See "Unauthenticated surfaces" below:
  nothing gates it, and nothing deletes it when an account is deleted.
- **The code-delivery trust gap.** The server ships the client's JavaScript, so
  a dishonest deployment could ship code that reads anything JavaScript can
  reach. A passkey's private key stays in the authenticator even then. Keys kept
  in IndexedDB, and WebAuthn PRF outputs, do not.
- **The learner shelf's contents and times.** A kept body is whatever the
  learner's own model wrote from a prompt that included their recent survey
  digest, so its text can say something about them; the server stores it
  verbatim and never reads it. Each item's `saved_at` says when the learner
  last kept those bytes, to the millisecond, which is a partial timeline of
  use: the price of listing the shelf in the order it was filled. Deleting an
  item or the account removes both.
- **The aph record and its times.** The entries are personal figures, with
  the owner's own labels and notes, and every revision is kept with when it
  was written, to the millisecond: when the phone sent each change, which,
  because the phone sends as soon as it can reach home, is close to when it
  was logged. That timeline is what the record is for. Deleting the account
  removes it; nothing else does.
- **The Jobs record and its times.** The events say which jobs the owner
  looked at, passed on and applied to, why, how applying went and what came
  of it, in the owner's own words, each with when the phone recorded it and
  when it reached home. Companies and job titles are in it, by design. Voice
  notes are not: an event names them by id, and their audio is the next
  item's. Deleting the account removes it; nothing else does.
- **Voice notes.** The audio of each note the phone sends home, kept for
  good, with what it was said for (a study session's reflection, a Jobs pick
  or scout, a drama's next scene) and when it was recorded. A voice
  identifies its speaker, and what the owner says in a note is theirs: their
  study, the jobs they weigh, what they want next. What home heard in each
  (its transcript, every attempt, every model) is kept beside it for good
  too, and says the same things in text. Deleting the account removes both;
  nothing else does.
- **The Jobs mailbox, at Cloudflare.** The daily digests (job postings
  picked for the owner, with companies and titles), and the reflections
  routine's word sets and dramas (Korean practice written for the owner: a
  word set is named for the study session it answers and quotes a few words
  of the owner's note in its `from`; a drama is named for the time of its
  prompt), and a record of each of that routine's runs (when it ran, how many
  reflections and prompts it found unanswered, the names and titles of what
  it added, and what it could not do, in a sentence), and which of those
  lessons the phone opened (their names, and when home filed them), live in
  a Cloudflare D1
  database before home reads them, and stay there until home archives them
  (invariant 21): then their bytes are home's alone, and the mailbox keeps
  each one's name, size, hash and dates. Cloudflare holds them at rest and
  sees the access pattern: home asking every `MAILBOX_SECONDS` from the home
  connection's address, and the routine putting a file each morning. Home's own copy
  (`mailbox_item`, invariant 19) says when home read each one. Deleting the
  account removes home's copy and its archive; the mailbox's live files and
  archived rows are the owner's to delete.
- **The LeetType session log and its times.** The events say which session
  files the owner played, how each round went and how long each pick took,
  which rounds they struggled with and returned to, which readings they
  opened, saved or read, and how each spoken answer went (how long it ran and
  whether it was strong, never what was said), each with when the phone
  recorded it, to the millisecond, and when it reached home. That timeline is
  what the record is for: it is how the owner's growth reads back. Deleting
  the account removes it; nothing else does.
- **When a phone began and stopped sharing.** `sharing_choice` says, to the
  millisecond, when each pairing began and stopped sending each kind of
  record home (invariant 17): one row per change, as many as its person
  flips a choice, with no cap. While the pairing's session lives, anyone
  holding the database can recompute `pairing_id` from that session's
  `auth_session` row and join the two, so the first time dates roughly when
  the phone was paired and later ones when it was used, which the auth
  tables alone do not say (invariant 10). It outlives the pairing it names,
  as what home received does. Deleting the account removes it; nothing else
  does.
- **Session expiry moves when the app is used.** A session renewed at
  `GET /auth/session` gets a new expiry of now plus 30 days, so the stored
  expiry says, to the day, when this browser last opened the app past its
  session's halfway point. Nothing older is kept.
- **A paired phone holds a session token.** Whoever has the phone's app data
  and the key protecting it can read and write the account's study data
  (sessions, the shelf, the aph and Jobs records, the LeetType session log,
  the reporting routes), and sign it out everywhere,
  until the session expires, the phone unpairs, or the account signs out
  everywhere. It cannot add a passkey, pair another phone, delete the
  account or act as an operator. The token
  travels in a request header, which nothing in `file_host` logs (invariant
  6), but which a proxy in front of it could if configured to.
- **Passkey sync providers.** A synced passkey lives in the person's Apple,
  Google or password-manager account. That provider knows the person holds a
  passkey for this site, which is the provider's knowledge, not this server's.
- **Anyone can make an account.** An open server can be filled with empty
  accounts up to the daily cap. They hold nothing and single nobody out.
- **Request URIs, outside the span.** `file_host`'s own HTTP span records a route
  template and no URI (invariant 13), but a query string is still part of the URI
  that every proxy in front of it sees, so identifying data must never travel in
  one.
- **Network addresses, outside this process.** `file_host` writes no address to
  a log line (invariant 2), but whatever fronts it sees every request's address
  and, unless it is told otherwise, logs it: nginx in the `some-ui` image, Caddy,
  the host. The `some-ui` nginx configuration turns its access log off and stops
  forwarding address headers to `file_host`; its error log still carries the
  client address and request line when an upstream fails. Docker keeps a
  container's output (`json-file`, until its size cap) wherever its log driver is
  pointed, and nothing in this repository controls backups, a log shipper or a
  collector.
- **Behaviour is stored under the subject.** The sessions a subject builds,
  when they start, finish or abandon them, the scores and elapsed times the
  nudge reads, a presence lease per open session and the engagement state
  derived from all of it are joined to the subject id. That is what an account
  is for, and it is what makes an account pseudonymous rather than anonymous:
  whoever can link the id to a person can read what that person studied, and
  when.
- **Logs and traces carry the subject id.** The waker logs subject ids, a
  refused operator is logged by id, and the OpenTelemetry layer exports span
  fields to whatever `OTEL_EXPORTER_OTLP_ENDPOINT` names. Ids are random and
  name nobody on their own; they are the join key above. Request bodies, query
  strings and push endpoints are not logged (invariants 6, 13 and 14).
- **Unauthenticated surfaces.** `tabs`, `mood_events`, `POST /now-playing`,
  `POST /utter` and `/ws` need no session. They hold or relay content that
  belongs to no subject (page URLs and titles, the video playing, text a
  browser extension sends), so no account deletion reaches it and nothing here
  ties it to a person except what it says. `tabs` is the one that stores
  something identifying, and it is under review for removal.
- **What deleting an account does not reach.** The subject's rows in the
  subject-scoped tables are deleted in one transaction. Not reached: copies in
  Redis, log lines and exported spans that carry the id, ceremonies held in
  memory (an open pairing code is closed), backups, and SQLite's free pages
  and write-ahead log, which keep deleted rows until they are overwritten or
  checkpointed. Nor does it reach what home never held: a paired phone keeps
  its own copy of what it sent, and the owner's Drive keeps its copies. And it
  does not reach the Jobs mailbox. A file still live there stays live: the
  poller keeps nothing and archives nothing for a subject with no account
  (`mailbox::pass`'s `a_file_home_cannot_store_stays_live`), so it never
  reaches the age at which home would archive it, and stays until the owner
  removes it from the mailbox. For a file the mailbox had already let go, only
  its row remains (name, size, hash and dates, invariant 21): its bytes were
  home's, and went with the account.
- **Retention that runs only sometimes.** The waker sweeps `intervention_log`
  (90 days) and `activity_outcome` (365 days) on each pass, so those horizons
  hold only while `NUDGE_ENABLED` is on. `tabs` has a 30-day prune behind
  `POST /tabs/prune`, which nothing calls. Expired `auth_session` rows are
  deleted when someone next signs in, not on a timer. `engagement_*`,
  `sessions`, the shelf, the aph and Jobs records, the LeetType session log,
  `sharing_choice` and the mailbox copy have no horizon.

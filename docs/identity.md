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
| `subject::SubjectId` | The subject a request acts for. Decided in one place, the extractor in `subject.rs`, from the request's passkey session. | **Yes, the only one.** |
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

`SubjectId::from_request_parts` is where a request's session cookie becomes a
subject, and a request without a live session is refused with `401`. There is no
fallback subject. Passkey auth changed that one function body; no subject-scoped
handler, repository or route path changed with it.

`subject::Delegated` is the one other way in: an OAuth access token, accepted
only by the MCP endpoint and limited to what its grant approved (see "AI
services acting for a subject"). It lives in the same module, so both ways a
request gets a subject are in one file.

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
| Sessions | `auth_session` | SHA-256 of each session token, its subject, and when it expires. At most 32 per account. |

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

## AI services acting for a subject (OAuth)

An AI service (claude.ai, ChatGPT, Claude Code) can act for a subject through
the MCP endpoint, after the subject approves it once. The service's servers
cannot do a passkey ceremony, so the subject signs in in their own browser and
the server hands the service a token. This is OAuth 2.1 with PKCE, as the MCP
authorization spec (2026-07-28) asks for, with `file_host` as its own
authorization server. Nothing here is "sign in with" another company: the
token is minted here, names a subject id and nothing else, and the service
never learns who the subject is.

### Where each step happens

| Step | Who calls | Where |
|---|---|---|
| Metadata (`/.well-known/oauth-authorization-server`) | the AI service's servers | public, through the tunnel |
| Register (`POST /oauth/register`, RFC 7591) | the AI service's servers | public |
| Approve (`OAUTH_AUTHORIZE_URL`, the app's page) | the subject's browser | the app's origin: passkeys are bound to `WEBAUTHN_RP_ID`, so on `nixos.local` this step needs the home network, once per service |
| Token (`POST /oauth/token`) | the AI service's servers | public |
| Connected services (`GET`/`DELETE /oauth/grants`) | the app, signed in | the app's origin |

The approval page is the app's, not this server's: it already holds the
passkey sign-in. It sends the authorization request's parameters to
`POST /oauth/authorize/requests`, which checks them against the registered
client and holds them for ten minutes in memory, and shows the subject the
service's name, the host it will redirect to and what it asks for. Approving is
`POST /oauth/authorize/requests/:id/approve`, a subject-scoped request with the
usual session, origin check and hold against deletion. It answers with the
redirect, carrying a one-time code, the request's `state` and `iss` (RFC 9207).
Declining answers the redirect with `access_denied` and needs no session.

### What a token can do

- **Only what was approved.** A grant names its permissions (`lessons:read`,
  `progress:read`, `shelf`), and every token carries the grant's.
- **Only at the MCP endpoint.** A token is accepted by one extractor,
  `subject::Delegated`, and every route that takes `SubjectId` keeps reading
  the session cookie and nothing else. So no token can sign out, add a passkey
  or delete the account, whatever its permissions say.
- **Only for its audience.** A grant records the resource it was issued for
  (`OAUTH_RESOURCE`, the MCP endpoint's URL), and a token whose grant names
  another one is refused.

### What is stored

| Stored | Where | What it is |
|---|---|---|
| A registered client | `oauth_client` | a random client id, the name the service gave, and its exact redirect URIs. About the service, not about anyone; at most `MAX_CLIENTS`. |
| A grant | `oauth_grant` | the subject, the client, the permissions and resource approved, the current and previous refresh-token hashes, and when the refresh token expires. At most `MAX_GRANTS_PER_SUBJECT`. |
| An access token | `oauth_access_token` | its SHA-256, its grant and subject, its permissions, and its expiry. At most one live per grant. |

A code and a pending request are never stored: they live in memory for a
minute and ten minutes, are taken once, and a restart costs only the approvals
in flight.

Access tokens last an hour. A refresh token lasts `AUTH_SESSION_DAYS` from its
last use and is replaced on every use. Presenting the one it replaced ends the
whole grant, since only a copy could still hold it.

Deleting a grant (Settings → Connected AI services) deletes its tokens. A grant
and its tokens are subject-scoped and leave with the account, and a grant tells
the server something it did not know before: which AI service a subject uses.
"What is still exposed" says so.

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
   session token's SHA-256, never the token, and the same for OAuth access
   and refresh tokens; an authorization code is never stored at all. No session token, credential id
   or user handle reaches a log line or span field, at any level the
   production subscriber lets through.
   *Enforced by* the `auth_session.token_hash` column (invariant 10's column
   test) and `handlers::auth`'s `no_secret_reaches_a_log_line`, which captures
   every field of a sign-up, a sign-in and a refused sign-in through
   `auth::loggable`. A companion test proves that filter is needed.

9. **An account takes all its rows with it.** Deleting an account deletes the
   subject's rows from every subject-scoped table, including a row a request
   that was already running when the deletion began writes.
   *Enforced by* `subject::SUBJECT_SCOPED_TABLES` being both the list
   `DELETE /auth/account` walks and the list invariant 1's schema test holds
   every table with a `subject_id` to, and by
   `deleting_an_account_removes_its_rows_from_every_subject_scoped_table_and_nobody_elses`.
   In-flight writes are covered by one rule: **every writer of subject-scoped
   rows holds `auth::DeletionLock` across its check that the subject still
   exists and its write.** A deletion holds it exclusively for its one
   transaction. There are three such writers today:
   - requests, through `SubjectId`, which takes the lock before looking up
     the session and holds it for the whole request
     (`a_write_already_in_flight_does_not_outlive_the_account_it_was_for`);
   - the OAuth token endpoint, which has no session: for a code and a
     refresh alike it holds the lock across its writes, and checks in the
     same transaction that the subject still has an account (a code) or that
     the grant still exists (a refresh), so neither writes for a deleted
     account (`a_code_exchanged_after_its_account_is_deleted_writes_nothing`);
   - the nudge waker, through `waker::unless_deleted`, which takes the lock
     around one re-check of the subject's `engagement_gate` row and one write
     (`a_write_for_a_deleted_subject_is_skipped_and_waits_out_a_deletion`).
     It wraps the writes that check nothing a deletion removes: the
     provisioned `sessions` draft, and a charge save. It never holds the lock
     across a push delivery, so a deletion waits for writes, not for a pass.
     The claim and its log entry need no wrapper: they share a transaction
     with an `UPDATE engagement_gate … WHERE subject_id = ?`.

   A new writer of subject-scoped rows outside a request must do the same,
   or its rows can outlive a deleted account. Every hold is short, so a
   deletion stays well inside the HTTP timeout.

10. **The auth tables keep no timeline.** `account`, `passkey` and
    `auth_session` hold exactly the columns listed under "What an account is",
    and `oauth_client`, `oauth_grant` and `oauth_access_token` those under
    "AI services acting for a subject": no creation, last-seen or last-used
    time. An expiry is the one instant stored.
    *Enforced by* `file_host::privacy`'s
    `the_auth_tables_hold_exactly_their_listed_columns`. A new column there
    fails it until someone decides what it tells the server and says why.

11. **The learner shelf holds content only** (#387). `learner_shelf` is the
    one subject-scoped table that stores something the learner wrote rather
    than something the server observed: items they generated and chose to
    keep (`PUT /shelf/:activity/:key`, docs/study-nudge.md, "Learner shelf").
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

Whether separately harmless fields combine into a fingerprint, and whether
timing correlates requests, cannot be linted. They belong to review. Raise them
the way `CLAUDE.md`'s "Drift is loud" asks.

---

## What is still exposed

- **Push endpoints.** A `push_subscriptions.endpoint` is a URL that is stable
  per browser, and the push service that issued it (Google, Mozilla, Apple) can
  tie it to a browser install. Payloads are encrypted end to end (RFC 8291), but
  the push service sees delivery timing.
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
- **Session expiry moves when the app is used.** A session renewed at
  `GET /auth/session` gets a new expiry of now plus 30 days, so the stored
  expiry says, to the day, when this browser last opened the app past its
  session's halfway point. Nothing older is kept.
- **Which AI services a subject connected.** A grant ties a subject id to a
  registered client such as claude.ai, with what was approved. Whoever reads
  the database learns which service a subject uses; deleting the grant or the
  account removes it. What the MCP endpoint's tools return goes into that
  service's context, by the subject's choice, and is then the service's to
  keep.
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
  memory, backups, and SQLite's free pages and write-ahead log, which keep
  deleted rows until they are overwritten or checkpointed.
- **Retention that runs only sometimes.** The waker sweeps `intervention_log`
  (90 days) and `activity_outcome` (365 days) on each pass, so those horizons
  hold only while `NUDGE_ENABLED` is on. `tabs` has a 30-day prune behind
  `POST /tabs/prune`, which nothing calls. Expired `auth_session` rows are
  deleted when someone next signs in, not on a timer. `engagement_*`,
  `sessions` and the shelf have no horizon.

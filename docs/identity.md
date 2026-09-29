# Identity and privacy

> The server can tell that the same subject came back. It can never tell who
> that subject is.

That sentence is the design goal. The client holds the secret: a passkey,
whose private key never leaves the person's authenticator. The server holds only
what it needs to recognise a returning subject and to keep one client from
starving another. Nothing it stores or logs should be able to single a person
out.

This document says who owns identity in `file_host`, lists the invariants that
make the sentence true, names where each one is enforced, and ends with what is
still exposed — stated rather than assumed.

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

---

## Passkey auth

A passkey is the only way in. There is no email, password, magic link, social
login or recovery code, so there is no contact detail or shared secret to store,
leak or recover with. Losing every copy of every passkey loses the account; that
is the cost of holding nothing that could recover it.

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
  deletion first waits for every request already acting for a subject, and
  any running waker pass, to finish. It holds new ones off until it commits
  (`AuthContext`'s deletion lock, held by `SubjectId` for the whole request
  and by `waker::pass` for the whole pass). So a write resolved
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

1. **Nothing at rest can single a person out.** No column is named for an
   address, a contact detail or a fingerprint, and every table is classified, in
   writing, as subject-scoped or not. A new table fails the test until someone
   decides which it is and says why.
   *Enforced by* `file_host::privacy`'s schema test, against the schema the
   migrations actually produce.

2. **The peer's address never leaves `net.rs`, and never reaches a log line.**
   *Enforced by* `clippy.toml`'s `disallowed-types` (`axum::extract::ConnectInfo`),
   through `lint.yml`'s clippy ratchet: `net::Peer` is the one `#[allow]`. Also by
   `file_host::privacy`'s capturing-layer tests. They record every field of every
   event and span on the rate limiter's rejection path and on WebSocket
   admission, the two paths that logged an address before #372, and on
   `ConnectionGuard`'s permit accounting, and fail if an address, or the
   process-stable admission key, appears anywhere in them.

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

6. **Every `#[instrument]` names what it skips.** Without `skip`/`skip_all`,
   tracing records every argument as a span field.
   *Enforced by* `scripts/check_instrument_skip.py`.

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
   session token's SHA-256, never the token. No session token, credential id
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
   rows holds `AuthContext`'s deletion lock while it runs**. There are two
   such writers today:
   - requests, through `SubjectId`, which holds the lock for the whole
     request (`a_write_already_in_flight_does_not_outlive_the_account_it_was_for`);
   - the nudge waker, through `waker::pass`, which holds it for a whole pass
     (`a_pass_waits_out_an_account_deletion`). A pass writes for subjects it
     read earlier, and `provision_if_absent` checks nothing about the
     account.

   A deletion takes the lock exclusively. A new background task that writes
   subject-scoped rows must take the lock too (`hold_against_deletion`), or
   its rows can outlive a deleted account.

10. **The auth tables keep no timeline.** `account`, `passkey` and
    `auth_session` hold exactly the columns listed under "What an account is":
    no creation, last-seen or last-used time. A session's expiry is the one
    instant stored.
    *Enforced by* `file_host::privacy`'s
    `the_auth_tables_hold_exactly_their_listed_columns`. A new column there
    fails it until someone decides what it tells the server and says why.

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
  but that does not make it harmless.
- **The code-delivery trust gap.** The server ships the client's JavaScript, so
  a dishonest deployment could ship code that reads anything JavaScript can
  reach. A passkey's private key stays in the authenticator even then. Keys kept
  in IndexedDB, and WebAuthn PRF outputs, do not.
- **Session expiry moves when the app is used.** A session renewed at
  `GET /auth/session` gets a new expiry of now plus 30 days, so the stored
  expiry says, to the day, when this browser last opened the app past its
  session's halfway point. Nothing older is kept.
- **Passkey sync providers.** A synced passkey lives in the person's Apple,
  Google or password-manager account. That provider knows the person holds a
  passkey for this site, which is the provider's knowledge, not this server's.
- **Anyone can make an account.** An open server can be filled with empty
  accounts up to the daily cap. They hold nothing and single nobody out.
- **HTTP tracing.** `TraceLayer::new_for_http()` uses tower-http's defaults,
  which record method, URI and version but not headers. A query string is part of
  the URI, so identifying data must never travel in one.

# Identity and privacy

> The server can tell that the same subject came back. It can never tell who
> that subject is.

That sentence is the design goal. The client holds the secret (a passkey, once
auth lands). The server holds only what it needs to recognise a returning
subject and to keep one client from starving another. Nothing it stores or
logs should be able to single a person out.

This document says who owns identity in `file_host`, lists the invariants that
make the sentence true, names where each one is enforced, and ends with what is
still exposed — stated rather than assumed.

---

## Who owns "who is calling"

There are four handles on a request. Exactly one of them is an identity.

| Handle | What it is | Identity? |
|---|---|---|
| `subject::SubjectId` | The subject a request acts for. Decided in one place, the extractor in `subject.rs`. | **Yes, the only one.** |
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

### When auth lands

Passkey auth changes the body of `SubjectId::from_request_parts` and nothing
downstream. Two conventions apply when it does:

- **Auth is its own context.** It arrives as an `AuthContext` whose extractor is
  bounded on `AuthContext: FromRef<S>` only, and its routes' builder asks for
  nothing else. It does not ask for the whole `AppState`. `AppState::build`
  connects to NATS, so a handler that takes the whole state cannot be tested
  through the router. `handlers/subjects.rs` and `handlers/outcomes.rs` each
  split their bodies into pool-only functions to work around exactly that, and
  auth should not inherit the problem. Existing handlers are not migrated.
- **Secrets are wrapped when they exist.** Credential IDs, session tokens and
  recovery material get a `Redacted<T>` whose `Debug` prints `[redacted]`. No
  such type exists yet, because nothing holds a secret yet.

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

7. **When auth lands:** WebAuthn `attestation: "none"`, since anything else
   reveals the authenticator model. The WebAuthn `user.id` is random bytes with
   no personal info in it. No email or other contact detail is required to hold
   an account.
   *Enforced by* nothing yet; this lands with auth.

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
- **HTTP tracing.** `TraceLayer::new_for_http()` uses tower-http's defaults,
  which record method, URI and version but not headers. A query string is part of
  the URI, so identifying data must never travel in one.

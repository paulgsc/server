//! `/api/v1/rust-rounds` and `/api/v1/rust-weeks` — Rust Rounds' rounds and
//! weeks on home, and what the rust-runner recorded for each of their
//! programs (paulgsc/some-ui#2234, RR3; #2687, RR12).
//!
//! A week is the routine's: it arrives through the mailbox (`mailbox::pass`
//! keeps it in `brief` as `rust-week-<id>.json` and queues its programs), so
//! there is no `PUT` for one, and these routes only read it.
//!
//! ```text
//! GET /rust-weeks        → { weeks: [{ id, sha256, receivedAt }] }: the caller's, newest week first
//! GET /rust-weeks/:id    → { id, sha256, receivedAt, week, tickets: [{ ticket, sourceHash,
//!                            runs: [{ toolchain, ended, refusal, rustc: RustcRun }] }] },
//!                          with an ETag, `304` on `If-None-Match`, or a JSON 404
//! GET /rust-rounds       → { rounds: [{ id, contentHash, receivedAt }] }: the caller's, newest first
//! GET /rust-rounds/:id   → { id, contentHash, receivedAt, round, programs: [{ card, sourceHash,
//!                            runs: [{ toolchain, ended, refusal, rustc: RustcRun }] }] },
//!                          with an ETag, `304` on `If-None-Match`, or a JSON 404
//! PUT /rust-rounds/:id   → { change, id, contentHash }: the body is the round's JSON;
//!                          201 added, 200 the same bytes again, 409 another body, 422 why,
//!                          413 past `ROUND_CEILING` + 1 bytes, refused before it is read whole
//!
//! PUT /rust-weeks/:id/sent        → { change, sent }: the body is the phone's `WeekSent`, the
//!                                   week's draft until its deadline; 201 added, 200 replaced (a
//!                                   later `at`), unchanged (the same value) or superseded (an
//!                                   earlier `at`; `sent` is the draft that stands), 409 another
//!                                   body at the draft's `at`, 422 why (an `at` after the deadline,
//!                                   or home's clock past it: the week is final), 404 no week
//! PUT /rust-weeks/:id/pastes/:at  → { change, paste: { at, text, note } }: the body is a paste, `{ at, text, note }`;
//!                                   201 added, 200 the same again, 409 another, 422 why, 404
//! GET /rust-weeks/:id/record      → { week, opens, due, closed,
//!                                   sent: { at, receivedAt, draft } | null,
//!                                   tickets: [{ ticket, answered, take, recordedAt, opens }],
//!                                   unanswered, pastes }, or a JSON 404
//! PUT /rust-round-log/sessions/:id → { change, id, revision }: the body is one session of the
//!                                   card-era log; 201 added or extended, 200 the same again,
//!                                   409 one that drops or changes what home holds, 422 why,
//!                                   413 past `SESSION_CEILING` + 1 bytes
//! GET /rust-round-log/sessions    → { sessions: [{ id, startedAt, revision, sha256, writtenAt }] }
//! GET /rust-round-log/sessions/:id → the session's last revision as sent, with an ETag, or a 404
//! ```
//!
//! **A week's record, silence included** (paulgsc/some-ui#2689, RR14): what
//! the owner sent of a week is a draft until its deadline, each send
//! replacing the last, and what stands at the deadline is final (RR22,
//! paulgsc/some-ui#2832); a send after the deadline is refused, since a take
//! not sent by then doesn't count. The record is read from the week, its
//! draft or final, the pastes and the phone's engagement events
//! (each ticket's opens), and names every ticket no take answered. The
//! card-era log's sessions are kept as the phone sent them, growing by
//! revisions. See `handlers::rust_week_record`.
//!
//! **Trust model:** a paired phone's routes. Every handler takes a
//! `Paired`, so a request without a session is `401`, a write from an
//! untrusted origin `403`, and a browser's cookie session `403` too. A round
//! is its putter's: each route reads and writes only the caller's subject's
//! rounds, and another subject's id is a `404` (docs/identity.md invariant
//! 28). The runs are its programs', by their bytes, and nobody's: a run
//! carries no time, so one subject cannot learn when another first put a
//! program.
//!
//! **Append-only, and kept until the account goes.** A `PUT` adds a round or
//! changes nothing; a round's id is never reused, so another body under an id
//! the subject holds is a `409`, never a revision; there is no `DELETE`, and
//! a round goes only with its account. **Nothing here compiles or
//! runs a program** (EX1): a `PUT` queues the round's programs and answers,
//! and `rust_rounds`' background pass asks the runner. See
//! `handlers::rust_rounds` and `rust_round_repo`.

use crate::auth::AuthContext;
use crate::handlers::rust_rounds as handlers;
use crate::handlers::rust_week_record as record;
use crate::routes::cors::allowlisted_cors_with_credentials;
use crate::routes::table::{Module, RouteTable};
use crate::Config;
use axum::{
	extract::{DefaultBodyLimit, FromRef},
	handler::Handler,
	http::{
		header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE, ETAG, IF_NONE_MATCH},
		Method,
	},
};
use rust_round_repo::log::SESSION_CEILING;
use rust_round_repo::round::ROUND_CEILING;
use sqlx::SqlitePool;
use tower_http::cors::CorsLayer;

pub fn rust_rounds<S>() -> Module<S>
where
	S: Clone + Send + Sync + 'static,
	SqlitePool: FromRef<S>,
	AuthContext: FromRef<S>,
{
	let table = RouteTable::new()
		.get("/rust-rounds", handlers::list_rounds)
		.get("/rust-rounds/:id", handlers::get_round)
		// A round is at most `ROUND_CEILING` bytes. The limit is one past it,
		// so a body of exactly one byte too many still gets the 422 that
		// names the ceiling, and anything longer a 413 before it is buffered,
		// well inside the server-wide `RequestBodyLimitLayer`.
		.put("/rust-rounds/:id", handlers::put_round.layer(DefaultBodyLimit::max(ROUND_CEILING + 1)))
		.get("/rust-weeks", handlers::list_weeks)
		.get("/rust-weeks/:id", handlers::get_week)
		.put("/rust-weeks/:id/sent", record::put_sent)
		.put("/rust-weeks/:id/pastes/:at", record::put_paste)
		.get("/rust-weeks/:id/record", record::get_record)
		// One past the ceiling, as a round's: the 422 names it, longer is a 413.
		.put("/rust-round-log/sessions/:id", record::put_session.layer(DefaultBodyLimit::max(SESSION_CEILING + 1)))
		.get("/rust-round-log/sessions", record::list_sessions)
		.get("/rust-round-log/sessions/:id", record::get_session);

	Module::versioned("rust_rounds", table).with_cors(cors)
}

fn cors(config: &Config) -> CorsLayer {
	allowlisted_cors_with_credentials(
		config,
		vec![Method::GET, Method::PUT, Method::OPTIONS],
		vec![CONTENT_TYPE, ACCEPT, AUTHORIZATION, IF_NONE_MATCH],
	)
	.expose_headers([ETAG])
}

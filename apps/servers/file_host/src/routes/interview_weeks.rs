//! `/api/v1/interview-weeks` — the interview week on home
//! (paulgsc/some-ui#2935, IW4, under epic #2931).
//!
//! A week is the routine's: it arrives through the mailbox (`mailbox::pass`
//! keeps it in `brief` as `interview-week-<id>.json`, as it keeps a week of
//! Rust Rounds), so there is no `PUT` for one, and these routes only read it.
//! What the owner does with it is theirs: one send of the week's takes, a
//! draft until its deadline and final at it (RR22's rules, which
//! `rust_round_repo::draft` keeps for both kinds of week).
//!
//! ```text
//! GET /interview-weeks          → { weeks: [{ id, sha256, receivedAt }] }: the caller's, newest week first
//! GET /interview-weeks/:id      → { id, sha256, receivedAt, week }, with an ETag, `304` on
//!                                 `If-None-Match`, or a JSON 404
//! PUT /interview-weeks/:id/sent → { change, sent }: the body is the phone's send,
//!                                 `{ week, at, answers: [{ question, take, recordedAt }],
//!                                 reads: [{ card, take, recordedAt }], passed, passedReads }`,
//!                                 the week's draft until its deadline; 201 added, 200 replaced
//!                                 (a later `at`), unchanged (the same value) or superseded (an
//!                                 earlier `at`; `sent` is the draft that stands), 409 another
//!                                 body at the draft's `at`, 422 why (an `at` after the deadline,
//!                                 or home's clock past it: the week is final), 404 no week
//! GET /interview-weeks/:id/record → { week, opens, due, closed,
//!                                 sent: { at, receivedAt, draft } | null,
//!                                 questions: [{ question, answered, take, recordedAt }],
//!                                 cards: [{ card, read, take, recordedAt }],
//!                                 unanswered, unread }, or a JSON 404
//! ```
//!
//! **Trust model:** a paired phone's routes, as Rust Rounds' are. Every
//! handler takes a `Paired`, so a request without a session is `401`, a write
//! from an untrusted origin `403`, and a browser's cookie session `403` too.
//! Each route reads and writes only the caller's subject's weeks and sends;
//! another subject's week is a `404` (docs/identity.md invariant 28). A take
//! is named by its voice note's id, never carried here (invariant 23). See
//! `handlers::interview_weeks`.

use crate::auth::AuthContext;
use crate::handlers::interview_weeks as handlers;
use crate::routes::cors::allowlisted_cors_with_credentials;
use crate::routes::table::{Module, RouteTable};
use crate::Config;
use axum::{
	extract::FromRef,
	http::{
		header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE, ETAG, IF_NONE_MATCH},
		Method,
	},
};
use sqlx::SqlitePool;
use tower_http::cors::CorsLayer;

pub fn interview_weeks<S>() -> Module<S>
where
	S: Clone + Send + Sync + 'static,
	SqlitePool: FromRef<S>,
	AuthContext: FromRef<S>,
{
	let table = RouteTable::new()
		.get("/interview-weeks", handlers::list_weeks)
		.get("/interview-weeks/:id", handlers::get_week)
		.put("/interview-weeks/:id/sent", handlers::put_sent)
		.get("/interview-weeks/:id/record", handlers::get_record);

	Module::versioned("interview_weeks", table).with_cors(cors)
}

fn cors(config: &Config) -> CorsLayer {
	allowlisted_cors_with_credentials(
		config,
		vec![Method::GET, Method::PUT, Method::OPTIONS],
		vec![CONTENT_TYPE, ACCEPT, AUTHORIZATION, IF_NONE_MATCH],
	)
	.expose_headers([ETAG])
}

//! `/api/v1/rust-rounds` — Rust Rounds' rounds on home, and what the
//! rust-runner recorded for each of their programs (paulgsc/some-ui#2234,
//! RR3).
//!
//! ```text
//! GET /rust-rounds       → { rounds: [{ id, contentHash, receivedAt }] }: the caller's, newest first
//! GET /rust-rounds/:id   → { id, contentHash, receivedAt, round, programs: [{ card, sourceHash,
//!                            runs: [{ toolchain, ended, refusal, rustc: RustcRun }] }] },
//!                          with an ETag, `304` on `If-None-Match`, or a JSON 404
//! PUT /rust-rounds/:id   → { change, id, contentHash }: the body is the round's JSON;
//!                          201 added, 200 the same bytes again, 409 another body, 422 why,
//!                          413 past `ROUND_CEILING` + 1 bytes, refused before it is read whole
//! ```
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
		.put("/rust-rounds/:id", handlers::put_round.layer(DefaultBodyLimit::max(ROUND_CEILING + 1)));

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

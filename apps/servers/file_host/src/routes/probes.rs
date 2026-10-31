//! `/api/v1/probes` — home's probes, read and run from the operator's web
//! page (paulgsc/some-ui#2383, CK4; the page is paulgsc/some-ui#2388, CK8).
//!
//! ```text
//! GET  /probes                → { probes: [{ probe, enabled, everySeconds, boundSeconds, runs }] },
//!                               each probe's newest runs, newest first, bounded
//! POST /probes/transcription  → 202 { run, joined }: a run started now, or the one already open
//! ```
//!
//! A run also starts by itself daily, on the transcriber's own tick
//! (`transcribe::probe`); these routes only read what is kept and ask for one
//! sooner. A run's result comes later, once the transcriber has heard its
//! note: the page reads it back from `GET /probes`.
//!
//! **Trust model:** an operator only, as `routes::db::curriculum_operator`.
//! Every handler takes `auth::operator::Operator`: a passkey session (`401`
//! without one, the origin check on the `POST`) whose subject is the owner's
//! account or listed in `OPERATOR_SUBJECTS` (`403` otherwise). A probe is
//! about home, never a subject, so nothing here is per subject.

use crate::auth::AuthContext;
use crate::handlers::probes as handlers;
use crate::routes::cors::allowlisted_cors_with_credentials;
use crate::routes::table::{Module, RouteTable};
use crate::transcribe::probe::Probing;
use crate::Config;
use axum::{
	extract::FromRef,
	http::{
		header::{ACCEPT, CONTENT_TYPE},
		Method,
	},
};
use sqlx::SqlitePool;
use tower_http::cors::CorsLayer;

pub fn probes<S>() -> Module<S>
where
	S: Clone + Send + Sync + 'static,
	SqlitePool: FromRef<S>,
	AuthContext: FromRef<S>,
	Probing: FromRef<S>,
{
	let table = RouteTable::new().get("/probes", handlers::list).post("/probes/transcription", handlers::run_transcription);

	Module::versioned("probes", table).with_cors(cors)
}

fn cors(config: &Config) -> CorsLayer {
	allowlisted_cors_with_credentials(config, vec![Method::GET, Method::POST, Method::OPTIONS], vec![CONTENT_TYPE, ACCEPT])
}

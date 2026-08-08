//! `GET /ready` — see `handlers::readiness`. Unversioned and un-CORS'd like
//! `/health` and `/metrics`: load balancers and `depends_on: condition:
//! service_healthy` read this, not a browser.

use crate::handlers::readiness as routes;
use crate::AppState;
use axum::{extract::FromRef, routing::get, Router};

pub fn get_readiness<S>() -> Router<S>
where
	S: Clone + Send + Sync + 'static,
	AppState: FromRef<S>,
{
	Router::new().route("/ready", get(routes::readiness))
}

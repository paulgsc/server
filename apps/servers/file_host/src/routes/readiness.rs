//! `GET /ready` — see `handlers::readiness`. Unversioned and un-CORS'd like
//! `/health` and `/metrics`: load balancers and `depends_on: condition:
//! service_healthy` read this, not a browser.

use crate::handlers::readiness as routes;
use crate::routes::table::{Module, RouteTable};
use crate::AppState;
use axum::extract::FromRef;

pub fn get_readiness<S>() -> Module<S>
where
	S: Clone + Send + Sync + 'static,
	AppState: FromRef<S>,
{
	Module::unversioned("readiness", RouteTable::new().get("/ready", routes::readiness))
}

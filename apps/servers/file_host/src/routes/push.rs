use crate::handlers::push as handlers;
use crate::routes::cors::allowlisted_cors;
use crate::{AppState, Config};
use axum::{
	extract::FromRef,
	http::{
		header::{AUTHORIZATION, CONTENT_TYPE},
		Method,
	},
	routing::{delete, get, post},
	Router,
};

/// Paths are declared relative: `main.rs` nests this under `API_V1_BASE_PATH`.
///
/// CORS comes from `ALLOWED_ORIGINS` via `routes/cors.rs` rather than a
/// hardcoded literal. Three existing route modules hardcode
/// `http://nixos.local:6006` — Storybook's port, not the app's — and a
/// subscription posted from the real HTTPS study origin matches none of them.
/// That is precisely the failure this route cannot afford, since a service
/// worker only exists on a secure context in the first place.
pub fn push<S>(config: &Config) -> Router<S>
where
	S: Clone + Send + Sync + 'static,
	AppState: FromRef<S>,
{
	let cors = allowlisted_cors(config, vec![Method::GET, Method::POST, Method::DELETE, Method::OPTIONS], vec![CONTENT_TYPE, AUTHORIZATION]);

	Router::new()
		// GET    /push/vapid-key     → applicationServerKey + the topics on offer
		// POST   /push/subscriptions → the browser's shape plus what they agreed to
		// DELETE /push/subscriptions → withdrawing consent, idempotent
		.route("/push/vapid-key", get(handlers::vapid_key))
		.route("/push/subscriptions", post(handlers::subscribe))
		.route("/push/subscriptions", delete(handlers::unsubscribe))
		// POST /push/test → send now, without waiting for engagement to decay.
		// The only honest end-to-end check is a human watching their desktop.
		.route("/push/test", post(handlers::send_test))
		.layer(cors)
}

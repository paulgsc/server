use crate::handlers::db::hopium as routes;
use crate::routes::cors::allowlisted_cors_with_credentials;
use crate::routes::table::{Module, RouteTable};
use crate::{AppState, Config};
use axum::{
	extract::FromRef,
	http::{
		header::{AUTHORIZATION, CONTENT_TYPE},
		Method,
	},
};
use tower_http::cors::CorsLayer;

pub fn mood_events<S>() -> Module<S>
where
	S: Clone + Send + Sync + 'static,
	AppState: FromRef<S>,
{
	let table = RouteTable::new()
		// Single mood event operations
		.post("/mood_events", routes::create_mood_event)
		.get("/mood_events", routes::get_all_mood_events)
		.get("/mood_events/:id", routes::get_mood_event_by_id)
		.patch("/mood_events/:id", routes::update_mood_event)
		.delete("/mood_events/:id", routes::delete_mood_event)
		// Batch operations
		.post("/mood_events/batch", routes::batch_create_mood_events)
		.patch("/mood_events/batch", routes::batch_update_mood_events)
		.delete("/mood_events/batch", routes::batch_delete_mood_events)
		// Query operations
		.get("/mood_events/week/:week", routes::get_mood_events_by_week)
		.get("/mood_events/team/:team", routes::get_mood_events_by_team)
		.get("/mood_events/stats", routes::get_mood_stats);

	Module::versioned("mood_events", table).with_cors(cors)
}

// Was a hardcoded `http://nixos.local:6006` — Storybook's port, not the
// app's. Now the same `ALLOWED_ORIGINS` list every other browser-facing
// route uses, which is what lets the HTTPS study origin reach it.
fn cors(config: &Config) -> CorsLayer {
	allowlisted_cors_with_credentials(config, vec![Method::GET, Method::POST, Method::PATCH, Method::DELETE], vec![CONTENT_TYPE, AUTHORIZATION])
}

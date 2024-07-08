use crate::http::handlers::{create_tab, delete_tab, get_tabs, update_tabs};
use axum::routing::{delete, get, post, put};
use axum::{http::Method, Router};
use tower_http::cors::{Any, CorsLayer};

pub fn routes() -> Router {
	let cors = CorsLayer::new().allow_methods([Method::GET, Method::POST, Method::PUT, Method::DELETE]).allow_origin(Any);

	let router = Router::new()
		.route("/api/chrome/tabs/update", put(update_tabs::update_tab))
		.route("/api/chrome/tabs/delete", delete(delete_tab::delete_tab))
		.route("/api/chrome/tabs/all", get(get_tabs::get_all_tabs))
		.route("/api/chrome/tabs/post", post(create_tab::create_tab))
		.layer(cors);
	router
}

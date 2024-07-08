use crate::http::handlers::{create_tab, get_tabs};
use axum::routing::{delete, get, post, put};
use axum::{http::Method, Router};
use tower_http::cors::{Any, CorsLayer};

pub fn routes() -> Router {
	let cors = CorsLayer::new().allow_methods([Method::GET, Method::POST, Method::PUT, Method::DELETE]).allow_origin(Any);

	let router = Router::new()
		.route("/api/chrome/tabs/update", put(user_handler::update_user_put))
		.route("/api/chrome/tabs/delete", delete(user_handler::delete_user_delete))
		.route("/api/chrome/tabs/all", get(user_handler::all_user_get))
		.route("/api/chrome/tabs/post", post(post_handler::create_post_post))
		.layer(cors);
	router
}

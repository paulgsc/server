use crate::handlers::gdrive_images as routes;
use crate::AppState;
use axum::routing::get;
use axum::{extract::FromRef, http::Method, Router};
use tower_http::cors::{Any, CorsLayer};

pub fn get_gdrive_image<S>() -> Router<S>
where
	S: Clone + Send + Sync + 'static,
	AppState: FromRef<S>,
{
	let cors = CorsLayer::new().allow_methods([Method::GET]).allow_origin(Any);

	Router::new().route("/gdrive/image/:image_id", get(routes::serve_gdrive_image)).layer(cors)
}

use crate::handlers::{gdrive_fs, gdrive_images};
use crate::routes::cors::allowlisted_cors;
use crate::{AppState, Config};
use axum::routing::{get, put};
use axum::{
	extract::FromRef,
	http::{
		header::{AUTHORIZATION, CONTENT_TYPE},
		Method,
	},
	Router,
};
use tower_http::cors::{Any, CorsLayer};

/// Read-only gdrive-fs surface: image serving plus folder listing and JSON
/// seed-file reads. Deliberately `Any`-origin like the rest of this module —
/// these are all read endpoints, no different in risk from the pre-existing
/// image route.
pub fn get_gdrive_image<S>() -> Router<S>
where
	S: Clone + Send + Sync + 'static,
	AppState: FromRef<S>,
{
	let cors = CorsLayer::new().allow_methods([Method::GET]).allow_origin(Any);

	Router::new()
		.route("/gdrive/image/:image_id", get(gdrive_images::serve_gdrive_image))
		.route("/gdrive/list", get(gdrive_fs::list_gdrive_root))
		.route("/gdrive/list/:folder_id", get(gdrive_fs::list_gdrive_folder))
		.route("/gdrive/json/:file_id", get(gdrive_fs::read_gdrive_json))
		.layer(cors)
}

/// Write surface for the gdrive-fs seed-data flow. Unlike the read routes
/// above, this mutates Drive state, so it goes through the same env-driven
/// CORS allowlist as the other state-adjacent browser-facing routes
/// (`sheets`, `audio_files`) instead of `Any`.
pub fn write_gdrive_fs<S>(config: &Config) -> Router<S>
where
	S: Clone + Send + Sync + 'static,
	AppState: FromRef<S>,
{
	let cors = allowlisted_cors(config, vec![Method::PUT], vec![CONTENT_TYPE, AUTHORIZATION]);

	Router::new().route("/gdrive/write/:folder_id/:name", put(gdrive_fs::upsert_gdrive_file)).layer(cors)
}

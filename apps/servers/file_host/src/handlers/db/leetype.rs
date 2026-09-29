//! Handlers for `routes::db::leetype` (#327, LTY-SRV3). See that module's doc
//! comment for the surface; this is the query-and-respond half.

use crate::handlers::db::activities::{etag_value, if_none_match_hits, not_modified};
use crate::FileHostError;
use axum::{
	body::Body,
	extract::{Path, State},
	http::{
		header::{CONTENT_TYPE, ETAG},
		HeaderMap, HeaderValue,
	},
	response::{IntoResponse, Response},
	Json,
};
use leetype_round_repo::{content_hash, RoundEntry, RoundRepository, Witness, MANIFEST_CEILING};
use serde::Serialize;
use sqlx::SqlitePool;
use tracing::instrument;

/// One listed round as the manifest describes it: enough to pick, cache and
/// sample rounds without fetching a body. `witnesses` is the round's `μ`, one
/// per member of its option set, in option order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestRound {
	pub id: String,
	pub version: i64,
	pub published_at: String,
	pub content_hash: String,
	pub witnesses: Vec<Witness>,
}

impl From<RoundEntry> for ManifestRound {
	fn from(entry: RoundEntry) -> Self {
		Self {
			id: entry.id,
			version: entry.version,
			published_at: entry.published_at,
			content_hash: entry.content_hash,
			witnesses: entry.witnesses,
		}
	}
}

/// `GET /leetype/rounds`
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RoundManifest {
	/// [`content_hash`] over the serialised `rounds` array, and the `ETag`.
	/// Opaque to the client; it changes exactly when anything listed does.
	pub version: String,
	pub rounds: Vec<ManifestRound>,
}

/// The listed rounds, by id, and the tag that names them.
///
/// The same shape of answer as the lesson manifest: one read of one past
/// [`MANIFEST_CEILING`], refused rather than truncated over it, and an empty
/// corpus is a valid, empty manifest.
pub(crate) async fn manifest(db: &SqlitePool) -> Result<RoundManifest, FileHostError> {
	let rows = RoundRepository::new(db.clone()).entries(MANIFEST_CEILING + 1).await?;
	#[allow(clippy::cast_possible_wrap)] // at most MANIFEST_CEILING + 1
	if rows.len() as i64 > MANIFEST_CEILING {
		return Err(FileHostError::MaxRecordLimitExceeded);
	}
	let rounds: Vec<ManifestRound> = rows.into_iter().map(ManifestRound::from).collect();
	// Disallowed for tracing; this is the hashed listing itself.
	#[allow(clippy::disallowed_methods)]
	let listing = serde_json::to_vec(&rounds)?;
	Ok(RoundManifest {
		version: content_hash(&listing),
		rounds,
	})
}

/// One round's stored bytes, verbatim, and their hash — or `NotFound`.
/// Retired rounds included: a learner may be part-way through one.
pub(crate) async fn round(db: &SqlitePool, id: &str) -> Result<(String, String), FileHostError> {
	RoundRepository::new(db.clone()).body(id).await?.ok_or(FileHostError::NotFound)
}

/// `GET /leetype/rounds`
///
/// # Errors
/// 400 for a corpus over the ceiling; 500 for a storage failure.
#[axum::debug_handler]
#[instrument(name = "leetype_rounds", skip_all, fields(otel.kind = "server"))]
pub async fn get_rounds(State(db): State<SqlitePool>, headers: HeaderMap) -> Result<Response, FileHostError> {
	let manifest = manifest(&db).await?;
	let etag = etag_value(&manifest.version)?;
	if if_none_match_hits(&headers, &etag) {
		return Ok(not_modified(etag));
	}
	let mut response = Json(&manifest).into_response();
	response.headers_mut().insert(ETAG, etag);
	Ok(response)
}

/// `GET /leetype/rounds/:id`
///
/// The round exactly as written — this server reads four fields of it and
/// never re-serialises it — with its content hash as the `ETag`. An unknown
/// id is a real `404` with a JSON error body.
///
/// # Errors
/// 404 for an unknown id; 500 for a storage failure.
#[axum::debug_handler]
#[instrument(name = "leetype_round", skip_all, fields(otel.kind = "server"))]
pub async fn get_round(State(db): State<SqlitePool>, headers: HeaderMap, Path(id): Path<String>) -> Result<Response, FileHostError> {
	let (hash, body) = round(&db, &id).await?;
	let etag = etag_value(&hash)?;
	if if_none_match_hits(&headers, &etag) {
		return Ok(not_modified(etag));
	}
	let mut response = Response::new(Body::from(body));
	response.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
	response.headers_mut().insert(ETAG, etag);
	Ok(response)
}

#[cfg(test)]
mod tests {
	//! The logic functions over a migrated pool, and the real routes through
	//! their router for what only a response shows: headers, `304`, `404`.

	use super::{manifest, round};
	use crate::FileHostError;
	use axum::{
		body::Body,
		http::{
			header::{CONTENT_TYPE, ETAG, IF_NONE_MATCH},
			Request, StatusCode,
		},
		Router,
	};
	use leetype_round_repo::{Change, RoundRepository, MANIFEST_CEILING};
	use serde_json::{json, Value};
	use sqlx::sqlite::SqlitePoolOptions;
	use sqlx::SqlitePool;
	use tower::ServiceExt;

	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");
	const T0: &str = "2026-09-29T00:00:00+00:00";

	async fn pool() -> SqlitePool {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();
		pool
	}

	/// A round body with deliberate, non-canonical whitespace, so "verbatim"
	/// is tested rather than assumed.
	fn body(id: &str, admissible: &str) -> String {
		let mut body = String::from("{ \"id\":  \"");
		body.push_str(id);
		body.push_str("\", \"algorithm\": {\"language\":\"rust\"}, \"graph\": {\"kind\": \"W\"},\n \"diffOptions\": [{\"member\": {\"propositionId\": \"");
		body.push_str(admissible);
		body.push_str("\", \"admissible\": true}}, {\"member\": {\"propositionId\": \"CW-P8\", \"admissible\": false}}] }\n");
		body
	}

	async fn put(pool: &SqlitePool, id: &str, admissible: &str, now: &str) -> Change {
		RoundRepository::upsert(&mut pool.acquire().await.unwrap(), id, body(id, admissible).as_bytes(), now, false)
			.await
			.unwrap()
	}

	fn app(pool: SqlitePool) -> Router {
		crate::routes::db::leetype::<SqlitePool>().into_table_router().with_state(pool)
	}

	async fn get(app: &Router, path: &str, if_none_match: Option<&str>) -> (StatusCode, Option<String>, Option<String>, String) {
		let mut request = Request::builder().uri(path);
		if let Some(tag) = if_none_match {
			request = request.header(IF_NONE_MATCH, tag);
		}
		let response = app.clone().oneshot(request.body(Body::empty()).unwrap()).await.unwrap();
		let status = response.status();
		let etag = response.headers().get(ETAG).map(|value| value.to_str().unwrap().to_owned());
		let content_type = response.headers().get(CONTENT_TYPE).map(|value| value.to_str().unwrap().to_owned());
		let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
		(status, etag, content_type, String::from_utf8(bytes.to_vec()).unwrap())
	}

	/// An empty corpus is a valid, empty manifest at 200.
	#[tokio::test]
	async fn an_empty_corpus_is_a_valid_empty_manifest() {
		let json = serde_json::to_value(manifest(&pool().await).await.unwrap()).unwrap();
		assert!(json["version"].is_string() && json["rounds"] == json!([]), "{json}");
	}

	/// The manifest's wire shape: listed rounds by id, camelCase, each with
	/// its witnesses in option order, and a version that moves exactly when a
	/// listing does.
	#[tokio::test]
	async fn the_manifest_lists_rounds_with_their_witnesses() {
		let pool = pool().await;
		put(&pool, "b", "CW-P7", T0).await;
		put(&pool, "a", "CW-P6", T0).await;
		let listed = manifest(&pool).await.unwrap();
		let hash = |id: &str| leetype_round_repo::content_hash(body(id, if id == "a" { "CW-P6" } else { "CW-P7" }).as_bytes());
		assert_eq!(
			serde_json::to_value(&listed).unwrap(),
			json!({ "version": listed.version, "rounds": [
				{ "id": "a", "version": 1, "publishedAt": T0, "contentHash": hash("a"),
					"witnesses": [{ "propositionId": "CW-P6", "admissible": true }, { "propositionId": "CW-P8", "admissible": false }] },
				{ "id": "b", "version": 1, "publishedAt": T0, "contentHash": hash("b"),
					"witnesses": [{ "propositionId": "CW-P7", "admissible": true }, { "propositionId": "CW-P8", "admissible": false }] }
			]})
		);
		#[allow(clippy::disallowed_methods)] // the hashed listing, not a tracing argument
		let serialized = serde_json::to_vec(&listed.rounds).unwrap();
		assert_eq!(listed.version, leetype_round_repo::content_hash(&serialized), "the version is the hash of the listing");
		assert_eq!(manifest(&pool).await.unwrap().version, listed.version, "stable while nothing changes");

		put(&pool, "a", "CW-P5", "2026-09-30T00:00:00+00:00").await;
		assert_ne!(manifest(&pool).await.unwrap().version, listed.version, "an edit changes it");
	}

	/// A retired round leaves the manifest and is still served by id.
	#[tokio::test]
	async fn a_retired_round_is_unlisted_but_still_served() {
		let pool = pool().await;
		put(&pool, "a", "CW-P6", T0).await;
		RoundRepository::retire(&mut pool.acquire().await.unwrap(), "a", T0).await.unwrap();
		assert!(manifest(&pool).await.unwrap().rounds.is_empty());
		assert_eq!(round(&pool, "a").await.unwrap().1, body("a", "CW-P6"));
	}

	/// Over the ceiling is a refusal, not a short manifest.
	#[tokio::test]
	async fn a_corpus_over_the_ceiling_is_refused_not_truncated() {
		let pool = pool().await;
		let mut conn = pool.acquire().await.unwrap();
		for i in 0..=MANIFEST_CEILING {
			let mut id = String::from("round-");
			id.push_str(&i.to_string());
			RoundRepository::upsert(&mut conn, &id, body(&id, "CW-P6").as_bytes(), T0, false).await.unwrap();
		}
		drop(conn);
		assert!(matches!(manifest(&pool).await, Err(FileHostError::MaxRecordLimitExceeded)));
	}

	/// Through the router: a round is its stored bytes as `application/json`
	/// with its content hash as the `ETag`; the tag answers `304`; the
	/// manifest carries its version as its tag; an unknown id is a JSON 404.
	#[tokio::test]
	async fn the_routes_serve_verbatim_bodies_with_etags_304s_and_json_404s() {
		let pool = pool().await;
		put(&pool, "a", "CW-P6", T0).await;
		let app = app(pool.clone());

		let (status, etag, content_type, text) = get(&app, "/leetype/rounds/a", None).await;
		assert_eq!((status, content_type.as_deref()), (StatusCode::OK, Some("application/json")));
		assert_eq!(text, body("a", "CW-P6"), "verbatim, never re-serialised");
		let etag = etag.unwrap();
		assert_eq!(etag, String::from("\"") + &leetype_round_repo::content_hash(text.as_bytes()) + "\"");
		let (status, again, _, text) = get(&app, "/leetype/rounds/a", Some(&etag)).await;
		assert_eq!((status, again.as_deref(), text.as_str()), (StatusCode::NOT_MODIFIED, Some(etag.as_str()), ""));

		let (status, manifest_etag, _, text) = get(&app, "/leetype/rounds", None).await;
		assert_eq!(status, StatusCode::OK);
		let listed: Value = serde_json::from_str(&text).unwrap();
		assert_eq!(manifest_etag.as_deref(), Some((String::from("\"") + listed["version"].as_str().unwrap() + "\"").as_str()));
		assert_eq!(get(&app, "/leetype/rounds", manifest_etag.as_deref()).await.0, StatusCode::NOT_MODIFIED);

		let (status, _, _, text) = get(&app, "/leetype/rounds/no-such-round", None).await;
		assert_eq!(status, StatusCode::NOT_FOUND);
		let error: Value = serde_json::from_str(&text).unwrap();
		assert_eq!(error["error"]["code"], "not_found");
		assert!(!text.contains("<html"), "an error shape, not a page");
	}
}

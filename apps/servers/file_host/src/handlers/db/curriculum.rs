//! Handlers for `routes::db::curriculum` (#276, CUR3). See that module's doc
//! comment for the surface; this is the query-and-respond half.

use crate::handlers::db::activities::{etag_value, if_none_match_hits, not_modified};
use crate::{AppState, FileHostError};
use axum::{
	body::Body,
	extract::{Path, Query, State},
	http::{
		header::{CONTENT_TYPE, ETAG},
		HeaderMap, HeaderValue,
	},
	response::{IntoResponse, Response},
	Json,
};
use curriculum_repo::{content_hash, CurriculumRepository, ManifestEntry, MANIFEST_CEILING};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tracing::instrument;

/// The manifest `@some-ui/topik` reads.
///
/// `TopikManifestSchema` (`paulgsc/some-ui`), and therefore `apps/www`'s
/// `manifestShapeSchema`: `{ version: string, topiks: TopikMetadata[] }`.
/// Transcribed, not designed — see #274's migration for the field mapping.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Manifest {
	/// The manifest's own content hash — see [`manifest`]. A string the
	/// client never interprets; it changes exactly when anything listed does.
	pub version: String,
	pub topiks: Vec<ManifestEntry>,
}

/// The activity a manifest lists when the request names none: the
/// conversation lessons `@some-ui/topik` has always read from the bare route.
///
/// Every client that predates `?activity=` reads the bare route and parses
/// what it lists as a conversation lesson, so the bare route keeps listing
/// exactly those, and another format (makjang's scene trees, `paulgsc/some-ui`
/// MKJ-S4) is only ever listed to a client that asks for it by name.
pub(crate) const DEFAULT_ACTIVITY: &str = "topik";

/// `?activity=<id>`: which activity's lessons a manifest lists.
#[derive(Debug, Deserialize)]
pub struct ManifestQuery {
	activity: Option<String>,
}

/// The manifest of one activity's lessons, and the tag that names it.
///
/// Both the `ETag` and `version` are [`content_hash`] over the serialised
/// `topiks` array — the same function #274 defines a lesson's hash with, so
/// "did the manifest change" and "did a lesson change" are one notion of
/// version. Over the listing rather than over `(key, version)` pairs, because
/// a rename changes the manifest a client caches without being new material.
///
/// An empty corpus is a valid, empty manifest — the client's own docs are
/// right that "nobody has published any lessons yet" is an honest answer,
/// not an error — and a corpus over [`MANIFEST_CEILING`] is refused rather
/// than silently truncated.
pub(crate) async fn manifest(db: &SqlitePool, activity_id: &str) -> Result<Manifest, FileHostError> {
	// One read of one past the ceiling, not a count and then a listing: two
	// reads can see two snapshots, and an import committing between them would
	// turn the refusal into a silently truncated manifest (a real
	// `chatgpt-codex-connector` finding on #366).
	let rows = CurriculumRepository::new(db.clone()).entries(activity_id, MANIFEST_CEILING + 1).await?;
	#[allow(clippy::cast_possible_wrap)] // at most MANIFEST_CEILING + 1
	if rows.len() as i64 > MANIFEST_CEILING {
		return Err(FileHostError::MaxRecordLimitExceeded);
	}
	let topiks: Vec<ManifestEntry> = rows.iter().map(curriculum_repo::CurriculumEntry::manifest_entry).collect();
	// Disallowed for tracing; this is the hashed listing itself.
	#[allow(clippy::disallowed_methods)]
	let listing = serde_json::to_vec(&topiks)?;
	Ok(Manifest {
		version: content_hash(&listing),
		topiks,
	})
}

/// `GET /curriculum/manifest[?activity=<id>]`
///
/// Lists [`DEFAULT_ACTIVITY`]'s lessons unless the query names another
/// activity. An activity with no listed lessons is an empty manifest, like an
/// empty corpus.
///
/// # Errors
/// 400 for a corpus over the ceiling; 500 for a storage failure.
#[axum::debug_handler]
#[instrument(name = "curriculum_manifest", skip_all, fields(otel.kind = "server"))]
pub async fn get_manifest(State(state): State<AppState>, headers: HeaderMap, Query(query): Query<ManifestQuery>) -> Result<Response, FileHostError> {
	let activity = query.activity.as_deref().unwrap_or(DEFAULT_ACTIVITY);
	let manifest = manifest(&state.core.shared_db, activity).await?;
	let etag = etag_value(&manifest.version)?;
	if if_none_match_hits(&headers, &etag) {
		return Ok(not_modified(etag));
	}
	let mut response = Json(&manifest).into_response();
	response.headers_mut().insert(ETAG, etag);
	Ok(response)
}

/// One lesson's stored bytes, verbatim, and its `ETag` — or `NotFound`.
///
/// `key` may carry the `.json` the client's file names have (`beginner.json`
/// for the lesson `beginner`). No lesson key ends in `.json`
/// (`curriculum_repo::is_plain_key`, enforced on every write), so the suffix
/// always means the file name and each request names exactly one lesson. The
/// exact key is still tried first, for a row written before that rule.
pub(crate) async fn lesson(db: &SqlitePool, key: &str) -> Result<(String, String), FileHostError> {
	let lessons = CurriculumRepository::new(db.clone());
	if let Some(found) = lessons.body(key).await? {
		return Ok(found);
	}
	match key.strip_suffix(".json") {
		Some(bare) => lessons.body(bare).await?.ok_or(FileHostError::NotFound),
		None => Err(FileHostError::NotFound),
	}
}

/// `GET /curriculum/:key`
///
/// The lesson file exactly as imported — this server never parses it
/// (`@some-ui/topik` owns its shape) — with `content_hash` as its `ETag`. An
/// unknown key is a real `404` with a JSON error body: the answer an nginx
/// `try_files` fallback structurally cannot give, and the reason this route
/// exists.
///
/// # Errors
/// 404 for an unknown key; 500 for a storage failure.
#[axum::debug_handler]
#[instrument(name = "curriculum_lesson", skip_all, fields(otel.kind = "server"))]
pub async fn get_lesson(State(state): State<AppState>, headers: HeaderMap, Path(key): Path<String>) -> Result<Response, FileHostError> {
	let (hash, body) = lesson(&state.core.shared_db, &key).await?;
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
	use super::{lesson, manifest, DEFAULT_ACTIVITY};
	use crate::FileHostError;
	use axum::http::StatusCode;
	use axum::response::IntoResponse;
	use curriculum_repo::{import_dir, CurriculumRepository, ManifestEntry, MANIFEST_CEILING};
	use sqlx::sqlite::SqlitePoolOptions;
	use sqlx::SqlitePool;

	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");

	async fn pool() -> SqlitePool {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();
		pool
	}

	fn entry(key: &str) -> ManifestEntry {
		ManifestEntry {
			key: key.to_owned(),
			display_name: key.to_owned(),
			description: "d".to_owned(),
			batch_count: 1,
			total_questions: 1,
			total_messages: 1,
			difficulty: None,
			tags: None,
		}
	}

	/// An empty corpus is a valid, empty manifest at 200 — not a 404.
	#[tokio::test]
	async fn an_empty_corpus_is_a_valid_empty_manifest() {
		let pool = pool().await;
		let empty = manifest(&pool, DEFAULT_ACTIVITY).await.unwrap();
		assert!(empty.topiks.is_empty());
		let json = serde_json::to_value(&empty).unwrap();
		assert!(json["version"].is_string() && json["topiks"].as_array().is_some_and(Vec::is_empty), "{json}");
	}

	/// The manifest is `{ version: string, topiks: TopikMetadata[] }`, its
	/// version changes when a listing does and only then, and a lesson comes
	/// back byte for byte with its content hash.
	#[tokio::test]
	async fn the_manifest_and_a_lesson_come_from_the_rows() {
		let pool = pool().await;
		let dir = tempfile::tempdir().unwrap();
		#[allow(clippy::disallowed_methods)] // a fixture file
		let manifest_json = serde_json::to_vec(&serde_json::json!({ "version": "x", "topiks": [
			{ "key": "beginner", "displayName": "Beginner", "description": "d", "batchCount": 2, "totalQuestions": 20, "totalMessages": 8, "difficulty": "beginner", "tags": ["a"] }
		]}))
		.unwrap();
		std::fs::write(dir.path().join("manifest.json"), manifest_json).unwrap();
		std::fs::write(dir.path().join("beginner.json"), b"{ \"batches\": [] }").unwrap();
		import_dir(&pool, dir.path(), "topik", "2026-09-24T00:00:00+00:00", false).await.unwrap();

		let listed = manifest(&pool, DEFAULT_ACTIVITY).await.unwrap();
		let json = serde_json::to_value(&listed).unwrap();
		assert_eq!(
			json["topiks"],
			serde_json::json!([{ "key": "beginner", "displayName": "Beginner", "description": "d", "batchCount": 2, "totalQuestions": 20, "totalMessages": 8, "difficulty": "beginner", "tags": ["a"] }]),
			"exactly TopikMetadata's fields, camelCased, and nothing of the server's own bookkeeping"
		);
		assert_eq!(manifest(&pool, DEFAULT_ACTIVITY).await.unwrap().version, listed.version, "stable while nothing changes");

		let (hash, body) = lesson(&pool, "beginner").await.unwrap();
		assert_eq!(body, "{ \"batches\": [] }", "verbatim, never re-serialised");
		assert_eq!(
			lesson(&pool, "beginner.json").await.unwrap(),
			(hash.clone(), body.clone()),
			"the client's `<key>.json` file name resolves to the same lesson"
		);
		assert!(matches!(lesson(&pool, "missing.json").await, Err(FileHostError::NotFound)));
		assert_eq!(hash, curriculum_repo::content_hash(body.as_bytes()));

		let mut renamed = entry("beginner");
		renamed.display_name = "Beginner, renamed".to_owned();
		CurriculumRepository::upsert(&mut pool.acquire().await.unwrap(), "topik", &renamed, body.as_bytes(), "2026-09-25T00:00:00+00:00", false)
			.await
			.unwrap();
		assert_ne!(
			manifest(&pool, DEFAULT_ACTIVITY).await.unwrap().version,
			listed.version,
			"a rename changes the manifest a client caches"
		);
	}

	/// A manifest lists one activity's lessons: the default one, unless the
	/// request names another. Another activity's lesson is never in the
	/// default manifest that every older client reads as conversation
	/// lessons, and is still served by key to the client that lists it.
	#[tokio::test]
	async fn a_manifest_lists_one_activity() {
		let pool = pool().await;
		let mut conn = pool.acquire().await.unwrap();
		CurriculumRepository::upsert(&mut conn, DEFAULT_ACTIVITY, &entry("week-40"), b"{ \"batches\": [] }", "2026-10-09T00:00:00+00:00", false)
			.await
			.unwrap();
		CurriculumRepository::upsert(&mut conn, "makjang", &entry("tree-40"), b"{ \"root\": {} }", "2026-10-09T00:00:00+00:00", false)
			.await
			.unwrap();
		drop(conn);

		let keys = |manifest: super::Manifest| manifest.topiks.into_iter().map(|entry| entry.key).collect::<Vec<_>>();
		assert_eq!(keys(manifest(&pool, DEFAULT_ACTIVITY).await.unwrap()), ["week-40"]);
		assert_eq!(keys(manifest(&pool, "makjang").await.unwrap()), ["tree-40"]);
		assert!(
			manifest(&pool, "no-such-activity").await.unwrap().topiks.is_empty(),
			"an activity with nothing listed is an empty manifest"
		);
		assert_eq!(lesson(&pool, "tree-40").await.unwrap().1, "{ \"root\": {} }", "served by key whatever its activity");
	}

	/// An unknown key is a 404 with a JSON error body — emphatically not HTML.
	#[tokio::test]
	async fn an_unknown_lesson_is_a_json_404() {
		let pool = pool().await;
		let err = lesson(&pool, "no-such-lesson").await.unwrap_err();
		assert!(matches!(err, FileHostError::NotFound));
		let response = err.into_response();
		assert_eq!(response.status(), StatusCode::NOT_FOUND);
		let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
		let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
		assert_eq!(body["error"]["code"], "not_found");
		assert!(!String::from_utf8_lossy(&bytes).contains("<html"), "an error shape, not a page");
	}

	/// Over the ceiling is a refusal, not a short manifest.
	#[tokio::test]
	async fn a_corpus_over_the_ceiling_is_refused_not_truncated() {
		let pool = pool().await;
		let mut conn = pool.acquire().await.unwrap();
		for i in 0..=MANIFEST_CEILING {
			let mut key = String::from("lesson-");
			key.push_str(&i.to_string());
			CurriculumRepository::upsert(&mut conn, "topik", &entry(&key), b"{}", "2026-09-24T00:00:00+00:00", false)
				.await
				.unwrap();
		}
		drop(conn);
		assert!(matches!(manifest(&pool, DEFAULT_ACTIVITY).await, Err(FileHostError::MaxRecordLimitExceeded)));
	}
}

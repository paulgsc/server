//! Handlers for `routes::db::curriculum_operator`. See that module's doc
//! comment for the surface; this is the validate-and-write half.

use crate::{AppState, FileHostError};
use axum::{
	extract::{Path, State},
	Json,
};
use chrono::Utc;
use curriculum_repo::{is_plain_key, Change, CurriculumEntry, CurriculumRepository, ManifestEntry, LESSON_BYTES_CEILING, MANIFEST_CEILING, OPERATOR_LISTING_CEILING};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tracing::instrument;

/// Keys the read routes' static paths shadow: `GET /curriculum/manifest` and
/// `/curriculum/manifest.json` win over `/curriculum/:key`, so a lesson with
/// either key could be written and never read back.
const SHADOWED_KEYS: [&str; 2] = ["manifest", "manifest.json"];

/// What the operator's tool sends to write one lesson.
///
/// `body` is the lesson file as text. The server stores it byte for byte and
/// hashes those bytes (`curriculum_repo::content_hash`), so the client decides
/// the exact serialisation once and every later read returns it unchanged.
/// `metadata` is the lesson's `TopikMetadata` entry, which the client derives
/// from the lesson itself (`intakeLesson` in `@some-ui/topik`); this server
/// neither derives nor checks it against `body`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LessonWrite {
	pub activity_id: String,
	pub metadata: ManifestEntry,
	pub body: String,
}

/// One lesson as the operator sees it: its manifest entry, plus everything
/// this server keeps about it, retired lessons included. Never the body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OperatorLesson {
	#[serde(flatten)]
	pub metadata: ManifestEntry,
	pub activity_id: String,
	pub published_at: String,
	pub version: i64,
	pub content_hash: String,
	/// `null` while the lesson is in the manifest.
	pub retired_at: Option<String>,
}

impl From<CurriculumEntry> for OperatorLesson {
	fn from(entry: CurriculumEntry) -> Self {
		Self {
			metadata: entry.manifest_entry(),
			activity_id: entry.activity_id,
			published_at: entry.published_at,
			version: entry.version,
			content_hash: entry.content_hash,
			retired_at: entry.retired_at,
		}
	}
}

/// `GET /curriculum/operator/lessons`
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OperatorListing {
	pub lessons: Vec<OperatorLesson>,
}

/// What a write did, and the lesson as it now stands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LessonWritten {
	/// `inserted`, `contentChanged`, `metadataChanged` or `unchanged` —
	/// `curriculum_repo::Change`. Only the first two are new material.
	pub change: &'static str,
	pub lesson: OperatorLesson,
}

const fn change_name(change: Change) -> &'static str {
	match change {
		Change::Inserted => "inserted",
		Change::ContentChanged => "contentChanged",
		Change::MetadataChanged => "metadataChanged",
		Change::Unchanged => "unchanged",
	}
}

/// Every lesson, listed and retired, by key.
///
/// Bounded like the manifest: one read of one past
/// [`OPERATOR_LISTING_CEILING`], refused rather than truncated over it.
pub(crate) async fn listing(db: &SqlitePool) -> Result<OperatorListing, FileHostError> {
	let rows = CurriculumRepository::new(db.clone()).all_entries(OPERATOR_LISTING_CEILING + 1).await?;
	#[allow(clippy::cast_possible_wrap)] // at most OPERATOR_LISTING_CEILING + 1
	if rows.len() as i64 > OPERATOR_LISTING_CEILING {
		return Err(FileHostError::MaxRecordLimitExceeded);
	}
	Ok(OperatorListing {
		lessons: rows.into_iter().map(OperatorLesson::from).collect(),
	})
}

/// Refuse a write whose key, metadata or body this server would store wrong.
///
/// Checks only what the server itself depends on: a key it can serve, one
/// key rather than two, an activity to announce the lesson under, and a body
/// that is JSON and within [`LESSON_BYTES_CEILING`]. What a lesson *is* stays
/// `@some-ui/topik`'s to check.
fn validate(key: &str, request: &LessonWrite) -> Result<(), FileHostError> {
	let mut errors: Vec<(&'static str, &'static str)> = Vec::new();
	if !is_plain_key(key) {
		errors.push(("key", "must be a lesson key, not a path or URL"));
	} else if SHADOWED_KEYS.contains(&key) {
		errors.push(("key", "is shadowed by the manifest route"));
	}
	if request.metadata.key != key {
		errors.push(("metadata.key", "must equal the key in the path"));
	}
	if request.activity_id.trim().is_empty() {
		errors.push(("activityId", "is required"));
	}
	if request.body.len() > LESSON_BYTES_CEILING {
		errors.push(("body", "is over the lesson size ceiling"));
	} else if serde_json::from_str::<serde::de::IgnoredAny>(&request.body).is_err() {
		errors.push(("body", "is not JSON"));
	}
	if errors.is_empty() {
		Ok(())
	} else {
		Err(FileHostError::unprocessable_entity(errors))
	}
}

/// Write one lesson, as `now`.
///
/// The same `CurriculumRepository::upsert` the importer uses, so content hash,
/// version and `published_at` mean one thing whichever way a lesson arrived.
/// Unlike the importer's first run, a write is never a baseline: a lesson the
/// operator writes here is one nobody has seen, and the waker's next detection
/// pass announces it (`PublicationRepository::detect_curriculum_publications`)
/// if it is listed. A retired lesson stays retired.
///
/// Adding a lesson past [`MANIFEST_CEILING`] listed is refused here, so the
/// manifest never starts refusing because of a write.
pub(crate) async fn write(db: &SqlitePool, key: &str, request: &LessonWrite, now: &str) -> Result<LessonWritten, FileHostError> {
	validate(key, request)?;
	// `BEGIN IMMEDIATE`, so the ceiling check and the insert it allows see one
	// snapshot: two concurrent writes cannot both take the last place.
	let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
	if CurriculumRepository::entry(&mut tx, key).await?.is_none() && CurriculumRepository::listed_count(&mut tx).await? >= MANIFEST_CEILING {
		return Err(FileHostError::MaxRecordLimitExceeded);
	}
	let change = CurriculumRepository::upsert(&mut tx, &request.activity_id, &request.metadata, request.body.as_bytes(), now, false).await?;
	let lesson = CurriculumRepository::entry(&mut tx, key).await?.ok_or(FileHostError::NotFound)?;
	tx.commit().await?;
	Ok(LessonWritten {
		change: change_name(change),
		lesson: lesson.into(),
	})
}

/// Take a lesson out of the manifest (`listed = false`) or put it back.
///
/// Idempotent both ways. Neither moves a version, so neither is a publication;
/// see `20260927000100_add_curriculum_retired_at.up.sql`. Putting a lesson
/// back is refused at [`MANIFEST_CEILING`], like adding one.
pub(crate) async fn set_listed(db: &SqlitePool, key: &str, listed: bool, now: &str) -> Result<OperatorLesson, FileHostError> {
	let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
	let current = CurriculumRepository::entry(&mut tx, key).await?.ok_or(FileHostError::NotFound)?;
	if listed {
		if current.retired_at.is_some() && CurriculumRepository::listed_count(&mut tx).await? >= MANIFEST_CEILING {
			return Err(FileHostError::MaxRecordLimitExceeded);
		}
		CurriculumRepository::restore(&mut tx, key).await?;
	} else {
		CurriculumRepository::retire(&mut tx, key, now).await?;
	}
	let lesson = CurriculumRepository::entry(&mut tx, key).await?.ok_or(FileHostError::NotFound)?;
	tx.commit().await?;
	Ok(lesson.into())
}

/// `GET /curriculum/operator/lessons`
///
/// # Errors
/// 400 for a table over the listing ceiling; 500 for a storage failure.
#[axum::debug_handler]
#[instrument(name = "curriculum_operator_listing", skip_all, fields(otel.kind = "server"))]
pub async fn get_lessons(State(state): State<AppState>) -> Result<Json<OperatorListing>, FileHostError> {
	listing(&state.core.shared_db).await.map(Json)
}

/// `PUT /curriculum/operator/lessons/:key`
///
/// # Errors
/// 422 for a body that fails validation, 400 for a new lesson past the
/// manifest ceiling, and 500 for a storage failure.
#[axum::debug_handler]
#[instrument(name = "curriculum_operator_write", skip_all, fields(otel.kind = "server"))]
pub async fn put_lesson(State(state): State<AppState>, Path(key): Path<String>, Json(request): Json<LessonWrite>) -> Result<Json<LessonWritten>, FileHostError> {
	write(&state.core.shared_db, &key, &request, &Utc::now().to_rfc3339()).await.map(Json)
}

/// `POST /curriculum/operator/lessons/:key/retire`
///
/// # Errors
/// 404 for an unknown key; 500 for a storage failure.
#[axum::debug_handler]
#[instrument(name = "curriculum_operator_retire", skip_all, fields(otel.kind = "server"))]
pub async fn retire_lesson(State(state): State<AppState>, Path(key): Path<String>) -> Result<Json<OperatorLesson>, FileHostError> {
	set_listed(&state.core.shared_db, &key, false, &Utc::now().to_rfc3339()).await.map(Json)
}

/// `POST /curriculum/operator/lessons/:key/restore`
///
/// # Errors
/// 404 for an unknown key, 400 past the manifest ceiling, and 500 for a
/// storage failure.
#[axum::debug_handler]
#[instrument(name = "curriculum_operator_restore", skip_all, fields(otel.kind = "server"))]
pub async fn restore_lesson(State(state): State<AppState>, Path(key): Path<String>) -> Result<Json<OperatorLesson>, FileHostError> {
	set_listed(&state.core.shared_db, &key, true, &Utc::now().to_rfc3339()).await.map(Json)
}

#[cfg(test)]
mod tests {
	use super::{listing, set_listed, write, LessonWrite};
	use crate::handlers::db::curriculum::{lesson, manifest};
	use crate::FileHostError;
	use curriculum_repo::{CurriculumRepository, ManifestEntry, LESSON_BYTES_CEILING, MANIFEST_CEILING};
	use sqlx::sqlite::SqlitePoolOptions;
	use sqlx::SqlitePool;

	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");
	const T0: &str = "2026-09-27T00:00:00+00:00";
	const T1: &str = "2026-09-28T00:00:00+00:00";

	async fn pool() -> SqlitePool {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();
		pool
	}

	fn request(key: &str, name: &str, body: &str) -> LessonWrite {
		LessonWrite {
			activity_id: "topik".to_owned(),
			metadata: ManifestEntry {
				key: key.to_owned(),
				display_name: name.to_owned(),
				description: "d".to_owned(),
				batch_count: 1,
				total_questions: 3,
				total_messages: 2,
				difficulty: None,
				tags: Some(vec!["relation:cause".to_owned()]),
			},
			body: body.to_owned(),
		}
	}

	fn field_errors(err: &FileHostError) -> Vec<String> {
		let FileHostError::UnprocessableEntity { errors } = err else {
			panic!("expected a 422, got {err:?}");
		};
		let mut fields: Vec<String> = errors.keys().map(ToString::to_string).collect();
		fields.sort();
		fields
	}

	async fn manifest_keys(pool: &SqlitePool) -> Vec<String> {
		manifest(pool).await.unwrap().topiks.into_iter().map(|entry| entry.key).collect()
	}

	/// A write is the importer's upsert: a new key is version 1, identical
	/// input writes nothing, a rename is metadata only, and changed bytes are a
	/// version bump with a new `published_at`. The body reads back verbatim.
	#[tokio::test]
	async fn a_write_reports_what_it_changed_and_the_body_reads_back_verbatim() {
		let pool = pool().await;
		let first = write(&pool, "week-39-a", &request("week-39-a", "A", "{ \"v\": 1 }"), T0).await.unwrap();
		assert_eq!(first.change, "inserted");
		assert_eq!(
			(first.lesson.version, first.lesson.published_at.as_str(), first.lesson.retired_at.as_deref()),
			(1, T0, None)
		);

		let again = write(&pool, "week-39-a", &request("week-39-a", "A", "{ \"v\": 1 }"), T1).await.unwrap();
		assert_eq!(again.change, "unchanged");
		assert_eq!(again.lesson.published_at, T0, "an identical write is not new");

		let renamed = write(&pool, "week-39-a", &request("week-39-a", "A, renamed", "{ \"v\": 1 }"), T1).await.unwrap();
		assert_eq!((renamed.change, renamed.lesson.version, renamed.lesson.published_at.as_str()), ("metadataChanged", 1, T0));

		let edited = write(&pool, "week-39-a", &request("week-39-a", "A, renamed", "{ \"v\": 2 }"), T1).await.unwrap();
		assert_eq!((edited.change, edited.lesson.version, edited.lesson.published_at.as_str()), ("contentChanged", 2, T1));

		let (hash, body) = lesson(&pool, "week-39-a").await.unwrap();
		assert_eq!(body, "{ \"v\": 2 }", "stored byte for byte");
		assert_eq!(hash, edited.lesson.content_hash);
		assert_eq!(manifest_keys(&pool).await, ["week-39-a"]);
	}

	/// Everything the server depends on is a 422 naming the field, and a
	/// refused write writes nothing.
	#[tokio::test]
	async fn a_write_the_server_would_store_wrong_is_refused_naming_the_field() {
		let pool = pool().await;
		let oversized = {
			let mut body = String::from("\"");
			body.push_str(&"x".repeat(LESSON_BYTES_CEILING));
			body.push('"');
			body
		};
		let mut no_activity = request("a", "A", "{}");
		no_activity.activity_id = " ".to_owned();
		let cases: [(&str, LessonWrite, &[&str]); 6] = [
			("a/b", request("a/b", "A", "{}"), &["key"]),
			("manifest", request("manifest", "M", "{}"), &["key"]),
			("a", request("b", "B", "{}"), &["metadata.key"]),
			("a", no_activity, &["activityId"]),
			("a", request("a", "A", "not json"), &["body"]),
			("a", request("a", "A", &oversized), &["body"]),
		];
		for (key, body, expected) in cases {
			let err = write(&pool, key, &body, T0).await.unwrap_err();
			assert_eq!(field_errors(&err), expected, "{key}");
		}
		assert!(listing(&pool).await.unwrap().lessons.is_empty(), "nothing was written");
	}

	/// Retiring takes a lesson out of the manifest without deleting it; the
	/// operator still sees it, a learner can still fetch it by key, and
	/// restoring puts it back. Both are idempotent, and neither moves a
	/// version.
	#[tokio::test]
	async fn retiring_unlists_without_deleting_and_restoring_lists_again() {
		let pool = pool().await;
		for key in ["a", "b"] {
			write(&pool, key, &request(key, key, "{}"), T0).await.unwrap();
		}

		let retired = set_listed(&pool, "a", false, T1).await.unwrap();
		assert_eq!(retired.retired_at.as_deref(), Some(T1));
		assert_eq!(manifest_keys(&pool).await, ["b"], "the manifest is the listed set");
		assert!(lesson(&pool, "a").await.is_ok(), "a retired lesson is still served by key");
		let again = set_listed(&pool, "a", false, "2026-10-01T00:00:00+00:00").await.unwrap();
		assert_eq!(again.retired_at.as_deref(), Some(T1), "retiring twice keeps when it was first retired");

		let lessons = listing(&pool).await.unwrap().lessons;
		assert_eq!(
			lessons.iter().map(|l| (l.metadata.key.as_str(), l.retired_at.is_some())).collect::<Vec<_>>(),
			[("a", true), ("b", false)]
		);

		let restored = set_listed(&pool, "a", true, T1).await.unwrap();
		assert_eq!((restored.retired_at, restored.version, restored.published_at.as_str()), (None, 1, T0));
		assert_eq!(manifest_keys(&pool).await, ["a", "b"]);
		assert!(set_listed(&pool, "a", true, T1).await.is_ok(), "restoring a listed lesson is a no-op");

		assert!(matches!(set_listed(&pool, "missing", false, T1).await, Err(FileHostError::NotFound)));
		assert!(matches!(set_listed(&pool, "missing", true, T1).await, Err(FileHostError::NotFound)));
	}

	/// A write retires nothing and restores nothing: an edit to a retired
	/// lesson leaves it retired.
	#[tokio::test]
	async fn writing_a_retired_lesson_leaves_it_retired() {
		let pool = pool().await;
		write(&pool, "a", &request("a", "A", "{}"), T0).await.unwrap();
		set_listed(&pool, "a", false, T0).await.unwrap();
		let edited = write(&pool, "a", &request("a", "A", "{\"v\":2}"), T1).await.unwrap();
		assert_eq!((edited.change, edited.lesson.retired_at.as_deref()), ("contentChanged", Some(T0)));
		assert!(manifest_keys(&pool).await.is_empty());
	}

	/// The manifest ceiling is enforced where a lesson becomes listed — a new
	/// write or a restore — so the manifest itself never starts refusing
	/// because of this route. Rewriting a listed lesson is always allowed.
	#[tokio::test]
	async fn listing_a_lesson_past_the_manifest_ceiling_is_refused() {
		let pool = pool().await;
		let mut conn = pool.acquire().await.unwrap();
		for i in 0..MANIFEST_CEILING {
			let mut key = String::from("lesson-");
			key.push_str(&i.to_string());
			CurriculumRepository::upsert(&mut conn, "topik", &request(&key, "x", "{}").metadata, b"{}", T0, false)
				.await
				.unwrap();
		}
		drop(conn);

		assert!(matches!(
			write(&pool, "one-more", &request("one-more", "x", "{}"), T0).await,
			Err(FileHostError::MaxRecordLimitExceeded)
		));
		assert!(
			write(&pool, "lesson-0", &request("lesson-0", "renamed", "{}"), T0).await.is_ok(),
			"an existing lesson can still be edited"
		);

		set_listed(&pool, "lesson-0", false, T0).await.unwrap();
		write(&pool, "one-more", &request("one-more", "x", "{}"), T0).await.unwrap();
		assert!(matches!(set_listed(&pool, "lesson-0", true, T0).await, Err(FileHostError::MaxRecordLimitExceeded)));
		assert!(manifest(&pool).await.is_ok(), "the manifest is at the ceiling, not over it");
	}

	/// The listing's wire shape: the manifest entry's camelCase fields, the
	/// server's bookkeeping, and `retiredAt` present as `null` while listed.
	#[tokio::test]
	async fn the_listing_is_manifest_fields_plus_bookkeeping() {
		let pool = pool().await;
		let written = write(&pool, "a", &request("a", "A", "{}"), T0).await.unwrap();
		let json = serde_json::to_value(listing(&pool).await.unwrap()).unwrap();
		assert_eq!(
			json,
			serde_json::json!({ "lessons": [{
				"key": "a", "displayName": "A", "description": "d", "batchCount": 1, "totalQuestions": 3, "totalMessages": 2,
				"tags": ["relation:cause"], "activityId": "topik", "publishedAt": T0, "version": 1,
				"contentHash": written.lesson.content_hash, "retiredAt": null
			}]})
		);
		let written = serde_json::to_value(write(&pool, "a", &request("a", "A", "{}"), T0).await.unwrap()).unwrap();
		assert_eq!(written["change"], "unchanged");
		assert_eq!(written["lesson"]["key"], "a");
	}
}

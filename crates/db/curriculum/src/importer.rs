//! The one-shot importer for an existing `topiks`-shaped corpus (#275, CUR2).
//!
//! **Import is not migration.** This runs as an operator command, offline —
//! not a startup hook, not a lazy backfill, not an endpoint — so nothing in
//! the request path, and nothing in `file_host`'s `main.rs`, ever waits on it.
//! `paulgsc/some-ui@apps/www/src/lib/tenant/sessions-migration.ts` is the
//! cautionary example: a one-time data move that runs where users are waiting
//! becomes a permanent startup barrier.
//!
//! **Idempotent, because it will be run more than once** — after adding a
//! file, after a failed partial run, on a second environment. Change is
//! decided by [`crate::content_hash`], never by mtime, name, or size; a
//! re-import of unchanged input writes nothing and moves no `published_at`.
//!
//! **Failures are per file.** A malformed manifest entry, a missing file, or a
//! file that is not JSON fails *that lesson*, loudly, and the rest proceed;
//! the report says which. A corpus that is 90% importable imports 90%.
//!
//! **Reads the filesystem, never a URL.** The client's own docs record that a
//! `public/topiks` served over nginx answers a missing file with `index.html`
//! at 200; reading from disk cannot be fooled that way.
//!
//! **The first import is a baseline, not news.** Into an empty `curriculum`
//! table, what is being imported is what the app has served all along; every
//! lesson it writes is recorded in `curriculum_publication` as a baseline row —
//! seen, so #277 never mistakes it for new, but not an epoch, so nobody falls
//! behind it. Later imports leave new and changed lessons for #277 to announce.
//! Nobody has to remember a flag.

use crate::model::ManifestEntry;
use crate::repository::{Change, CurriculumRepository};
use publication_repo::PublicationRepository;
use serde::Deserialize;
use sqlx::SqlitePool;
use std::path::Path;

/// `curriculum_publication.source` for lessons (#277).
pub const PUBLICATION_SOURCE: &str = "curriculum";

#[derive(Debug, Deserialize)]
struct ManifestFile {
	#[allow(dead_code)] // required by the shape; the table has no use for it
	version: String,
	topiks: Vec<serde_json::Value>,
}

/// What one run did, lesson by lesson.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ImportReport {
	pub inserted: Vec<String>,
	pub content_changed: Vec<String>,
	pub metadata_changed: Vec<String>,
	pub unchanged: Vec<String>,
	/// `(lesson key or manifest position, why)` for every lesson that failed.
	pub failed: Vec<(String, String)>,
	/// Whether this run was the corpus's first import, recorded as a baseline.
	pub baseline: bool,
}

impl ImportReport {
	/// How many lessons were (or, in a dry run, would be) written.
	#[must_use]
	pub const fn writes(&self) -> usize {
		self.inserted.len() + self.content_changed.len() + self.metadata_changed.len()
	}
}

/// Why a whole import could not run at all — as opposed to one lesson
/// failing, which is recorded in [`ImportReport::failed`].
#[derive(Debug)]
pub enum ImportError {
	/// `manifest.json` is missing or unreadable.
	Manifest(std::io::Error),
	/// `manifest.json` is not a `{ version, topiks: [...] }` manifest.
	ManifestShape(serde_json::Error),
	Storage(sqlx::Error),
}

impl std::fmt::Display for ImportError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::Manifest(err) => write!(f, "could not read manifest.json: {err}"),
			Self::ManifestShape(err) => write!(f, "manifest.json is not a {{ version, topiks }} manifest: {err}"),
			Self::Storage(err) => write!(f, "database error: {err}"),
		}
	}
}

impl std::error::Error for ImportError {}

impl From<sqlx::Error> for ImportError {
	fn from(err: sqlx::Error) -> Self {
		Self::Storage(err)
	}
}

/// Import every lesson `dir/manifest.json` lists, from `dir/<key>.json`, for
/// `activity_id`. With `dry_run`, reports what would change and writes
/// nothing — not even the baseline.
///
/// # Errors
/// Only for a manifest that cannot be read or is not a manifest at all, or a
/// storage failure. A single lesson failing is recorded in the report, not
/// returned.
pub async fn import_dir(pool: &SqlitePool, dir: &Path, activity_id: &str, now: &str, dry_run: bool) -> Result<ImportReport, ImportError> {
	let manifest_bytes = std::fs::read(dir.join("manifest.json")).map_err(ImportError::Manifest)?;
	let manifest: ManifestFile = serde_json::from_slice(&manifest_bytes).map_err(ImportError::ManifestShape)?;

	let lessons = CurriculumRepository::new(pool.clone());
	let mut report = ImportReport {
		baseline: lessons.count().await? == 0,
		..ImportReport::default()
	};

	for (position, raw) in manifest.topiks.into_iter().enumerate() {
		let entry: ManifestEntry = match serde_json::from_value(raw) {
			Ok(entry) => entry,
			Err(err) => {
				report.failed.push((position_label(position), err.to_string()));
				continue;
			}
		};
		match read_lesson(dir, &entry.key) {
			Ok(body) => match lessons.upsert(activity_id, &entry, &body, now, dry_run).await? {
				Change::Inserted => report.inserted.push(entry.key),
				Change::ContentChanged => report.content_changed.push(entry.key),
				Change::MetadataChanged => report.metadata_changed.push(entry.key),
				Change::Unchanged => report.unchanged.push(entry.key),
			},
			Err(reason) => report.failed.push((entry.key, reason)),
		}
	}

	if report.baseline && !dry_run {
		let publications = PublicationRepository::new(pool.clone());
		for key in &report.inserted {
			publications.record_baseline(PUBLICATION_SOURCE, key, 1, now).await?;
		}
	}

	Ok(report)
}

fn position_label(position: usize) -> String {
	let mut label = String::from("topiks[");
	label.push_str(&position.to_string());
	label.push(']');
	label
}

/// `dir/<key>.json`, refusing a key that is a path or URL rather than an
/// identifier — the client passes those through to fetch from elsewhere, and
/// an offline importer has no business following them.
fn read_lesson(dir: &Path, key: &str) -> Result<Vec<u8>, String> {
	if key.is_empty() || key.contains('/') || key.contains('\\') || key.starts_with("http") || key.starts_with('.') {
		return Err("not a plain lesson key; paths and URLs are not imported".to_owned());
	}
	let mut file = String::from(key);
	file.push_str(".json");
	let body = std::fs::read(dir.join(file)).map_err(|err| err.to_string())?;
	serde_json::from_slice::<serde_json::Value>(&body).map_err(|err| {
		let mut reason = String::from("not JSON: ");
		reason.push_str(&err.to_string());
		reason
	})?;
	Ok(body)
}

#[cfg(test)]
mod tests {
	use super::import_dir;
	use publication_repo::PublicationRepository;
	use sqlx::sqlite::SqlitePoolOptions;
	use sqlx::SqlitePool;
	use std::path::Path;

	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");

	async fn pool() -> SqlitePool {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();
		pool
	}

	fn entry(key: &str, name: &str) -> serde_json::Value {
		serde_json::json!({
			"key": key, "displayName": name, "description": "d",
			"batchCount": 1, "totalQuestions": 10, "totalMessages": 5, "difficulty": "beginner"
		})
	}

	#[allow(clippy::disallowed_methods)] // a test fixture file, not a tracing argument
	fn write_corpus(dir: &Path, entries: &[serde_json::Value]) {
		std::fs::write(
			dir.join("manifest.json"),
			serde_json::to_vec(&serde_json::json!({ "version": "1", "topiks": entries })).unwrap(),
		)
		.unwrap();
	}

	async fn published_at(pool: &SqlitePool, key: &str) -> (String, i64) {
		let row = sqlx::query!("SELECT published_at, version FROM curriculum WHERE key = ?", key)
			.fetch_one(pool)
			.await
			.unwrap();
		(row.published_at, row.version)
	}

	/// The whole contract, in order: a first import writes and baselines;
	/// a second over unchanged input writes nothing and moves nothing; a
	/// changed file is a version bump and only that file; a renamed lesson is
	/// written without looking new.
	#[tokio::test]
	async fn reimporting_is_idempotent_and_only_changed_bytes_are_new() {
		let pool = pool().await;
		let dir = tempfile::tempdir().unwrap();
		write_corpus(dir.path(), &[entry("beginner", "Beginner"), entry("intermediate", "Intermediate")]);
		std::fs::write(dir.path().join("beginner.json"), br#"{"batches":[1]}"#).unwrap();
		std::fs::write(dir.path().join("intermediate.json"), br#"{"batches":[2]}"#).unwrap();

		let epoch_before = PublicationRepository::new(pool.clone()).newest().await.unwrap();
		let first = import_dir(&pool, dir.path(), "topik", "2026-09-01T00:00:00+00:00", false).await.unwrap();
		assert_eq!(first.inserted.len(), 2);
		assert!(first.baseline, "an empty table's first import is the baseline");
		let baselined: i64 = sqlx::query_scalar!(r#"SELECT COUNT(*) AS "count!: i64" FROM curriculum_publication WHERE source = 'curriculum' AND baseline = 1"#)
			.fetch_one(&pool)
			.await
			.unwrap();
		assert_eq!(baselined, 2, "and is recorded as seen, so #277 never mistakes it for new");
		assert_eq!(
			PublicationRepository::new(pool.clone()).newest().await.unwrap(),
			epoch_before,
			"without moving the epoch, so nobody falls behind it"
		);

		let second = import_dir(&pool, dir.path(), "topik", "2026-09-02T00:00:00+00:00", false).await.unwrap();
		assert_eq!(second.writes(), 0, "unchanged input, zero writes: {second:?}");
		assert!(!second.baseline);
		assert_eq!(
			published_at(&pool, "beginner").await,
			("2026-09-01T00:00:00+00:00".to_owned(), 1),
			"published_at did not move"
		);

		std::fs::write(dir.path().join("beginner.json"), br#"{"batches":[1, 3]}"#).unwrap();
		write_corpus(dir.path(), &[entry("beginner", "Beginner"), entry("intermediate", "Intermediate, renamed")]);
		let third = import_dir(&pool, dir.path(), "topik", "2026-09-03T00:00:00+00:00", false).await.unwrap();
		assert_eq!(third.content_changed, ["beginner"]);
		assert_eq!(third.metadata_changed, ["intermediate"]);
		assert_eq!(
			published_at(&pool, "beginner").await,
			("2026-09-03T00:00:00+00:00".to_owned(), 2),
			"changed bytes: new version, new published_at"
		);
		assert_eq!(
			published_at(&pool, "intermediate").await,
			("2026-09-01T00:00:00+00:00".to_owned(), 1),
			"a rename is not new material"
		);
	}

	/// One bad lesson fails alone; the rest import; the report names it.
	#[tokio::test]
	async fn a_bad_lesson_fails_alone() {
		let pool = pool().await;
		let dir = tempfile::tempdir().unwrap();
		write_corpus(
			dir.path(),
			&[
				entry("good", "Good"),
				entry("missing", "Missing"),
				entry("broken", "Broken"),
				entry("../escape", "Escape"),
				serde_json::json!({ "key": "no-fields" }),
			],
		);
		std::fs::write(dir.path().join("good.json"), b"{}").unwrap();
		std::fs::write(dir.path().join("broken.json"), b"<!doctype html>").unwrap();

		let report = import_dir(&pool, dir.path(), "topik", "2026-09-01T00:00:00+00:00", false).await.unwrap();
		assert_eq!(report.inserted, ["good"]);
		let failed: Vec<&str> = report.failed.iter().map(|(what, _)| what.as_str()).collect();
		assert_eq!(failed, ["missing", "broken", "../escape", "topiks[4]"]);
	}

	/// `--dry-run` reports what would change and writes nothing at all.
	#[tokio::test]
	async fn a_dry_run_writes_nothing() {
		let pool = pool().await;
		let dir = tempfile::tempdir().unwrap();
		write_corpus(dir.path(), &[entry("beginner", "Beginner")]);
		std::fs::write(dir.path().join("beginner.json"), b"{}").unwrap();

		let report = import_dir(&pool, dir.path(), "topik", "2026-09-01T00:00:00+00:00", true).await.unwrap();
		assert_eq!(report.inserted, ["beginner"], "reported");
		let rows: i64 = sqlx::query_scalar!(r#"SELECT COUNT(*) AS "count!: i64" FROM curriculum"#).fetch_one(&pool).await.unwrap();
		assert_eq!(rows, 0, "but not written");
	}

	/// A directory with no manifest is an error for the whole run.
	#[tokio::test]
	async fn no_manifest_is_a_whole_run_failure() {
		let pool = pool().await;
		let dir = tempfile::tempdir().unwrap();
		assert!(import_dir(&pool, dir.path(), "topik", "now", false).await.is_err());
	}
}

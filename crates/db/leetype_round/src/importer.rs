//! The importer for a round corpus exported by `@some-ui/leetype` (#326,
//! LTY-SRV2).
//!
//! The same discipline as `curriculum_repo::importer`, and for the same
//! reasons: an **offline operator command**, never a startup hook or a request
//! path; **idempotent**, deciding change by [`crate::content_hash`] over each
//! file's exact bytes; **failures are per round**, so one bad file fails alone
//! and is named in the report; and it **reads the filesystem, never a URL**.
//!
//! The directory is the client's export (`packages/ui/leetype/corpus/rounds/`
//! in `paulgsc/some-ui`): `manifest.json`, `{ "rounds": ["<id>", ...] }`, and
//! one `<id>.json` per round, stored byte for byte as read.
//!
//! **No baseline, and no publication log.** Unlike lessons, rounds do not feed
//! `curriculum_publication` or the study nudge, so there is no first import to
//! tell apart from later ones and no all-or-nothing rule to protect one:
//! whatever imports, imports. Whether a new round should ever be announced as
//! new material is left for later, and would be its own change.
//!
//! It never deletes, retires or restores a round: a round missing from the
//! directory stays as it is, and a retired round stays retired through a
//! re-import.

use crate::model::is_plain_key;
use crate::repository::{Change, RoundRepository, WriteError, MANIFEST_CEILING};
use serde::Deserialize;
use sqlx::SqlitePool;
use std::collections::BTreeSet;
use std::path::Path;

#[derive(Debug, Deserialize)]
struct ManifestFile {
	rounds: Vec<serde_json::Value>,
}

/// What one run did, round by round.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ImportReport {
	pub inserted: Vec<String>,
	pub content_changed: Vec<String>,
	pub unchanged: Vec<String>,
	/// `(round id or manifest position, why)` for every round that failed.
	pub failed: Vec<(String, String)>,
}

impl ImportReport {
	/// How many rounds were (or, in a dry run, would be) written.
	#[must_use]
	pub const fn writes(&self) -> usize {
		self.inserted.len() + self.content_changed.len()
	}
}

/// Why a whole import could not run at all — as opposed to one round failing,
/// which is recorded in [`ImportReport::failed`].
#[derive(Debug)]
pub enum ImportError {
	/// `manifest.json` is missing or unreadable.
	Manifest(std::io::Error),
	/// `manifest.json` is not a `{ rounds: [...] }` manifest.
	ManifestShape(serde_json::Error),
	Storage(sqlx::Error),
}

impl std::fmt::Display for ImportError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::Manifest(err) => write!(f, "could not read manifest.json: {err}"),
			Self::ManifestShape(err) => write!(f, "manifest.json is not a {{ rounds }} manifest: {err}"),
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

/// Import every round `dir/manifest.json` lists, from `dir/<id>.json`. With
/// `dry_run`, reports what would change and writes nothing.
///
/// # Errors
/// Only for a manifest that cannot be read or is not a manifest, or a storage
/// failure. A single round failing is recorded in the report, not returned.
pub async fn import_dir(pool: &SqlitePool, dir: &Path, now: &str, dry_run: bool) -> Result<ImportReport, ImportError> {
	let manifest_bytes = std::fs::read(dir.join("manifest.json")).map_err(ImportError::Manifest)?;
	let manifest: ManifestFile = serde_json::from_slice(&manifest_bytes).map_err(ImportError::ManifestShape)?;

	// One transaction: a run commits whole or not at all, and `BEGIN
	// IMMEDIATE` holds off the operator's write route for its length, so the
	// ceiling count below cannot move under it. Local file reads and at most
	// `MANIFEST_CEILING`-ish small writes; this is an offline command.
	let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
	let mut listed = RoundRepository::listed_count(&mut tx).await?;
	let mut report = ImportReport::default();
	let mut seen = BTreeSet::new();

	for (position, raw) in manifest.rounds.into_iter().enumerate() {
		let Some(key) = raw.as_str().map(str::to_owned) else {
			report.failed.push((position_label(position), "not a round id string".to_owned()));
			continue;
		};
		if !seen.insert(key.clone()) {
			report.failed.push((key, "listed more than once in manifest.json".to_owned()));
			continue;
		}
		let body = match read_round(dir, &key) {
			Ok(body) => body,
			Err(reason) => {
				report.failed.push((key, reason));
				continue;
			}
		};
		// Adding a round past the ceiling would make the manifest refuse;
		// refuse the round instead, as the write route does.
		if listed >= MANIFEST_CEILING && RoundRepository::entry(&mut tx, &key).await?.is_none() {
			report.failed.push((key, "would list more rounds than the manifest ceiling".to_owned()));
			continue;
		}
		match RoundRepository::upsert(&mut tx, &key, &body, now, dry_run).await {
			Ok(Change::Inserted) => {
				listed += 1;
				report.inserted.push(key);
			}
			Ok(Change::ContentChanged) => report.content_changed.push(key),
			Ok(Change::Unchanged) => report.unchanged.push(key),
			Err(WriteError::Invalid(problems)) => {
				let mut reason = String::new();
				for (field, why) in problems {
					if !reason.is_empty() {
						reason.push_str("; ");
					}
					reason.push_str(&field);
					reason.push(' ');
					reason.push_str(why);
				}
				report.failed.push((key, reason));
			}
			Err(WriteError::Storage(err)) => return Err(ImportError::Storage(err)),
		}
	}

	if dry_run {
		tx.rollback().await?;
	} else {
		tx.commit().await?;
	}
	Ok(report)
}

fn position_label(position: usize) -> String {
	let mut label = String::from("rounds[");
	label.push_str(&position.to_string());
	label.push(']');
	label
}

/// `dir/<id>.json`, verbatim, refusing an id that is a path or URL rather than
/// an identifier before it is ever joined to `dir`.
fn read_round(dir: &Path, key: &str) -> Result<Vec<u8>, String> {
	if !is_plain_key(key) {
		return Err("not a plain round id; paths and URLs are not imported".to_owned());
	}
	let mut file = String::from(key);
	file.push_str(".json");
	std::fs::read(dir.join(file)).map_err(|err| err.to_string())
}

#[cfg(test)]
mod tests {
	use super::import_dir;
	use crate::repository::RoundRepository;
	use serde_json::{json, Value};
	use sqlx::sqlite::SqlitePoolOptions;
	use sqlx::SqlitePool;
	use std::path::Path;

	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");

	async fn pool() -> SqlitePool {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();
		pool
	}

	fn round(id: &str, admissible: &str) -> String {
		json!({
			"id": id, "algorithm": { "language": "rust", "source": "fn f() {}" },
			"diffOptions": [
				{ "member": { "propositionId": admissible, "admissible": true } },
				{ "member": { "propositionId": "CW-P8", "admissible": false } }
			]
		})
		.to_string()
	}

	fn write(dir: &Path, name: &str, contents: &str) {
		let mut file = name.to_owned();
		file.push_str(".json");
		std::fs::write(dir.join(file), contents).unwrap();
	}

	fn write_manifest(dir: &Path, rounds: &Value) {
		write(dir, "manifest", &json!({ "rounds": rounds }).to_string());
	}

	async fn stored(pool: &SqlitePool, id: &str) -> (String, i64, String) {
		let row = sqlx::query!("SELECT published_at, version, body FROM leetype_round WHERE id = ?", id)
			.fetch_one(pool)
			.await
			.unwrap();
		(row.published_at, row.version, row.body)
	}

	/// A first import writes; a second over unchanged files writes nothing
	/// and moves nothing; a changed file is a version bump, a new
	/// `published_at`, and new witness rows, and only for that round.
	#[tokio::test]
	async fn reimporting_is_idempotent_and_only_changed_bytes_are_new() {
		let pool = pool().await;
		let dir = tempfile::tempdir().unwrap();
		write_manifest(dir.path(), &json!(["a", "b"]));
		write(dir.path(), "a", &round("a", "CW-P6"));
		write(dir.path(), "b", &round("b", "CW-P7"));

		let first = import_dir(&pool, dir.path(), "2026-09-29T00:00:00+00:00", false).await.unwrap();
		assert_eq!(
			(first.inserted.as_slice(), first.failed.as_slice()),
			(["a", "b"].map(String::from).as_slice(), [].as_slice())
		);

		let second = import_dir(&pool, dir.path(), "2026-09-30T00:00:00+00:00", false).await.unwrap();
		assert_eq!(second.writes(), 0, "{second:?}");
		assert_eq!(second.unchanged, ["a", "b"]);
		assert_eq!(stored(&pool, "a").await.0, "2026-09-29T00:00:00+00:00", "published_at did not move");

		write(dir.path(), "a", &round("a", "CW-P5"));
		let third = import_dir(&pool, dir.path(), "2026-10-01T00:00:00+00:00", false).await.unwrap();
		assert_eq!(third.content_changed, ["a"]);
		assert_eq!(third.unchanged, ["b"]);
		let (published_at, version, body) = stored(&pool, "a").await;
		assert_eq!((published_at.as_str(), version, body), ("2026-10-01T00:00:00+00:00", 2, round("a", "CW-P5")));
		let witnessing = RoundRepository::new(pool.clone()).rounds_witnessing("CW-P6", 10).await.unwrap();
		assert!(witnessing.is_empty(), "the old witness rows went with the old bytes: {witnessing:?}");
		assert_eq!(RoundRepository::new(pool.clone()).rounds_witnessing("CW-P5", 10).await.unwrap().len(), 1);
	}

	/// One bad round fails alone and is named; the rest import.
	#[tokio::test]
	async fn a_bad_round_fails_alone() {
		let pool = pool().await;
		let dir = tempfile::tempdir().unwrap();
		write_manifest(dir.path(), &json!(["good", "missing", "broken", "../escape", "mismatch", 7, "good"]));
		write(dir.path(), "good", &round("good", "CW-P6"));
		write(dir.path(), "broken", "<!doctype html>");
		write(dir.path(), "mismatch", &round("other", "CW-P6"));

		let report = import_dir(&pool, dir.path(), "2026-09-29T00:00:00+00:00", false).await.unwrap();
		assert_eq!(report.inserted, ["good"]);
		let failed: Vec<&str> = report.failed.iter().map(|(what, _)| what.as_str()).collect();
		assert_eq!(failed, ["missing", "broken", "../escape", "mismatch", "rounds[5]", "good"]);
		assert!(report.failed[3].1.contains("body.id"), "{:?}", report.failed[3]);
	}

	/// `--dry-run` reports what would change and writes nothing at all.
	#[tokio::test]
	async fn a_dry_run_writes_nothing() {
		let pool = pool().await;
		let dir = tempfile::tempdir().unwrap();
		write_manifest(dir.path(), &json!(["a"]));
		write(dir.path(), "a", &round("a", "CW-P6"));

		let report = import_dir(&pool, dir.path(), "2026-09-29T00:00:00+00:00", true).await.unwrap();
		assert_eq!(report.inserted, ["a"], "reported");
		let rows: (i64, i64) = sqlx::query_as("SELECT (SELECT COUNT(*) FROM leetype_round), (SELECT COUNT(*) FROM leetype_round_witness)")
			.fetch_one(&pool)
			.await
			.unwrap();
		assert_eq!(rows, (0, 0), "but not written");
	}

	/// A directory with no manifest, or one that is not a manifest, is an
	/// error for the whole run.
	#[tokio::test]
	async fn no_manifest_is_a_whole_run_failure() {
		let pool = pool().await;
		let dir = tempfile::tempdir().unwrap();
		assert!(import_dir(&pool, dir.path(), "now", false).await.is_err());
		write(dir.path(), "manifest", r#"{"topiks": []}"#);
		assert!(import_dir(&pool, dir.path(), "now", false).await.is_err());
	}
}

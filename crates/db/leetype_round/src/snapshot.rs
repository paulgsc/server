//! The static snapshot of the round corpus and its recorded runs (#328,
//! LTY-SRV4), for `apps/www`'s `DATA_MODE === "static"` build, which has no
//! server to ask.
//!
//! ```text
//! <out>/rounds/manifest.json   {"rounds": [<id>, …]}, the listed rounds by id
//! <out>/rounds/<id>.json       the stored body, byte for byte
//! <out>/runs/<id>.json         GET /leetype/rounds/:id/runs's body
//! ```
//!
//! `rounds/` is the layout `@some-ui/leetype`'s `export-round-corpus.ts`
//! writes and `import-leetype-rounds` reads, so a snapshot round file is the
//! authored export's file exactly. `runs/` is [`RoundRuns`], the route's own
//! type, so a static round and a live one read the same transcript.
//!
//! **Deterministic.** Rounds by id, runs in transcript order, JSON written the
//! way the client's export writes it (two-space indent, a final newline), and
//! nothing that moves when nothing changed: `recorded_at` is not part of
//! [`RoundRuns`]. A file whose bytes would not change is not rewritten, and a
//! file in either directory that the snapshot no longer contains (a retired
//! round's) is removed. So regenerating from an unchanged database is a
//! no-op diff.

use crate::repository::{RoundRepository, MANIFEST_CEILING};
use crate::runs::RoundRuns;
use serde::Serialize;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// What one dump wrote — or, checking, would have written.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SnapshotReport {
	/// The listed rounds, by id.
	pub rounds: Vec<String>,
	/// Files whose bytes changed (or that are new), relative to `out`.
	pub written: Vec<String>,
	/// Files removed because the snapshot no longer contains them.
	pub removed: Vec<String>,
	/// Rounds with no runs recorded for their current version.
	pub unrecorded: Vec<String>,
}

/// Why a dump stopped. Files already written stay written; a rerun converges.
#[derive(Debug)]
pub enum SnapshotError {
	/// More listed rounds than [`MANIFEST_CEILING`]: refused, never truncated.
	OverCeiling,
	/// A round changed between reading its body and reading its runs.
	Changed(String),
	/// A round id whose file would collide with the manifest or, on a
	/// case-insensitive filesystem, with another round's: `manifest`, or two
	/// ids equal but for case. Refused before anything is written (review,
	/// #399).
	Collides(String),
	Io(PathBuf, std::io::Error),
	Json(serde_json::Error),
	Storage(sqlx::Error),
}

impl std::fmt::Display for SnapshotError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::OverCeiling => f.write_str("more listed rounds than the manifest ceiling"),
			Self::Changed(id) => write!(f, "round {id} changed while it was being dumped; run the dump again"),
			Self::Collides(id) => write!(f, "round id {id} collides with the manifest or another round's file; rename it"),
			Self::Io(path, err) => write!(f, "{}: {err}", path.display()),
			Self::Json(err) => write!(f, "could not serialise: {err}"),
			Self::Storage(err) => write!(f, "database error: {err}"),
		}
	}
}

impl std::error::Error for SnapshotError {}

impl From<sqlx::Error> for SnapshotError {
	fn from(err: sqlx::Error) -> Self {
		Self::Storage(err)
	}
}

#[derive(Serialize)]
struct Manifest<'a> {
	rounds: &'a [String],
}

/// `value` as the client's export writes JSON: `JSON.stringify(value, null,
/// 2) + "\n"`.
///
/// # Errors
/// Only if `value` cannot be serialised.
pub fn pretty<T: Serialize>(value: &T) -> Result<Vec<u8>, serde_json::Error> {
	let mut bytes = serde_json::to_vec_pretty(value)?;
	bytes.push(b'\n');
	Ok(bytes)
}

/// Whether [`dump_snapshot`] writes, or only reports what it would write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotMode {
	Write,
	/// Write nothing; a report with anything in `written` or `removed` means
	/// `out` is not the snapshot of this database. The CI check #328 asks for.
	Check,
}

/// Write the snapshot of every listed round into `out` (or, with
/// [`SnapshotMode::Check`], compare `out` against it).
///
/// # Errors
/// See [`SnapshotError`].
pub async fn dump_snapshot(repository: &RoundRepository, out: &Path, mode: SnapshotMode) -> Result<SnapshotReport, SnapshotError> {
	let entries = repository.entries(MANIFEST_CEILING + 1).await?;
	#[allow(clippy::cast_possible_wrap)] // at most MANIFEST_CEILING + 1
	if entries.len() as i64 > MANIFEST_CEILING {
		return Err(SnapshotError::OverCeiling);
	}
	let mut report = SnapshotReport {
		rounds: entries.into_iter().map(|entry| entry.id).collect(),
		..SnapshotReport::default()
	};

	let mut files = BTreeSet::from([String::from("manifest")]);
	for id in &report.rounds {
		if !files.insert(id.to_lowercase()) {
			return Err(SnapshotError::Collides(id.clone()));
		}
	}

	let rounds_dir = out.join("rounds");
	let runs_dir = out.join("runs");
	if mode == SnapshotMode::Write {
		for dir in [&rounds_dir, &runs_dir] {
			std::fs::create_dir_all(dir).map_err(|err| SnapshotError::Io(dir.clone(), err))?;
		}
	}
	let write = |dir: &Path, name: &str, bytes: &[u8], prefix: &str, written: &mut Vec<String>| write_if_changed(mode, dir, name, bytes, prefix, written);

	let mut keep_rounds = BTreeSet::from([String::from("manifest.json")]);
	let mut keep_runs = BTreeSet::new();
	let manifest = pretty(&Manifest { rounds: &report.rounds }).map_err(SnapshotError::Json)?;
	write(&rounds_dir, "manifest.json", &manifest, "rounds/", &mut report.written)?;

	for id in report.rounds.clone() {
		let Some((hash, body)) = repository.body(&id).await? else {
			return Err(SnapshotError::Changed(id));
		};
		let Some(runs) = repository.runs(&id).await? else {
			return Err(SnapshotError::Changed(id));
		};
		if runs.content_hash != hash {
			return Err(SnapshotError::Changed(id));
		}
		if runs.runs.is_empty() {
			report.unrecorded.push(id.clone());
		}
		let mut file = id.clone();
		file.push_str(".json");
		write(&rounds_dir, &file, body.as_bytes(), "rounds/", &mut report.written)?;
		write(&runs_dir, &file, &runs_file(&runs)?, "runs/", &mut report.written)?;
		keep_rounds.insert(file.clone());
		keep_runs.insert(file);
	}

	remove_others(mode, &rounds_dir, &keep_rounds, "rounds/", &mut report.removed)?;
	remove_others(mode, &runs_dir, &keep_runs, "runs/", &mut report.removed)?;
	Ok(report)
}

/// `runs/<id>.json`'s bytes.
fn runs_file(runs: &RoundRuns) -> Result<Vec<u8>, SnapshotError> {
	pretty(runs).map_err(SnapshotError::Json)
}

fn write_if_changed(mode: SnapshotMode, dir: &Path, name: &str, bytes: &[u8], prefix: &str, written: &mut Vec<String>) -> Result<(), SnapshotError> {
	let path = dir.join(name);
	if std::fs::read(&path).is_ok_and(|current| current == bytes) {
		return Ok(());
	}
	if mode == SnapshotMode::Write {
		std::fs::write(&path, bytes).map_err(|err| SnapshotError::Io(path, err))?;
	}
	written.push(String::from(prefix) + name);
	Ok(())
}

fn remove_others(mode: SnapshotMode, dir: &Path, keep: &BTreeSet<String>, prefix: &str, removed: &mut Vec<String>) -> Result<(), SnapshotError> {
	let listing = match std::fs::read_dir(dir) {
		Ok(listing) => listing,
		// Checking a directory that was never written: nothing to remove.
		Err(err) if err.kind() == std::io::ErrorKind::NotFound && mode == SnapshotMode::Check => return Ok(()),
		Err(err) => return Err(SnapshotError::Io(dir.to_path_buf(), err)),
	};
	let mut names = Vec::new();
	for entry in listing {
		let entry = entry.map_err(|err| SnapshotError::Io(dir.to_path_buf(), err))?;
		names.push(entry.file_name().to_string_lossy().into_owned());
	}
	names.sort();
	for name in names {
		if !keep.contains(&name) {
			if mode == SnapshotMode::Write {
				let path = dir.join(&name);
				std::fs::remove_file(&path).map_err(|err| SnapshotError::Io(path, err))?;
			}
			removed.push(String::from(prefix) + &name);
		}
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::{dump_snapshot, SnapshotMode};
	use crate::runs::{Bounds, Elapsed, Observation, RecordedRun, RunResult, Variant};
	use crate::{import_dir, RoundRepository};
	use sqlx::sqlite::SqlitePoolOptions;
	use std::collections::BTreeMap;
	use std::path::{Path, PathBuf};

	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");
	const T0: &str = "2026-09-29T00:00:00+00:00";

	fn fixture() -> PathBuf {
		PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testdata").join("rounds")
	}

	/// Every file under `dir`, relative path → bytes.
	fn tree(dir: &Path) -> BTreeMap<String, Vec<u8>> {
		let mut files = BTreeMap::new();
		for sub in ["rounds", "runs"] {
			for entry in std::fs::read_dir(dir.join(sub)).unwrap() {
				let entry = entry.unwrap();
				let name = String::from(sub) + "/" + &entry.file_name().to_string_lossy();
				files.insert(name, std::fs::read(entry.path()).unwrap());
			}
		}
		files
	}

	fn recorded(output: &str) -> RecordedRun {
		RecordedRun {
			variant: Variant::Algorithm,
			bounds: Bounds::Before,
			sizes: BTreeMap::from([(String::from("n"), 1000)]),
			result: RunResult::Ok {
				input_size: 1000,
				observation: Observation {
					output: output.to_owned(),
					logs: vec![],
					elapsed: Elapsed { milliseconds: 4 },
				},
			},
		}
	}

	/// A round named `manifest` would overwrite the manifest, and two ids
	/// equal but for case would share a file on a case-insensitive disk:
	/// refused before anything is written (review, #399).
	#[tokio::test]
	async fn a_round_id_that_collides_with_a_file_is_refused_before_writing() {
		for ids in [&["manifest"][..], &["Pair", "pair"][..]] {
			let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
			MIGRATOR.run(&pool).await.unwrap();
			// A named round, not the directory's first file: `read_dir` order
			// is the filesystem's, and the fixture also holds its manifest.
			let template = fixture().join("has-duplicate-sort-adjacent.json");
			let mut round: serde_json::Value = serde_json::from_slice(&std::fs::read(template).unwrap()).unwrap();
			let mut conn = pool.acquire().await.unwrap();
			for id in ids {
				round["id"] = serde_json::Value::from(*id);
				RoundRepository::upsert(&mut conn, id, round.to_string().as_bytes(), T0, false).await.unwrap();
			}
			drop(conn);
			let out = tempfile::tempdir().unwrap();
			let err = dump_snapshot(&RoundRepository::new(pool), out.path(), SnapshotMode::Write).await.unwrap_err();
			assert!(matches!(err, super::SnapshotError::Collides(_)), "{err}");
			assert!(!out.path().join("rounds").exists(), "nothing written");
		}
	}

	/// The fixture's round files come back byte for byte; runs are the
	/// route's body; a second dump from the same database changes no byte and
	/// writes no file; a re-recording at another time changes nothing either,
	/// because `recorded_at` is not dumped; and a retired round's files go.
	#[tokio::test]
	async fn a_dump_is_the_export_byte_for_byte_and_regenerating_it_is_a_no_op() {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();
		let report = import_dir(&pool, &fixture(), T0, false).await.unwrap();
		assert!(report.failed.is_empty(), "{report:?}");
		let repository = RoundRepository::new(pool.clone());
		let id = report.inserted[0].clone();
		let (hash, _) = repository.body(&id).await.unwrap().unwrap();
		assert!(repository.replace_runs(&id, &hash, &[recorded("false")], T0).await.unwrap());

		let out = tempfile::tempdir().unwrap();
		let first = dump_snapshot(&repository, out.path(), SnapshotMode::Write).await.unwrap();
		let mut ids = report.inserted.clone();
		ids.sort();
		assert_eq!(first.rounds, ids, "listed rounds, by id");
		assert_eq!(first.unrecorded.len(), ids.len() - 1);
		let files = tree(out.path());
		for round in &ids {
			let name = String::from(round) + ".json";
			assert_eq!(files[&(String::from("rounds/") + &name)], std::fs::read(fixture().join(&name)).unwrap(), "{round} verbatim");
		}
		let manifest: serde_json::Value = serde_json::from_slice(&files["rounds/manifest.json"]).unwrap();
		assert_eq!(manifest, serde_json::json!({ "rounds": ids }));
		assert!(files["rounds/manifest.json"].starts_with(b"{\n  \"rounds\": [\n    \"") && files["rounds/manifest.json"].ends_with(b"]\n}\n"));
		let runs: serde_json::Value = serde_json::from_slice(&files[&(String::from("runs/") + &id + ".json")]).unwrap();
		assert_eq!(runs, serde_json::to_value(repository.runs(&id).await.unwrap().unwrap()).unwrap(), "the route's body");
		assert!(runs.get("recordedAt").is_none() && !String::from_utf8_lossy(&files[&(String::from("runs/") + &id + ".json")]).contains(T0));

		let second = dump_snapshot(&repository, out.path(), SnapshotMode::Write).await.unwrap();
		assert_eq!((second.written.len(), second.removed.len()), (0, 0), "{second:?}");
		assert_eq!(tree(out.path()), files, "byte-identical");

		assert!(repository.replace_runs(&id, &hash, &[recorded("false")], "2026-10-01T00:00:00+00:00").await.unwrap());
		assert_eq!(
			dump_snapshot(&repository, out.path(), SnapshotMode::Write).await.unwrap().written.len(),
			0,
			"re-recording the same result moves nothing"
		);

		let check = dump_snapshot(&repository, out.path(), SnapshotMode::Check).await.unwrap();
		assert_eq!((check.written.len(), check.removed.len()), (0, 0), "a current snapshot checks clean");
		let fresh = tempfile::tempdir().unwrap();
		let missing = dump_snapshot(&repository, fresh.path(), SnapshotMode::Check).await.unwrap();
		assert_eq!(missing.written.len(), 1 + 2 * ids.len(), "every file of an absent snapshot differs");
		assert!(std::fs::read_dir(fresh.path()).unwrap().next().is_none(), "and checking writes nothing");

		RoundRepository::retire(&mut pool.acquire().await.unwrap(), &id, T0).await.unwrap();
		let stale = dump_snapshot(&repository, out.path(), SnapshotMode::Check).await.unwrap();
		assert_eq!((stale.written.len(), stale.removed.len()), (1, 2), "a retirement shows up in the check");
		assert_eq!(tree(out.path()), files, "without touching the snapshot");
		let after = dump_snapshot(&repository, out.path(), SnapshotMode::Write).await.unwrap();
		let gone = String::from(&id) + ".json";
		assert_eq!(after.removed, [String::from("rounds/") + &gone, String::from("runs/") + &gone]);
		assert_eq!(after.written, ["rounds/manifest.json"]);
	}
}

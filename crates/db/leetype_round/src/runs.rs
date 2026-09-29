//! Recorded runs of a round's program (#381, LTY-EXEC; #328, LTY-SRV4).
//!
//! The wire types mirror `@some-ui/leetype`'s `RunResult`
//! (`lib/leetype/run-result`) exactly: the client writes its own schema by
//! hand, so these are the contract to match, not types to generate. They are
//! produced by `leetype_runner` offline, stored in `leetype_round_run` by
//! [`RoundRepository::replace_runs`], and read back by
//! [`RoundRepository::runs`] — for the route and for the static snapshot
//! alike, so a live answer and a static one are the same bytes' worth of
//! JSON.
//!
//! **No complexity claim, anywhere** (#381's never #3): no class, no Θ, no
//! "admissible". A [`RecordedRun`] says what one program printed at one set of
//! sizes, and how long that took on the machine that recorded it — nothing
//! Thm. 4.1 would let anybody infer a class from.

use crate::repository::RoundRepository;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Which of a round's two constraint sets a run's sizes came from:
/// `constraintDiff.before` (`C`) or `constraintDiff.after` (`C′`).
///
/// Declared in that order, so sorting puts `before` first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Bounds {
	Before,
	After,
}

impl Bounds {
	/// Both, in the order a transcript lists them.
	pub const ALL: [Self; 2] = [Self::Before, Self::After];

	/// The value `leetype_round_run.bounds` holds, and the wire's.
	#[must_use]
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::Before => "before",
			Self::After => "after",
		}
	}

	/// The inverse of [`Self::as_str`].
	#[must_use]
	pub fn parse(text: &str) -> Option<Self> {
		match text {
			"before" => Some(Self::Before),
			"after" => Some(Self::After),
			_ => None,
		}
	}
}

/// Which program a run ran: `A` itself, or `A + d` for the member of the
/// option set at that position.
///
/// On the wire and in the table, `"A"` and `"d0"` … `"d4"`. Declared `A`
/// first, so sorting lists `A` before every `A + d`, and those in option
/// order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(into = "String", try_from = "String")]
pub enum Variant {
	Algorithm,
	Diff(u8),
}

impl Variant {
	/// `"A"` or `"d<i>"`.
	#[must_use]
	pub fn label(self) -> String {
		match self {
			Self::Algorithm => String::from("A"),
			Self::Diff(index) => {
				let mut label = String::from("d");
				label.push_str(&index.to_string());
				label
			}
		}
	}

	/// The inverse of [`Self::label`]: `A`, or `d` and one digit.
	#[must_use]
	pub fn parse(text: &str) -> Option<Self> {
		if text == "A" {
			return Some(Self::Algorithm);
		}
		let digits = text.strip_prefix('d')?;
		if digits.len() != 1 {
			return None;
		}
		digits.parse().ok().map(Self::Diff)
	}
}

impl From<Variant> for String {
	fn from(variant: Variant) -> Self {
		variant.label()
	}
}

impl TryFrom<String> for Variant {
	type Error = String;

	fn try_from(text: String) -> Result<Self, Self::Error> {
		Self::parse(&text).ok_or(text)
	}
}

/// The three ways a run fails (Def. 4.1), spelled as the client spells them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ErrorClass {
	#[serde(rename = "compile")]
	Compile,
	#[serde(rename = "runtime")]
	Runtime,
	#[serde(rename = "budget-exceeded")]
	BudgetExceeded,
}

impl ErrorClass {
	/// The client's spelling, as the wire carries it.
	#[must_use]
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::Compile => "compile",
			Self::Runtime => "runtime",
			Self::BudgetExceeded => "budget-exceeded",
		}
	}
}

/// `ExecutionError`: which way a run failed, and what the toolchain or the
/// program said.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecutionError {
	pub error_class: ErrorClass,
	pub message: String,
}

/// `ElapsedMs`: wall-clock milliseconds, wrapped as the client wraps them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Elapsed {
	pub milliseconds: u64,
}

/// `ExecutionObservation`: what a run that finished printed, and how long it
/// took.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Observation {
	/// Standard output, without its final newline, at most the runner's
	/// output ceiling.
	pub output: String,
	/// Standard error's lines, plus a note for anything truncated.
	pub logs: Vec<String>,
	pub elapsed: Elapsed,
}

/// `RunResult`, exactly. Both branches carry `inputSize`: a measurement with
/// no size attached cannot establish "a concrete fact about one input", the
/// one job Prop. 4.1 allows a run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum RunResult {
	Ok { input_size: u64, observation: Observation },
	Error { input_size: u64, error: ExecutionError },
}

impl RunResult {
	/// The size the run was given for its constraint set's first dimension.
	#[must_use]
	pub const fn input_size(&self) -> u64 {
		match self {
			Self::Ok { input_size, .. } | Self::Error { input_size, .. } => *input_size,
		}
	}
}

/// One entry of a round's transcript: which program, at which constraint
/// set's bounds, the full size of every dimension it was given, and its
/// [`RunResult`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecordedRun {
	pub variant: Variant,
	pub bounds: Bounds,
	/// Dimension → the bound the harness was given for it.
	pub sizes: BTreeMap<String, u64>,
	pub result: RunResult,
}

/// `GET /leetype/rounds/:id/runs`, and `runs/<id>.json` in the static
/// snapshot: a round's transcript for the version it is now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RoundRuns {
	pub round_id: String,
	/// The round's current content hash: the version these runs belong to.
	pub content_hash: String,
	/// `A` first, then `d0` …, each `before` then `after`. Empty when
	/// nothing is recorded for this version.
	pub runs: Vec<RecordedRun>,
}

/// A stored run that does not read back as what [`RoundRepository::replace_runs`]
/// writes: a decode failure, as far as callers are concerned.
fn corrupt(field: &'static str, err: impl std::error::Error + Send + Sync + 'static) -> sqlx::Error {
	sqlx::Error::ColumnDecode {
		index: field.to_owned(),
		source: Box::new(err),
	}
}

#[derive(Debug)]
struct UnknownLabel(String);

impl std::fmt::Display for UnknownLabel {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "not a label this table writes: {:?}", self.0)
	}
}

impl std::error::Error for UnknownLabel {}

impl RoundRepository {
	/// `id`'s transcript for its **current** content hash, or `None` for an
	/// id this table does not hold. Retired rounds included, as
	/// [`Self::body`] does.
	///
	/// One query: the round's hash and the runs recorded for that hash are
	/// read together, so a run recorded for an older body is never paired
	/// with a newer one — it is simply not returned.
	///
	/// # Errors
	/// Propagates any `sqlx` failure; a stored row that does not decode is a
	/// `ColumnDecode` error.
	pub async fn runs(&self, id: &str) -> Result<Option<RoundRuns>, sqlx::Error> {
		let rows = sqlx::query!(
			r#"
			SELECT r.content_hash AS "content_hash!", run.variant AS "variant?", run.bounds AS "bounds?",
			       run.sizes AS "sizes?", run.result AS "result?"
			FROM leetype_round r
			LEFT JOIN leetype_round_run run ON run.round_id = r.id AND run.content_hash = r.content_hash
			WHERE r.id = ?1
			"#,
			id
		)
		.fetch_all(&self.pool)
		.await?;
		let Some(first) = rows.first() else {
			return Ok(None);
		};
		let content_hash = first.content_hash.clone();
		let mut runs = Vec::with_capacity(rows.len());
		for row in rows {
			let (Some(variant), Some(bounds), Some(sizes), Some(result)) = (row.variant, row.bounds, row.sizes, row.result) else {
				continue;
			};
			runs.push(RecordedRun {
				variant: Variant::parse(&variant).ok_or_else(|| corrupt("variant", UnknownLabel(variant)))?,
				bounds: Bounds::parse(&bounds).ok_or_else(|| corrupt("bounds", UnknownLabel(bounds)))?,
				sizes: serde_json::from_str(&sizes).map_err(|err| corrupt("sizes", err))?,
				result: serde_json::from_str(&result).map_err(|err| corrupt("result", err))?,
			});
		}
		runs.sort_by_key(|run| (run.variant, run.bounds));
		Ok(Some(RoundRuns {
			round_id: id.to_owned(),
			content_hash,
			runs,
		}))
	}

	/// Whether any run is recorded for `id` at `content_hash`.
	///
	/// # Errors
	/// Propagates any `sqlx` failure.
	pub async fn has_runs(&self, id: &str, content_hash: &str) -> Result<bool, sqlx::Error> {
		sqlx::query_scalar!(
			r#"SELECT EXISTS (SELECT 1 FROM leetype_round_run WHERE round_id = ?1 AND content_hash = ?2) AS "recorded!: bool""#,
			id,
			content_hash
		)
		.fetch_one(&self.pool)
		.await
	}

	/// Replace `id`'s whole transcript with `runs`, recorded against
	/// `content_hash`, as of `now` — in one transaction, and only while the
	/// round's current hash is still `content_hash`.
	///
	/// Returns `false`, writing nothing, when the round is gone or its body
	/// changed since the runs were recorded: a transcript belongs to the bytes
	/// it was recorded from, and storing it under another version's hash would
	/// serve it for a program it never ran.
	///
	/// # Errors
	/// Propagates any `sqlx` failure.
	pub async fn replace_runs(&self, id: &str, content_hash: &str, runs: &[RecordedRun], now: &str) -> Result<bool, sqlx::Error> {
		// `BEGIN IMMEDIATE`: the hash check and the writes are one decision,
		// and no write to the round can land between them.
		let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
		let current = sqlx::query_scalar!("SELECT content_hash FROM leetype_round WHERE id = ?", id)
			.fetch_optional(&mut *tx)
			.await?;
		if current.as_deref() != Some(content_hash) {
			tx.rollback().await?;
			return Ok(false);
		}
		sqlx::query!("DELETE FROM leetype_round_run WHERE round_id = ?", id).execute(&mut *tx).await?;
		for run in runs {
			let variant = run.variant.label();
			let bounds = run.bounds.as_str();
			// Bound parameters, not tracing arguments.
			#[allow(clippy::disallowed_methods)]
			let sizes = serde_json::to_string(&run.sizes).map_err(|err| sqlx::Error::Encode(Box::new(err)))?;
			#[allow(clippy::disallowed_methods)]
			let result = serde_json::to_string(&run.result).map_err(|err| sqlx::Error::Encode(Box::new(err)))?;
			sqlx::query!(
				"INSERT INTO leetype_round_run (round_id, content_hash, variant, bounds, sizes, result, recorded_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
				id,
				content_hash,
				variant,
				bounds,
				sizes,
				result,
				now,
			)
			.execute(&mut *tx)
			.await?;
		}
		tx.commit().await?;
		Ok(true)
	}
}

#[cfg(test)]
mod tests {
	use super::{Bounds, Elapsed, ErrorClass, ExecutionError, Observation, RecordedRun, RunResult, Variant};
	use crate::{Change, RoundRepository};
	use serde_json::json;
	use sqlx::sqlite::SqlitePoolOptions;
	use sqlx::SqlitePool;
	use std::collections::BTreeMap;

	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");
	const PREVIOUS_MIGRATION: i64 = 20_260_929_000_100;
	const T0: &str = "2026-09-29T00:00:00+00:00";

	async fn pool() -> SqlitePool {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();
		pool
	}

	fn body(id: &str, admissible: &str) -> Vec<u8> {
		json!({ "id": id, "algorithm": { "language": "rust" }, "diffOptions": [
			{ "member": { "propositionId": admissible, "admissible": true } },
			{ "member": { "propositionId": "CW-P8", "admissible": false } }
		]})
		.to_string()
		.into_bytes()
	}

	async fn put(pool: &SqlitePool, id: &str, admissible: &str) -> String {
		let bytes = body(id, admissible);
		let change = RoundRepository::upsert(&mut pool.acquire().await.unwrap(), id, &bytes, T0, false).await.unwrap();
		assert_ne!(change, Change::Unchanged);
		crate::content_hash(&bytes)
	}

	fn run(variant: Variant, bounds: Bounds, output: &str) -> RecordedRun {
		RecordedRun {
			variant,
			bounds,
			sizes: BTreeMap::from([(String::from("n"), 1000)]),
			result: RunResult::Ok {
				input_size: 1000,
				observation: Observation {
					output: output.to_owned(),
					logs: vec![],
					elapsed: Elapsed { milliseconds: 3 },
				},
			},
		}
	}

	#[test]
	fn the_wire_shape_is_the_clients_run_result() {
		let ok = run(Variant::Diff(1), Bounds::After, "false");
		assert_eq!(
			serde_json::to_value(&ok).unwrap(),
			json!({ "variant": "d1", "bounds": "after", "sizes": { "n": 1000 }, "result":
				{ "kind": "ok", "inputSize": 1000, "observation": { "output": "false", "logs": [], "elapsed": { "milliseconds": 3 } } } })
		);
		for (class, spelled) in [
			(ErrorClass::Compile, "compile"),
			(ErrorClass::Runtime, "runtime"),
			(ErrorClass::BudgetExceeded, "budget-exceeded"),
		] {
			let result = RunResult::Error {
				input_size: 7,
				error: ExecutionError {
					error_class: class,
					message: String::from("m"),
				},
			};
			assert_eq!(class.as_str(), spelled);
			let value = serde_json::to_value(&result).unwrap();
			assert_eq!(value, json!({ "kind": "error", "inputSize": 7, "error": { "errorClass": spelled, "message": "m" } }));
			assert_eq!(serde_json::from_value::<RunResult>(value).unwrap(), result, "round-trips");
		}
	}

	#[test]
	fn variants_and_bounds_label_and_order_as_the_transcript_lists_them() {
		for (variant, label) in [(Variant::Algorithm, "A"), (Variant::Diff(0), "d0"), (Variant::Diff(4), "d4")] {
			assert_eq!(variant.label(), label);
			assert_eq!(Variant::parse(label), Some(variant));
		}
		for label in ["", "a", "d", "d10", "dx", "B"] {
			assert_eq!(Variant::parse(label), None, "{label}");
		}
		let mut keys = vec![
			(Variant::Diff(1), Bounds::Before),
			(Variant::Algorithm, Bounds::After),
			(Variant::Diff(0), Bounds::After),
			(Variant::Algorithm, Bounds::Before),
		];
		keys.sort();
		assert_eq!(
			keys,
			[
				(Variant::Algorithm, Bounds::Before),
				(Variant::Algorithm, Bounds::After),
				(Variant::Diff(0), Bounds::After),
				(Variant::Diff(1), Bounds::Before)
			]
		);
	}

	#[tokio::test]
	async fn the_migration_round_trips_and_the_table_holds_its_described_columns() {
		let pool = pool().await;
		let count = || async {
			sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'leetype_round_run'")
				.fetch_one(&pool)
				.await
				.unwrap()
		};
		assert_eq!(count().await, 1);
		let mut columns: Vec<String> = sqlx::query_scalar("SELECT name FROM pragma_table_info('leetype_round_run')")
			.fetch_all(&pool)
			.await
			.unwrap();
		columns.sort();
		assert_eq!(columns, ["bounds", "content_hash", "recorded_at", "result", "round_id", "sizes", "variant"]);
		MIGRATOR.undo(&pool, PREVIOUS_MIGRATION).await.unwrap();
		assert_eq!(count().await, 0, "down drops it");
		MIGRATOR.run(&pool).await.unwrap();
		assert_eq!(count().await, 1, "up recreates it");
	}

	/// Unknown round: `None`. Known with nothing recorded: an empty
	/// transcript. Recorded: in transcript order, whatever order it was
	/// written in.
	#[tokio::test]
	async fn runs_are_none_empty_or_ordered() {
		let pool = pool().await;
		let repository = RoundRepository::new(pool.clone());
		assert_eq!(repository.runs("a").await.unwrap(), None);
		let hash = put(&pool, "a", "CW-P6").await;
		let empty = repository.runs("a").await.unwrap().unwrap();
		assert_eq!((empty.round_id.as_str(), empty.content_hash.as_str(), empty.runs.len()), ("a", hash.as_str(), 0));
		assert!(!repository.has_runs("a", &hash).await.unwrap());

		let written = [
			run(Variant::Diff(0), Bounds::After, "3"),
			run(Variant::Algorithm, Bounds::After, "1"),
			run(Variant::Diff(0), Bounds::Before, "2"),
			run(Variant::Algorithm, Bounds::Before, "0"),
		];
		assert!(repository.replace_runs("a", &hash, &written, T0).await.unwrap());
		assert!(repository.has_runs("a", &hash).await.unwrap());
		let read = repository.runs("a").await.unwrap().unwrap();
		let outputs: Vec<String> = read
			.runs
			.iter()
			.map(|run| match &run.result {
				RunResult::Ok { observation, .. } => observation.output.clone(),
				RunResult::Error { .. } => String::new(),
			})
			.collect();
		assert_eq!(outputs, ["0", "1", "2", "3"]);
	}

	/// A body edit leaves the old runs unserved, and a recording for a hash
	/// that is no longer current writes nothing; a new recording replaces the
	/// whole set.
	#[tokio::test]
	async fn stale_runs_are_never_served_or_written() {
		let pool = pool().await;
		let repository = RoundRepository::new(pool.clone());
		let old = put(&pool, "a", "CW-P6").await;
		assert!(repository.replace_runs("a", &old, &[run(Variant::Algorithm, Bounds::Before, "old")], T0).await.unwrap());
		let new = put(&pool, "a", "CW-P5").await;
		assert_eq!(repository.runs("a").await.unwrap().unwrap().runs, [], "recorded for another version");
		assert!(!repository.replace_runs("a", &old, &[run(Variant::Algorithm, Bounds::Before, "late")], T0).await.unwrap());
		assert!(!repository.replace_runs("missing", &old, &[], T0).await.unwrap());

		assert!(repository.replace_runs("a", &new, &[run(Variant::Algorithm, Bounds::After, "new")], T0).await.unwrap());
		let read = repository.runs("a").await.unwrap().unwrap();
		assert_eq!(read.runs, [run(Variant::Algorithm, Bounds::After, "new")], "replaced whole, not merged");
		let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM leetype_round_run").fetch_one(&pool).await.unwrap();
		assert_eq!(rows, 1);
	}

	/// The schema refuses a label this module never writes.
	#[tokio::test]
	async fn the_checks_hold_at_the_schema() {
		let pool = pool().await;
		put(&pool, "a", "CW-P6").await;
		for (variant, bounds, ok) in [
			("A", "before", true),
			("d4", "after", true),
			("d10", "after", false),
			("B", "after", false),
			("d1", "during", false),
		] {
			let inserted =
				sqlx::query("INSERT INTO leetype_round_run (round_id, content_hash, variant, bounds, sizes, result, recorded_at) VALUES ('a', 'h', ?, ?, '{}', '{}', 'now')")
					.bind(variant)
					.bind(bounds)
					.execute(&pool)
					.await;
			assert_eq!(inserted.is_ok(), ok, "{variant} {bounds}");
		}
	}
}

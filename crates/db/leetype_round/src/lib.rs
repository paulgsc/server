//! The `leetype` activity's server-owned round corpus (#324; #325, LTY-SRV1).
//!
//! One row per round, its body stored verbatim, plus one edge row per member
//! of its option set recording `μ`: the `CW-P` proposition that member
//! witnesses and whether it is the admissible one. See
//! `20260929000100_create_leetype_round.up.sql` for why it is a blob *and* an
//! edge table. [`parse_round`] is the only reading of a body this server does,
//! and [`content_hash`] (shared with `curriculum_repo`) the one definition of
//! "did this round change" the importer, the operator's write route and the
//! `ETag` share.
//!
//! [`runs`] holds each round's recorded runs (#381): `RunResult`s that
//! `leetype_runner` produced offline, stored per round version and served by
//! `GET /leetype/rounds/:id/runs`. [`snapshot`] writes the listed rounds and
//! their runs as files for the static build (#328).

pub mod importer;
pub mod model;
pub mod repository;
pub mod runs;
pub mod snapshot;

pub use importer::{import_dir, ImportError, ImportReport};
pub use model::{
	content_hash, is_plain_key, is_proposition_id, parse_round, validate_round, ParsedRound, Problem, RoundEntry, Witness, MAX_DIFF_OPTIONS, MIN_DIFF_OPTIONS,
	ROUND_BYTES_CEILING,
};
pub use repository::{Change, RoundRepository, WitnessingRound, WriteError, MANIFEST_CEILING, OPERATOR_LISTING_CEILING};
pub use runs::{Bounds, Elapsed, ErrorClass, ExecutionError, Observation, RecordedRun, RoundRuns, RunResult, Variant};
pub use snapshot::{dump_snapshot, SnapshotError, SnapshotMode, SnapshotReport};

#[cfg(test)]
mod tests {
	use crate::{Change, RoundRepository};
	use serde_json::json;
	use sqlx::sqlite::SqlitePoolOptions;
	use sqlx::SqlitePool;

	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");
	const PREVIOUS_MIGRATION: i64 = 20_260_927_000_200;
	const T0: &str = "2026-09-29T00:00:00+00:00";

	async fn pool() -> SqlitePool {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();
		pool
	}

	async fn tables(pool: &SqlitePool) -> i64 {
		sqlx::query_scalar!(r#"SELECT COUNT(*) AS "count!: i64" FROM sqlite_master WHERE type = 'table' AND name IN ('leetype_round', 'leetype_round_witness')"#)
			.fetch_one(pool)
			.await
			.unwrap()
	}

	fn round(id: &str, options: &[(&str, bool)]) -> Vec<u8> {
		let options: Vec<serde_json::Value> = options
			.iter()
			.map(|(proposition, admissible)| json!({ "member": { "propositionId": proposition, "admissible": admissible } }))
			.collect();
		json!({ "id": id, "algorithm": { "language": "rust" }, "diffOptions": options }).to_string().into_bytes()
	}

	async fn put(pool: &SqlitePool, id: &str, options: &[(&str, bool)]) -> Change {
		RoundRepository::upsert(&mut pool.acquire().await.unwrap(), id, &round(id, options), T0, false)
			.await
			.unwrap()
	}

	#[tokio::test]
	async fn the_migration_round_trips() {
		let pool = pool().await;
		assert_eq!(tables(&pool).await, 2);
		MIGRATOR.undo(&pool, PREVIOUS_MIGRATION).await.unwrap();
		assert_eq!(tables(&pool).await, 0, "down drops both tables");
		MIGRATOR.run(&pool).await.unwrap();
		assert_eq!(tables(&pool).await, 2, "up recreates them");
	}

	/// The columns the migration's header describes, and no others: a new
	/// column is a decision about the blob-and-edge split, not a detail.
	#[tokio::test]
	async fn the_tables_hold_exactly_their_described_columns() {
		let pool = pool().await;
		for (table, expected) in [
			("leetype_round", vec!["body", "content_hash", "id", "language", "published_at", "retired_at", "version"]),
			("leetype_round_witness", vec!["admissible", "member_index", "proposition_id", "round_id"]),
		] {
			let mut columns: Vec<String> = sqlx::query_scalar("SELECT name FROM pragma_table_info(?)").bind(table).fetch_all(&pool).await.unwrap();
			columns.sort();
			assert_eq!(columns, expected, "{table}");
		}
	}

	/// "Which rounds witness CW-P5" is a range read of the witness index, and
	/// the manifest reads the listed set from its partial index.
	#[tokio::test]
	async fn the_edge_query_and_the_listed_set_read_their_indexes() {
		let pool = pool().await;
		for (query, index) in [
			(
				"EXPLAIN QUERY PLAN SELECT w.round_id, MAX(w.admissible) FROM leetype_round_witness w JOIN leetype_round r ON r.id = w.round_id WHERE w.proposition_id = 'CW-P5' AND r.retired_at IS NULL GROUP BY w.round_id ORDER BY w.round_id LIMIT 10",
				"idx_leetype_round_witness_proposition",
			),
			(
				"EXPLAIN QUERY PLAN SELECT id FROM leetype_round WHERE retired_at IS NULL ORDER BY id LIMIT 10",
				"idx_leetype_round_listed",
			),
			("EXPLAIN QUERY PLAN SELECT COUNT(*) FROM leetype_round WHERE retired_at IS NULL", "idx_leetype_round_listed"),
		] {
			let plan: Vec<(i64, i64, i64, String)> = sqlx::query_as(query).fetch_all(&pool).await.unwrap();
			assert!(plan.iter().any(|(_, _, _, detail)| detail.contains(index)), "{query}: {plan:?}");
			assert!(!plan.iter().any(|(_, _, _, detail)| detail.contains("TEMP B-TREE")), "no sort: {query}: {plan:?}");
		}
	}

	/// The schema refuses a language other than Rust and an admissibility
	/// that is not 0 or 1, and a witness row for a round that does not exist.
	#[tokio::test]
	async fn the_checks_hold_at_the_schema() {
		let pool = pool().await;
		for (id, language, ok) in [("a", "rust", true), ("b", "python", false)] {
			let inserted = sqlx::query!(
				"INSERT INTO leetype_round (id, language, published_at, version, content_hash, body) VALUES (?, ?, 'now', 1, 'h', '{}')",
				id,
				language
			)
			.execute(&pool)
			.await;
			assert_eq!(inserted.is_ok(), ok, "{language}");
		}
		for (round, admissible, ok) in [("a", 1, true), ("a", 2, false), ("missing", 0, false)] {
			let inserted = sqlx::query(
				"INSERT INTO leetype_round_witness (round_id, member_index, proposition_id, admissible) VALUES (?, (SELECT COUNT(*) FROM leetype_round_witness), 'CW-P1', ?)",
			)
			.bind(round)
			.bind(admissible)
			.execute(&pool)
			.await;
			assert_eq!(inserted.is_ok(), ok, "{round} {admissible}");
		}
	}

	/// The edge query answers from the witness rows, listed rounds only, and
	/// says whether the proposition is a round's answer or a distractor.
	#[tokio::test]
	async fn rounds_witnessing_reads_mu_for_listed_rounds() {
		let pool = pool().await;
		assert_eq!(put(&pool, "b", &[("CW-P5", false), ("CW-P6", true)]).await, Change::Inserted);
		put(&pool, "a", &[("CW-P5", true), ("CW-P8", false)]).await;
		put(&pool, "c", &[("CW-P5", true), ("CW-P9", false)]).await;
		let repository = RoundRepository::new(pool.clone());
		let witnessing = |rounds: Vec<crate::WitnessingRound>| rounds.into_iter().map(|round| (round.id, round.admissible)).collect::<Vec<_>>();
		assert_eq!(
			witnessing(repository.rounds_witnessing("CW-P5", 10).await.unwrap()),
			[("a".to_owned(), true), ("b".to_owned(), false), ("c".to_owned(), true)]
		);
		assert_eq!(repository.rounds_witnessing("CW-P5", 2).await.unwrap().len(), 2, "bounded");

		RoundRepository::retire(&mut pool.acquire().await.unwrap(), "c", T0).await.unwrap();
		assert_eq!(
			witnessing(repository.rounds_witnessing("CW-P5", 10).await.unwrap()).len(),
			2,
			"retired rounds are not sampled"
		);
	}

	/// Entries carry their witnesses in option order, loaded for the whole
	/// set at once; the manifest's are the listed rounds only.
	#[tokio::test]
	async fn entries_carry_their_witnesses_and_the_listed_ones_exclude_the_retired() {
		let pool = pool().await;
		put(&pool, "a", &[("CW-P8", false), ("CW-P6", true)]).await;
		put(&pool, "b", &[("CW-P7", true), ("CW-P9", false)]).await;
		RoundRepository::retire(&mut pool.acquire().await.unwrap(), "b", T0).await.unwrap();
		let repository = RoundRepository::new(pool.clone());

		let listed = repository.entries(10).await.unwrap();
		assert_eq!(listed.iter().map(|entry| entry.id.as_str()).collect::<Vec<_>>(), ["a"]);
		let witnesses: Vec<(&str, bool)> = listed[0].witnesses.iter().map(|w| (w.proposition_id.as_str(), w.admissible)).collect();
		assert_eq!(witnesses, [("CW-P8", false), ("CW-P6", true)]);

		let all = repository.all_entries(10).await.unwrap();
		assert_eq!(
			all.iter().map(|entry| (entry.id.as_str(), entry.retired_at.is_some())).collect::<Vec<_>>(),
			[("a", false), ("b", true)]
		);
		assert_eq!(all[1].witnesses.len(), 2);
		let witnesses = RoundRepository::witnesses_for(&mut pool.acquire().await.unwrap(), &["a", "b", "missing"]).await.unwrap();
		assert_eq!(witnesses.keys().collect::<Vec<_>>(), ["a", "b"]);
	}

	/// An invalid body is refused before anything is written.
	#[tokio::test]
	async fn an_invalid_round_writes_nothing() {
		let pool = pool().await;
		let err = RoundRepository::upsert(&mut pool.acquire().await.unwrap(), "a", &round("a", &[("CW-P6", true)]), T0, false)
			.await
			.unwrap_err();
		assert!(matches!(err, crate::WriteError::Invalid(_)), "{err}");
		assert_eq!(RoundRepository::count(&mut pool.acquire().await.unwrap()).await.unwrap(), 0);
	}
}

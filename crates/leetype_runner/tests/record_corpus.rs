//! Recording end to end: the round fixture `import-leetype-rounds` is tested
//! against, imported into a fresh database, one round recorded by the real
//! runner and `rustc`, and the transcript read back as the route serves it.
//!
//! One round, not five, with a 1 s ceiling rather than the default 2 s: the
//! point is the path from a stored body to a stored transcript, and what the
//! transcript of a real round says, at the cost of a few compiles and two
//! runs that hit the ceiling.

use leetype_round_repo::{import_dir, Bounds, ErrorClass, RoundRepository, RunResult, Variant};
use leetype_runner::{record_listed, Limits, RoundOutcome, Runner, Selection};
use sqlx::sqlite::SqlitePoolOptions;
use std::path::PathBuf;
use std::time::Duration;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations");
const ROUND: &str = "has-duplicate-sort-adjacent";

fn fixture() -> PathBuf {
	PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../db/leetype_round/testdata/rounds")
}

fn output(result: &RunResult) -> &str {
	match result {
		RunResult::Ok { observation, .. } => &observation.output,
		RunResult::Error { .. } => panic!("expected ok: {result:?}"),
	}
}

/// `A` finishes at `C` and runs past the ceiling at `C′`; the admissible
/// member finishes at both and prints what `A` printed at `C`; the
/// distractor, like `A`, runs past the ceiling at `C′`. And a second
/// recording finds the round up to date.
#[tokio::test]
async fn recording_one_fixture_round_stores_its_transcript_for_its_version() {
	let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
	MIGRATOR.run(&pool).await.unwrap();
	let imported = import_dir(&pool, &fixture(), "2026-09-29T00:00:00+00:00", false).await.unwrap();
	assert!(imported.failed.is_empty() && imported.inserted.len() == 5, "{imported:?}");
	let repository = RoundRepository::new(pool.clone());
	let runner = Runner::new(Limits {
		run_ceiling: Duration::from_millis(1_000),
		..Limits::default()
	})
	.unwrap();
	let selection = Selection {
		all: false,
		only: Some(String::from(ROUND)),
	};

	let report = record_listed(&repository, &runner, &selection, "2026-09-29T01:00:00+00:00").await.unwrap();
	let [(id, RoundOutcome::Recorded(runs))] = report.rounds.as_slice() else {
		panic!("{report:?}");
	};
	assert_eq!(id, ROUND);
	let keys: Vec<(Variant, Bounds)> = runs.iter().map(|run| (run.variant, run.bounds)).collect();
	assert_eq!(
		keys,
		[
			(Variant::Algorithm, Bounds::Before),
			(Variant::Algorithm, Bounds::After),
			(Variant::Diff(0), Bounds::Before),
			(Variant::Diff(0), Bounds::After),
			(Variant::Diff(1), Bounds::Before),
			(Variant::Diff(1), Bounds::After),
		]
	);
	for run in runs {
		let expected = if run.bounds == Bounds::Before { 1_000 } else { 100_000 };
		assert_eq!(run.sizes.get("n"), Some(&expected));
		assert_eq!(run.result.input_size(), expected);
	}

	let a_before = output(&runs[0].result);
	assert_eq!(a_before, "false", "every value distinct: no duplicate");
	for index in [1, 5] {
		let RunResult::Error { error, .. } = &runs[index].result else {
			panic!("{:?} at C′ should not finish: {:?}", runs[index].variant, runs[index].result);
		};
		assert_eq!(error.error_class, ErrorClass::BudgetExceeded);
	}
	assert_eq!(output(&runs[3].result), a_before, "the admissible member at C′ answers what A answered at C");
	assert_eq!(output(&runs[2].result), a_before);

	let served = repository.runs(ROUND).await.unwrap().unwrap();
	assert_eq!(&served.runs, runs, "stored as recorded");
	assert_eq!(served.content_hash, repository.body(ROUND).await.unwrap().unwrap().0);

	let again = record_listed(&repository, &runner, &selection, "2026-09-29T02:00:00+00:00").await.unwrap();
	assert_eq!(again.rounds, [(String::from(ROUND), RoundOutcome::UpToDate)]);
	assert!(!again.incomplete());
}

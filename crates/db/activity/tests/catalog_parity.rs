//! The parity test between the seeded `activities` table and
//! `paulgsc/some-ui@packages/activity-catalog/src/lib/catalog.ts`. #270.
//!
//! ## The user story
//!
//! "As the person who adds a fifth activity, I want to find out at build
//! time that I updated one repo and not the other." This file is that
//! tripwire.
//!
//! ## Mechanism chosen, and the ones rejected
//!
//! #270 named three options and called "(3) both" the likely answer. This
//! PR ships **(1) only** -- a checked-in fixture
//! (`testdata/activity_catalog.snapshot.json`), exported from the client by
//! `paulgsc/some-ui`'s `scripts/dump-activity-catalog.ts`
//! (`pnpm dump:activity-catalog > activity-catalog.snapshot.json`, then
//! copied here by hand) and compared against the seeded rows below.
//!
//! Rejected, for now:
//!
//! - **(2) a conformance contract** (`contract-harness/activities.contract.ts`
//!   in `some-ui`, needs a live `file_host`). This is the two-repo,
//!   simultaneous-landing shape #261 took (server PR + client companion),
//!   and it buys end-to-end coverage this story's user story does not ask
//!   for: the fixture above already answers "did someone update one repo and
//!   not the other" at build/test time, with no server process required.
//!   A conformance contract earns its cost once `GET /activities` (#271)
//!   exists to actually probe -- today there is no route for it to hit that
//!   isn't better covered by #271's own tests once that story lands.
//! - **(3) both** -- rejected for the same reason as (2): the marginal
//!   coverage over (1) alone is an end-to-end guarantee this story doesn't
//!   need yet, at the cost of a second repo's PR landing in lockstep with
//!   this one.
//!
//! This mirrors an existing asymmetry in this codebase, not a new pattern:
//! `routes.server.json` (`packages/contract-harness/`, `some-ui`) is exactly
//! this shape in the other direction -- the server's route surface,
//! exported by `cargo run --bin dump-routes`, copied by hand into the client
//! repo, and diffed there. #267 (SC2) is the story that would make *that*
//! copy stop being manual; it has not landed as of this writing, so this
//! fixture's freshness is the same by-hand discipline `routes.server.json`
//! already runs on, not a regression from some sharper mechanism this story
//! skipped.
//!
//! ## What a failure here means
//!
//! Every assertion below names the activity id and the field it checked, so
//! a divergence reads as e.g. `` `leetype`.registryKey diverged from the
//! client `` rather than "catalogues differ". If this test starts failing
//! after touching `catalog.ts`, regenerate the fixture (see the module doc
//! on `scripts/dump-activity-catalog.ts` in `some-ui`) and update
//! `20260823000700_seed_activities.up.sql` to match; if it fails after
//! touching the seed migration, the fixture is telling you the client and
//! the server no longer agree on what a real device would receive.

use std::collections::BTreeSet;

use activity_repo::{derive_min_duration_ms, ActivityMaturity, LayoutTree};
use serde::Deserialize;
use serde_json::Value;
use sqlx::sqlite::SqlitePoolOptions;
use sqlx::SqlitePool;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");

/// One entry in the checked-in fixture: `ActivityDefinition`
/// (`packages/activity-catalog/src/lib/types.ts`, `paulgsc/some-ui`) minus
/// `toSceneProps`. `toSceneProps` is a closure -- JSON has no way to
/// represent one, and `JSON.stringify` (what the dump script uses) drops a
/// function-valued property the same way it drops `undefined`, so the
/// fixture never had one to transcribe. That is CAT4/#272's boundary, not a
/// gap in this fixture.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FixtureActivity {
	id: String,
	name: String,
	description: String,
	icon: String,
	registry_key: String,
	layout_tree: String,
	fields: Vec<Value>,
	default_config: Value,
	#[serde(default)]
	audio: Option<Value>,
	/// Absent means `"ready"` -- `types.ts` calls it "the quiet default".
	/// The fixture is a literal transcription of `catalog.ts`, so it leaves
	/// this absent exactly where the client does; resolving it to `"ready"`
	/// is this test's job, not the fixture's, so that the resolution itself
	/// stays an assertion rather than something baked silently into the data.
	#[serde(default)]
	maturity: Option<String>,
}

async fn pool() -> SqlitePool {
	// One connection: SQLite's `:memory:` database is private to the
	// connection that opened it, matching `activity_repo`'s own test pool.
	let pool = SqlitePoolOptions::new()
		.max_connections(1)
		.connect("sqlite::memory:")
		.await
		.expect("open an in-memory sqlite database");
	MIGRATOR.run(&pool).await.expect("run the workspace migration history, including #270's seed");
	pool
}

/// Every observable field the seed wrote for each of the four activities
/// matches the checked-in fixture, `min_duration_ms` was derived rather than
/// guessed, and nothing is seeded that the fixture doesn't know about.
#[tokio::test]
async fn seeded_activities_match_the_clients_catalogue_field_by_field() {
	let pool = pool().await;

	let fixture: Vec<FixtureActivity> = serde_json::from_str(include_str!("../testdata/activity_catalog.snapshot.json")).expect("parse the checked-in fixture as valid JSON");
	assert!(!fixture.is_empty(), "the checked-in fixture is empty -- did the regen step run correctly?");

	for activity in &fixture {
		let row = sqlx::query!(
			r#"
			SELECT name, description, icon, registry_key, layout_tree, maturity,
			       min_duration_ms, fields, default_config, audio
			FROM activities WHERE id = ?1
			"#,
			activity.id
		)
		.fetch_optional(&pool)
		.await
		.expect("query the seeded row")
		.unwrap_or_else(|| {
			panic!(
				"`{}` is in the client's catalogue but was not seeded -- update 20260823000700_seed_activities.up.sql",
				activity.id
			)
		});

		assert_eq!(row.name, activity.name, "`{}`.name diverged from the client", activity.id);
		assert_eq!(row.description, activity.description, "`{}`.description diverged from the client", activity.id);
		assert_eq!(row.icon, activity.icon, "`{}`.icon diverged from the client", activity.id);
		assert_eq!(row.registry_key, activity.registry_key, "`{}`.registryKey diverged from the client", activity.id);
		assert_eq!(row.layout_tree, activity.layout_tree, "`{}`.layoutTree diverged from the client", activity.id);
		assert!(
			LayoutTree::parse(&row.layout_tree).is_some(),
			"`{}` seeded an unrecognised layout_tree {:?}",
			activity.id,
			row.layout_tree
		);

		let expected_maturity = activity.maturity.as_deref().unwrap_or("ready");
		assert_eq!(
			row.maturity, expected_maturity,
			"`{}`.maturity diverged from the client (absent on the client resolves to \"ready\")",
			activity.id
		);
		assert!(
			ActivityMaturity::parse(&row.maturity).is_some(),
			"`{}` seeded an unrecognised maturity {:?}",
			activity.id,
			row.maturity
		);

		let seeded_fields: Vec<Value> = serde_json::from_str(&row.fields).expect("seeded fields column is valid JSON");
		assert_eq!(seeded_fields, activity.fields, "`{}`.fields diverged from the client", activity.id);

		let seeded_default_config: Value = serde_json::from_str(&row.default_config).expect("seeded default_config column is valid JSON");
		assert_eq!(seeded_default_config, activity.default_config, "`{}`.defaultConfig diverged from the client", activity.id);

		let seeded_audio: Option<Value> = row.audio.as_deref().map(|raw| serde_json::from_str(raw).expect("seeded audio column is valid JSON"));
		assert_eq!(seeded_audio, activity.audio, "`{}`.audio diverged from the client", activity.id);

		// The derivation, not the hand-written literal: recomputed from the
		// fixture's own `fields` and checked against what the migration
		// actually inserted, per #270's acceptance criterion.
		let derived = derive_min_duration_ms(&activity.fields);
		assert_eq!(
			row.min_duration_ms, derived,
			"`{}`.min_duration_ms ({:?}) does not match the value derived from `fields` ({:?}) -- the seed's literal is wrong",
			activity.id, row.min_duration_ms, derived
		);
	}

	// The other direction: nothing seeded that the client doesn't know
	// about. A phantom row would otherwise pass every check above simply by
	// never being visited.
	let seeded_ids: Vec<String> = sqlx::query_scalar!(r#"SELECT id as "id!" FROM activities"#)
		.fetch_all(&pool)
		.await
		.expect("list seeded ids");
	let fixture_ids: BTreeSet<&str> = fixture.iter().map(|activity| activity.id.as_str()).collect();
	for id in &seeded_ids {
		assert!(
			fixture_ids.contains(id.as_str()),
			"`{id}` is seeded but is not in the client's catalogue -- either the seed is stale or the fixture needs regenerating"
		);
	}
}

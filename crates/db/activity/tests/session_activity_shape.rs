//! The server-side half of CAT4/#272's round-trip claim: every seeded
//! activity's `default_config` is shaped exactly like the client's
//! `ActivityConfigValues` (`Record<string, string | number>`,
//! `packages/activity-catalog/src/lib/types.ts`, `paulgsc/some-ui`) -- a flat
//! object of scalars, nothing nested, nothing boolean or null.
//!
//! #272's conclusion is that this server writes `activities: [{ activityId,
//! config }]` and nothing else -- no `props`, no `scenes`. That claim only
//! holds if every `config` this server could ever hand over is already
//! shaped the way the client's type demands. `default_config` is opaque
//! `serde_json::Value` by `ActivityRecord`'s own design (see
//! `crates/db/activity/src/model.rs`), so nothing here stops a future seed
//! or a future write path from smuggling in a nested object or a boolean --
//! it would compile, and fail silently on the client, one repo away from
//! where anyone would notice. This test is what makes that failure loud,
//! and here, instead.
//!
//! The client-side half of the same claim -- that a `SessionActivity` built
//! from exactly this shape survives an actual JSON round trip into a
//! playable scene -- lives in `paulgsc/some-ui`'s
//! `packages/activity-catalog/src/lib/to-scene-config/session-activity-round-trip.test.ts`,
//! per #272's own acceptance criterion that this is "most cheaply ... a
//! fixture consumed by a test in `paulgsc/some-ui`."

use serde_json::Value;
use sqlx::sqlite::SqlitePoolOptions;
use sqlx::SqlitePool;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");

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

/// A value `ActivityConfigValues` can hold. Anything else -- a nested
/// object, an array, a bool, or null -- is a shape `SessionActivity.config`
/// cannot carry once it crosses into TypeScript.
const fn is_scalar_config_value(value: &Value) -> bool {
	matches!(value, Value::String(_) | Value::Number(_))
}

/// Every seeded activity's `default_config` is a flat JSON object whose
/// values are all strings or numbers -- the same shape the recommender
/// (`#279`-`#285`) will eventually hand a client verbatim as
/// `SessionActivity.config`, with no transformation in between.
#[tokio::test]
async fn every_seeded_default_config_is_shaped_like_activityconfigvalues() {
	let pool = pool().await;

	let rows = sqlx::query!(r#"SELECT id as "id!", default_config FROM activities"#)
		.fetch_all(&pool)
		.await
		.expect("list every seeded activity's default_config");

	assert!(!rows.is_empty(), "no activities were seeded -- did #270's migration run?");

	for row in rows {
		let parsed: Value = serde_json::from_str(&row.default_config).unwrap_or_else(|error| panic!("`{}`.default_config is not valid JSON: {error}", row.id));

		let object = parsed
			.as_object()
			.unwrap_or_else(|| panic!("`{}`.default_config is not a JSON object, got {parsed}", row.id));

		for (key, value) in object {
			assert!(
				is_scalar_config_value(value),
				"`{}`.default_config.{key} is {value}, which `ActivityConfigValues` (string | number) cannot hold",
				row.id
			);
		}
	}
}

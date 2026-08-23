//! The server-owned activity catalogue.
//!
//! [`ActivityRecord`] mirrors `ActivityDefinition` in
//! `packages/activity-catalog/src/lib/types.ts` (`paulgsc/some-ui`) field for
//! field, the same relationship `session_repo::SessionRecord` has to the
//! client's `SessionRecord`. Unlike every table `#259` touched, this one
//! carries no `subject_id`: an activity is catalogue-wide, not owned by
//! whoever plays it, so per-subject state (played, dismissed -- #258's table)
//! is deliberately a different table, not a column here.
//!
//! The `toSceneProps` replacement (#272) and `GET /activities` (#271) are
//! still out of scope for this crate -- see #269's own out-of-scope note.
//! What is here is the schema (`20260822000600_create_activities.up.sql`),
//! the model it round-trips through, the seed (#270,
//! `20260823000700_seed_activities.up.sql`), and the parity test
//! (`tests/catalog_parity.rs`) that fails when the seed and
//! `paulgsc/some-ui@packages/activity-catalog/src/lib/catalog.ts` disagree.

pub mod duration;
pub mod model;

pub use duration::derive_min_duration_ms;
pub use model::{ActivityMaturity, ActivityRecord, LayoutTree};

/// #269's acceptance criteria, made concrete against a real database rather
/// than argued about in the abstract: the migration's columns, the UNIQUE
/// constraint on `registry_key`, and the model's `parse` functions all agree
/// with each other. Embedded and run against a fresh in-memory database per
/// test, same reasoning as `session_repo`'s and `nudge::waker`'s test
/// modules: the externally-applied `DATABASE_URL` scratch database
/// `cargo test` needs for `sqlx::query!` to compile is one file shared by
/// every crate's tests in one `rust_ci` run, and a table only this test
/// writes to needs its own.
///
/// `pool()` now also runs #270's seed migration, so every fresh database
/// here already has `honeycomb`/`topik`/`interview`/`leetype` in it -- a
/// test that inserts its own row must pick an id and registry_key that isn't
/// one of those four (see `a_full_row_round_trips_through_insert_and_select`).
#[cfg(test)]
mod tests {
	use super::model::{ActivityMaturity, LayoutTree};
	use sqlx::sqlite::SqlitePoolOptions;
	use sqlx::SqlitePool;

	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");

	async fn pool() -> SqlitePool {
		// One connection: SQLite's `:memory:` database is private to the
		// connection that opened it, so a pool free to open a second one
		// could silently hand a query an empty, unmigrated schema.
		let pool = SqlitePoolOptions::new()
			.max_connections(1)
			.connect("sqlite::memory:")
			.await
			.expect("open an in-memory sqlite database");
		MIGRATOR.run(&pool).await.expect("run the workspace migration history");
		pool
	}

	/// A row written with every column populated -- including the nullable
	/// `min_duration_ms` and `audio` -- comes back out with `layout_tree`
	/// and `maturity` parsing to the same variant that went in, and every
	/// other value equal to what was written.
	///
	/// Uses a synthetic id/registry_key rather than a real catalogued one
	/// (e.g. `"honeycomb"`/`"hangul"`) on purpose: #270's seed migration now
	/// runs as part of `MIGRATOR` in every test in this module, so a literal
	/// real id here would collide with the seeded row on the `id` PRIMARY
	/// KEY and fail for a reason this test isn't about.
	#[tokio::test]
	async fn a_full_row_round_trips_through_insert_and_select() {
		let pool = pool().await;

		sqlx::query!(
			r#"
			INSERT INTO activities (
			    id, name, description, icon, registry_key, layout_tree, maturity,
			    min_duration_ms, published_at, version, fields, default_config, audio
			) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
			"#,
			"test-round-trip-row",
			"Test Round Trip Row",
			"A synthetic row, not one of the seeded catalogue entries.",
			"hexagon",
			"test-round-trip-registry-key",
			"study",
			"ready",
			300_000_i64,
			"2026-08-01T00:00:00Z",
			1_i64,
			r#"[{"kind":"select","key":"mode"}]"#,
			r#"{"mode":"completion"}"#,
			r#"{"channels":["effects"],"blurb":"Uses game sound effects"}"#,
		)
		.execute(&pool)
		.await
		.expect("insert a fully-populated activity row");

		let row = sqlx::query!(
			r#"
			SELECT id as "id!", name, description, icon, registry_key, layout_tree, maturity,
			       min_duration_ms, published_at, version, fields, default_config, audio
			FROM activities WHERE id = ?1
			"#,
			"test-round-trip-row"
		)
		.fetch_one(&pool)
		.await
		.expect("read the row back");

		assert_eq!(row.id, "test-round-trip-row");
		assert_eq!(row.registry_key, "test-round-trip-registry-key");
		assert_eq!(LayoutTree::parse(&row.layout_tree), Some(LayoutTree::Study));
		assert_eq!(ActivityMaturity::parse(&row.maturity), Some(ActivityMaturity::Ready));
		assert_eq!(row.min_duration_ms, Some(300_000));
		assert_eq!(row.published_at, "2026-08-01T00:00:00Z");
		assert_eq!(row.version, 1);
		assert!(row.audio.is_some());
	}

	/// The other half of `min_duration_ms`'s argument: a row that omits it
	/// (and `audio`) entirely is not an error, and reads back as a real
	/// `None` rather than some substituted value -- the case the migration's
	/// comment names, for whichever activity is next to have no duration
	/// field, now that `leetype` no longer does.
	#[tokio::test]
	async fn an_activity_with_no_duration_field_stores_a_real_null() {
		let pool = pool().await;

		sqlx::query!(
			r#"
			INSERT INTO activities (
			    id, name, description, icon, registry_key, layout_tree, maturity,
			    min_duration_ms, published_at, version, fields, default_config, audio
			) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, ?8, ?9, ?10, ?11, NULL)
			"#,
			"no-duration-yet",
			"Placeholder",
			"An activity with no duration field.",
			"keyboard",
			"placeholder",
			"drama",
			"preview",
			"2026-08-22T00:00:00Z",
			1_i64,
			"[]",
			"{}",
		)
		.execute(&pool)
		.await
		.expect("insert a row with no duration field");

		let row = sqlx::query!("SELECT min_duration_ms, audio FROM activities WHERE id = ?1", "no-duration-yet")
			.fetch_one(&pool)
			.await
			.expect("read the row back");

		assert_eq!(row.min_duration_ms, None);
		assert_eq!(row.audio, None);
	}

	/// `registry_key` is enforced UNIQUE at the schema level, per #269's
	/// acceptance criterion -- this is the property that index exists to
	/// guarantee, not just a lookup it happens to speed up.
	#[tokio::test]
	async fn a_duplicate_registry_key_is_refused() {
		let pool = pool().await;

		sqlx::query!(
			r#"
			INSERT INTO activities (
			    id, name, description, icon, registry_key, layout_tree, maturity,
			    min_duration_ms, published_at, version, fields, default_config, audio
			) VALUES ('first', 'n', 'd', 'i', 'shared-key', 'study', 'ready', NULL, '2026-08-22T00:00:00Z', 1, '[]', '{}', NULL)
			"#
		)
		.execute(&pool)
		.await
		.expect("the first row with this registry_key succeeds");

		let second = sqlx::query!(
			r#"
			INSERT INTO activities (
			    id, name, description, icon, registry_key, layout_tree, maturity,
			    min_duration_ms, published_at, version, fields, default_config, audio
			) VALUES ('second', 'n', 'd', 'i', 'shared-key', 'study', 'ready', NULL, '2026-08-22T00:00:00Z', 1, '[]', '{}', NULL)
			"#
		)
		.execute(&pool)
		.await;

		assert!(second.is_err(), "a second row claiming the same registry_key must be refused");
	}
}

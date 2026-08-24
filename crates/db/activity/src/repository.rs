use crate::model::{ActivityMaturity, ActivityRecord, LayoutTree};
use sqlx::SqlitePool;
use std::collections::hash_map::DefaultHasher;
use std::fmt::Write as _;
use std::hash::{Hash, Hasher};

/// The ceiling `list()` refuses to exceed.
///
/// Deliberately generous relative to today's four rows -- #271's own
/// argument is that the client never wants the whole catalogue anyway (it
/// ranks, pages, or searches), so this exists to catch a runaway catalogue,
/// not to bound ordinary use.
pub const CATALOG_CEILING: i64 = 500;

/// The row as `SQLite` hands it back.
#[derive(sqlx::FromRow)]
struct ActivityRow {
	id: String,
	name: String,
	description: String,
	icon: String,
	registry_key: String,
	layout_tree: String,
	maturity: String,
	min_duration_ms: Option<i64>,
	published_at: String,
	version: i64,
	fields: String,
	default_config: String,
	audio: Option<String>,
}

/// A row this schema did not write.
#[derive(Debug)]
pub enum RowError {
	UnknownLayoutTree(String),
	UnknownMaturity(String),
	MalformedJson(&'static str, serde_json::Error),
}

impl std::fmt::Display for RowError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::UnknownLayoutTree(raw) => write!(f, "unknown layout tree: {raw}"),
			Self::UnknownMaturity(raw) => write!(f, "unknown activity maturity: {raw}"),
			Self::MalformedJson(column, err) => write!(f, "malformed JSON in `{column}`: {err}"),
		}
	}
}

impl std::error::Error for RowError {}

fn parse_json<T: serde::de::DeserializeOwned>(column: &'static str, raw: &str) -> Result<T, RowError> {
	serde_json::from_str(raw).map_err(|err| RowError::MalformedJson(column, err))
}

impl TryFrom<ActivityRow> for ActivityRecord {
	type Error = RowError;

	fn try_from(row: ActivityRow) -> Result<Self, Self::Error> {
		Ok(Self {
			layout_tree: LayoutTree::parse(&row.layout_tree).ok_or_else(|| RowError::UnknownLayoutTree(row.layout_tree.clone()))?,
			maturity: ActivityMaturity::parse(&row.maturity).ok_or_else(|| RowError::UnknownMaturity(row.maturity.clone()))?,
			fields: parse_json("fields", &row.fields)?,
			default_config: parse_json("default_config", &row.default_config)?,
			audio: row.audio.map(|raw| parse_json("audio", &raw)).transpose()?,
			id: row.id,
			name: row.name,
			description: row.description,
			icon: row.icon,
			registry_key: row.registry_key,
			min_duration_ms: row.min_duration_ms,
			published_at: row.published_at,
			version: row.version,
		})
	}
}

/// Anything that can go wrong reading the catalogue.
#[derive(Debug)]
pub enum ActivityRepoError {
	Sqlx(sqlx::Error),
	Row(RowError),
	/// The table holds more rows than [`CATALOG_CEILING`] allows. Refused
	/// outright rather than handed back as a silently truncated prefix -- see
	/// #271's "silent truncation is the one option that is definitely wrong."
	CatalogTooLarge {
		rows: i64,
		ceiling: i64,
	},
}

impl std::fmt::Display for ActivityRepoError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::Sqlx(err) => write!(f, "{err}"),
			Self::Row(err) => write!(f, "{err}"),
			Self::CatalogTooLarge { rows, ceiling } => write!(f, "catalogue holds {rows} rows, over the {ceiling} ceiling; refusing rather than truncating"),
		}
	}
}

impl std::error::Error for ActivityRepoError {}

impl From<sqlx::Error> for ActivityRepoError {
	fn from(err: sqlx::Error) -> Self {
		Self::Sqlx(err)
	}
}

impl From<RowError> for ActivityRepoError {
	fn from(err: RowError) -> Self {
		Self::Row(err)
	}
}

pub struct ActivityRepository {
	pool: SqlitePool,
}

impl ActivityRepository {
	#[must_use]
	pub const fn new(pool: SqlitePool) -> Self {
		Self { pool }
	}

	/// The full catalogue, ordered by `id` for a stable response across
	/// otherwise-identical requests.
	///
	/// Bounded by [`CATALOG_CEILING`], and over that ceiling is a refusal
	/// ([`ActivityRepoError::CatalogTooLarge`]), not a truncated `Vec` --
	/// #271 is explicit that handing back a silent prefix is the one wrong
	/// answer to "what does a client do when there are more than that."
	///
	/// # Errors
	/// Fails on any `sqlx` error, if a stored row does not parse, or with
	/// [`ActivityRepoError::CatalogTooLarge`] if the table has grown past the
	/// ceiling.
	pub async fn list(&self) -> Result<Vec<ActivityRecord>, ActivityRepoError> {
		let total = sqlx::query_scalar!(r#"SELECT COUNT(*) as "count!: i64" FROM activities"#).fetch_one(&self.pool).await?;
		if total > CATALOG_CEILING {
			return Err(ActivityRepoError::CatalogTooLarge {
				rows: total,
				ceiling: CATALOG_CEILING,
			});
		}

		let rows = sqlx::query_as!(
			ActivityRow,
			r#"
			SELECT id as "id!", name, description, icon, registry_key, layout_tree, maturity,
			       min_duration_ms, published_at, version, fields, default_config, audio
			FROM activities
			ORDER BY id
			LIMIT ?
			"#,
			CATALOG_CEILING
		)
		.fetch_all(&self.pool)
		.await?;

		rows.into_iter().map(|row| ActivityRecord::try_from(row).map_err(Into::into)).collect()
	}

	/// One activity, or `None` if `id` names no row.
	///
	/// # Errors
	/// Fails on any `sqlx` error, or if the stored row does not parse.
	pub async fn get(&self, id: &str) -> Result<Option<ActivityRecord>, ActivityRepoError> {
		let row = sqlx::query_as!(
			ActivityRow,
			r#"
			SELECT id as "id!", name, description, icon, registry_key, layout_tree, maturity,
			       min_duration_ms, published_at, version, fields, default_config, audio
			FROM activities
			WHERE id = ?
			"#,
			id
		)
		.fetch_optional(&self.pool)
		.await?;

		row.map(|row| ActivityRecord::try_from(row).map_err(Into::into)).transpose()
	}

	/// A value that changes exactly when the published catalogue does --
	/// a publish, an edit, or a removal, not just an increasing `version`
	/// somewhere in the set.
	///
	/// This is the one notion of "the catalogue changed" #271's `ETag` and
	/// #273's `CurriculumUpdated` producer both have to agree on; #271's own
	/// out-of-scope note is explicit that a second, independent computation
	/// of "changed" in #273 would be the wrong shape. Callers needing that
	/// question answered call this rather than deriving their own.
	///
	/// Hashed over every row's `(id, version)`, sorted by `id` for a stable
	/// result independent of insertion order -- a `MAX(version)` alone would
	/// miss a row being removed, which is still a real catalogue change.
	///
	/// # Errors
	/// Fails on any `sqlx` error.
	pub async fn fingerprint(&self) -> Result<String, ActivityRepoError> {
		let rows = sqlx::query!(r#"SELECT id as "id!", version FROM activities ORDER BY id"#).fetch_all(&self.pool).await?;

		let mut hasher = DefaultHasher::new();
		for row in &rows {
			row.id.hash(&mut hasher);
			row.version.hash(&mut hasher);
		}

		// `write!` rather than `format!`: `clippy.toml` disallows `format!`
		// (eager allocation ahead of tracing) -- same substitution
		// `ts_emitter::render_ts` already makes.
		let mut out = String::new();
		let _ = write!(out, "{:016x}", hasher.finish());
		Ok(out)
	}
}

#[cfg(test)]
mod tests {
	use super::{ActivityRepoError, ActivityRepository, CATALOG_CEILING};
	use sqlx::sqlite::SqlitePoolOptions;
	use sqlx::SqlitePool;
	use std::fmt::Write as _;

	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");

	async fn pool() -> SqlitePool {
		// One connection: SQLite's `:memory:` database is private to the
		// connection that opened it, matching this crate's other test pools.
		let pool = SqlitePoolOptions::new()
			.max_connections(1)
			.connect("sqlite::memory:")
			.await
			.expect("open an in-memory sqlite database");
		MIGRATOR.run(&pool).await.expect("run the workspace migration history, including #270's seed");
		pool
	}

	#[tokio::test]
	async fn list_returns_every_seeded_activity_ordered_by_id() {
		let pool = pool().await;
		let repo = ActivityRepository::new(pool);

		let activities = repo.list().await.expect("list the seeded catalogue");
		assert_eq!(activities.len(), 4, "the seed migration writes four rows");

		let ids: Vec<&str> = activities.iter().map(|a| a.id.as_str()).collect();
		let mut sorted = ids.clone();
		sorted.sort_unstable();
		assert_eq!(ids, sorted, "list() must already be in id order");
	}

	#[tokio::test]
	async fn get_returns_none_for_an_unknown_id() {
		let pool = pool().await;
		let repo = ActivityRepository::new(pool);

		assert!(repo.get("no-such-activity").await.expect("query should not fail").is_none());
	}

	#[tokio::test]
	async fn get_returns_the_row_for_a_known_id() {
		let pool = pool().await;
		let repo = ActivityRepository::new(pool);

		let activities = repo.list().await.expect("list the seeded catalogue");
		let first = &activities[0];

		let fetched = repo.get(&first.id).await.expect("query should not fail").expect("the id exists");
		assert_eq!(fetched.id, first.id);
		assert_eq!(fetched.registry_key, first.registry_key);
	}

	#[tokio::test]
	async fn fingerprint_is_stable_across_calls_when_nothing_changed() {
		let pool = pool().await;
		let repo = ActivityRepository::new(pool);

		let first = repo.fingerprint().await.expect("compute the fingerprint");
		let second = repo.fingerprint().await.expect("compute the fingerprint again");
		assert_eq!(first, second, "an unchanged catalogue must fingerprint identically");
	}

	#[tokio::test]
	async fn fingerprint_changes_when_a_row_is_removed() {
		let pool = pool().await;
		let repo = ActivityRepository::new(pool.clone());

		let before = repo.fingerprint().await.expect("compute the fingerprint");

		let activities = repo.list().await.expect("list the seeded catalogue");
		sqlx::query!("DELETE FROM activities WHERE id = ?", activities[0].id)
			.execute(&pool)
			.await
			.expect("remove one row");

		let after = repo.fingerprint().await.expect("compute the fingerprint after a removal");
		assert_ne!(before, after, "removing a row must change the fingerprint even though no remaining version increased");
	}

	#[tokio::test]
	async fn list_refuses_rather_than_truncates_once_the_catalogue_exceeds_the_ceiling() {
		let pool = pool().await;

		// Push the row count past `CATALOG_CEILING` directly -- the four
		// seeded rows plus enough synthetic ones to cross the ceiling. One
		// transaction rather than one round trip per row, since this needs
		// hundreds of inserts just to set up the case.
		let mut tx = pool.begin().await.expect("open a transaction");
		for n in 0..=CATALOG_CEILING {
			// `write!` rather than `format!`: `clippy.toml` disallows `format!`
			// even in test code.
			let mut id = String::from("ceiling-probe-");
			let _ = write!(id, "{n}");
			let mut registry_key = String::from("ceiling-probe-key-");
			let _ = write!(registry_key, "{n}");
			sqlx::query!(
				r#"
				INSERT INTO activities (
				    id, name, description, icon, registry_key, layout_tree, maturity,
				    min_duration_ms, published_at, version, fields, default_config, audio
				) VALUES (?1, 'n', 'd', 'i', ?2, 'study', 'ready', NULL, '2026-08-24T00:00:00Z', 1, '[]', '{}', NULL)
				"#,
				id,
				registry_key,
			)
			.execute(&mut *tx)
			.await
			.expect("insert a synthetic row to cross the ceiling");
		}
		tx.commit().await.expect("commit the synthetic rows");

		let repo = ActivityRepository::new(pool);
		let result = repo.list().await;

		assert!(
			matches!(result, Err(ActivityRepoError::CatalogTooLarge { .. })),
			"a catalogue over the ceiling must be refused outright, not truncated: got {result:?}"
		);
	}
}

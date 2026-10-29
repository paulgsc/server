//! What home keeps, by table, once a day (paulgsc/server#542, STORAGE1):
//! `storage_sample`, and the growth figures read from it.
//!
//! `20261029000100_create_storage_sample.up.sql` records the table's design
//! and `docs/storage.md` the whole measurement; in short:
//!
//! - **Measured, a b-tree at a time.** [`btrees`] lists every table and index
//!   the file holds, and [`btree_bytes`] reads one of them from `SQLite`'s
//!   `dbstat` table, so the caller can stop between any two (a deadline, a
//!   shutdown) and let other work have the pool in between. [`footprint`]
//!   reads the file's size and its free pages from the header.
//! - **One row set per UTC day, append-only.** [`record`] writes a day's set
//!   in one transaction, or nothing if that day is already held, so a restart
//!   never samples a day twice. The triggers abort every `UPDATE` and `DELETE`.
//! - **The windows are read, not kept.** [`figures`] reads the newest set and
//!   the oldest set inside each window: three index range reads, whatever the
//!   history's length.
//! - **Nobody's record.** Bytes per table and file; not subject-scoped.

use sqlx::{Connection, Row, SqliteConnection};

/// What a row measures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
	/// One table's b-tree.
	Table,
	/// One index's b-tree.
	Index,
	/// The whole database file, `page_count * page_size`.
	Database,
	/// The `-wal` file's length.
	Wal,
	/// The bytes free to the server on the database's filesystem.
	DiskFree,
}

impl Kind {
	/// The value `storage_sample.kind` stores, and a metric's `kind` label.
	#[must_use]
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::Table => "table",
			Self::Index => "index",
			Self::Database => "database",
			Self::Wal => "wal",
			Self::DiskFree => "disk_free",
		}
	}

	fn parse(kind: &str) -> Option<Self> {
		match kind {
			"table" => Some(Self::Table),
			"index" => Some(Self::Index),
			"database" => Some(Self::Database),
			"wal" => Some(Self::Wal),
			"disk_free" => Some(Self::DiskFree),
			_ => None,
		}
	}
}

/// One row of a day's set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Size {
	pub kind: Kind,
	/// The table's or index's name; the kind's own name for the others.
	pub name: String,
	pub bytes: i64,
}

impl Size {
	/// A row for one of the whole-file kinds, named after its kind.
	#[must_use]
	pub fn whole(kind: Kind, bytes: i64) -> Self {
		Self {
			kind,
			name: String::from(kind.as_str()),
			bytes,
		}
	}
}

/// What the database file's header says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Footprint {
	/// The main database's path, as `SQLite` opened it; empty for an
	/// in-memory database.
	pub file: String,
	/// `page_count * page_size`: the file's length.
	pub database_bytes: i64,
	/// `freelist_count * page_size`: pages the file holds and no b-tree uses.
	pub freelist_bytes: i64,
}

/// The database file's path, size and free pages, from its header.
///
/// # Errors
/// Any `sqlx` failure.
pub async fn footprint(conn: &mut SqliteConnection) -> Result<Footprint, sqlx::Error> {
	let row = sqlx::query(
		"SELECT (SELECT file FROM pragma_database_list WHERE name = 'main') AS file,
		        (SELECT page_count FROM pragma_page_count) * (SELECT page_size FROM pragma_page_size) AS database_bytes,
		        (SELECT freelist_count FROM pragma_freelist_count) * (SELECT page_size FROM pragma_page_size) AS freelist_bytes",
	)
	.fetch_one(&mut *conn)
	.await?;
	Ok(Footprint {
		file: row.try_get::<Option<String>, _>("file")?.unwrap_or_default(),
		database_bytes: row.try_get("database_bytes")?,
		freelist_bytes: row.try_get("freelist_bytes")?,
	})
}

/// The name `dbstat` gives the schema table's own b-tree (page 1).
pub const SCHEMA_TABLE: &str = "sqlite_schema";

/// Every b-tree the file holds: each table and index with pages of its own
/// (a view or a virtual table has none), and the schema table itself.
///
/// # Errors
/// Any `sqlx` failure.
pub async fn btrees(conn: &mut SqliteConnection) -> Result<Vec<(Kind, String)>, sqlx::Error> {
	let rows = sqlx::query("SELECT type, name FROM sqlite_schema WHERE type IN ('table', 'index') AND rootpage > 0 ORDER BY type DESC, name")
		.fetch_all(&mut *conn)
		.await?;
	let mut btrees = vec![(Kind::Table, String::from(SCHEMA_TABLE))];
	for row in rows {
		let kind = if row.try_get::<String, _>("type")? == "index" { Kind::Index } else { Kind::Table };
		btrees.push((kind, row.try_get("name")?));
	}
	Ok(btrees)
}

/// One b-tree's bytes: every page it holds, overflow pages included, read
/// from `dbstat`. 0 for a name the file holds no pages for.
///
/// Reads every page of that b-tree once, so its cost is the b-tree's size
/// (`docs/storage.md`, "What the daily pass costs").
///
/// # Errors
/// Any `sqlx` failure; `no such table: dbstat` on a `SQLite` built without
/// `SQLITE_ENABLE_DBSTAT_VTAB` (this workspace's bundled one has it, and
/// `dbstat_is_in_this_build` says so).
pub async fn btree_bytes(conn: &mut SqliteConnection, name: &str) -> Result<i64, sqlx::Error> {
	sqlx::query_scalar("SELECT COALESCE(SUM(pgsize), 0) FROM dbstat WHERE name = ?1 AND aggregate = TRUE")
		.bind(name)
		.fetch_one(&mut *conn)
		.await
}

/// Whether home already holds a set for `day` (`YYYY-MM-DD`, UTC).
///
/// # Errors
/// Any `sqlx` failure.
pub async fn holds_day(conn: &mut SqliteConnection, day: &str) -> Result<bool, sqlx::Error> {
	let held: Option<i64> = sqlx::query_scalar("SELECT 1 FROM storage_sample WHERE day = ?1 LIMIT 1")
		.bind(day)
		.fetch_optional(&mut *conn)
		.await?;
	Ok(held.is_some())
}

/// What [`record`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
	/// The day's set is stored now: this many rows.
	Added(usize),
	/// The day was already held: nothing written.
	Unchanged,
}

/// Store `sizes` as `day`'s set, taken at `sampled_at` (ISO-8601 UTC, whose
/// date is `day`), in one transaction. A day already held is left as it was.
///
/// # Errors
/// Any `sqlx` failure, the table's checks included; nothing is written then.
pub async fn record(conn: &mut SqliteConnection, day: &str, sampled_at: &str, sizes: &[Size]) -> Result<Change, sqlx::Error> {
	let mut tx = conn.begin().await?;
	if holds_day(&mut tx, day).await? {
		return Ok(Change::Unchanged);
	}
	for size in sizes {
		sqlx::query("INSERT INTO storage_sample (day, sampled_at, kind, name, bytes) VALUES (?1, ?2, ?3, ?4, ?5)")
			.bind(day)
			.bind(sampled_at)
			.bind(size.kind.as_str())
			.bind(&size.name)
			.bind(size.bytes)
			.execute(&mut *tx)
			.await?;
	}
	tx.commit().await?;
	Ok(Change::Added(sizes.len()))
}

/// One day's set, by kind and then name.
///
/// # Errors
/// Any `sqlx` failure.
pub async fn day_sizes(conn: &mut SqliteConnection, day: &str) -> Result<Vec<Size>, sqlx::Error> {
	let rows = sqlx::query("SELECT kind, name, bytes FROM storage_sample WHERE day = ?1")
		.bind(day)
		.fetch_all(&mut *conn)
		.await?;
	let mut sizes = Vec::with_capacity(rows.len());
	for row in rows {
		// The table's CHECK admits only the kinds `Kind` names.
		if let Some(kind) = Kind::parse(&row.try_get::<String, _>("kind")?) {
			sizes.push(Size {
				kind,
				name: row.try_get("name")?,
				bytes: row.try_get("bytes")?,
			});
		}
	}
	sizes.sort_by(|a, b| a.kind.cmp(&b.kind).then_with(|| a.name.cmp(&b.name)));
	Ok(sizes)
}

/// The short window: the rate now, and what each table's share is read over.
pub const SHORT_WINDOW_DAYS: i64 = 7;
/// The long window: the rate the short one is held against.
pub const LONG_WINDOW_DAYS: i64 = 90;

/// The database's growth over one window, newest set against the oldest
/// set inside it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Window {
	/// The window asked for.
	pub days: i64,
	/// The days the held history covers of it: less than `days` until home
	/// has sampled that long, 0 with a single day.
	pub span_days: i64,
	/// The database file's change over the span, per day; `None` while the
	/// span is 0.
	pub bytes_per_day: Option<f64>,
}

impl Window {
	/// A window the history does not reach into yet.
	const fn unread(days: i64) -> Self {
		Self {
			days,
			span_days: 0,
			bytes_per_day: None,
		}
	}
}

/// How long until the disk fills at the short window's rate.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Runway {
	/// No rate yet (a single day held), or no free-space row.
	Unknown,
	/// The database did not grow over the short window.
	NotGrowing,
	/// Free space ÷ the short window's daily rate, a straight line.
	Days(f64),
}

/// One table's or index's change over the short window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Growth {
	pub kind: Kind,
	pub name: String,
	/// Bytes now less bytes at the window's start; a b-tree new inside the
	/// window grew by all it holds. Negative when it shrank.
	pub bytes: i64,
}

/// What the newest set says, and what the windows behind it say.
#[derive(Debug, Clone, PartialEq)]
pub struct Figures {
	pub day: String,
	pub sampled_at: String,
	/// The newest set.
	pub sizes: Vec<Size>,
	pub short: Window,
	pub long: Window,
	/// Each table's and index's change over the short window; empty while
	/// its span is 0.
	pub growth: Vec<Growth>,
	pub runway: Runway,
}

/// The newest set and the figures read against it, or `None` while home
/// holds no set.
///
/// Reads three sets at most (the newest, and the oldest inside each
/// window), each by a range read on the primary key: the cost does not grow
/// with the history.
///
/// # Errors
/// Any `sqlx` failure.
pub async fn figures(conn: &mut SqliteConnection) -> Result<Option<Figures>, sqlx::Error> {
	let Some(newest) = sqlx::query("SELECT day, sampled_at FROM storage_sample ORDER BY day DESC LIMIT 1")
		.fetch_optional(&mut *conn)
		.await?
	else {
		return Ok(None);
	};
	let day: String = newest.try_get("day")?;
	let sampled_at: String = newest.try_get("sampled_at")?;
	let sizes = day_sizes(conn, &day).await?;

	let (short, short_base) = window(conn, &day, &sizes, SHORT_WINDOW_DAYS).await?;
	let (long, _) = window(conn, &day, &sizes, LONG_WINDOW_DAYS).await?;
	let growth = short_base.map_or_else(Vec::new, |base| growth(&sizes, &base));
	let runway = runway(&sizes, short.bytes_per_day);
	Ok(Some(Figures {
		day,
		sampled_at,
		sizes,
		short,
		long,
		growth,
		runway,
	}))
}

/// The window of `days` ending at `day`, and the set it starts from when
/// that set is an earlier day's.
async fn window(conn: &mut SqliteConnection, day: &str, newest: &[Size], days: i64) -> Result<(Window, Option<Vec<Size>>), sqlx::Error> {
	let base = sqlx::query(
		"SELECT day, CAST(julianday(?1) - julianday(day) AS INTEGER) AS span
		 FROM storage_sample WHERE day >= date(?1, '-' || ?2 || ' days') ORDER BY day LIMIT 1",
	)
	.bind(day)
	.bind(days)
	.fetch_optional(&mut *conn)
	.await?;
	let Some(base) = base else {
		return Ok((Window::unread(days), None));
	};
	let span_days: i64 = base.try_get("span")?;
	if span_days <= 0 {
		return Ok((Window::unread(days), None));
	}
	let base_sizes = day_sizes(conn, &base.try_get::<String, _>("day")?).await?;
	let bytes_per_day = match (database_bytes(newest), database_bytes(&base_sizes)) {
		// Byte counts and day spans stay far inside f64's exact range.
		#[allow(clippy::cast_precision_loss)]
		(Some(now), Some(then)) => Some((now - then) as f64 / span_days as f64),
		_ => None,
	};
	Ok((Window { days, span_days, bytes_per_day }, Some(base_sizes)))
}

fn database_bytes(sizes: &[Size]) -> Option<i64> {
	sizes.iter().find(|size| size.kind == Kind::Database).map(|size| size.bytes)
}

fn growth(newest: &[Size], base: &[Size]) -> Vec<Growth> {
	newest
		.iter()
		.filter(|size| matches!(size.kind, Kind::Table | Kind::Index))
		.map(|size| {
			let then = base.iter().find(|old| old.kind == size.kind && old.name == size.name).map_or(0, |old| old.bytes);
			Growth {
				kind: size.kind,
				name: size.name.clone(),
				bytes: size.bytes - then,
			}
		})
		.collect()
}

fn runway(newest: &[Size], bytes_per_day: Option<f64>) -> Runway {
	let free = newest.iter().find(|size| size.kind == Kind::DiskFree).map(|size| size.bytes);
	match (free, bytes_per_day) {
		(Some(_), Some(rate)) if rate <= 0.0 => Runway::NotGrowing,
		// Free bytes stay far inside f64's exact range.
		#[allow(clippy::cast_precision_loss)]
		(Some(free), Some(rate)) => Runway::Days(free as f64 / rate),
		_ => Runway::Unknown,
	}
}

#[cfg(test)]
mod tests {
	use super::{btree_bytes, btrees, day_sizes, figures, footprint, holds_day, record, Change, Kind, Runway, Size, SCHEMA_TABLE};
	use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};

	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");

	async fn pool() -> SqlitePool {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();
		pool
	}

	fn table(name: &str, bytes: i64) -> Size {
		Size {
			kind: Kind::Table,
			name: String::from(name),
			bytes,
		}
	}

	/// A set as the sampler writes one.
	fn set(database: i64, free: i64, tables: &[(&str, i64)]) -> Vec<Size> {
		let mut sizes: Vec<Size> = tables.iter().map(|(name, bytes)| table(name, *bytes)).collect();
		sizes.push(Size::whole(Kind::Database, database));
		sizes.push(Size::whole(Kind::Wal, 0));
		sizes.push(Size::whole(Kind::DiskFree, free));
		sizes
	}

	fn at(day: &str) -> String {
		String::from(day) + "T03:00:00.000Z"
	}

	#[tokio::test]
	async fn dbstat_is_in_this_build() {
		// The bundled SQLite `libsqlite3-sys` compiles sets
		// SQLITE_ENABLE_DBSTAT_VTAB; were that to change, the sampler's
		// per-table bytes would be gone, and this says so first.
		let pool = pool().await;
		let mut conn = pool.acquire().await.unwrap();
		let btrees = btrees(&mut conn).await.unwrap();
		assert!(btrees.contains(&(Kind::Table, String::from("storage_sample"))));
		assert!(btrees.contains(&(Kind::Index, String::from("sqlite_autoindex_storage_sample_1"))));
		assert_eq!(btrees[0], (Kind::Table, String::from(SCHEMA_TABLE)));
		let page = footprint(&mut conn).await.unwrap().database_bytes / {
			let pages: i64 = sqlx::query_scalar("SELECT page_count FROM pragma_page_count").fetch_one(&mut *conn).await.unwrap();
			pages
		};
		// Every b-tree has a root page, the schema table's included, and
		// together they are every page no freelist holds.
		let mut total = 0;
		for (_, name) in &btrees {
			let bytes = btree_bytes(&mut conn, name).await.unwrap();
			assert!(bytes >= page, "{name} holds at least its root page");
			total += bytes;
		}
		let footprint = footprint(&mut conn).await.unwrap();
		assert_eq!(total + footprint.freelist_bytes, footprint.database_bytes);
		assert_eq!(btree_bytes(&mut conn, "no_such_table").await.unwrap(), 0);
	}

	#[tokio::test]
	async fn a_day_is_recorded_once_and_never_rewritten_or_deleted() {
		let pool = pool().await;
		let mut conn = pool.acquire().await.unwrap();
		let day = "2026-10-29";
		assert!(!holds_day(&mut conn, day).await.unwrap());
		let sizes = set(4096, 1000, &[("voice_note", 2048)]);
		assert_eq!(record(&mut conn, day, &at(day), &sizes).await.unwrap(), Change::Added(4));
		assert!(holds_day(&mut conn, day).await.unwrap());
		assert_eq!(record(&mut conn, day, &at(day), &set(1, 1, &[])).await.unwrap(), Change::Unchanged);
		assert_eq!(day_sizes(&mut conn, day).await.unwrap().len(), 4, "the second set added nothing");

		let update = sqlx::query("UPDATE storage_sample SET bytes = 0").execute(&mut *conn).await;
		assert!(update.unwrap_err().to_string().contains("append-only"), "the trigger refuses a rewrite");
		let delete = sqlx::query("DELETE FROM storage_sample").execute(&mut *conn).await;
		assert!(delete.unwrap_err().to_string().contains("append-only"), "the trigger refuses a delete");
		let replace = sqlx::query("INSERT OR REPLACE INTO storage_sample (day, sampled_at, kind, name, bytes) VALUES (?1, ?2, 'table', 'voice_note', 0)")
			.bind(day)
			.bind(at(day))
			.execute(&mut *conn)
			.await;
		assert!(replace.unwrap_err().to_string().contains("append-only"), "the trigger refuses a replace");
		let mut kept = sizes;
		kept.sort_by(|a, b| a.kind.cmp(&b.kind).then_with(|| a.name.cmp(&b.name)));
		assert_eq!(day_sizes(&mut conn, day).await.unwrap(), kept, "every row as it was written");
	}

	#[tokio::test]
	async fn a_set_the_checks_refuse_writes_nothing() {
		let pool = pool().await;
		let mut conn = pool.acquire().await.unwrap();
		let day = "2026-10-29";
		let mut sizes = set(4096, 1000, &[("voice_note", 2048)]);
		sizes.push(table("negative", -1));
		assert!(record(&mut conn, day, &at(day), &sizes).await.is_err());
		assert!(!holds_day(&mut conn, day).await.unwrap(), "all of a set or none of it");
		assert!(
			record(&mut conn, day, "2026-10-28T23:59:59.000Z", &set(1, 1, &[])).await.is_err(),
			"sampled_at is on its day"
		);
		for day in ["2026-13-01", "2026-02-30", "20261029x0"] {
			assert!(record(&mut conn, day, &at(day), &set(1, 1, &[])).await.is_err(), "{day} is not a day");
		}
	}

	#[tokio::test]
	async fn no_history_has_no_figures_and_one_day_has_no_rate() {
		let pool = pool().await;
		let mut conn = pool.acquire().await.unwrap();
		assert_eq!(figures(&mut conn).await.unwrap(), None);
		record(&mut conn, "2026-10-29", &at("2026-10-29"), &set(4096, 1000, &[("voice_note", 2048)])).await.unwrap();
		let figures = figures(&mut conn).await.unwrap().unwrap();
		assert_eq!(figures.day, "2026-10-29");
		assert_eq!(figures.sizes.len(), 4);
		assert_eq!((figures.short.span_days, figures.short.bytes_per_day), (0, None));
		assert_eq!((figures.long.span_days, figures.long.bytes_per_day), (0, None));
		assert!(figures.growth.is_empty());
		assert_eq!(figures.runway, Runway::Unknown);
	}

	#[tokio::test]
	async fn each_window_reads_from_the_oldest_set_inside_it() {
		let pool = pool().await;
		let mut conn = pool.acquire().await.unwrap();
		// 100 days of history, with gaps: the host was down some days.
		for (day, database, voice, jobs) in [
			("2026-07-20", 10_000, 1_000, 500), // outside the 90-day window
			("2026-08-01", 20_000, 2_000, 500), // the long window's start, 89 days back
			("2026-10-20", 90_000, 50_000, 900),
			("2026-10-23", 100_000, 60_000, 1_000), // the short window's start, 6 days back
			("2026-10-29", 160_000, 110_000, 1_000),
		] {
			record(&mut conn, day, &at(day), &set(database, 1_200_000, &[("voice_note", voice), ("jobs", jobs)]))
				.await
				.unwrap();
		}
		let figures = figures(&mut conn).await.unwrap().unwrap();
		assert_eq!(figures.day, "2026-10-29");
		assert_eq!((figures.short.days, figures.short.span_days), (7, 6));
		assert_eq!(figures.short.bytes_per_day, Some(10_000.0));
		assert_eq!((figures.long.days, figures.long.span_days), (90, 89));
		assert_eq!(figures.long.bytes_per_day, Some(140_000.0 / 89.0));
		let growth: Vec<(&str, i64)> = figures.growth.iter().map(|g| (g.name.as_str(), g.bytes)).collect();
		assert_eq!(growth, [("jobs", 0), ("voice_note", 50_000)]);
		assert_eq!(figures.runway, Runway::Days(120.0));
	}

	#[tokio::test]
	async fn a_database_that_did_not_grow_has_no_runway_to_count() {
		let pool = pool().await;
		let mut conn = pool.acquire().await.unwrap();
		for (day, database, new_table) in [("2026-10-28", 5_000, None), ("2026-10-29", 5_000, Some(("game_feed_play", 300)))] {
			let mut sizes = set(database, 1_000, &[("voice_note", 4_000)]);
			sizes.extend(new_table.map(|(name, bytes)| table(name, bytes)));
			record(&mut conn, day, &at(day), &sizes).await.unwrap();
		}
		let figures = figures(&mut conn).await.unwrap().unwrap();
		assert_eq!(figures.short.bytes_per_day, Some(0.0));
		assert_eq!(figures.runway, Runway::NotGrowing);
		let growth: Vec<(&str, i64)> = figures.growth.iter().map(|g| (g.name.as_str(), g.bytes)).collect();
		assert_eq!(growth, [("game_feed_play", 300), ("voice_note", 0)], "a table new inside the window grew by all it holds");
	}
}

//! A deploy applies its migrations, with a backup first (AS16, #563).
//!
//! Before this, applying migrations was an operator step nobody's deploy
//! took: on 2026-10-30 home pulled an image five migrations ahead of its
//! database (#559), and the game feed, game bites and the storage sample
//! stayed broken until someone ran `sqlx migrate run` by hand. Now compose's
//! one-shot `migrate` service (`infra/compose/file_host.yml`) runs the
//! `migrate` binary from the same image before `file-host` starts, and
//! `make up` runs it once more before compose replaces the running
//! container, so a migration that fails leaves the old one serving.
//!
//! [`deploy`] does, in order:
//!
//! 1. **Refuses a database it cannot trust.** No file at the path is a volume
//!    that is not mounted, not a new host, unless `init` says it is; so is a
//!    file with no `_sqlx_migrations` table. A migration recorded as failed,
//!    or edited after it was applied, is refused with [`Drift`]'s own words:
//!    `sqlx` would refuse it too, after the backup.
//! 2. **Nothing pending, nothing done.** Compose runs this on every
//!    `docker compose up`, so an up-to-date database (or one ahead of this
//!    build, the ordinary state after a rollback: `schema`) takes no backup.
//! 3. **Backs up, and verifies the backup.** `VACUUM INTO` writes a
//!    consistent copy while the old `file_host` still reads and writes, to
//!    `<database dir>/backups/<stem>-<UTC time>-before-<first pending
//!    version>.db`. The copy is opened read-only and must pass
//!    `PRAGMA integrity_check` and hold every migration the live database
//!    had applied. Every migrating run is held to this, not only one whose
//!    migrations drop or rebuild a table: telling those apart from the SQL
//!    is a guess, and the one that dropped a table (`drop_sharing_choice`)
//!    looked like any other file.
//! 4. **Applies** [`MIGRATOR`](crate::schema::MIGRATOR)'s pending migrations,
//!    each in its own transaction, then reads the drift back: anything
//!    still pending is a failure.
//! 5. **Prunes** this binary's own backups beyond the newest `keep`, only
//!    after a run that left the database current. A failed run removes
//!    nothing.
//!
//! Any failure is an `Err`, and the binary exits non-zero with it, so
//! compose does not start the new `file_host` on a schema half applied.
//! The migrations before the one that failed stay applied; a database ahead
//! of its binary is one the old `file_host` serves (`schema`).

use crate::schema::{drift, Drift};
use chrono::{DateTime, Utc};
use sqlx::migrate::{MigrateError, Migrator};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::str::FromStr;

/// How many of its own backups a run keeps, by default: the newest five.
///
/// A backup is taken only by a run that has something to apply, so five is
/// the last five deploys that changed the schema, not the last five days.
/// Each is a whole copy of the database on the same volume, so this is also
/// a bound on the space they take: five times the database.
pub const DEFAULT_KEEP_BACKUPS: u16 = 5;

/// The directory beside the database the backups go to, by default.
pub const BACKUP_DIR: &str = "backups";

/// What one run is asked to do.
#[derive(Debug, Clone)]
pub struct Options {
	/// The database, as `sqlite://<path>`.
	pub database_url: String,
	/// Where backups go; `<database dir>/backups` when `None`.
	pub backup_dir: Option<PathBuf>,
	/// How many of this binary's backups to keep, at least one.
	pub keep: usize,
	/// A new host: create the database if no file is there, and migrate one
	/// that has never been migrated.
	pub init: bool,
}

/// What a run did, when it succeeded.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
	/// Nothing was pending: no backup, nothing applied.
	Current {
		/// Backups removed beyond `keep`.
		pruned: Vec<PathBuf>,
	},
	/// Migrations were applied.
	Applied {
		/// The verified copy taken first; `None` only for a database `init`
		/// had just created, which held nothing to copy.
		backup: Option<PathBuf>,
		/// What was applied, oldest first, as `"<version> <description>"`.
		applied: Vec<String>,
		/// Backups removed beyond `keep`.
		pruned: Vec<PathBuf>,
	},
}

/// Why a run refused or failed. Every one leaves the database as the last
/// successful migration left it.
#[derive(Debug, thiserror::Error)]
pub enum Refused {
	#[error("no database at {0}: is its volume mounted? A first deploy on a new host passes --init")]
	NoDatabase(PathBuf),
	#[error("{0} has never been migrated (no _sqlx_migrations table), so it is empty or the wrong file: is its volume mounted? A first deploy on a new host passes --init")]
	NeverMigrated(PathBuf),
	#[error("refusing to migrate: {0}")]
	Drift(Drift),
	#[error("could not open {url}: {source}")]
	Open { url: String, source: sqlx::Error },
	#[error("could not read the applied migrations: {0}")]
	Read(sqlx::Error),
	#[error("could not make {path}: {source}")]
	BackupDir { path: PathBuf, source: std::io::Error },
	#[error("{0} already exists; not overwriting a backup")]
	BackupExists(PathBuf),
	#[error("backing up to {path} failed, so nothing was applied: {source}")]
	Backup { path: PathBuf, source: sqlx::Error },
	#[error("the backup {path} does not verify, so nothing was applied (it was removed): {reason}")]
	Unverified { path: PathBuf, reason: String },
	#[error("applying migrations failed: {source}. The ones before it stay applied; {}", backup_note(.backup.as_deref()))]
	Apply { backup: Option<PathBuf>, source: MigrateError },
	#[error("after applying, the database is still not current: {0}")]
	StillBehind(Drift),
	#[error("could not prune old backups in {path}: {source}")]
	Prune { path: PathBuf, source: std::io::Error },
}

fn backup_note(backup: Option<&Path>) -> String {
	backup.map_or_else(
		|| String::from("there was no database to back up"),
		|path| {
			let mut note = String::from("the database as it was before this run is ");
			note.push_str(&path.display().to_string());
			note
		},
	)
}

/// Back up `options.database_url`'s database and apply `migrator`'s pending
/// migrations to it, as the module doc says.
///
/// `migrator` is [`crate::schema::MIGRATOR`] in the binary; a test passes
/// its own.
///
/// # Errors
///
/// Every [`Refused`]: the database is missing, never migrated or drifted;
/// the backup could not be made or does not verify; a migration failed; or
/// the database is still behind afterwards.
pub async fn deploy(options: &Options, migrator: &Migrator) -> Result<Outcome, Refused> {
	deploy_at(options, migrator, Utc::now()).await
}

async fn deploy_at(options: &Options, migrator: &Migrator, now: DateTime<Utc>) -> Result<Outcome, Refused> {
	let open = |source| Refused::Open {
		url: options.database_url.clone(),
		source,
	};
	let connect = SqliteConnectOptions::from_str(&options.database_url).map_err(open)?;
	let path = connect.get_filename().to_path_buf();
	let existed = path.exists();
	if !existed && !options.init {
		return Err(Refused::NoDatabase(path));
	}

	let connect = crate::db::connect_options(&options.database_url).map_err(open)?.create_if_missing(options.init);
	let pool = SqlitePoolOptions::new().max_connections(1).connect_with(connect).await.map_err(open)?;
	let result = migrate(&pool, &path, existed, options, migrator, now).await;
	pool.close().await;
	result
}

async fn migrate(pool: &SqlitePool, path: &Path, existed: bool, options: &Options, migrator: &Migrator, now: DateTime<Utc>) -> Result<Outcome, Refused> {
	let before = drift(pool, migrator).await.map_err(Refused::Read)?;
	if before.never_migrated && !options.init {
		return Err(Refused::NeverMigrated(path.to_path_buf()));
	}
	if !before.failed.is_empty() || !before.modified.is_empty() {
		return Err(Refused::Drift(before));
	}

	let backup_dir = options
		.backup_dir
		.clone()
		.unwrap_or_else(|| path.parent().unwrap_or_else(|| Path::new(".")).join(BACKUP_DIR));
	let stem = path.file_stem().map_or_else(|| String::from("database"), |stem| stem.to_string_lossy().into_owned());

	if before.pending.is_empty() {
		let pruned = prune(&backup_dir, &stem, options.keep)?;
		return Ok(Outcome::Current { pruned });
	}

	let backup = if existed {
		let first = before.pending.first().and_then(|name| name.split(' ').next()).unwrap_or("unknown");
		Some(back_up(pool, &backup_dir, &stem, first, now).await?)
	} else {
		None
	};

	if let Err(source) = migrator.run(pool).await {
		return Err(Refused::Apply { backup, source });
	}

	let after = drift(pool, migrator).await.map_err(Refused::Read)?;
	if !after.is_current() {
		return Err(Refused::StillBehind(after));
	}

	let pruned = prune(&backup_dir, &stem, options.keep)?;
	Ok(Outcome::Applied {
		backup,
		applied: before.pending,
		pruned,
	})
}

/// The file name a backup taken at `now`, before `first_pending`, gets.
/// Fixed-width, so names sort as the times they were taken.
fn backup_name(stem: &str, first_pending: &str, now: DateTime<Utc>) -> String {
	let mut name = String::from(stem);
	let _ = write!(name, "-{}-before-{first_pending}.db", now.format("%Y%m%dT%H%M%SZ"));
	name
}

/// Is `name` one of this binary's backups of the database `stem`?
fn is_backup_of(name: &str, stem: &str) -> bool {
	name
		.strip_prefix(stem)
		.and_then(|rest| rest.strip_prefix('-'))
		.and_then(|rest| rest.strip_suffix(".db"))
		.and_then(|rest| rest.split_once("-before-"))
		.is_some_and(|(time, version)| time.len() == 16 && time.ends_with('Z') && !version.is_empty() && version.bytes().all(|b| b.is_ascii_digit()))
}

async fn back_up(pool: &SqlitePool, dir: &Path, stem: &str, first_pending: &str, now: DateTime<Utc>) -> Result<PathBuf, Refused> {
	std::fs::create_dir_all(dir).map_err(|source| Refused::BackupDir { path: dir.to_path_buf(), source })?;
	let path = dir.join(backup_name(stem, first_pending, now));
	if path.exists() {
		return Err(Refused::BackupExists(path));
	}

	if let Err(source) = sqlx::query("VACUUM INTO ?").bind(path.to_string_lossy().into_owned()).execute(pool).await {
		let _ = std::fs::remove_file(&path);
		return Err(Refused::Backup { path, source });
	}

	if let Err(reason) = verify(pool, &path).await {
		let _ = std::fs::remove_file(&path);
		return Err(Refused::Unverified { path, reason });
	}
	Ok(path)
}

/// Open `backup` read-only: it must pass `PRAGMA integrity_check` and hold
/// as many applied migrations as the live database.
async fn verify(live: &SqlitePool, backup: &Path) -> Result<(), String> {
	let applied = "SELECT COUNT(*) FROM _sqlx_migrations WHERE success";
	let has_table = "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = '_sqlx_migrations')";

	let copy = SqlitePoolOptions::new()
		.max_connections(1)
		.connect_with(SqliteConnectOptions::new().filename(backup).read_only(true))
		.await
		.map_err(|err| err.to_string())?;
	let checked = async {
		let integrity: Vec<String> = sqlx::query_scalar("PRAGMA integrity_check").fetch_all(&copy).await.map_err(|err| err.to_string())?;
		if integrity != ["ok"] {
			return Err(integrity.join("; "));
		}
		let live_has: bool = sqlx::query_scalar(has_table).fetch_one(live).await.map_err(|err| err.to_string())?;
		if !live_has {
			return Ok(());
		}
		let live_count: i64 = sqlx::query_scalar(applied).fetch_one(live).await.map_err(|err| err.to_string())?;
		let copy_count: i64 = sqlx::query_scalar(applied).fetch_one(&copy).await.map_err(|err| err.to_string())?;
		if live_count == copy_count {
			Ok(())
		} else {
			let mut reason = String::from("it holds ");
			let _ = write!(reason, "{copy_count} applied migrations and the database {live_count}");
			Err(reason)
		}
	}
	.await;
	copy.close().await;
	checked
}

/// Remove this binary's backups of `stem` in `dir` beyond the newest `keep`.
/// Anything else in `dir` is left alone, and so is a missing `dir`.
fn prune(dir: &Path, stem: &str, keep: usize) -> Result<Vec<PathBuf>, Refused> {
	let failed = |source| Refused::Prune { path: dir.to_path_buf(), source };
	let entries = match std::fs::read_dir(dir) {
		Ok(entries) => entries,
		Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
		Err(err) => return Err(failed(err)),
	};
	let mut names = Vec::new();
	for entry in entries {
		let entry = entry.map_err(failed)?;
		if let Some(name) = entry.file_name().to_str() {
			if is_backup_of(name, stem) && entry.file_type().map_err(failed)?.is_file() {
				names.push(name.to_owned());
			}
		}
	}
	names.sort_unstable_by(|a, b| b.cmp(a));
	let mut pruned = Vec::new();
	for name in names.into_iter().skip(keep.max(1)) {
		let path = dir.join(name);
		std::fs::remove_file(&path).map_err(failed)?;
		pruned.push(path);
	}
	Ok(pruned)
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::schema::MIGRATOR;
	use chrono::TimeZone;

	fn options(dir: &Path) -> Options {
		let mut url = String::from("sqlite://");
		url.push_str(&dir.join("hopium.db").to_string_lossy());
		Options {
			database_url: url,
			backup_dir: None,
			keep: usize::from(DEFAULT_KEEP_BACKUPS),
			init: false,
		}
	}

	fn up_versions() -> Vec<i64> {
		MIGRATOR.iter().filter(|m| !m.migration_type.is_down_migration()).map(|m| m.version).collect()
	}

	fn at(second: u32) -> DateTime<Utc> {
		Utc.with_ymd_and_hms(2026, 10, 31, 12, 0, second).unwrap()
	}

	async fn open(options: &Options) -> SqlitePool {
		SqlitePoolOptions::new()
			.max_connections(1)
			.connect_with(crate::db::connect_options(&options.database_url).unwrap().create_if_missing(true))
			.await
			.unwrap()
	}

	/// A database `behind` migrations behind this build, as home's was on
	/// 2026-10-30: migrated by the real history, then the newest `behind`
	/// undone by their own down migrations.
	async fn database_behind(options: &Options, behind: usize) -> SqlitePool {
		let pool = open(options).await;
		MIGRATOR.run(&pool).await.unwrap();
		let versions = up_versions();
		MIGRATOR.undo(&pool, versions[versions.len() - 1 - behind]).await.unwrap();
		pool
	}

	fn backups(dir: &Path) -> Vec<String> {
		let mut names: Vec<String> = std::fs::read_dir(dir.join(BACKUP_DIR))
			.map(|entries| entries.map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned()).collect())
			.unwrap_or_default();
		names.sort();
		names
	}

	/// Done means (#563): a database N migrations behind is current after one
	/// run, with no manual step, and a verified copy of it as it was is kept.
	#[tokio::test]
	async fn a_database_behind_its_build_is_backed_up_then_brought_current() {
		let dir = tempfile::tempdir().unwrap();
		let options = options(dir.path());
		let pool = database_behind(&options, 5).await;
		assert_eq!(drift(&pool, &MIGRATOR).await.unwrap().pending.len(), 5);
		pool.close().await;

		let outcome = deploy_at(&options, &MIGRATOR, at(0)).await.unwrap();

		let Outcome::Applied { backup, applied, pruned } = outcome else {
			panic!("expected migrations applied, got {outcome:?}");
		};
		assert_eq!(applied.len(), 5, "{applied:?}");
		assert!(pruned.is_empty());
		let backup = backup.unwrap();
		let first_pending = up_versions()[up_versions().len() - 5].to_string();
		assert!(backup.ends_with(backup_name("hopium", &first_pending, at(0))), "{}", backup.display());

		let pool = open(&options).await;
		assert!(drift(&pool, &MIGRATOR).await.unwrap().is_current());
		let copy = SqlitePoolOptions::new()
			.max_connections(1)
			.connect_with(SqliteConnectOptions::new().filename(&backup).read_only(true))
			.await
			.unwrap();
		assert_eq!(drift(&copy, &MIGRATOR).await.unwrap().pending.len(), 5, "the backup is the database as it was");
	}

	/// Compose runs `migrate` on every `up`: with nothing pending it takes no
	/// backup, and a database ahead of this build (a rollback) is left alone.
	#[tokio::test]
	async fn nothing_pending_takes_no_backup_even_ahead_of_the_build() {
		let dir = tempfile::tempdir().unwrap();
		let options = options(dir.path());
		let pool = open(&options).await;
		MIGRATOR.run(&pool).await.unwrap();
		sqlx::query("INSERT INTO _sqlx_migrations (version, description, success, checksum, execution_time) VALUES (99991231000000, 'from the future', TRUE, X'00', 0)")
			.execute(&pool)
			.await
			.unwrap();
		pool.close().await;

		assert_eq!(deploy_at(&options, &MIGRATOR, at(0)).await.unwrap(), Outcome::Current { pruned: Vec::new() });
		assert!(backups(dir.path()).is_empty());
	}

	/// An unmounted volume is a missing file (or a fresh empty one), not a new
	/// host: refused, and nothing is created, unless `init` says otherwise.
	#[tokio::test]
	async fn a_missing_or_never_migrated_database_is_refused_without_init() {
		let dir = tempfile::tempdir().unwrap();
		let mut options = options(dir.path());

		let refused = deploy_at(&options, &MIGRATOR, at(0)).await.unwrap_err();
		assert!(matches!(refused, Refused::NoDatabase(_)), "{refused}");
		assert!(refused.to_string().contains("volume mounted"), "{refused}");
		assert!(!dir.path().join("hopium.db").exists(), "refusing creates nothing");

		open(&options).await.close().await;
		let refused = deploy_at(&options, &MIGRATOR, at(0)).await.unwrap_err();
		assert!(matches!(refused, Refused::NeverMigrated(_)), "{refused}");

		std::fs::remove_file(dir.path().join("hopium.db")).unwrap();
		options.init = true;
		let Outcome::Applied { backup, applied, .. } = deploy_at(&options, &MIGRATOR, at(0)).await.unwrap() else {
			panic!("init applies every migration");
		};
		assert_eq!(backup, None, "a database init just made holds nothing to back up");
		assert_eq!(applied.len(), up_versions().len());
	}

	/// A migration recorded as edited after it was applied is refused before
	/// any backup is taken, in `Drift`'s words.
	#[tokio::test]
	async fn a_drifted_database_is_refused_before_a_backup() {
		let dir = tempfile::tempdir().unwrap();
		let options = options(dir.path());
		let pool = database_behind(&options, 1).await;
		sqlx::query("UPDATE _sqlx_migrations SET checksum = X'00' WHERE version = ?")
			.bind(up_versions()[0])
			.execute(&pool)
			.await
			.unwrap();
		pool.close().await;

		let refused = deploy_at(&options, &MIGRATOR, at(0)).await.unwrap_err();
		assert!(matches!(refused, Refused::Drift(_)), "{refused}");
		assert!(refused.to_string().contains("modified after apply"), "{refused}");
		assert!(backups(dir.path()).is_empty());
	}

	/// Done means (#563): a failed migration says why, keeps its backup, and
	/// leaves the database as the last good migration left it.
	#[tokio::test]
	async fn a_failed_migration_says_why_and_keeps_the_backup() {
		let dir = tempfile::tempdir().unwrap();
		let source = dir.path().join("migrations");
		std::fs::create_dir(&source).unwrap();
		std::fs::write(source.join("1_first.sql"), "CREATE TABLE kept (id INTEGER PRIMARY KEY);").unwrap();
		let first = Migrator::new(source.as_path()).await.unwrap();
		std::fs::write(source.join("2_second.sql"), "CREATE TABLE fine (id INTEGER PRIMARY KEY);").unwrap();
		std::fs::write(source.join("3_broken.sql"), "INSERT INTO nowhere VALUES (1);").unwrap();
		let all = Migrator::new(source.as_path()).await.unwrap();

		let options = options(dir.path());
		let pool = open(&options).await;
		first.run(&pool).await.unwrap();
		sqlx::query("INSERT INTO kept (id) VALUES (7)").execute(&pool).await.unwrap();
		pool.close().await;

		let refused = deploy_at(&options, &all, at(0)).await.unwrap_err();
		let message = refused.to_string();
		assert!(matches!(refused, Refused::Apply { backup: Some(_), .. }), "{message}");
		assert!(message.contains("nowhere") && message.contains("before-2.db"), "{message}");
		assert_eq!(backups(dir.path()), ["hopium-20261031T120000Z-before-2.db"]);

		let pool = open(&options).await;
		let left = drift(&pool, &all).await.unwrap();
		assert_eq!(left.pending, ["3 broken"], "the migration before the broken one stays applied");
		let kept: i64 = sqlx::query_scalar("SELECT id FROM kept").fetch_one(&pool).await.unwrap();
		assert_eq!(kept, 7);
	}

	/// Only this binary's backups count toward `keep`, newest kept, and a run
	/// with nothing pending prunes too, so lowering `keep` takes effect.
	#[tokio::test]
	async fn backups_beyond_keep_are_pruned_newest_first() {
		let dir = tempfile::tempdir().unwrap();
		let mut options = options(dir.path());
		options.keep = 2;
		let pool = database_behind(&options, 1).await;
		pool.close().await;
		let backup_dir = dir.path().join(BACKUP_DIR);
		std::fs::create_dir(&backup_dir).unwrap();
		for second in 1..=3 {
			std::fs::write(backup_dir.join(backup_name("hopium", "1", at(second))), "old").unwrap();
		}
		std::fs::write(backup_dir.join("hopium-by-hand.db"), "the owner's").unwrap();
		std::fs::write(backup_dir.join(backup_name("other", "1", at(1))), "another database's").unwrap();

		let Outcome::Applied { pruned, .. } = deploy_at(&options, &MIGRATOR, at(9)).await.unwrap() else {
			panic!("one migration was pending");
		};
		assert_eq!(pruned.len(), 2, "{pruned:?}");
		let newest = backup_name("hopium", &up_versions()[up_versions().len() - 1].to_string(), at(9));
		assert_eq!(
			backups(dir.path()),
			[
				String::from("hopium-20261031T120003Z-before-1.db"),
				newest,
				String::from("hopium-by-hand.db"),
				String::from("other-20261031T120001Z-before-1.db"),
			]
		);

		options.keep = 1;
		let Outcome::Current { pruned } = deploy_at(&options, &MIGRATOR, at(10)).await.unwrap() else {
			panic!("nothing was pending");
		};
		assert_eq!(pruned, [backup_dir.join("hopium-20261031T120003Z-before-1.db")]);
	}

	#[test]
	fn only_names_this_binary_writes_are_its_backups() {
		assert!(is_backup_of(&backup_name("hopium", "20261031000400", at(0)), "hopium"));
		assert!(!is_backup_of("hopium-20261031T120000Z-before-.db", "hopium"));
		assert!(!is_backup_of("hopium-copy.db", "hopium"));
		assert!(!is_backup_of("hopium.db", "hopium"));
		assert!(!is_backup_of(&backup_name("hopium-old", "1", at(0)), "hopium"));
	}
}

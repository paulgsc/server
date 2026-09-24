//! Does the database this process opened carry every migration this binary
//! was compiled against?
//!
//! `cargo check` and CI can't answer that. Both compile `sqlx::query!`
//! against a database *they* just migrated, so a query that names a column
//! only a newer migration adds type-checks fine — and then fails at runtime
//! against a local `SQLite` file nobody ran `sqlx migrate run` on. Before
//! this, that surfaced as whatever happened to query the missing table
//! first: the waker's `no such table: engagement_gate`, one `error!` line
//! per tick, and a LOOPS panel that said STALLED without saying why.
//!
//! [`MIGRATOR`] embeds `migrations/` at compile time — the same `.up.sql`
//! set CI migrates its scratch database with — so the binary knows exactly
//! which versions it expects. [`drift`] compares that list with
//! `_sqlx_migrations` in the live database, read-only: it never creates the
//! bookkeeping table or applies anything. Applying migrations stays an
//! operator step; this only makes forgetting it visible, by name, in
//! `/ready` (`handlers::readiness`) and at startup (`main.rs`).

use sqlx::migrate::Migrator;
use sqlx::{Row, SqlitePool};
use std::collections::HashMap;
use std::fmt;

/// Every migration under the workspace's `migrations/`, embedded at compile
/// time. `build.rs` reruns on changes there, so this can't go stale against
/// the tree it was built from.
pub static MIGRATOR: Migrator = sqlx::migrate!("../../../migrations");

/// How the live schema differs from what [`MIGRATOR`] expects.
///
/// A database *ahead* of the binary — versions applied that this build
/// doesn't know about, the ordinary state during an image rollback — is
/// deliberately not drift: every migration in this repo so far is additive,
/// and refusing readiness on it would turn every rollback into an outage.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Drift {
	/// Expected but not applied, oldest first — what `sqlx migrate run`
	/// would apply. Formatted `"<version> <description>"`.
	pub pending: Vec<String>,
	/// Applied rows with `success = false`: a migration that started and
	/// didn't finish. `sqlx migrate run` refuses to continue past one.
	pub failed: Vec<i64>,
	/// Applied, but the file changed since. The schema is whatever the old
	/// file produced, not what this build's queries were checked against.
	pub modified: Vec<i64>,
}

impl Drift {
	#[must_use]
	pub const fn is_current(&self) -> bool {
		self.pending.is_empty() && self.failed.is_empty() && self.modified.is_empty()
	}
}

impl fmt::Display for Drift {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		if self.is_current() {
			return f.write_str("current");
		}
		if !self.pending.is_empty() {
			write!(f, "{} pending ({}); run `sqlx migrate run`", self.pending.len(), self.pending.join(", "))?;
		}
		if !self.failed.is_empty() {
			if !self.pending.is_empty() {
				f.write_str("; ")?;
			}
			write!(f, "failed: {:?}", self.failed)?;
		}
		if !self.modified.is_empty() {
			if !self.pending.is_empty() || !self.failed.is_empty() {
				f.write_str("; ")?;
			}
			write!(f, "modified after apply: {:?}", self.modified)?;
		}
		Ok(())
	}
}

/// Compare `migrator`'s up-migrations with what `pool`'s database has applied.
///
/// A database with no `_sqlx_migrations` table at all — a file
/// `create_if_missing` just made, or one never migrated — has everything
/// pending, not an error: that is precisely the state this exists to name.
///
/// # Errors
/// Only if the database can't be read at all; that is the `sqlite`
/// dependency's failure to report, not this one's.
pub async fn drift(pool: &SqlitePool, migrator: &Migrator) -> Result<Drift, sqlx::Error> {
	let has_table: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = '_sqlx_migrations')")
		.fetch_one(pool)
		.await?;

	let applied: HashMap<i64, (bool, Vec<u8>)> = if has_table {
		sqlx::query("SELECT version, success, checksum FROM _sqlx_migrations")
			.fetch_all(pool)
			.await?
			.into_iter()
			.map(|row| (row.get("version"), (row.get("success"), row.get("checksum"))))
			.collect()
	} else {
		HashMap::new()
	};

	let mut drift = Drift::default();
	for migration in migrator.iter().filter(|m| !m.migration_type.is_down_migration()) {
		match applied.get(&migration.version) {
			None => {
				let mut name = migration.version.to_string();
				name.push(' ');
				name.push_str(&migration.description);
				drift.pending.push(name);
			}
			Some((false, _)) => drift.failed.push(migration.version),
			Some((true, checksum)) if checksum.as_slice() != &*migration.checksum => drift.modified.push(migration.version),
			Some(_) => {}
		}
	}
	Ok(drift)
}

#[cfg(test)]
mod tests {
	use super::*;
	use sqlx::sqlite::SqlitePoolOptions;

	// One connection: `:memory:` is private to the connection that opened
	// it, so a second pooled connection would see an empty database.
	async fn memory_pool() -> SqlitePool {
		SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap()
	}

	fn up_versions() -> Vec<i64> {
		MIGRATOR.iter().filter(|m| !m.migration_type.is_down_migration()).map(|m| m.version).collect()
	}

	#[tokio::test]
	async fn a_never_migrated_database_has_every_migration_pending() {
		let pool = memory_pool().await;
		let drift = drift(&pool, &MIGRATOR).await.unwrap();

		assert!(!drift.is_current());
		assert_eq!(drift.pending.len(), up_versions().len());
		assert!(drift.failed.is_empty() && drift.modified.is_empty());
	}

	#[tokio::test]
	async fn a_fully_migrated_database_is_current() {
		let pool = memory_pool().await;
		MIGRATOR.run(&pool).await.unwrap();

		let drift = drift(&pool, &MIGRATOR).await.unwrap();
		assert!(drift.is_current(), "{drift}");
		assert_eq!(drift.to_string(), "current");
	}

	/// The case this module exists for: the tree gained a migration, the
	/// local database didn't. The message names it and says what to run.
	#[tokio::test]
	async fn a_database_one_migration_behind_names_that_migration() {
		let pool = memory_pool().await;
		MIGRATOR.run(&pool).await.unwrap();
		let latest = *up_versions().last().unwrap();
		sqlx::query("DELETE FROM _sqlx_migrations WHERE version = ?").bind(latest).execute(&pool).await.unwrap();

		let drift = drift(&pool, &MIGRATOR).await.unwrap();
		assert_eq!(drift.pending.len(), 1);
		assert!(drift.pending[0].starts_with(&latest.to_string()), "{:?}", drift.pending);
		let message = drift.to_string();
		assert!(message.contains("1 pending") && message.contains("sqlx migrate run"), "{message}");
	}

	#[tokio::test]
	async fn an_unfinished_migration_is_failed_not_pending() {
		let pool = memory_pool().await;
		MIGRATOR.run(&pool).await.unwrap();
		let first = up_versions()[0];
		sqlx::query("UPDATE _sqlx_migrations SET success = FALSE WHERE version = ?")
			.bind(first)
			.execute(&pool)
			.await
			.unwrap();

		let drift = drift(&pool, &MIGRATOR).await.unwrap();
		assert_eq!(drift.failed, vec![first]);
		assert!(drift.pending.is_empty());
	}

	#[tokio::test]
	async fn a_migration_edited_after_apply_is_modified() {
		let pool = memory_pool().await;
		MIGRATOR.run(&pool).await.unwrap();
		let first = up_versions()[0];
		sqlx::query("UPDATE _sqlx_migrations SET checksum = X'00' WHERE version = ?")
			.bind(first)
			.execute(&pool)
			.await
			.unwrap();

		let drift = drift(&pool, &MIGRATOR).await.unwrap();
		assert_eq!(drift.modified, vec![first]);
	}

	#[test]
	fn every_kind_of_drift_is_named_once_and_separated() {
		let drift = Drift {
			pending: vec!["3 c".to_owned()],
			failed: vec![1],
			modified: vec![2],
		};
		assert_eq!(drift.to_string(), "1 pending (3 c); run `sqlx migrate run`; failed: [1]; modified after apply: [2]");
		let drift = Drift {
			modified: vec![2],
			..Drift::default()
		};
		assert_eq!(drift.to_string(), "modified after apply: [2]");
	}

	/// An older binary on a newer database (a rollback) is not drift.
	#[tokio::test]
	async fn a_database_ahead_of_the_binary_is_current() {
		let pool = memory_pool().await;
		MIGRATOR.run(&pool).await.unwrap();
		sqlx::query("INSERT INTO _sqlx_migrations (version, description, success, checksum, execution_time) VALUES (99991231000000, 'from the future', TRUE, X'00', 0)")
			.execute(&pool)
			.await
			.unwrap();

		assert!(drift(&pool, &MIGRATOR).await.unwrap().is_current());
	}
}

//! What a paired phone's person chose to keep sending to home, and when each
//! choice began and ended (paulgsc/some-ui `apps/www/src/lib/home-link`, its
//! consent log). `20261019000100_create_sharing_choice.up.sql` records the
//! design; in short:
//!
//! - **Per pairing.** A choice is made on one phone, under one pairing
//!   (`device_id`, a digest of that pairing's session), and counts for it
//!   alone, as it does on the phone.
//! - **Append-only, two times per choice.** [`choose`] adds a row only when
//!   a choice changes: one when sharing began, one when it ended. No row is
//!   ever updated (a trigger aborts an `UPDATE`), and nothing here deletes;
//!   account deletion is the one delete. Nothing records when anything was
//!   sent under a choice.
//! - **Home refuses what was stopped.** [`refuses`] is what a write from a
//!   pairing checks first, once that pairing has told home any choice.

use serde::Serialize;
use sqlx::SqliteConnection;

/// What a phone can keep sending home. Closed, as the phone's `SCOPES` is: a
/// new one is a reviewed diff here and in the migration's CHECK.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
	Sessions,
	Shelf,
	Aph,
	Jobs,
}

impl Scope {
	pub const ALL: [Self; 4] = [Self::Sessions, Self::Shelf, Self::Aph, Self::Jobs];

	/// The wire and stored name.
	#[must_use]
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::Sessions => "sessions",
			Self::Shelf => "shelf",
			Self::Aph => "aph",
			Self::Jobs => "jobs",
		}
	}

	/// The scope `name` names, if any.
	#[must_use]
	pub fn named(name: &str) -> Option<Self> {
		Self::ALL.into_iter().find(|scope| scope.as_str() == name)
	}
}

/// One change of a choice, as home heard it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Change {
	pub scope: Scope,
	/// Sharing began (`true`) or ended (`false`).
	pub kept: bool,
	/// When home heard it, ISO-8601 UTC.
	pub at: String,
}

/// Every change of a pairing's choices, in the order home heard them.
///
/// # Errors
/// Any `sqlx` failure.
pub async fn changes(conn: &mut SqliteConnection, subject: &str, device: &str) -> Result<Vec<Change>, sqlx::Error> {
	let rows = sqlx::query!(
		"SELECT scope, kept, at FROM sharing_choice WHERE subject_id = ?1 AND device_id = ?2 ORDER BY seq",
		subject,
		device
	)
	.fetch_all(&mut *conn)
	.await?;
	Ok(
		rows
			.into_iter()
			.filter_map(|row| {
				Some(Change {
					scope: Scope::named(&row.scope)?,
					kept: row.kept == 1,
					at: row.at,
				})
			})
			.collect(),
	)
}

/// Whether `scope` is kept under the pairing now: its latest change began
/// sharing.
fn kept_in(changes: &[Change], scope: Scope) -> bool {
	changes.iter().rev().find(|change| change.scope == scope).is_some_and(|change| change.kept)
}

/// Records that the pairing keeps sending `scope` (`kept`) or stopped, as of
/// `now`, and answers whether that changed anything: choosing what already
/// holds writes nothing.
///
/// Run it in a `BEGIN IMMEDIATE` transaction, so the read that decides and
/// the write see one snapshot.
///
/// # Errors
/// Any `sqlx` failure.
pub async fn choose(conn: &mut SqliteConnection, subject: &str, device: &str, scope: Scope, kept: bool, now: &str) -> Result<bool, sqlx::Error> {
	let standing = changes(conn, subject, device).await?;
	if kept_in(&standing, scope) == kept {
		return Ok(false);
	}
	let seq = i64::try_from(standing.len()).unwrap_or(i64::MAX - 1) + 1;
	let name = scope.as_str();
	let flag = i64::from(kept);
	sqlx::query!(
		"INSERT INTO sharing_choice (subject_id, device_id, seq, scope, kept, at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
		subject,
		device,
		seq,
		name,
		flag,
		now
	)
	.execute(&mut *conn)
	.await?;
	Ok(true)
}

/// Whether home refuses a write of `scope` from the pairing.
///
/// It does once the pairing has told home a choice and `scope` is not kept
/// under it now. A pairing that never told home any (one made before home
/// kept choices) is refused nothing.
///
/// # Errors
/// Any `sqlx` failure.
pub async fn refuses(conn: &mut SqliteConnection, subject: &str, device: &str, scope: Scope) -> Result<bool, sqlx::Error> {
	let standing = changes(conn, subject, device).await?;
	Ok(!standing.is_empty() && !kept_in(&standing, scope))
}

#[cfg(test)]
mod tests {
	use super::{changes, choose, refuses, Scope};
	use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};

	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");
	const PREVIOUS_MIGRATION: i64 = 20_261_018_000_100;
	const A: &str = "subject-a";
	const B: &str = "subject-b";

	async fn pool() -> SqlitePool {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();
		pool
	}

	#[tokio::test]
	async fn the_migration_round_trips() {
		let pool = pool().await;
		let tables = || sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'sharing_choice'");
		assert_eq!(tables().fetch_one(&pool).await.unwrap(), 1);
		MIGRATOR.undo(&pool, PREVIOUS_MIGRATION).await.unwrap();
		assert_eq!(tables().fetch_one(&pool).await.unwrap(), 0, "down drops it");
		MIGRATOR.run(&pool).await.unwrap();
		assert_eq!(tables().fetch_one(&pool).await.unwrap(), 1, "up recreates it");
	}

	/// The columns the migration describes and no others: a new one is a
	/// decision about what the server learns from a choice.
	#[tokio::test]
	async fn the_table_holds_exactly_its_described_columns() {
		let pool = pool().await;
		let columns: Vec<String> = sqlx::query_scalar("SELECT name FROM pragma_table_info('sharing_choice') ORDER BY cid")
			.fetch_all(&pool)
			.await
			.unwrap();
		assert_eq!(columns, ["subject_id", "device_id", "seq", "scope", "kept", "at"]);
	}

	#[tokio::test]
	async fn a_choice_is_two_rows_at_most_and_no_row_changes() {
		let pool = pool().await;
		let conn = &mut pool.acquire().await.unwrap();
		assert!(choose(conn, A, "d1", Scope::Aph, true, "t1").await.unwrap());
		assert!(!choose(conn, A, "d1", Scope::Aph, true, "t2").await.unwrap(), "keeping what is kept writes nothing");
		assert!(!choose(conn, A, "d1", Scope::Jobs, false, "t2").await.unwrap(), "stopping what is not kept writes nothing");
		assert!(choose(conn, A, "d1", Scope::Aph, false, "t3").await.unwrap());
		let held = changes(conn, A, "d1").await.unwrap();
		assert_eq!(
			held.iter().map(|change| (change.scope, change.kept, change.at.as_str())).collect::<Vec<_>>(),
			[(Scope::Aph, true, "t1"), (Scope::Aph, false, "t3")]
		);
		assert!(changes(conn, A, "d2").await.unwrap().is_empty(), "another pairing's are its own");
		assert!(changes(conn, B, "d1").await.unwrap().is_empty(), "another subject's are its own");

		let update = sqlx::query("UPDATE sharing_choice SET kept = 1").execute(&mut **conn).await;
		assert!(update.unwrap_err().to_string().contains("append-only"), "no row is ever updated");
	}

	#[tokio::test]
	async fn home_refuses_only_what_a_pairing_that_told_it_stopped() {
		let pool = pool().await;
		let conn = &mut pool.acquire().await.unwrap();
		assert!(!refuses(conn, A, "d1", Scope::Jobs).await.unwrap(), "a pairing that told home nothing");
		choose(conn, A, "d1", Scope::Aph, true, "t1").await.unwrap();
		assert!(!refuses(conn, A, "d1", Scope::Aph).await.unwrap());
		assert!(refuses(conn, A, "d1", Scope::Jobs).await.unwrap(), "never kept under a pairing that told home");
		choose(conn, A, "d1", Scope::Aph, false, "t2").await.unwrap();
		assert!(refuses(conn, A, "d1", Scope::Aph).await.unwrap(), "stopped");
		assert!(!refuses(conn, A, "d2", Scope::Aph).await.unwrap(), "another pairing told home nothing");
	}

	#[tokio::test]
	async fn the_choices_leave_with_the_account() {
		let pool = pool().await;
		sqlx::query("INSERT INTO account (subject_id, user_handle) VALUES (?1, ?2)")
			.bind(A)
			.bind(A.as_bytes())
			.execute(&pool)
			.await
			.unwrap();
		choose(&mut pool.acquire().await.unwrap(), A, "d1", Scope::Aph, true, "t1").await.unwrap();
		sqlx::query("DELETE FROM account WHERE subject_id = ?1").bind(A).execute(&pool).await.unwrap();
		assert!(changes(&mut pool.acquire().await.unwrap(), A, "d1").await.unwrap().is_empty());
	}
}

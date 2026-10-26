//! What a paired phone's person chose to keep sending to home, and when each
//! choice began and ended (paulgsc/some-ui `apps/www/src/lib/home-link`, its
//! consent log). `20261019000100_create_sharing_choice.up.sql` records the
//! design, and `20261025000300_add_voice_to_sharing_choice.up.sql` and
//! `20261026000300_add_korean_to_sharing_choice.up.sql` add voice notes and
//! Korean's record to what can be chosen; in short:
//!
//! - **Per pairing.** A choice is made on one phone, under one pairing
//!   (`pairing_id`, a digest of that pairing's session), and counts for it
//!   alone, as it does on the phone.
//! - **Append-only, one row per change.** [`choose`] adds a row only when
//!   a choice changes: when sharing began, or ended. No row is ever updated
//!   (a trigger aborts an `UPDATE`), and nothing here deletes; account
//!   deletion is the one delete. Nothing records when anything was sent
//!   under a choice.
//! - **Home refuses what was stopped.** [`refuses`] is what a write from a
//!   pairing checks first: a scope whose latest change stopped it.

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
	/// Voice notes, audio included (`PUT /voice-notes/:id`).
	Voice,
	/// Korean's word sets and reading records (`PUT` and
	/// `DELETE /korean/:kind/:key`).
	Korean,
}

impl Scope {
	pub const ALL: [Self; 6] = [Self::Sessions, Self::Shelf, Self::Aph, Self::Jobs, Self::Voice, Self::Korean];

	/// The wire and stored name.
	#[must_use]
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::Sessions => "sessions",
			Self::Shelf => "shelf",
			Self::Aph => "aph",
			Self::Jobs => "jobs",
			Self::Voice => "voice",
			Self::Korean => "korean",
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
pub async fn changes(conn: &mut SqliteConnection, subject: &str, pairing: &str) -> Result<Vec<Change>, sqlx::Error> {
	let rows = sqlx::query!(
		"SELECT scope, kept, at FROM sharing_choice WHERE subject_id = ?1 AND pairing_id = ?2 ORDER BY seq",
		subject,
		pairing
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

/// The latest change of `scope` under the pairing: `Some(true)` while it is
/// kept, `Some(false)` once stopped, `None` if the pairing never named it.
async fn latest(conn: &mut SqliteConnection, subject: &str, pairing: &str, scope: Scope) -> Result<Option<bool>, sqlx::Error> {
	let name = scope.as_str();
	let kept = sqlx::query_scalar!(
		"SELECT kept FROM sharing_choice WHERE subject_id = ?1 AND pairing_id = ?2 AND scope = ?3 ORDER BY seq DESC LIMIT 1",
		subject,
		pairing,
		name
	)
	.fetch_optional(&mut *conn)
	.await?;
	Ok(kept.map(|kept| kept == 1))
}

/// Records that the pairing keeps sending `scope` (`kept`) or stopped, as of
/// `now`, and answers with the pairing's changes after it.
///
/// Choosing what the latest change already says writes nothing. Run it in a `BEGIN IMMEDIATE`
/// transaction, so the read that decides and the write see one snapshot.
///
/// # Errors
/// Any `sqlx` failure.
pub async fn choose(conn: &mut SqliteConnection, subject: &str, pairing: &str, scope: Scope, kept: bool, now: &str) -> Result<Vec<Change>, sqlx::Error> {
	// A first stop is written too: a pairing made before home kept choices
	// sends until it names a scope, and stopping one names it.
	if latest(conn, subject, pairing, scope).await? != Some(kept) {
		let name = scope.as_str();
		let flag = i64::from(kept);
		// The next `seq` from the table, not from `changes`, which leaves out
		// a scope this binary does not know.
		sqlx::query!(
			"INSERT INTO sharing_choice (subject_id, pairing_id, seq, scope, kept, at)
			 SELECT ?1, ?2, COALESCE(MAX(seq), 0) + 1, ?3, ?4, ?5 FROM sharing_choice WHERE subject_id = ?1 AND pairing_id = ?2",
			subject,
			pairing,
			name,
			flag,
			now
		)
		.execute(&mut *conn)
		.await?;
	}
	changes(conn, subject, pairing).await
}

/// Whether home refuses a write of `scope` from the pairing: its person
/// stopped sending it, and has not kept it again since.
///
/// A scope the pairing never named is refused nothing, so a pairing made
/// before home kept choices, or one still telling home its choices one scope
/// at a time, writes as before.
///
/// # Errors
/// Any `sqlx` failure.
pub async fn refuses(conn: &mut SqliteConnection, subject: &str, pairing: &str, scope: Scope) -> Result<bool, sqlx::Error> {
	Ok(latest(conn, subject, pairing, scope).await? == Some(false))
}

#[cfg(test)]
mod tests {
	use super::{changes, choose, refuses, Scope};
	use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};

	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");
	const PREVIOUS_MIGRATION: i64 = 20_261_018_000_100;
	/// The migration before `20261025000300_add_voice_to_sharing_choice`.
	const BEFORE_VOICE: i64 = 20_261_025_000_200;
	/// The migration before `20261026000300_add_korean_to_sharing_choice`.
	const BEFORE_KOREAN: i64 = 20_261_026_000_200;
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
		assert_eq!(columns, ["subject_id", "pairing_id", "seq", "scope", "kept", "at"]);
	}

	#[tokio::test]
	async fn a_change_is_one_row_and_no_row_changes() {
		let pool = pool().await;
		let conn = &mut pool.acquire().await.unwrap();
		assert_eq!(choose(conn, A, "d1", Scope::Aph, true, "t1").await.unwrap().len(), 1);
		assert_eq!(choose(conn, A, "d1", Scope::Aph, true, "t2").await.unwrap().len(), 1, "keeping what is kept writes nothing");
		assert_eq!(choose(conn, A, "d1", Scope::Jobs, false, "t2").await.unwrap().len(), 2, "a first stop is written");
		assert_eq!(
			choose(conn, A, "d1", Scope::Jobs, false, "t3").await.unwrap().len(),
			2,
			"stopping what is stopped writes nothing"
		);
		choose(conn, A, "d1", Scope::Aph, false, "t3").await.unwrap();
		let held = choose(conn, A, "d1", Scope::Aph, true, "t4").await.unwrap();
		assert_eq!(
			held.iter().map(|change| (change.scope, change.kept, change.at.as_str())).collect::<Vec<_>>(),
			[(Scope::Aph, true, "t1"), (Scope::Jobs, false, "t2"), (Scope::Aph, false, "t3"), (Scope::Aph, true, "t4")]
		);
		assert_eq!(held, changes(conn, A, "d1").await.unwrap());
		assert!(changes(conn, A, "d2").await.unwrap().is_empty(), "another pairing's are its own");
		assert!(changes(conn, B, "d1").await.unwrap().is_empty(), "another subject's are its own");

		let update = sqlx::query("UPDATE sharing_choice SET kept = 1").execute(&mut **conn).await;
		assert!(update.unwrap_err().to_string().contains("append-only"), "no row is ever updated");
	}

	/// A later binary's scope this one does not know still holds its place
	/// in the order: the next change follows it.
	#[tokio::test]
	async fn a_change_follows_one_this_binary_cannot_read() {
		let pool = pool().await;
		let conn = &mut pool.acquire().await.unwrap();
		choose(conn, A, "d1", Scope::Aph, true, "t1").await.unwrap();
		sqlx::query("PRAGMA ignore_check_constraints = ON").execute(&mut **conn).await.unwrap();
		sqlx::query("INSERT INTO sharing_choice (subject_id, pairing_id, seq, scope, kept, at) VALUES (?1, 'd1', 2, 'later', 1, 't2')")
			.bind(A)
			.execute(&mut **conn)
			.await
			.unwrap();
		let held = choose(conn, A, "d1", Scope::Jobs, true, "t3").await.unwrap();
		assert_eq!(held.iter().map(|change| change.scope).collect::<Vec<_>>(), [Scope::Aph, Scope::Jobs]);
	}

	#[tokio::test]
	async fn home_refuses_only_what_a_pairing_stopped() {
		let pool = pool().await;
		let conn = &mut pool.acquire().await.unwrap();
		assert!(!refuses(conn, A, "d1", Scope::Jobs).await.unwrap(), "a pairing that told home nothing");
		choose(conn, A, "d1", Scope::Aph, true, "t1").await.unwrap();
		assert!(!refuses(conn, A, "d1", Scope::Aph).await.unwrap());
		assert!(!refuses(conn, A, "d1", Scope::Jobs).await.unwrap(), "a scope this pairing has not named");
		choose(conn, A, "d1", Scope::Aph, false, "t2").await.unwrap();
		assert!(refuses(conn, A, "d1", Scope::Aph).await.unwrap(), "stopped");
		assert!(!refuses(conn, A, "d2", Scope::Aph).await.unwrap(), "another pairing's stop is its own");
		choose(conn, A, "d1", Scope::Aph, true, "t3").await.unwrap();
		assert!(!refuses(conn, A, "d1", Scope::Aph).await.unwrap(), "kept again");
		choose(conn, A, "d1", Scope::Jobs, false, "t4").await.unwrap();
		assert!(refuses(conn, A, "d1", Scope::Jobs).await.unwrap(), "stopped without ever being kept here");
	}

	async fn account(pool: &SqlitePool, subject: &str) {
		sqlx::query("INSERT INTO account (subject_id, user_handle) VALUES (?1, ?2)")
			.bind(subject)
			.bind(subject.as_bytes())
			.execute(pool)
			.await
			.unwrap();
	}

	/// Rebuilding the table to take `voice` copies every change as it was,
	/// keeps both triggers, and lets a pairing keep or stop voice notes.
	#[tokio::test]
	async fn adding_voice_keeps_every_change_and_both_triggers() {
		let pool = pool().await;
		MIGRATOR.undo(&pool, BEFORE_VOICE).await.unwrap();
		account(&pool, A).await;
		let rows = || sqlx::query_as::<_, (String, String, i64, String, i64, String)>("SELECT * FROM sharing_choice ORDER BY subject_id, pairing_id, seq");
		// The pool holds one connection: each borrow ends before a migration runs.
		let mut conn = pool.acquire().await.unwrap();
		for (subject, pairing, scope, kept, at) in [
			(A, "d1", Scope::Aph, true, "t1"),
			(A, "d1", Scope::Jobs, false, "t2"),
			(A, "d2", Scope::Sessions, true, "t3"),
			(B, "d1", Scope::Shelf, true, "t4"),
			(A, "d1", Scope::Aph, false, "t5"),
		] {
			choose(&mut conn, subject, pairing, scope, kept, at).await.unwrap();
		}
		let before = rows().fetch_all(&mut *conn).await.unwrap();
		assert_eq!(before.len(), 5);
		drop(conn);

		MIGRATOR.run(&pool).await.unwrap();
		let mut conn = pool.acquire().await.unwrap();
		assert_eq!(rows().fetch_all(&mut *conn).await.unwrap(), before, "every change, as it was");

		choose(&mut conn, A, "d1", Scope::Voice, false, "t6").await.unwrap();
		assert!(refuses(&mut conn, A, "d1", Scope::Voice).await.unwrap());
		let held = choose(&mut conn, A, "d1", Scope::Voice, true, "t7").await.unwrap();
		assert_eq!(held.last().map(|change| (change.scope, change.kept)), Some((Scope::Voice, true)));
		assert!(!refuses(&mut conn, A, "d1", Scope::Voice).await.unwrap());

		let update = sqlx::query("UPDATE sharing_choice SET kept = 1").execute(&mut *conn).await;
		assert!(update.unwrap_err().to_string().contains("append-only"), "no row is ever updated");
		sqlx::query("DELETE FROM account WHERE subject_id = ?1").bind(A).execute(&mut *conn).await.unwrap();
		let left: Vec<String> = sqlx::query_scalar("SELECT DISTINCT subject_id FROM sharing_choice").fetch_all(&mut *conn).await.unwrap();
		assert_eq!(left, [B], "A's choices left with A's account, and only A's");
	}

	/// Rolling back past `voice` once a pairing chose it fails and changes
	/// nothing, rather than discarding the choice.
	#[tokio::test]
	async fn rolling_back_past_voice_refuses_rather_than_discards() {
		let pool = pool().await;
		account(&pool, A).await;
		let mut conn = pool.acquire().await.unwrap();
		choose(&mut conn, A, "d1", Scope::Voice, false, "t1").await.unwrap();
		drop(conn);
		let refused = MIGRATOR.undo(&pool, BEFORE_VOICE).await.unwrap_err().to_string();
		assert!(refused.contains("CHECK constraint failed"), "{refused}");

		let mut conn = pool.acquire().await.unwrap();
		assert!(refuses(&mut conn, A, "d1", Scope::Voice).await.unwrap(), "the choice is still held");
		let update = sqlx::query("UPDATE sharing_choice SET kept = 1").execute(&mut *conn).await;
		assert!(update.unwrap_err().to_string().contains("append-only"), "the append-only trigger is still there");
		sqlx::query("DELETE FROM account WHERE subject_id = ?1").bind(A).execute(&mut *conn).await.unwrap();
		assert!(changes(&mut conn, A, "d1").await.unwrap().is_empty(), "the account trigger is still there");
	}

	/// `korean` is a scope like the rest, and rolling back past it once a
	/// pairing chose it fails and changes nothing.
	#[tokio::test]
	async fn korean_is_chosen_like_the_rest_and_a_rollback_refuses_to_discard_it() {
		let pool = pool().await;
		account(&pool, A).await;
		let mut conn = pool.acquire().await.unwrap();
		choose(&mut conn, A, "d1", Scope::Voice, true, "t1").await.unwrap();
		choose(&mut conn, A, "d1", Scope::Korean, true, "t2").await.unwrap();
		assert!(!refuses(&mut conn, A, "d1", Scope::Korean).await.unwrap());
		let held = choose(&mut conn, A, "d1", Scope::Korean, false, "t3").await.unwrap();
		assert_eq!(held.last().map(|change| (change.scope, change.kept)), Some((Scope::Korean, false)));
		assert!(refuses(&mut conn, A, "d1", Scope::Korean).await.unwrap());
		assert_eq!(Scope::named("korean"), Some(Scope::Korean));
		drop(conn);

		let refused = MIGRATOR.undo(&pool, BEFORE_KOREAN).await.unwrap_err().to_string();
		assert!(refused.contains("CHECK constraint failed"), "{refused}");
		let mut conn = pool.acquire().await.unwrap();
		assert_eq!(changes(&mut conn, A, "d1").await.unwrap().len(), 3, "every choice is still held");
		let update = sqlx::query("UPDATE sharing_choice SET kept = 1").execute(&mut *conn).await;
		assert!(update.unwrap_err().to_string().contains("append-only"), "the append-only trigger is still there");
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

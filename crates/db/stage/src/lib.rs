//! What waits on what across home's shared stages (paulgsc/server#1091).
//!
//! `stage_want` holds one row per item a waiter is waiting on right now
//! (`migrations/20261121000400_create_stage_want.up.sql`). A waiter's pass
//! [`replace`]s all its own rows in one transaction with what it waits on
//! now, so the table is always the waiters' latest view and needs no clean-up;
//! a stage's queue joins it to put first what someone downstream waits on.
//!
//! The waiter and stage names are `file_host`'s closed `stage::Waiter` and
//! `stage::Stage`; the table's checks hold the same lists.

use sqlx::SqliteConnection;

/// One item a waiter waits on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Want<'a> {
	/// Which of the waiter's kind waits: a cards essay's key.
	pub waiter_key: &'a str,
	/// The stage it waits on.
	pub stage: &'a str,
	/// Whose item it is.
	pub subject: &'a str,
	/// The item, by its stage's own key: a note id.
	pub item: &'a str,
}

/// Replaces every row of `waiter` with `wants`.
///
/// A want listed twice is kept once, and one for an item its subject no
/// longer holds (a transcriber's note gone with its account, or deleted) is
/// not written: checked in the same statement, so in a transaction it cannot
/// outlive the account (docs/identity.md invariant 9). Run it in one
/// transaction.
///
/// # Errors
/// Any `sqlx` failure, a name the table's checks refuse among them.
pub async fn replace(conn: &mut SqliteConnection, waiter: &str, wants: &[Want<'_>]) -> Result<(), sqlx::Error> {
	sqlx::query("DELETE FROM stage_want WHERE waiter = ?1").bind(waiter).execute(&mut *conn).await?;
	for want in wants {
		// The transcriber's items are voice notes, the only stage there is;
		// a new stage adds its own item's check here.
		sqlx::query(
			"INSERT INTO stage_want (waiter, waiter_key, stage, subject_id, item_key)
			 SELECT ?1, ?2, ?3, ?4, ?5
			 WHERE ?3 = 'transcriber' AND EXISTS (SELECT 1 FROM voice_note n WHERE n.subject_id = ?4 AND n.note_id = ?5)
			 ON CONFLICT DO NOTHING",
		)
		.bind(waiter)
		.bind(want.waiter_key)
		.bind(want.stage)
		.bind(want.subject)
		.bind(want.item)
		.execute(&mut *conn)
		.await?;
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::{replace, Want};
	use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};

	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");

	async fn pool() -> SqlitePool {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();
		sqlx::query("INSERT INTO account (subject_id, user_handle) VALUES ('subject-a', x'00')")
			.execute(&pool)
			.await
			.unwrap();
		for note in ["n1", "n2", "n3"] {
			sqlx::query(
				"INSERT INTO voice_note (subject_id, note_id, of_kind, of_ref, recorded_at, content_type, size, sha256, stored_at)
				 VALUES ('subject-a', ?1, 'reflection', 'r', 'then', 'audio/webm', 1, 's', 'now')",
			)
			.bind(note)
			.execute(&pool)
			.await
			.unwrap();
		}
		pool
	}

	fn want<'a>(essay: &'a str, note: &'a str) -> Want<'a> {
		Want {
			waiter_key: essay,
			stage: "transcriber",
			subject: "subject-a",
			item: note,
		}
	}

	async fn rows(pool: &SqlitePool) -> Vec<(String, String)> {
		sqlx::query_as("SELECT waiter_key, item_key FROM stage_want ORDER BY waiter_key, item_key")
			.fetch_all(pool)
			.await
			.unwrap()
	}

	async fn count(pool: &SqlitePool) -> i64 {
		sqlx::query_scalar("SELECT COUNT(*) FROM stage_want").fetch_one(pool).await.unwrap()
	}

	/// A pass replaces its waits whole: what it lists now, once, and nothing
	/// it no longer lists.
	#[tokio::test]
	async fn a_pass_replaces_its_waits_whole() {
		let pool = pool().await;
		let mut conn = pool.acquire().await.unwrap();
		replace(&mut conn, "cards_essays", &[want("e1", "n1"), want("e1", "n2"), want("e1", "n2")]).await.unwrap();
		drop(conn);
		assert_eq!(count(&pool).await, 2, "a want listed twice is kept once");
		let mut conn = pool.acquire().await.unwrap();
		replace(&mut conn, "cards_essays", &[want("e1", "n2"), want("e2", "n3")]).await.unwrap();
		drop(conn);
		let expected = [("e1", "n2"), ("e2", "n3")].map(|(a, b)| (a.to_owned(), b.to_owned()));
		assert_eq!(rows(&pool).await, expected);
		let mut conn = pool.acquire().await.unwrap();
		replace(&mut conn, "cards_essays", &[]).await.unwrap();
		drop(conn);
		assert_eq!(count(&pool).await, 0, "nothing waits");
	}

	/// The table refuses a waiter outside the closed set, and nothing is
	/// written for a stage it doesn't know or a note its subject doesn't hold.
	#[tokio::test]
	async fn an_unknown_waiter_is_refused_and_an_unknown_item_is_not_wanted() {
		let pool = pool().await;
		let mut conn = pool.acquire().await.unwrap();
		assert!(replace(&mut conn, "nobody", &[want("e1", "n1")]).await.is_err());
		let stray = Want {
			stage: "nowhere",
			..want("e1", "n1")
		};
		let elsewhere = Want {
			subject: "subject-b",
			..want("e1", "n1")
		};
		replace(&mut conn, "cards_essays", &[stray, elsewhere, want("e1", "gone")]).await.unwrap();
		drop(conn);
		assert_eq!(count(&pool).await, 0);
	}

	/// A subject's waits leave with the account.
	#[tokio::test]
	async fn waits_leave_with_the_account() {
		let pool = pool().await;
		let mut conn = pool.acquire().await.unwrap();
		replace(&mut conn, "cards_essays", &[want("e1", "n1")]).await.unwrap();
		sqlx::query("DELETE FROM account WHERE subject_id = 'subject-a'").execute(&mut *conn).await.unwrap();
		drop(conn);
		assert_eq!(count(&pool).await, 0);
	}
}

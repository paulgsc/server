//! The hunches no phrase did (paulgsc/server#1261, KPLAY's KP4;
//! paulgsc/some-ui#2853): `phrase_hunch`, one row per hunch the owner asked
//! home that home's meaning index matched to no Korean phrase.
//!
//! `20261127000100_create_phrase_hunch.up.sql` records the design; in short:
//!
//! - **Kept in the answer.** `file_host`'s `POST /phrases/lookup` calls
//!   [`keep`] in the request that answers `kept`, so a miss is never lost
//!   between two calls. Nothing else writes the table.
//! - **Once per hunch.** The phone names each hunch (its shop runtime's id and
//!   the hunch's number), the same each time it is sent again, and [`keep`]
//!   keeps the first and changes nothing after.
//! - **The owner's history.** Append-only (a trigger aborts an `UPDATE`) and
//!   kept for good: account deletion's sweep and the migration's trigger are
//!   the one delete (docs/identity.md invariant 41). Nothing here logs.
//! - **Words only.** The hunch's id, the owner's words and when; no vector,
//!   and no take: a spoken hunch's take is a voice note of kind `hunch` whose
//!   `ref` is the same hunch id.

use sqlx::SqliteConnection;

/// The most characters a hunch's words may hold, as a take library search's
/// asked words may (`voice_repo::takes`).
pub const MAX_WORDS_CHARS: usize = 200;

/// The most characters a hunch's id may hold: a voice note's id's limit, so a
/// spoken hunch's take can be named by it.
pub const MAX_HUNCH_CHARS: usize = 64;

/// Whether `hunch` reads as the phone's name for a hunch.
///
/// That is 1 to [`MAX_HUNCH_CHARS`] of `A-Z a-z 0-9 _ . : -`, the characters
/// a voice note's `ref` takes (`<runtime uuid>-<number>`, from the shop's
/// runtime).
#[must_use]
pub fn is_hunch(hunch: &str) -> bool {
	!hunch.is_empty() && hunch.len() <= MAX_HUNCH_CHARS && hunch.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b':' | b'-'))
}

/// Whether `words` are a hunch's: 1 to [`MAX_WORDS_CHARS`] characters once
/// trimmed.
#[must_use]
pub fn is_words(words: &str) -> bool {
	let words = words.trim();
	!words.is_empty() && words.chars().count() <= MAX_WORDS_CHARS
}

/// What [`keep`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kept {
	/// The hunch is kept now.
	Added,
	/// It was kept already, by an earlier send of the same hunch: nothing
	/// changed, its words and time included.
	Already,
}

/// Keeps `subject`'s hunch `hunch`, the owner's `words` (trimmed), as of
/// `now`; a hunch kept before is left as it was.
///
/// The caller checks `hunch` and `words` ([`is_hunch`], [`is_words`]); the
/// schema refuses either out of shape.
///
/// # Errors
/// Any `sqlx` failure; nothing is written then.
pub async fn keep(conn: &mut SqliteConnection, subject: &str, hunch: &str, words: &str, now: &str) -> Result<Kept, sqlx::Error> {
	let words = words.trim();
	let added = sqlx::query!(
		"INSERT INTO phrase_hunch (subject_id, hunch, words, asked_at) VALUES (?1, ?2, ?3, ?4)
		 ON CONFLICT (subject_id, hunch) DO NOTHING",
		subject,
		hunch,
		words,
		now
	)
	.execute(&mut *conn)
	.await?
	.rows_affected();
	Ok(if added == 0 { Kept::Already } else { Kept::Added })
}

#[cfg(test)]
mod tests {
	use super::{is_hunch, is_words, keep, Kept};
	use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};

	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");
	/// The migration before `20261127000100_create_phrase_hunch`.
	const PREVIOUS_MIGRATION: i64 = 20_261_126_000_100;
	const T0: &str = "2026-11-27T07:00:00.000Z";
	const T1: &str = "2026-11-27T08:00:00.000Z";
	const A: &str = "subject-a";
	const B: &str = "subject-b";
	const HUNCH: &str = "0b9c1f0e-5a3f-4c1e-9d6b-2f1d8e7a6c5b-1";

	async fn pool() -> SqlitePool {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();
		pool
	}

	async fn kept(pool: &SqlitePool) -> Vec<(String, String, String, String)> {
		sqlx::query_as("SELECT subject_id, hunch, words, asked_at FROM phrase_hunch ORDER BY subject_id, hunch")
			.fetch_all(pool)
			.await
			.unwrap()
	}

	#[test]
	fn a_hunch_is_named_as_a_notes_ref_is_and_its_words_are_bounded() {
		assert!(is_hunch(HUNCH));
		assert!(is_hunch("a") && is_hunch(&"a".repeat(64)));
		assert!(!is_hunch("") && !is_hunch(&"a".repeat(65)));
		assert!(!is_hunch("a b") && !is_hunch("a/b") && !is_hunch("한"));
		assert!(is_words("where's my phone") && is_words(&"가".repeat(200)));
		assert!(!is_words("") && !is_words("   ") && !is_words(&"a".repeat(201)));
		assert!(is_words(&(String::from("  ") + &"a".repeat(200) + "  ")), "trimmed first");
	}

	/// The first send keeps the hunch, trimmed; a send again keeps nothing
	/// more and changes nothing, even with other words.
	#[tokio::test]
	async fn a_hunch_is_kept_once_however_often_it_is_sent() {
		let pool = pool().await;
		let mut conn = pool.acquire().await.unwrap();
		assert_eq!(keep(&mut conn, A, HUNCH, "  how do I say I'm sleepy ", T0).await.unwrap(), Kept::Added);
		assert_eq!(keep(&mut conn, A, HUNCH, "something else", T1).await.unwrap(), Kept::Already);
		drop(conn);
		assert_eq!(kept(&pool).await, [(A.into(), HUNCH.into(), "how do I say I'm sleepy".into(), T0.into())]);
	}

	/// Each subject keeps its own, under the same hunch id.
	#[tokio::test]
	async fn each_subject_keeps_its_own() {
		let pool = pool().await;
		let mut conn = pool.acquire().await.unwrap();
		assert_eq!(keep(&mut conn, A, HUNCH, "a's", T0).await.unwrap(), Kept::Added);
		assert_eq!(keep(&mut conn, B, HUNCH, "b's", T1).await.unwrap(), Kept::Added);
		drop(conn);
		assert_eq!(kept(&pool).await.len(), 2);
	}

	/// The schema holds what the callers check, and the table is append-only
	/// and leaves with its account.
	#[tokio::test]
	async fn the_table_is_append_only_bounded_and_leaves_with_the_account() {
		let pool = pool().await;
		let mut conn = pool.acquire().await.unwrap();
		assert!(keep(&mut conn, A, "a b", "w", T0).await.is_err(), "an id out of shape");
		assert!(keep(&mut conn, A, HUNCH, "", T0).await.is_err(), "no words");
		assert!(keep(&mut conn, A, HUNCH, &"a".repeat(201), T0).await.is_err(), "too many words");
		keep(&mut conn, A, HUNCH, "w", T0).await.unwrap();
		drop(conn);
		assert!(sqlx::query("UPDATE phrase_hunch SET words = 'x'").execute(&pool).await.is_err(), "append-only");
		sqlx::query("INSERT INTO account (subject_id, user_handle) VALUES (?1, x'01')")
			.bind(A)
			.execute(&pool)
			.await
			.unwrap();
		sqlx::query("DELETE FROM account WHERE subject_id = ?1").bind(A).execute(&pool).await.unwrap();
		assert!(kept(&pool).await.is_empty());
	}

	/// Down refuses while the table holds a row, and drops it once empty.
	#[tokio::test]
	async fn the_migration_round_trips_and_refuses_to_drop_a_hunch() {
		let pool = pool().await;
		let tables = || sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'phrase_hunch'");
		MIGRATOR.undo(&pool, PREVIOUS_MIGRATION).await.unwrap();
		assert_eq!(tables().fetch_one(&pool).await.unwrap(), 0, "down drops an empty table");
		MIGRATOR.run(&pool).await.unwrap();
		keep(&mut pool.acquire().await.unwrap(), A, HUNCH, "w", T0).await.unwrap();
		assert!(MIGRATOR.undo(&pool, PREVIOUS_MIGRATION).await.is_err(), "down refuses a table holding a hunch");
		assert_eq!(tables().fetch_one(&pool).await.unwrap(), 1);
	}
}

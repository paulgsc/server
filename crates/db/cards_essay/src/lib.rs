//! Home's record of the cards essays (CARDS7, #854, paulgsc/some-ui#2979):
//! `cards_essay`, each essay once by its key, append-only, and
//! `cards_essay_link`, each essay's `YouTube` link (paulgsc/some-ui#2999),
//! append-only too, and `cards_essay_video_deletion`, each step of an
//! essay's video going (paulgsc/server#1055): [`withdraw`] when a take its
//! spoken cards play goes from home. The rules are
//! `20261114000100_create_cards_essay.up.sql`'s,
//! `20261115000200_create_cards_essay_link.up.sql`'s and
//! `20261121000100_create_cards_essay_video_deletion.up.sql`'s.

use sha2::{Digest, Sha256};
use sqlx::SqliteConnection;

/// An essay as the repository served it.
#[derive(Debug, Clone, Copy)]
pub struct Essay<'a> {
	/// `<slug>-<YYYY-MM-DD>`: its file's name.
	pub key: &'a str,
	/// The key's day, `YYYY-MM-DD`.
	pub day: &'a str,
	/// The cut list's bytes, exactly as served.
	pub body: &'a [u8],
	/// ISO-8601 UTC, when home took it.
	pub taken_at: &'a str,
}

/// What [`keep`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kept {
	/// The essay is new to home, and stored.
	Added,
	/// Home already holds an essay of that key: nothing is written.
	Held,
}

/// An essay home holds, and when it took it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeldEssay {
	pub key: String,
	pub day: String,
	/// ISO-8601 UTC, as [`Essay::taken_at`] stored it.
	pub taken_at: String,
}

/// Lowercase hex SHA-256 of `bytes`.
fn sha256_hex(bytes: &[u8]) -> String {
	hex::encode(Sha256::digest(bytes))
}

/// Stores `essay` unless home already holds one of its key.
///
/// # Errors
/// A storage failure, or an essay the table's checks refuse (a key that is
/// not `<slug>-<YYYY-MM-DD>` of its day).
pub async fn keep(conn: &mut SqliteConnection, essay: &Essay<'_>) -> Result<Kept, sqlx::Error> {
	let sha256 = sha256_hex(essay.body);
	let size = i64::try_from(essay.body.len()).unwrap_or(i64::MAX);
	let done = sqlx::query!(
		"INSERT INTO cards_essay (essay_key, day, sha256, size, body, taken_at) VALUES (?, ?, ?, ?, ?, ?)
		 ON CONFLICT (essay_key) DO NOTHING",
		essay.key,
		essay.day,
		sha256,
		size,
		essay.body,
		essay.taken_at,
	)
	.execute(&mut *conn)
	.await?;
	Ok(if done.rows_affected() == 1 { Kept::Added } else { Kept::Held })
}

/// Every essay home holds, oldest day first (then by key): the order the
/// render worker takes them in.
///
/// # Errors
/// A storage failure.
pub async fn held(conn: &mut SqliteConnection) -> Result<Vec<HeldEssay>, sqlx::Error> {
	sqlx::query_as!(
		HeldEssay,
		r#"SELECT essay_key AS "key!", day AS "day!", taken_at AS "taken_at!" FROM cards_essay ORDER BY day, essay_key"#
	)
	.fetch_all(&mut *conn)
	.await
}

/// The bytes home holds for the essay `key`, or `None`.
///
/// # Errors
/// A storage failure.
pub async fn body(conn: &mut SqliteConnection, key: &str) -> Result<Option<Vec<u8>>, sqlx::Error> {
	sqlx::query_scalar!("SELECT body FROM cards_essay WHERE essay_key = ?", key)
		.fetch_optional(&mut *conn)
		.await
}

/// Whether home holds an essay of `key`.
///
/// # Errors
/// A storage failure.
pub async fn holds(conn: &mut SqliteConnection, key: &str) -> Result<bool, sqlx::Error> {
	Ok(
		sqlx::query_scalar!(r#"SELECT 1 AS "one!: i64" FROM cards_essay WHERE essay_key = ?"#, key)
			.fetch_optional(&mut *conn)
			.await?
			.is_some(),
	)
}

/// Each essay's title, by key.
///
/// Its cut list's top-level `title` when that is a string, extracted by
/// `SQLite` so no body crosses into the server (`SQLite` still parses each one,
/// so a call is O(essays held), as the feed's list already is); `None` for
/// an essay whose cut list has none (or one that is not text).
///
/// # Errors
/// A storage failure.
pub async fn titles(conn: &mut SqliteConnection) -> Result<Vec<(String, Option<String>)>, sqlx::Error> {
	let rows = sqlx::query!(
		r#"SELECT essay_key AS "key!",
		          CASE WHEN json_valid(CAST(body AS TEXT))
		               THEN CASE WHEN json_type(CAST(body AS TEXT), '$.title') = 'text'
		                         THEN json_extract(CAST(body AS TEXT), '$.title') END
		          END AS "title?: String"
		   FROM cards_essay"#
	)
	.fetch_all(&mut *conn)
	.await?;
	Ok(rows.into_iter().map(|row| (row.key, row.title)).collect())
}

/// What [`record_link`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Linked {
	/// The link is new for the essay, and kept.
	Added,
	/// The essay already holds this video's link: nothing is written.
	Held,
	/// Home holds no essay of that key.
	NoEssay,
}

/// A link recorded for an essay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
	/// The essay's key.
	pub key: String,
	/// The video's 11-character id.
	pub youtube_id: String,
	/// ISO-8601 UTC, when home recorded it.
	pub recorded_at: String,
}

/// Records `youtube_id` as the essay `key`'s link, once per video.
///
/// # Errors
/// A storage failure, or an id the table's checks refuse.
pub async fn record_link(conn: &mut SqliteConnection, key: &str, youtube_id: &str, now: &str) -> Result<Linked, sqlx::Error> {
	if !holds(conn, key).await? {
		return Ok(Linked::NoEssay);
	}
	let done = sqlx::query!(
		"INSERT INTO cards_essay_link (essay_key, youtube_id, recorded_at) VALUES (?, ?, ?)
		 ON CONFLICT (essay_key, youtube_id) DO NOTHING",
		key,
		youtube_id,
		now,
	)
	.execute(&mut *conn)
	.await?;
	Ok(if done.rows_affected() == 1 { Linked::Added } else { Linked::Held })
}

/// Every link recorded, by essay, oldest first within each: the last of an
/// essay's is its link now.
///
/// # Errors
/// A storage failure.
pub async fn links(conn: &mut SqliteConnection) -> Result<Vec<Link>, sqlx::Error> {
	sqlx::query_as!(
		Link,
		r#"SELECT essay_key AS "key!", youtube_id AS "youtube_id!", recorded_at AS "recorded_at!"
		   FROM cards_essay_link ORDER BY essay_key, recorded_at, youtube_id"#
	)
	.fetch_all(&mut *conn)
	.await
}

/// An essay withdrawn because a take it plays went from home.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Withdrawn {
	pub key: String,
	/// The deleted take's voice-note id.
	pub note: String,
	/// The bytes that went, video and rest together.
	pub size: i64,
	/// The link it had, if it had one.
	pub youtube_id: Option<String>,
	pub deleted_at: String,
}

/// Withdraws the essay `key` for `subject`'s deleted take `note`; whether
/// it did.
///
/// Both steps' receipts, in the caller's transaction, as the Rust Rounds
/// essay's are (`rust_round_repo::essay_video::withdraw`), with the link it
/// had. An essay already gone (withdrawn, or let go) keeps what it has.
///
/// # Errors
/// A storage failure, or an essay home does not hold.
pub async fn withdraw(conn: &mut SqliteConnection, key: &str, subject: &str, note: &str, size: i64, now: &str) -> Result<bool, sqlx::Error> {
	let had = sqlx::query_scalar!(r#"SELECT COUNT(*) AS "n!: i64" FROM cards_essay_video_deletion WHERE essay_key = ?"#, key)
		.fetch_one(&mut *conn)
		.await?;
	if had > 0 {
		// Gone already: its video went, and with it the takes it played.
		return Ok(false);
	}
	let link = links(conn).await?.into_iter().filter(|link| link.key == key).last().map(|link| link.youtube_id);
	for (step, bytes) in [("video", size), ("rest", 0)] {
		sqlx::query!(
			"INSERT INTO cards_essay_video_deletion (essay_key, step, cause, subject_id, youtube_id, note_id, size, deleted_at)
			 VALUES (?, ?, 'take-deleted', ?, ?, ?, ?, ?)",
			key,
			step,
			subject,
			link,
			note,
			bytes,
			now,
		)
		.execute(&mut *conn)
		.await?;
	}
	Ok(true)
}

/// Every essay withdrawn for `subject`'s deleted take, by key.
///
/// # Errors
/// A storage failure.
pub async fn withdrawn(conn: &mut SqliteConnection, subject: &str) -> Result<Vec<Withdrawn>, sqlx::Error> {
	sqlx::query_as!(
		Withdrawn,
		r#"SELECT essay_key AS "key!", note_id AS "note!", size AS "size!: i64", youtube_id, deleted_at AS "deleted_at!"
		   FROM cards_essay_video_deletion
		   WHERE subject_id = ? AND cause = 'take-deleted' AND step = 'video' ORDER BY essay_key"#,
		subject
	)
	.fetch_all(&mut *conn)
	.await
}

/// The essays whose video went, withdrawn or (once some-ui#2999 lets one
/// go) let go: the render worker is never given them again, and no list
/// shows them. Keys alone, whoever's take it was.
///
/// # Errors
/// A storage failure.
pub async fn gone(conn: &mut SqliteConnection) -> Result<Vec<String>, sqlx::Error> {
	sqlx::query_scalar!(r#"SELECT essay_key AS "key!" FROM cards_essay_video_deletion WHERE step = 'video' ORDER BY essay_key"#)
		.fetch_all(&mut *conn)
		.await
}

#[cfg(test)]
mod tests {
	use super::*;
	use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};

	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");

	async fn pool() -> SqlitePool {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();
		pool
	}

	fn essay<'a>(key: &'a str, body: &'a [u8], taken_at: &'a str) -> Essay<'a> {
		Essay {
			key,
			day: &key[key.len() - 10..],
			body,
			taken_at,
		}
	}

	#[tokio::test]
	async fn an_essay_is_kept_once_as_served() {
		let pool = pool().await;
		let mut conn = pool.acquire().await.unwrap();
		let first = br#"{"schema":"some-ui.cut-list/2"}"#;
		let at = "2026-11-14T08:00:00.000Z";
		assert_eq!(keep(&mut conn, &essay("rust-scratchpad-2026-11-14", first, at)).await.unwrap(), Kept::Added);
		let edited = br#"{"schema":"some-ui.cut-list/2","edited":true}"#;
		let later = "2026-11-14T09:00:00.000Z";
		assert_eq!(
			keep(&mut conn, &essay("rust-scratchpad-2026-11-14", edited, later)).await.unwrap(),
			Kept::Held,
			"an edit is not taken"
		);
		assert_eq!(body(&mut conn, "rust-scratchpad-2026-11-14").await.unwrap().as_deref(), Some(&first[..]));
		assert_eq!(keep(&mut conn, &essay("zfs-2026-11-02", edited, later)).await.unwrap(), Kept::Added);
		assert_eq!(keep(&mut conn, &essay("nixos-upgrade-2026-11-14", edited, later)).await.unwrap(), Kept::Added);

		let keys: Vec<String> = held(&mut conn).await.unwrap().into_iter().map(|held| held.key).collect();
		assert_eq!(keys, ["zfs-2026-11-02", "nixos-upgrade-2026-11-14", "rust-scratchpad-2026-11-14"], "by day, not by name");
	}

	#[tokio::test]
	async fn an_essay_is_held_by_its_key_and_titled_by_its_cut_list() {
		let pool = pool().await;
		let mut conn = pool.acquire().await.unwrap();
		let at = "2026-11-14T08:00:00.000Z";
		keep(&mut conn, &essay("nixos-upgrade-2026-11-14", br#"{"title":"A machine upgrades itself"}"#, at))
			.await
			.unwrap();
		keep(&mut conn, &essay("zfs-2026-11-02", br#"{"schema":"some-ui.cut-list/2"}"#, at)).await.unwrap();
		keep(&mut conn, &essay("btrfs-2026-11-03", br#"{"title":7}"#, at)).await.unwrap();
		keep(&mut conn, &essay("ext4-2026-11-04", b"not json", at)).await.unwrap();
		assert!(holds(&mut conn, "zfs-2026-11-02").await.unwrap());
		assert!(!holds(&mut conn, "zfs-2026-11-03").await.unwrap());
		let mut titles = titles(&mut conn).await.unwrap();
		titles.sort();
		assert_eq!(
			titles,
			[
				(String::from("btrfs-2026-11-03"), None),
				(String::from("ext4-2026-11-04"), None),
				(String::from("nixos-upgrade-2026-11-14"), Some(String::from("A machine upgrades itself"))),
				(String::from("zfs-2026-11-02"), None),
			],
			"a title only where the cut list has one that is text"
		);
	}

	#[tokio::test]
	async fn a_key_that_is_not_its_day_is_refused() {
		let pool = pool().await;
		let mut conn = pool.acquire().await.unwrap();
		for key in ["NixOS-2026-11-14", "../etc-2026-11-14", "x_y-2026-11-14"] {
			assert!(keep(&mut conn, &essay(key, b"{}", "t")).await.is_err(), "{key} is refused");
		}
		let mismatched = Essay {
			key: "nixos-2026-11-14",
			day: "2026-11-15",
			body: b"{}",
			taken_at: "t",
		};
		assert!(keep(&mut conn, &mismatched).await.is_err(), "a day that is not the key's is refused");
	}

	#[tokio::test]
	async fn no_kept_essay_is_rewritten_or_deleted() {
		let pool = pool().await;
		let mut conn = pool.acquire().await.unwrap();
		keep(&mut conn, &essay("nixos-upgrade-2026-11-14", b"{}", "2026-11-14T08:00:00.000Z")).await.unwrap();
		let update = sqlx::query("UPDATE cards_essay SET body = x'00'").execute(&mut *conn).await;
		assert!(update.is_err(), "UPDATE is refused");
		let delete = sqlx::query("DELETE FROM cards_essay").execute(&mut *conn).await;
		assert!(delete.is_err(), "DELETE is refused");
		assert_eq!(held(&mut conn).await.unwrap().len(), 1);
	}

	#[tokio::test]
	async fn a_link_is_kept_once_per_video_only_for_an_essay_held_and_never_rewritten() {
		let pool = pool().await;
		let mut conn = pool.acquire().await.unwrap();
		keep(&mut conn, &essay("zfs-2026-11-02", b"{}", "2026-11-14T08:00:00.000Z")).await.unwrap();
		let (first, second) = ("dQw4w9WgXcQ", "aaaaaaaaaaa");
		assert_eq!(record_link(&mut conn, "zfs-2026-11-02", first, "2026-11-15T08:00:00.000Z").await.unwrap(), Linked::Added);
		assert_eq!(record_link(&mut conn, "zfs-2026-11-02", first, "2026-11-15T09:00:00.000Z").await.unwrap(), Linked::Held);
		assert_eq!(record_link(&mut conn, "zfs-2026-11-02", second, "2026-11-15T10:00:00.000Z").await.unwrap(), Linked::Added);
		assert_eq!(record_link(&mut conn, "ext4-2026-11-03", first, "2026-11-15T10:00:00.000Z").await.unwrap(), Linked::NoEssay);
		assert!(record_link(&mut conn, "zfs-2026-11-02", "not-an-id", "t").await.is_err(), "the table checks the id");
		let links = links(&mut conn).await.unwrap();
		assert_eq!(
			links
				.iter()
				.map(|link| (link.key.as_str(), link.youtube_id.as_str(), link.recorded_at.as_str()))
				.collect::<Vec<_>>(),
			[
				("zfs-2026-11-02", first, "2026-11-15T08:00:00.000Z"),
				("zfs-2026-11-02", second, "2026-11-15T10:00:00.000Z")
			],
			"oldest first, a repeat kept as first recorded"
		);
		assert!(
			sqlx::query("UPDATE cards_essay_link SET recorded_at = 'x'").execute(&mut *conn).await.is_err(),
			"UPDATE is refused"
		);
		assert!(sqlx::query("DELETE FROM cards_essay_link").execute(&mut *conn).await.is_err(), "DELETE is refused");
	}

	#[tokio::test]
	async fn an_essay_whose_take_is_deleted_is_withdrawn_both_steps_at_once_linked_or_not_and_leaves_with_the_account() {
		let pool = pool().await;
		let mut conn = pool.acquire().await.unwrap();
		for subject in ["me", "them"] {
			sqlx::query("INSERT INTO account (subject_id, user_handle) VALUES (?1, ?1)")
				.bind(subject)
				.execute(&mut *conn)
				.await
				.unwrap();
		}
		for key in ["zfs-2026-11-02", "ext4-2026-11-03", "btrfs-2026-11-04"] {
			keep(&mut conn, &essay(key, b"{}", "t")).await.unwrap();
		}
		record_link(&mut conn, "ext4-2026-11-03", "dQw4w9WgXcQ", "t").await.unwrap();
		assert!(withdraw(&mut conn, "zfs-2026-11-02", "me", "note-1", 1234, "2026-11-21T00:00:00.000Z").await.unwrap());
		assert!(!withdraw(&mut conn, "zfs-2026-11-02", "me", "note-2", 1, "t").await.unwrap(), "once");
		assert!(withdraw(&mut conn, "ext4-2026-11-03", "me", "note-3", 0, "t").await.unwrap());
		assert!(withdraw(&mut conn, "btrfs-2026-11-04", "them", "note-4", 0, "t").await.unwrap());
		assert!(withdraw(&mut conn, "xfs-2026-11-05", "me", "note-5", 0, "t").await.is_err(), "no such essay");
		assert_eq!(gone(&mut conn).await.unwrap(), ["btrfs-2026-11-04", "ext4-2026-11-03", "zfs-2026-11-02"]);
		let mine = withdrawn(&mut conn, "me").await.unwrap();
		assert_eq!(
			mine.iter().map(|w| (w.key.as_str(), w.note.as_str(), w.size, w.youtube_id.as_deref())).collect::<Vec<_>>(),
			[("ext4-2026-11-03", "note-3", 0, Some("dQw4w9WgXcQ")), ("zfs-2026-11-02", "note-1", 1234, None)],
			"its link kept beside it; only mine"
		);
		for statement in [
			"UPDATE cards_essay_video_deletion SET size = 0",
			"DELETE FROM cards_essay_video_deletion",
			"INSERT INTO cards_essay_video_deletion VALUES ('ext4-2026-11-03', 'video', 'take-deleted', 'me', NULL, NULL, 0, 't')",
			"INSERT INTO cards_essay_video_deletion VALUES ('ext4-2026-11-03', 'video', 'let-go', NULL, 'aaaaaaaaaaa', NULL, 0, 't')",
		] {
			assert!(sqlx::query(statement).execute(&mut *conn).await.is_err(), "{statement}");
		}

		sqlx::query("DELETE FROM account WHERE subject_id = 'me'").execute(&mut *conn).await.unwrap();
		assert!(withdrawn(&mut conn, "me").await.unwrap().is_empty(), "a receipt naming my note leaves with my account");
		assert_eq!(gone(&mut conn).await.unwrap(), ["btrfs-2026-11-04"], "theirs stays");
	}
}

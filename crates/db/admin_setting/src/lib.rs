//! Home's admin settings: the levers the operator turns while home runs
//! (paulgsc/server#501, ADMIN1; paulgsc/some-ui#2357).
//!
//! - **Append-only.** [`append`] adds a revision, or nothing when the value
//!   is the one standing; nothing updates or deletes a row (the migration's
//!   triggers abort both), so every value a setting held, who set it and
//!   when, stays readable through [`history`].
//! - **Values only, never schemas.** This crate stores a namespace's JSON as
//!   its caller gives it. What a namespace may hold, and its default when
//!   nothing was ever set, live in `file_host::settings`, which checks a
//!   value before it reaches [`append`].
//! - **Nobody's.** A row names a role ([`SetBy`]), never a subject, so the
//!   history outlives any account and names no person (docs/identity.md
//!   invariant 37).
//!
//! The only writer is `file_host`'s `PUT /api/v1/admin/settings/:namespace`,
//! behind the operator gate: `git grep -n admin_setting -- '*.rs'` shows no
//! other.

use sqlx::SqliteConnection;

/// The largest value a namespace may hold, in bytes of JSON. A lever is a
/// handful of fields; this leaves room for a few dozen.
pub const BODY_CEILING: usize = 4 * 1024;

/// The most revisions [`history`] reads at once, newest first.
pub const HISTORY_CEILING: i64 = 100;

/// Who set a revision: a role, never a subject.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetBy {
	/// The owner's account: the only account, with `OPERATOR_SUBJECTS` unset
	/// (docs/identity.md, "The owner's account").
	Owner,
	/// A subject `OPERATOR_SUBJECTS` lists.
	Operator,
}

impl SetBy {
	/// The role as stored and served.
	#[must_use]
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::Owner => "owner",
			Self::Operator => "operator",
		}
	}

	fn parse(stored: &str) -> Result<Self, sqlx::Error> {
		match stored {
			"owner" => Ok(Self::Owner),
			"operator" => Ok(Self::Operator),
			other => Err(sqlx::Error::Decode((String::from("admin_setting.set_by is not a role: ") + other).into())),
		}
	}
}

/// One revision of one namespace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Revision {
	pub revision: i64,
	/// The value as JSON, as it was accepted.
	pub body: String,
	pub set_by: SetBy,
	/// ISO-8601 UTC.
	pub set_at: String,
}

/// What [`append`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
	/// A new revision.
	Added,
	/// The value was the one standing; nothing was written.
	Unchanged,
}

/// The namespace's standing revision: its highest, or `None` while the
/// operator has never set it.
///
/// # Errors
/// A storage failure.
pub async fn current(conn: &mut SqliteConnection, namespace: &str) -> Result<Option<Revision>, sqlx::Error> {
	let row = sqlx::query!(
		"SELECT revision, body, set_by, set_at FROM admin_setting WHERE namespace = ?1 ORDER BY revision DESC LIMIT 1",
		namespace
	)
	.fetch_optional(conn)
	.await?;
	row
		.map(|row| {
			Ok(Revision {
				revision: row.revision,
				body: row.body,
				set_by: SetBy::parse(&row.set_by)?,
				set_at: row.set_at,
			})
		})
		.transpose()
}

/// The namespace's revisions, newest first, at most [`HISTORY_CEILING`].
///
/// # Errors
/// A storage failure.
pub async fn history(conn: &mut SqliteConnection, namespace: &str) -> Result<Vec<Revision>, sqlx::Error> {
	let rows = sqlx::query!(
		"SELECT revision, body, set_by, set_at FROM admin_setting WHERE namespace = ?1 ORDER BY revision DESC LIMIT ?2",
		namespace,
		HISTORY_CEILING
	)
	.fetch_all(conn)
	.await?;
	rows
		.into_iter()
		.map(|row| {
			Ok(Revision {
				revision: row.revision,
				body: row.body,
				set_by: SetBy::parse(&row.set_by)?,
				set_at: row.set_at,
			})
		})
		.collect()
}

/// Sets `namespace` to `body` as `set_by`, as of `now`: a new revision, or
/// nothing when `body` is byte for byte the value standing.
///
/// Run it in a `BEGIN IMMEDIATE` transaction, so the standing revision it
/// reads and the one it writes see one snapshot, and two writes racing
/// cannot take the same number.
///
/// # Errors
/// A storage failure.
pub async fn append(conn: &mut SqliteConnection, namespace: &str, body: &str, set_by: SetBy, now: &str) -> Result<(Change, Revision), sqlx::Error> {
	let standing = current(conn, namespace).await?;
	if let Some(standing) = standing.as_ref().filter(|standing| standing.body == body) {
		return Ok((Change::Unchanged, standing.clone()));
	}
	let revision = standing.map_or(1, |standing| standing.revision + 1);
	let role = set_by.as_str();
	sqlx::query!(
		"INSERT INTO admin_setting (namespace, revision, body, set_by, set_at) VALUES (?1, ?2, ?3, ?4, ?5)",
		namespace,
		revision,
		body,
		role,
		now
	)
	.execute(conn)
	.await?;
	Ok((
		Change::Added,
		Revision {
			revision,
			body: body.to_owned(),
			set_by,
			set_at: now.to_owned(),
		},
	))
}

#[cfg(test)]
mod tests {
	use super::{append, current, history, Change, SetBy, HISTORY_CEILING};
	use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};

	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");
	const NS: &str = "rust-rounds";
	const T0: &str = "2026-11-08T00:00:00Z";
	const T1: &str = "2026-11-08T01:00:00Z";
	const T2: &str = "2026-11-08T02:00:00Z";

	async fn pool() -> SqlitePool {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();
		pool
	}

	#[tokio::test]
	async fn an_unset_namespace_has_no_revision() {
		let pool = pool().await;
		let mut conn = pool.acquire().await.unwrap();
		assert_eq!(current(&mut conn, NS).await.unwrap(), None);
		assert!(history(&mut conn, NS).await.unwrap().is_empty());
	}

	/// ADMIN1's acceptance: an update adds a revision, and every earlier one
	/// still reads.
	#[tokio::test]
	async fn each_change_adds_a_revision_and_every_earlier_one_still_reads() {
		let pool = pool().await;
		let mut conn = pool.acquire().await.unwrap();
		let (change, first) = append(&mut conn, NS, r#"{"recording":false}"#, SetBy::Owner, T0).await.unwrap();
		assert_eq!((change, first.revision), (Change::Added, 1));
		let (change, second) = append(&mut conn, NS, r#"{"recording":true}"#, SetBy::Operator, T1).await.unwrap();
		assert_eq!((change, second.revision), (Change::Added, 2));

		assert_eq!(current(&mut conn, NS).await.unwrap(), Some(second.clone()));
		assert_eq!(history(&mut conn, NS).await.unwrap(), [second, first]);
	}

	#[tokio::test]
	async fn the_value_standing_again_writes_nothing() {
		let pool = pool().await;
		let mut conn = pool.acquire().await.unwrap();
		let (_, first) = append(&mut conn, NS, r#"{"recording":false}"#, SetBy::Owner, T0).await.unwrap();
		let (change, standing) = append(&mut conn, NS, r#"{"recording":false}"#, SetBy::Owner, T1).await.unwrap();
		assert_eq!((change, &standing), (Change::Unchanged, &first), "still dated when it was set");
		assert_eq!(history(&mut conn, NS).await.unwrap().len(), 1);
	}

	#[tokio::test]
	async fn namespaces_are_apart() {
		let pool = pool().await;
		let mut conn = pool.acquire().await.unwrap();
		append(&mut conn, NS, r#"{"recording":false}"#, SetBy::Owner, T0).await.unwrap();
		let (_, other) = append(&mut conn, "elsewhere", "{}", SetBy::Owner, T1).await.unwrap();
		assert_eq!(other.revision, 1);
		assert_eq!(
			current(&mut conn, NS).await.unwrap().map(|standing| standing.body),
			Some(r#"{"recording":false}"#.to_owned())
		);
	}

	#[tokio::test]
	async fn no_row_is_ever_updated_or_deleted() {
		let pool = pool().await;
		let mut conn = pool.acquire().await.unwrap();
		append(&mut conn, NS, r#"{"recording":false}"#, SetBy::Owner, T0).await.unwrap();
		assert!(sqlx::query("UPDATE admin_setting SET body = '{}'").execute(&mut *conn).await.is_err());
		assert!(sqlx::query("DELETE FROM admin_setting").execute(&mut *conn).await.is_err());
		assert!(
			sqlx::query("INSERT INTO admin_setting (namespace, revision, body, set_by, set_at) VALUES ('x', 1, '{}', 'subject-a', ?1)")
				.bind(T2)
				.execute(&mut *conn)
				.await
				.is_err()
		);
		assert_eq!(history(&mut conn, NS).await.unwrap().len(), 1);
	}

	#[tokio::test]
	async fn history_is_bounded_newest_first() {
		let pool = pool().await;
		let mut conn = pool.acquire().await.unwrap();
		for step in 0..=HISTORY_CEILING {
			append(&mut conn, NS, &(String::from("{\"step\":") + &step.to_string() + "}"), SetBy::Owner, T0)
				.await
				.unwrap();
		}
		let read = history(&mut conn, NS).await.unwrap();
		assert_eq!(read.len(), usize::try_from(HISTORY_CEILING).unwrap());
		assert_eq!(read.first().map(|revision| revision.revision), Some(HISTORY_CEILING + 1));
	}
}

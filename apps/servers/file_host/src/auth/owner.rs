//! The owner's account: the only account, when exactly one exists.
//!
//! Home is one person's box (paulgsc/some-ui#2174, #510). The owner's account
//! runs the operator routes ([`super::operator`]) and is whose copy the Jobs
//! mailbox poller keeps (`crate::mailbox`), and nothing in the environment
//! names it: no subject id is copied out of a log line into `.env`.
//!
//! **Why "the only account" and not "the first".** The auth tables keep no
//! timeline (docs/identity.md invariant 10): nothing records which account
//! came first, and `SQLite`'s rowid is no order to lean on (a `TEXT PRIMARY
//! KEY` table's rowid can change under `VACUUM`). "The only account" needs no
//! new column and no state outside the `account` table, so it is the same
//! after a restart, a backup restored or a `VACUUM`, and it reads the same
//! to every caller. It holds because account creation closes once an
//! account exists (`AUTH_OPEN_SIGNUP`, off by default), so a box has one
//! account unless its owner opened sign-up for a product; then
//! `OPERATOR_SUBJECTS` and `MAILBOX_SUBJECT` say who is who, and until they
//! do nobody is the owner.
//!
//! Resolved on each use, never cached: the owner registers after the box
//! boots, and deleting the account un-owns it the same moment.

use auth_repo::{Accounts, AuthRepository};
use sqlx::SqlitePool;

/// Who the owner is, as far as the `account` table says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Owner {
	/// The only account's subject.
	Subject(String),
	/// No account exists yet: the box is waiting for its owner's passkey.
	NoAccount,
	/// More than one account exists, so none is the owner by itself:
	/// `OPERATOR_SUBJECTS` and `MAILBOX_SUBJECT` have to say.
	Several,
}

impl Owner {
	/// The owner's subject, if there is one.
	#[must_use]
	pub fn subject(&self) -> Option<&str> {
		match self {
			Self::Subject(subject) => Some(subject),
			Self::NoAccount | Self::Several => None,
		}
	}

	/// A short word for a log line: never the subject id.
	#[must_use]
	pub const fn describe(&self) -> &'static str {
		match self {
			Self::Subject(_) => "one account",
			Self::NoAccount => "no account yet",
			Self::Several => "several accounts",
		}
	}
}

impl From<Accounts> for Owner {
	fn from(accounts: Accounts) -> Self {
		match accounts {
			Accounts::None => Self::NoAccount,
			Accounts::One(subject) => Self::Subject(subject),
			Accounts::Several => Self::Several,
		}
	}
}

/// The owner's account, read from `account` now.
///
/// # Errors
/// A storage failure.
pub async fn owner(pool: &SqlitePool) -> Result<Owner, sqlx::Error> {
	AuthRepository::new(pool.clone()).accounts().await.map(Owner::from)
}

/// The subject a deployment's background work runs for: `configured` (an
/// override such as `MAILBOX_SUBJECT`) when it is set, else the owner's
/// account.
///
/// # Errors
/// A storage failure, when the owner had to be read.
pub async fn configured_or_owner(pool: &SqlitePool, configured: Option<&str>) -> Result<Owner, sqlx::Error> {
	match configured {
		Some(subject) => Ok(Owner::Subject(subject.to_owned())),
		None => owner(pool).await,
	}
}

#[cfg(test)]
mod tests {
	use super::{configured_or_owner, owner, Owner};
	use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};

	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");

	async fn pool() -> SqlitePool {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();
		pool
	}

	async fn account(pool: &SqlitePool, subject: &str) {
		sqlx::query("INSERT INTO account (subject_id, user_handle) VALUES (?1, ?2)")
			.bind(subject)
			.bind(subject.as_bytes())
			.execute(pool)
			.await
			.unwrap();
	}

	/// The definition, end to end: nobody, then the only account, then
	/// nobody again once a second exists, and the first again once the
	/// second is gone. Nothing but the `account` table is consulted.
	#[tokio::test]
	async fn the_owner_is_the_only_account_and_nobody_otherwise() {
		let pool = pool().await;
		assert_eq!(owner(&pool).await.unwrap(), Owner::NoAccount);

		account(&pool, "subject-a").await;
		assert_eq!(owner(&pool).await.unwrap(), Owner::Subject(String::from("subject-a")));
		assert_eq!(owner(&pool).await.unwrap().subject(), Some("subject-a"));

		account(&pool, "subject-b").await;
		assert_eq!(owner(&pool).await.unwrap(), Owner::Several);
		assert_eq!(owner(&pool).await.unwrap().subject(), None);

		sqlx::query("DELETE FROM account WHERE subject_id = 'subject-b'").execute(&pool).await.unwrap();
		assert_eq!(owner(&pool).await.unwrap(), Owner::Subject(String::from("subject-a")));
	}

	/// An override names its subject whatever the table holds; without one,
	/// the owner's account is read.
	#[tokio::test]
	async fn an_override_wins_and_an_empty_one_reads_the_owner() {
		let pool = pool().await;
		assert_eq!(configured_or_owner(&pool, None).await.unwrap(), Owner::NoAccount);
		assert_eq!(
			configured_or_owner(&pool, Some("subject-configured")).await.unwrap(),
			Owner::Subject(String::from("subject-configured"))
		);

		account(&pool, "subject-a").await;
		assert_eq!(configured_or_owner(&pool, None).await.unwrap(), Owner::Subject(String::from("subject-a")));
		assert_eq!(
			configured_or_owner(&pool, Some("subject-configured")).await.unwrap(),
			Owner::Subject(String::from("subject-configured")),
			"the override still wins with an account present"
		);
	}

	#[test]
	fn a_log_word_never_carries_the_subject() {
		assert_eq!(Owner::Subject(String::from("subject-secret")).describe(), "one account");
		assert_eq!(Owner::NoAccount.describe(), "no account yet");
		assert_eq!(Owner::Several.describe(), "several accounts");
	}
}

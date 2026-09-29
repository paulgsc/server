use crate::{MAX_PASSKEYS_PER_SUBJECT, MAX_SESSIONS_PER_SUBJECT};
use sqlx::{FromRow, SqlitePool};

/// One passkey as stored: the authenticator's credential id, and the
/// serialised public key and counter `file_host`'s WebAuthn library reads
/// back. Opaque to this crate.
#[derive(Clone, PartialEq, Eq, FromRow)]
pub struct StoredPasskey {
	pub credential_id: Vec<u8>,
	pub passkey: String,
}

// Hand-written so a `{:?}` of a row never prints a credential id; see
// docs/identity.md, "Secrets are wrapped when they exist".
impl std::fmt::Debug for StoredPasskey {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("StoredPasskey").finish_non_exhaustive()
	}
}

/// A live session: whose it is and when it ends (unix seconds).
#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct SessionRow {
	pub subject_id: String,
	pub expires_at: i64,
}

/// What adding a passkey to an existing account did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddPasskey {
	Added,
	/// The account already holds [`MAX_PASSKEYS_PER_SUBJECT`].
	AtCapacity,
	/// No account has this subject id (it was deleted mid-ceremony).
	NoAccount,
}

pub struct AuthRepository {
	pool: SqlitePool,
}

impl AuthRepository {
	#[must_use]
	pub const fn new(pool: SqlitePool) -> Self {
		Self { pool }
	}

	/// Create an account with its first passkey.
	///
	/// Which subject id it gets is the caller's decision. A subject id that
	/// already has an account is a unique violation, which is what refuses a
	/// second claim on the same one.
	///
	/// # Errors
	/// Propagates any `sqlx` failure, including a unique violation if the
	/// subject id, user handle or credential id is already stored.
	pub async fn create_account(&self, subject_id: &str, user_handle: &[u8], passkey: &StoredPasskey) -> Result<(), sqlx::Error> {
		let mut tx = self.pool.begin().await?;

		sqlx::query!("INSERT INTO account (subject_id, user_handle) VALUES (?1, ?2)", subject_id, user_handle)
			.execute(&mut *tx)
			.await?;

		sqlx::query!(
			"INSERT INTO passkey (credential_id, subject_id, passkey) VALUES (?1, ?2, ?3)",
			passkey.credential_id,
			subject_id,
			passkey.passkey
		)
		.execute(&mut *tx)
		.await?;

		tx.commit().await
	}

	/// Whether a subject id has an account.
	///
	/// # Errors
	/// Propagates any `sqlx` failure.
	pub async fn has_account(&self, subject_id: &str) -> Result<bool, sqlx::Error> {
		let count = sqlx::query_scalar!("SELECT COUNT(*) FROM account WHERE subject_id = ?1", subject_id)
			.fetch_one(&self.pool)
			.await?;
		Ok(count > 0)
	}

	/// The account a WebAuthn user handle belongs to, if any.
	///
	/// # Errors
	/// Propagates any `sqlx` failure.
	pub async fn subject_for_user_handle(&self, user_handle: &[u8]) -> Result<Option<String>, sqlx::Error> {
		sqlx::query_scalar!("SELECT subject_id FROM account WHERE user_handle = ?1", user_handle)
			.fetch_optional(&self.pool)
			.await
	}

	/// The WebAuthn user handle of a subject's account, if it has one.
	///
	/// # Errors
	/// Propagates any `sqlx` failure.
	pub async fn user_handle(&self, subject_id: &str) -> Result<Option<Vec<u8>>, sqlx::Error> {
		sqlx::query_scalar!("SELECT user_handle FROM account WHERE subject_id = ?1", subject_id)
			.fetch_optional(&self.pool)
			.await
	}

	/// Every passkey a subject holds. Bounded by
	/// [`MAX_PASSKEYS_PER_SUBJECT`], which [`Self::add_passkey`] enforces.
	///
	/// # Errors
	/// Propagates any `sqlx` failure.
	pub async fn passkeys(&self, subject_id: &str) -> Result<Vec<StoredPasskey>, sqlx::Error> {
		sqlx::query_as!(
			StoredPasskey,
			"SELECT credential_id, passkey FROM passkey WHERE subject_id = ?1 ORDER BY credential_id LIMIT ?2",
			subject_id,
			MAX_PASSKEYS_PER_SUBJECT
		)
		.fetch_all(&self.pool)
		.await
	}

	/// Add a passkey to an existing account, refusing past
	/// [`MAX_PASSKEYS_PER_SUBJECT`].
	///
	/// # Errors
	/// Propagates any `sqlx` failure, including a unique violation if the
	/// credential id is already stored.
	pub async fn add_passkey(&self, subject_id: &str, passkey: &StoredPasskey) -> Result<AddPasskey, sqlx::Error> {
		let mut tx = self.pool.begin().await?;

		let has_account = sqlx::query_scalar!("SELECT COUNT(*) FROM account WHERE subject_id = ?1", subject_id)
			.fetch_one(&mut *tx)
			.await?;
		if has_account == 0 {
			return Ok(AddPasskey::NoAccount);
		}

		let held = sqlx::query_scalar!("SELECT COUNT(*) FROM passkey WHERE subject_id = ?1", subject_id)
			.fetch_one(&mut *tx)
			.await?;
		if held >= MAX_PASSKEYS_PER_SUBJECT {
			return Ok(AddPasskey::AtCapacity);
		}

		sqlx::query!(
			"INSERT INTO passkey (credential_id, subject_id, passkey) VALUES (?1, ?2, ?3)",
			passkey.credential_id,
			subject_id,
			passkey.passkey
		)
		.execute(&mut *tx)
		.await?;

		tx.commit().await?;
		Ok(AddPasskey::Added)
	}

	/// Store a passkey's updated counter and backup state after a sign-in.
	/// Scoped by subject as well as credential id, so a sign-in can only
	/// ever rewrite a passkey belonging to the account it opened.
	///
	/// # Errors
	/// Propagates any `sqlx` failure.
	pub async fn update_passkey(&self, subject_id: &str, passkey: &StoredPasskey) -> Result<(), sqlx::Error> {
		sqlx::query!(
			"UPDATE passkey SET passkey = ?1 WHERE credential_id = ?2 AND subject_id = ?3",
			passkey.passkey,
			passkey.credential_id,
			subject_id
		)
		.execute(&self.pool)
		.await?;
		Ok(())
	}

	/// Start a session, if the subject still has an account, and say whether
	/// it did: store the token's hash, and, in the same transaction, drop
	/// every expired session (anyone's) and trim this subject to
	/// [`MAX_SESSIONS_PER_SUBJECT`], ending the ones closest to expiring.
	///
	/// The account check and the insert are one statement. A sign-in that
	/// verified a passkey just before the account was deleted therefore gets
	/// no session, rather than an orphan one that outlives the account.
	///
	/// # Errors
	/// Propagates any `sqlx` failure.
	pub async fn create_session(&self, token_hash: &[u8], subject_id: &str, expires_at: i64, now: i64) -> Result<bool, sqlx::Error> {
		let mut tx = self.pool.begin().await?;

		sqlx::query!("DELETE FROM auth_session WHERE expires_at <= ?1", now).execute(&mut *tx).await?;

		let created = sqlx::query!(
			"INSERT INTO auth_session (token_hash, subject_id, expires_at)
			 SELECT ?1, ?2, ?3 WHERE EXISTS (SELECT 1 FROM account WHERE subject_id = ?2)",
			token_hash,
			subject_id,
			expires_at
		)
		.execute(&mut *tx)
		.await?
		.rows_affected();
		if created == 0 {
			return Ok(false);
		}

		sqlx::query!(
			"DELETE FROM auth_session
			 WHERE subject_id = ?1
			   AND token_hash NOT IN (
			       SELECT token_hash FROM auth_session
			       WHERE subject_id = ?1
			       ORDER BY expires_at DESC
			       LIMIT ?2
			   )",
			subject_id,
			MAX_SESSIONS_PER_SUBJECT
		)
		.execute(&mut *tx)
		.await?;

		tx.commit().await?;
		Ok(true)
	}

	/// The session a token hash names, if it has not expired by `now`.
	///
	/// # Errors
	/// Propagates any `sqlx` failure.
	pub async fn live_session(&self, token_hash: &[u8], now: i64) -> Result<Option<SessionRow>, sqlx::Error> {
		sqlx::query_as!(
			SessionRow,
			"SELECT subject_id, expires_at FROM auth_session WHERE token_hash = ?1 AND expires_at > ?2",
			token_hash,
			now
		)
		.fetch_optional(&self.pool)
		.await
	}

	/// Move a live session's expiry.
	///
	/// # Errors
	/// Propagates any `sqlx` failure.
	pub async fn extend_session(&self, token_hash: &[u8], expires_at: i64) -> Result<(), sqlx::Error> {
		sqlx::query!("UPDATE auth_session SET expires_at = ?1 WHERE token_hash = ?2", expires_at, token_hash)
			.execute(&self.pool)
			.await?;
		Ok(())
	}

	/// End one session.
	///
	/// # Errors
	/// Propagates any `sqlx` failure.
	pub async fn end_session(&self, token_hash: &[u8]) -> Result<(), sqlx::Error> {
		sqlx::query!("DELETE FROM auth_session WHERE token_hash = ?1", token_hash).execute(&self.pool).await?;
		Ok(())
	}

	/// End every session a subject holds, on every device.
	///
	/// # Errors
	/// Propagates any `sqlx` failure.
	pub async fn end_all_sessions(&self, subject_id: &str) -> Result<(), sqlx::Error> {
		sqlx::query!("DELETE FROM auth_session WHERE subject_id = ?1", subject_id).execute(&self.pool).await?;
		Ok(())
	}
}

#[cfg(test)]
mod tests {
	use super::{AddPasskey, AuthRepository, StoredPasskey};
	use crate::{MAX_PASSKEYS_PER_SUBJECT, MAX_SESSIONS_PER_SUBJECT};
	use sqlx::sqlite::SqlitePoolOptions;

	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");

	async fn repo() -> AuthRepository {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();
		AuthRepository::new(pool)
	}

	fn passkey(id: u8) -> StoredPasskey {
		StoredPasskey {
			credential_id: vec![id; 16],
			passkey: String::from("{}"),
		}
	}

	#[tokio::test]
	async fn an_account_is_found_by_its_user_handle_and_a_subject_holds_one() {
		let repo = repo().await;

		repo.create_account("subject-a", &[1; 16], &passkey(1)).await.unwrap();
		repo.create_account("subject-b", &[2; 16], &passkey(2)).await.unwrap();

		assert_eq!(repo.subject_for_user_handle(&[2; 16]).await.unwrap().as_deref(), Some("subject-b"));
		assert_eq!(repo.user_handle("subject-a").await.unwrap(), Some(vec![1; 16]));
		assert_eq!(repo.passkeys("subject-b").await.unwrap(), vec![passkey(2)]);
		assert!(repo.has_account("subject-a").await.unwrap());
		assert!(!repo.has_account("subject-c").await.unwrap());

		let taken = repo.create_account("subject-a", &[3; 16], &passkey(3)).await.unwrap_err();
		assert!(taken.as_database_error().unwrap().is_unique_violation(), "a subject id holds one account");
		assert!(repo.subject_for_user_handle(&[3; 16]).await.unwrap().is_none(), "and the refused one left nothing behind");
	}

	#[tokio::test]
	async fn a_passkey_past_the_cap_is_refused_not_evicted() {
		let repo = repo().await;
		repo.create_account("subject-local", &[1; 16], &passkey(0)).await.unwrap();

		for id in 1..MAX_PASSKEYS_PER_SUBJECT {
			let id = u8::try_from(id).unwrap();
			assert_eq!(repo.add_passkey("subject-local", &passkey(id)).await.unwrap(), AddPasskey::Added);
		}
		assert_eq!(repo.add_passkey("subject-local", &passkey(200)).await.unwrap(), AddPasskey::AtCapacity);
		assert_eq!(repo.add_passkey("subject-gone", &passkey(201)).await.unwrap(), AddPasskey::NoAccount);
		assert!(repo.passkeys("subject-local").await.unwrap().contains(&passkey(0)), "the first passkey is still there");
	}

	#[tokio::test]
	async fn an_update_cannot_reach_another_subjects_passkey() {
		let repo = repo().await;
		repo.create_account("subject-local", &[1; 16], &passkey(1)).await.unwrap();

		let rewritten = StoredPasskey {
			credential_id: vec![1; 16],
			passkey: String::from("{\"forged\":true}"),
		};
		repo.update_passkey("subject-other", &rewritten).await.unwrap();
		assert_eq!(repo.passkeys("subject-local").await.unwrap(), vec![passkey(1)]);

		repo.update_passkey("subject-local", &rewritten).await.unwrap();
		assert_eq!(repo.passkeys("subject-local").await.unwrap(), vec![rewritten]);
	}

	#[tokio::test]
	async fn sessions_expire_and_a_subject_holds_a_bounded_number() {
		let repo = repo().await;
		repo.create_account("subject-a", &[1; 16], &passkey(1)).await.unwrap();
		repo.create_account("subject-b", &[2; 16], &passkey(2)).await.unwrap();

		assert!(!repo.create_session(b"orphan", "subject-gone", 100, 50).await.unwrap(), "no account, no session");
		assert!(repo.live_session(b"orphan", 60).await.unwrap().is_none());

		assert!(repo.create_session(b"old", "subject-a", 100, 50).await.unwrap());
		assert!(repo.live_session(b"old", 99).await.unwrap().is_some());
		assert!(repo.live_session(b"old", 100).await.unwrap().is_none(), "expiry is exclusive");

		// Creating another session at t=200 purges the expired one outright.
		repo.create_session(b"new", "subject-b", 1_000, 200).await.unwrap();
		repo.extend_session(b"old", 5_000).await.unwrap();
		assert!(repo.live_session(b"old", 300).await.unwrap().is_none(), "a purged session cannot be revived");

		for n in 0..=MAX_SESSIONS_PER_SUBJECT {
			let token = n.to_le_bytes();
			repo.create_session(&token, "subject-a", 10_000 + n, 200).await.unwrap();
		}
		assert!(repo.live_session(&0_i64.to_le_bytes(), 300).await.unwrap().is_none(), "the one closest to expiring ended");
		assert!(repo.live_session(&MAX_SESSIONS_PER_SUBJECT.to_le_bytes(), 300).await.unwrap().is_some());
		assert!(repo.live_session(b"new", 300).await.unwrap().is_some(), "another subject's session is untouched");

		repo.end_all_sessions("subject-a").await.unwrap();
		assert!(repo.live_session(&MAX_SESSIONS_PER_SUBJECT.to_le_bytes(), 300).await.unwrap().is_none());
		repo.end_session(b"new").await.unwrap();
		assert!(repo.live_session(b"new", 300).await.unwrap().is_none());
	}
}

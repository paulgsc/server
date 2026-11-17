use crate::{MAX_PASSKEYS_PER_SUBJECT, MAX_PHONES_PER_SUBJECT, MAX_SESSIONS_PER_SUBJECT};
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

/// A live session: whose it is, when it ends (unix seconds), and whether it
/// is known to be a paired phone's.
#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct SessionRow {
	pub subject_id: String,
	pub expires_at: i64,
	/// `true` once the row says it is a phone's (`kind = 'phone'`). A phone
	/// paired before rows said so reads `false` until it is tagged
	/// ([`AuthRepository::tag_phone`]).
	pub is_phone: bool,
}

/// One paired phone, as its owner may see it. Never its token or the token's
/// hash: `pairing_id` is random and names nothing else.
#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct PairedPhone {
	pub pairing_id: Vec<u8>,
	/// The UTC day it paired (`YYYY-MM-DD`), or `None` for a phone paired
	/// before home kept the day.
	pub paired_on: Option<String>,
	/// The name the owner typed for it, if any. Already bounded by the
	/// caller; this crate stores what it is given.
	pub label: Option<String>,
	pub expires_at: i64,
}

/// A phone being paired: its token's hash, its random pairing id, the UTC day
/// (`YYYY-MM-DD`) and the name the owner typed, if any.
#[derive(Clone, Copy)]
pub struct NewPhone<'a> {
	pub token_hash: &'a [u8],
	pub pairing_id: &'a [u8],
	pub paired_on: &'a str,
	pub label: Option<&'a str>,
}

// Hand-written so a `{:?}` never prints the token's hash.
impl std::fmt::Debug for NewPhone<'_> {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("NewPhone").finish_non_exhaustive()
	}
}

/// What pairing a phone did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairPhone {
	Paired,
	/// The account already holds [`MAX_PHONES_PER_SUBJECT`] live phones.
	AtCapacity,
	/// No account has this subject id (it was deleted while the code was open).
	NoAccount,
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

/// When a new account may be created.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Admission {
	/// Whenever asked.
	Open,
	/// Only while no account exists. The box is one person's
	/// (docs/identity.md, "Who can make an account"), and the check and the
	/// insert are one statement under an immediate transaction, so two first
	/// registrations racing cannot both land.
	FirstOnly,
}

/// How many accounts there are, up to two, and the one's subject when there
/// is exactly one. `file_host` reads the owner's account off this
/// (docs/identity.md, "The owner's account").
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Accounts {
	None,
	One(String),
	Several,
}

pub struct AuthRepository {
	pool: SqlitePool,
}

impl AuthRepository {
	#[must_use]
	pub const fn new(pool: SqlitePool) -> Self {
		Self { pool }
	}

	/// Create an account with its first passkey, if `admission` allows one.
	///
	/// Which subject id it gets is the caller's decision. Returns `false`,
	/// having written nothing, when `admission` is [`Admission::FirstOnly`]
	/// and an account already exists.
	///
	/// # Errors
	/// Propagates any `sqlx` failure, including a unique violation if the
	/// subject id, user handle or credential id is already stored.
	pub async fn create_account(&self, subject_id: &str, user_handle: &[u8], passkey: &StoredPasskey, admission: Admission) -> Result<bool, sqlx::Error> {
		// Immediate, so the existence check below and this insert are one
		// writer's view: a second first registration waits, then finds the
		// first.
		let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;

		let first_only = admission == Admission::FirstOnly;
		let created = sqlx::query!(
			"INSERT INTO account (subject_id, user_handle) SELECT ?1, ?2 WHERE NOT (?3 AND EXISTS (SELECT 1 FROM account))",
			subject_id,
			user_handle,
			first_only
		)
		.execute(&mut *tx)
		.await?
		.rows_affected();
		if created == 0 {
			return Ok(false);
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
		Ok(true)
	}

	/// How many accounts there are, up to two, and the only one's subject.
	///
	/// # Errors
	/// Propagates any `sqlx` failure.
	pub async fn accounts(&self) -> Result<Accounts, sqlx::Error> {
		let mut subjects = sqlx::query_scalar!("SELECT subject_id FROM account LIMIT 2").fetch_all(&self.pool).await?;
		Ok(match subjects.len() {
			0 => Accounts::None,
			1 => Accounts::One(subjects.remove(0)),
			_ => Accounts::Several,
		})
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

	/// Start a browser's session, if the subject still has an account, and say
	/// whether it did: store the token's hash, and, in the same transaction,
	/// drop every expired session (anyone's) and trim this subject's browser
	/// sessions to [`MAX_SESSIONS_PER_SUBJECT`], ending the ones closest to
	/// expiring. A paired phone's session is never trimmed here
	/// ([`Self::pair_phone`]).
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
			   AND kind = 'browser'
			   AND token_hash NOT IN (
			       SELECT token_hash FROM auth_session
			       WHERE subject_id = ?1 AND kind = 'browser'
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

	/// Start a paired phone's session, if the subject still has an account
	/// and fewer than [`MAX_PHONES_PER_SUBJECT`] live phones. A phone past the
	/// cap is refused, never evicted, and no browser sign-in ever ends one: a
	/// phone ends on its own sign-out, on sign-out everywhere, with the
	/// account, or when it expires.
	///
	/// `paired_on` is the UTC day, computed by the caller, and `label` the
	/// name the owner typed, already bounded by the caller. Expired sessions
	/// (anyone's) are dropped first, so an expired phone frees its place.
	///
	/// # Errors
	/// Propagates any `sqlx` failure.
	pub async fn pair_phone(&self, subject_id: &str, phone: &NewPhone<'_>, expires_at: i64, now: i64) -> Result<PairPhone, sqlx::Error> {
		let NewPhone {
			token_hash,
			pairing_id,
			paired_on,
			label,
		} = *phone;
		let mut tx = self.pool.begin().await?;

		sqlx::query!("DELETE FROM auth_session WHERE expires_at <= ?1", now).execute(&mut *tx).await?;

		// One statement, so two pairings racing cannot both take the last place.
		let created = sqlx::query!(
			"INSERT INTO auth_session (token_hash, subject_id, expires_at, kind, pairing_id, paired_on, label)
			 SELECT ?1, ?2, ?3, 'phone', ?4, ?5, ?7
			 WHERE EXISTS (SELECT 1 FROM account WHERE subject_id = ?2)
			   AND (SELECT COUNT(*) FROM auth_session WHERE subject_id = ?2 AND kind = 'phone') < ?6",
			token_hash,
			subject_id,
			expires_at,
			pairing_id,
			paired_on,
			MAX_PHONES_PER_SUBJECT,
			label
		)
		.execute(&mut *tx)
		.await?
		.rows_affected();
		if created == 0 {
			let has_account = sqlx::query_scalar!("SELECT COUNT(*) FROM account WHERE subject_id = ?1", subject_id)
				.fetch_one(&mut *tx)
				.await?;
			return Ok(if has_account == 0 { PairPhone::NoAccount } else { PairPhone::AtCapacity });
		}

		tx.commit().await?;
		Ok(PairPhone::Paired)
	}

	/// Mark a session read through a phone's bearer token as a phone's, with
	/// `pairing_id`, if the row does not say so yet: a phone paired before
	/// rows said what they are. Its day stays unknown. Once tagged, a row is
	/// never written here again. Not held to [`MAX_PHONES_PER_SUBJECT`]: the
	/// phone is already paired, and ending it is what this change prevents;
	/// such rows came from the browser cap, so they stay bounded, and a new
	/// pairing is refused until the owner is back under the cap.
	///
	/// # Errors
	/// Propagates any `sqlx` failure.
	pub async fn tag_phone(&self, token_hash: &[u8], pairing_id: &[u8]) -> Result<(), sqlx::Error> {
		sqlx::query!(
			"UPDATE auth_session SET kind = 'phone', pairing_id = ?2 WHERE token_hash = ?1 AND kind = 'browser'",
			token_hash,
			pairing_id
		)
		.execute(&self.pool)
		.await?;
		Ok(())
	}

	/// The subject's live paired phones, the earliest paired first (a phone
	/// whose day is unknown first of all).
	///
	/// # Errors
	/// Propagates any `sqlx` failure.
	pub async fn phones(&self, subject_id: &str, now: i64) -> Result<Vec<PairedPhone>, sqlx::Error> {
		sqlx::query_as!(
			PairedPhone,
			r#"SELECT pairing_id AS "pairing_id!", paired_on, label, expires_at FROM auth_session
			   WHERE subject_id = ?1 AND kind = 'phone' AND pairing_id IS NOT NULL AND expires_at > ?2
			   ORDER BY paired_on IS NOT NULL, paired_on, expires_at"#,
			subject_id,
			now
		)
		.fetch_all(&self.pool)
		.await
	}

	/// Name one of the subject's live phones (`None` clears the name), and
	/// say whether there was one. Scoped by subject, like [`Self::end_phone`];
	/// an expired phone, listed by nothing, is not renamed.
	///
	/// # Errors
	/// Propagates any `sqlx` failure.
	pub async fn rename_phone(&self, subject_id: &str, pairing_id: &[u8], label: Option<&str>, now: i64) -> Result<bool, sqlx::Error> {
		let renamed = sqlx::query!(
			"UPDATE auth_session SET label = ?3 WHERE subject_id = ?1 AND pairing_id = ?2 AND kind = 'phone' AND expires_at > ?4",
			subject_id,
			pairing_id,
			label,
			now
		)
		.execute(&self.pool)
		.await?
		.rows_affected();
		Ok(renamed > 0)
	}

	/// Sign one of the subject's phones out, and say whether a live one was
	/// there. Scoped by subject, so a pairing id can only ever end a phone of
	/// the account that asks. An expired row is removed too, but reads as
	/// absent, as it does in [`Self::phones`].
	///
	/// # Errors
	/// Propagates any `sqlx` failure.
	pub async fn end_phone(&self, subject_id: &str, pairing_id: &[u8], now: i64) -> Result<bool, sqlx::Error> {
		let live = sqlx::query_scalar!(
			r#"DELETE FROM auth_session WHERE subject_id = ?1 AND pairing_id = ?2 AND kind = 'phone' RETURNING expires_at > ?3 AS "live!: bool""#,
			subject_id,
			pairing_id,
			now
		)
		.fetch_optional(&self.pool)
		.await?;
		Ok(live == Some(true))
	}

	/// The session a token hash names, if it has not expired by `now`.
	///
	/// # Errors
	/// Propagates any `sqlx` failure.
	pub async fn live_session(&self, token_hash: &[u8], now: i64) -> Result<Option<SessionRow>, sqlx::Error> {
		sqlx::query_as!(
			SessionRow,
			r#"SELECT subject_id, expires_at, kind = 'phone' AS "is_phone!: bool" FROM auth_session WHERE token_hash = ?1 AND expires_at > ?2"#,
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
	use super::{Accounts, AddPasskey, Admission, AuthRepository, NewPhone, PairPhone, StoredPasskey};
	use crate::{MAX_PASSKEYS_PER_SUBJECT, MAX_PHONES_PER_SUBJECT, MAX_SESSIONS_PER_SUBJECT};
	use sqlx::sqlite::SqlitePoolOptions;

	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");

	async fn repo() -> AuthRepository {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();
		AuthRepository::new(pool)
	}

	fn phone<'a>(token_hash: &'a [u8], pairing_id: &'a [u8], paired_on: &'a str) -> NewPhone<'a> {
		NewPhone {
			token_hash,
			pairing_id,
			paired_on,
			label: None,
		}
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

		assert!(repo.create_account("subject-a", &[1; 16], &passkey(1), Admission::Open).await.unwrap());
		assert!(repo.create_account("subject-b", &[2; 16], &passkey(2), Admission::Open).await.unwrap());

		assert_eq!(repo.subject_for_user_handle(&[2; 16]).await.unwrap().as_deref(), Some("subject-b"));
		assert_eq!(repo.user_handle("subject-a").await.unwrap(), Some(vec![1; 16]));
		assert_eq!(repo.passkeys("subject-b").await.unwrap(), vec![passkey(2)]);
		assert!(repo.has_account("subject-a").await.unwrap());
		assert!(!repo.has_account("subject-c").await.unwrap());

		let taken = repo.create_account("subject-a", &[3; 16], &passkey(3), Admission::Open).await.unwrap_err();
		assert!(taken.as_database_error().unwrap().is_unique_violation(), "a subject id holds one account");
		assert!(repo.subject_for_user_handle(&[3; 16]).await.unwrap().is_none(), "and the refused one left nothing behind");
	}

	/// The box is one person's: once an account exists, a registration that
	/// may only be the first lands nothing, and the only account is what
	/// `file_host` reads as the owner's.
	#[tokio::test]
	async fn the_first_account_closes_sign_up_and_is_the_only_one() {
		let repo = repo().await;
		assert_eq!(repo.accounts().await.unwrap(), Accounts::None);

		assert!(repo.create_account("subject-a", &[1; 16], &passkey(1), Admission::FirstOnly).await.unwrap());
		assert_eq!(repo.accounts().await.unwrap(), Accounts::One(String::from("subject-a")));

		assert!(!repo.create_account("subject-b", &[2; 16], &passkey(2), Admission::FirstOnly).await.unwrap());
		assert!(!repo.has_account("subject-b").await.unwrap(), "nothing of the refused account was written");
		assert!(repo.subject_for_user_handle(&[2; 16]).await.unwrap().is_none());
		assert!(repo.passkeys("subject-b").await.unwrap().is_empty());

		assert!(
			repo.create_account("subject-b", &[2; 16], &passkey(2), Admission::Open).await.unwrap(),
			"open sign-up admits a second"
		);
		assert_eq!(repo.accounts().await.unwrap(), Accounts::Several, "and then no account is the only one");
	}

	#[tokio::test]
	async fn a_passkey_past_the_cap_is_refused_not_evicted() {
		let repo = repo().await;
		assert!(repo.create_account("subject-local", &[1; 16], &passkey(0), Admission::Open).await.unwrap());

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
		assert!(repo.create_account("subject-local", &[1; 16], &passkey(1), Admission::Open).await.unwrap());

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
		assert!(repo.create_account("subject-a", &[1; 16], &passkey(1), Admission::Open).await.unwrap());
		assert!(repo.create_account("subject-b", &[2; 16], &passkey(2), Admission::Open).await.unwrap());

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

	/// some-ui#3092: "keep phone pairings past 32". Browser sign-ins past the
	/// browser cap end browsers only, never a phone.
	#[tokio::test]
	async fn browser_sign_ins_past_the_cap_never_end_a_phone() {
		let repo = repo().await;
		assert!(repo.create_account("subject-a", &[1; 16], &passkey(1), Admission::Open).await.unwrap());

		// The phone is the session closest to expiring, the one the old cap
		// would have ended first.
		assert_eq!(
			repo.pair_phone("subject-a", &phone(b"phone", &[7; 16], "2026-11-17"), 5_000, 200).await.unwrap(),
			PairPhone::Paired
		);
		for n in 0..=MAX_SESSIONS_PER_SUBJECT {
			repo.create_session(&n.to_le_bytes(), "subject-a", 10_000 + n, 200).await.unwrap();
		}

		let phone = repo.live_session(b"phone", 300).await.unwrap().unwrap();
		assert!(phone.is_phone, "the phone outlived 33 browser sign-ins");
		assert!(repo.live_session(&0_i64.to_le_bytes(), 300).await.unwrap().is_none(), "the oldest browser ended");
		assert!(!repo.live_session(&1_i64.to_le_bytes(), 300).await.unwrap().unwrap().is_phone);

		let phones = repo.phones("subject-a", 300).await.unwrap();
		assert_eq!(phones.len(), 1);
		assert_eq!(phones[0].pairing_id, vec![7; 16]);
		assert_eq!(phones[0].paired_on.as_deref(), Some("2026-11-17"));
		assert_eq!(phones[0].label, None);
		assert!(repo.rename_phone("subject-a", &[7; 16], Some("Pixel"), 300).await.unwrap());
		assert!(!repo.rename_phone("subject-b", &[7; 16], Some("not theirs"), 300).await.unwrap());
		assert_eq!(repo.phones("subject-a", 300).await.unwrap()[0].label.as_deref(), Some("Pixel"));
		assert!(repo.rename_phone("subject-a", &[7; 16], None, 300).await.unwrap());
		assert_eq!(repo.phones("subject-a", 300).await.unwrap()[0].label, None);
		assert_eq!(phones[0].expires_at, 5_000);

		assert!(repo.phones("subject-a", 5_000).await.unwrap().is_empty(), "an expired phone is not listed");
		assert!(!repo.rename_phone("subject-a", &[7; 16], Some("late"), 5_000).await.unwrap(), "nor renamed");
		assert!(!repo.end_phone("subject-a", &[7; 16], 5_000).await.unwrap(), "nor reads as signed out");
		assert!(!repo.rename_phone("subject-a", &[7; 16], None, 0).await.unwrap(), "but its row is gone");
	}

	#[tokio::test]
	async fn a_phone_past_its_cap_is_refused_not_evicted() {
		let repo = repo().await;
		assert!(repo.create_account("subject-a", &[1; 16], &passkey(1), Admission::Open).await.unwrap());

		for n in 0..MAX_PHONES_PER_SUBJECT {
			let id = [u8::try_from(n).unwrap(); 16];
			assert_eq!(
				repo.pair_phone("subject-a", &phone(&n.to_le_bytes(), &id, "2026-11-17"), 10_000, 200).await.unwrap(),
				PairPhone::Paired
			);
		}
		assert_eq!(
			repo.pair_phone("subject-a", &phone(b"one-more", &[200; 16], "2026-11-17"), 10_000, 200).await.unwrap(),
			PairPhone::AtCapacity
		);
		assert!(repo.live_session(&0_i64.to_le_bytes(), 300).await.unwrap().is_some(), "the first phone is still paired");
		assert_eq!(
			repo.pair_phone("subject-gone", &phone(b"orphan", &[201; 16], "2026-11-17"), 10_000, 200).await.unwrap(),
			PairPhone::NoAccount
		);

		// An expired phone frees its place.
		repo.extend_session(&0_i64.to_le_bytes(), 250).await.unwrap();
		assert_eq!(
			repo.pair_phone("subject-a", &phone(b"one-more", &[200; 16], "2026-11-18"), 10_000, 300).await.unwrap(),
			PairPhone::Paired
		);
	}

	#[tokio::test]
	async fn one_phone_signs_out_only_for_its_own_account() {
		let repo = repo().await;
		assert!(repo.create_account("subject-a", &[1; 16], &passkey(1), Admission::Open).await.unwrap());
		assert!(repo.create_account("subject-b", &[2; 16], &passkey(2), Admission::Open).await.unwrap());
		repo.pair_phone("subject-a", &phone(b"phone-a", &[7; 16], "2026-11-17"), 10_000, 200).await.unwrap();
		repo.pair_phone("subject-b", &phone(b"phone-b", &[7; 16], "2026-11-17"), 10_000, 200).await.unwrap();

		assert!(!repo.end_phone("subject-b", &[8; 16], 300).await.unwrap(), "no such phone");
		assert!(repo.end_phone("subject-b", &[7; 16], 300).await.unwrap());
		assert!(repo.live_session(b"phone-b", 300).await.unwrap().is_none());
		assert!(
			repo.live_session(b"phone-a", 300).await.unwrap().is_some(),
			"the same id on another account is another phone"
		);

		repo.end_all_sessions("subject-a").await.unwrap();
		assert!(repo.live_session(b"phone-a", 300).await.unwrap().is_none(), "sign-out everywhere ends phones too");
	}

	/// A phone paired before rows said what they are reads as a browser
	/// until its bearer token is read, then is tagged once, with no day.
	#[tokio::test]
	async fn a_phone_paired_before_kinds_is_tagged_once() {
		let repo = repo().await;
		assert!(repo.create_account("subject-a", &[1; 16], &passkey(1), Admission::Open).await.unwrap());
		repo.create_session(b"legacy", "subject-a", 10_000, 200).await.unwrap();
		assert!(!repo.live_session(b"legacy", 300).await.unwrap().unwrap().is_phone);

		repo.tag_phone(b"legacy", &[9; 16]).await.unwrap();
		repo.tag_phone(b"legacy", &[10; 16]).await.unwrap();
		assert!(repo.live_session(b"legacy", 300).await.unwrap().unwrap().is_phone);
		let phones = repo.phones("subject-a", 300).await.unwrap();
		assert_eq!(phones.len(), 1);
		assert_eq!(phones[0].pairing_id, vec![9; 16], "a second tag changes nothing");
		assert_eq!(phones[0].paired_on, None);
	}
}

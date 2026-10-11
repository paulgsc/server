//! Storage for AI services acting for a subject.
//!
//! The clients that registered, what each subject approved for them (a
//! grant), and the access tokens a grant has handed out. See docs/identity.md,
//! "AI services acting for a subject".
//!
//! Like the rest of this crate: storage only. Tokens arrive already hashed,
//! ids already minted and expiries already computed by `file_host`. Nothing
//! here writes a timestamp of its own, and no column records when anything was
//! made or used.

use crate::{MAX_CLIENTS, MAX_GRANTS_PER_SUBJECT};
use sqlx::{FromRow, SqlitePool};

/// A registered client.
#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct ClientRow {
	pub client_id: String,
	pub client_name: String,
	/// A JSON array of strings, as stored.
	pub redirect_uris: String,
}

/// A grant about to be written, with the first refresh token's hash.
pub struct NewGrant<'a> {
	pub grant_id: &'a str,
	pub subject_id: &'a str,
	pub client_id: &'a str,
	pub scope: &'a str,
	pub resource: &'a str,
	pub refresh_hash: &'a [u8],
	pub refresh_expires_at: i64,
}

/// An access token about to be written.
pub struct NewAccessToken<'a> {
	pub token_hash: &'a [u8],
	pub expires_at: i64,
}

/// What a live access token stands for.
#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct AccessRow {
	pub subject_id: String,
	pub scope: String,
	/// The audience its grant was issued for.
	pub resource: String,
}

/// One connected service, as the subject sees it in Settings.
#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct GrantView {
	pub grant_id: String,
	pub client_name: String,
	pub scope: String,
}

/// What [`OAuthRepository::create_grant`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreateGrant {
	Created,
	/// No account has this subject id: it was deleted after approving.
	NoAccount,
}

/// What [`OAuthRepository::refresh`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refresh {
	/// Rotated: the presented token is now the previous one, and a new access
	/// token was written with the grant's scope.
	Rotated { scope: String },
	/// The presented token had already been replaced, so only a copy could
	/// still hold it. The grant and its tokens are gone.
	Reused,
	/// No live grant holds it, or it belongs to another client.
	Unknown,
}

pub struct OAuthRepository {
	pool: SqlitePool,
}

impl OAuthRepository {
	#[must_use]
	pub const fn new(pool: SqlitePool) -> Self {
		Self { pool }
	}

	/// Register a client. With [`MAX_CLIENTS`] already registered, clients
	/// that hold no grant are removed first (a client mid-approval then
	/// registers again); if every one holds a grant, the new one is refused.
	///
	/// # Errors
	/// Propagates any `sqlx` failure, including a unique violation on a
	/// reused client id.
	pub async fn register_client(&self, client: &ClientRow) -> Result<bool, sqlx::Error> {
		let mut tx = self.pool.begin().await?;
		let count = sqlx::query_scalar!(r#"SELECT COUNT(*) AS "count!: i64" FROM oauth_client"#).fetch_one(&mut *tx).await?;
		if count >= MAX_CLIENTS {
			sqlx::query!("DELETE FROM oauth_client WHERE client_id NOT IN (SELECT client_id FROM oauth_grant)")
				.execute(&mut *tx)
				.await?;
			let left = sqlx::query_scalar!(r#"SELECT COUNT(*) AS "count!: i64" FROM oauth_client"#).fetch_one(&mut *tx).await?;
			if left >= MAX_CLIENTS {
				return Ok(false);
			}
		}
		sqlx::query!(
			"INSERT INTO oauth_client (client_id, client_name, redirect_uris) VALUES (?1, ?2, ?3)",
			client.client_id,
			client.client_name,
			client.redirect_uris
		)
		.execute(&mut *tx)
		.await?;
		tx.commit().await?;
		Ok(true)
	}

	/// A registered client.
	///
	/// # Errors
	/// Propagates any `sqlx` failure.
	pub async fn client(&self, client_id: &str) -> Result<Option<ClientRow>, sqlx::Error> {
		sqlx::query_as!(ClientRow, "SELECT client_id, client_name, redirect_uris FROM oauth_client WHERE client_id = ?1", client_id)
			.fetch_optional(&self.pool)
			.await
	}

	/// Write a grant and its first access token, if the subject still has an
	/// account. A subject past [`MAX_GRANTS_PER_SUBJECT`] loses the grant
	/// closest to expiring, with its tokens; expired grants go first.
	///
	/// The caller holds the account-deletion lock across this call
	/// (docs/identity.md invariant 9); the account check and the writes share
	/// one transaction.
	///
	/// # Errors
	/// Propagates any `sqlx` failure.
	pub async fn create_grant(&self, grant: &NewGrant<'_>, access: &NewAccessToken<'_>, now: i64) -> Result<CreateGrant, sqlx::Error> {
		let mut tx = self.pool.begin().await?;

		let has_account = sqlx::query_scalar!(r#"SELECT EXISTS (SELECT 1 FROM account WHERE subject_id = ?1) AS "exists!: bool""#, grant.subject_id)
			.fetch_one(&mut *tx)
			.await?;
		if !has_account {
			return Ok(CreateGrant::NoAccount);
		}

		sqlx::query!(
			"DELETE FROM oauth_access_token WHERE grant_id IN (SELECT grant_id FROM oauth_grant WHERE subject_id = ?1 AND expires_at <= ?2)",
			grant.subject_id,
			now
		)
		.execute(&mut *tx)
		.await?;
		sqlx::query!("DELETE FROM oauth_grant WHERE subject_id = ?1 AND expires_at <= ?2", grant.subject_id, now)
			.execute(&mut *tx)
			.await?;

		sqlx::query!(
			"INSERT INTO oauth_grant (grant_id, subject_id, client_id, scope, resource, refresh_hash, previous_refresh_hash, expires_at)
			 VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, ?7)",
			grant.grant_id,
			grant.subject_id,
			grant.client_id,
			grant.scope,
			grant.resource,
			grant.refresh_hash,
			grant.refresh_expires_at
		)
		.execute(&mut *tx)
		.await?;
		sqlx::query!(
			"INSERT INTO oauth_access_token (token_hash, grant_id, subject_id, scope, expires_at) VALUES (?1, ?2, ?3, ?4, ?5)",
			access.token_hash,
			grant.grant_id,
			grant.subject_id,
			grant.scope,
			access.expires_at
		)
		.execute(&mut *tx)
		.await?;

		// Past the cap: the grants closest to expiring go, tokens first.
		let surplus = sqlx::query_scalar!(
			"SELECT grant_id FROM oauth_grant WHERE subject_id = ?1
			 ORDER BY expires_at DESC LIMIT -1 OFFSET ?2",
			grant.subject_id,
			MAX_GRANTS_PER_SUBJECT
		)
		.fetch_all(&mut *tx)
		.await?;
		for grant_id in surplus {
			sqlx::query!("DELETE FROM oauth_access_token WHERE grant_id = ?1", grant_id).execute(&mut *tx).await?;
			sqlx::query!("DELETE FROM oauth_grant WHERE grant_id = ?1", grant_id).execute(&mut *tx).await?;
		}

		tx.commit().await?;
		Ok(CreateGrant::Created)
	}

	/// Trade a refresh token for a new one and a new access token.
	///
	/// The grant's other access tokens end, so a grant holds at most one live
	/// access token. Every write here is to a grant row that already exists:
	/// once an account deletion has removed it, a refresh finds nothing and
	/// writes nothing.
	///
	/// # Errors
	/// Propagates any `sqlx` failure.
	pub async fn refresh(
		&self,
		presented: &[u8],
		client_id: &str,
		next_refresh: &[u8],
		refresh_expires_at: i64,
		access: &NewAccessToken<'_>,
		now: i64,
	) -> Result<Refresh, sqlx::Error> {
		let mut tx = self.pool.begin().await?;

		let live = sqlx::query!(
			"SELECT grant_id, subject_id, scope FROM oauth_grant WHERE refresh_hash = ?1 AND client_id = ?2 AND expires_at > ?3",
			presented,
			client_id,
			now
		)
		.fetch_optional(&mut *tx)
		.await?;

		let Some(grant) = live else {
			let replaced = sqlx::query_scalar!("SELECT grant_id FROM oauth_grant WHERE previous_refresh_hash = ?1", presented)
				.fetch_optional(&mut *tx)
				.await?;
			let Some(grant_id) = replaced else {
				return Ok(Refresh::Unknown);
			};
			sqlx::query!("DELETE FROM oauth_access_token WHERE grant_id = ?1", grant_id).execute(&mut *tx).await?;
			sqlx::query!("DELETE FROM oauth_grant WHERE grant_id = ?1", grant_id).execute(&mut *tx).await?;
			tx.commit().await?;
			return Ok(Refresh::Reused);
		};

		sqlx::query!(
			"UPDATE oauth_grant SET previous_refresh_hash = refresh_hash, refresh_hash = ?1, expires_at = ?2 WHERE grant_id = ?3",
			next_refresh,
			refresh_expires_at,
			grant.grant_id
		)
		.execute(&mut *tx)
		.await?;
		sqlx::query!("DELETE FROM oauth_access_token WHERE grant_id = ?1", grant.grant_id).execute(&mut *tx).await?;
		sqlx::query!(
			"INSERT INTO oauth_access_token (token_hash, grant_id, subject_id, scope, expires_at) VALUES (?1, ?2, ?3, ?4, ?5)",
			access.token_hash,
			grant.grant_id,
			grant.subject_id,
			grant.scope,
			access.expires_at
		)
		.execute(&mut *tx)
		.await?;

		tx.commit().await?;
		Ok(Refresh::Rotated { scope: grant.scope })
	}

	/// What a live access token stands for.
	///
	/// # Errors
	/// Propagates any `sqlx` failure.
	pub async fn access_token(&self, token_hash: &[u8], now: i64) -> Result<Option<AccessRow>, sqlx::Error> {
		sqlx::query_as!(
			AccessRow,
			"SELECT t.subject_id, t.scope, g.resource
			 FROM oauth_access_token t JOIN oauth_grant g ON g.grant_id = t.grant_id
			 WHERE t.token_hash = ?1 AND t.expires_at > ?2",
			token_hash,
			now
		)
		.fetch_optional(&self.pool)
		.await
	}

	/// The services a subject has connected, live ones only.
	///
	/// # Errors
	/// Propagates any `sqlx` failure.
	pub async fn grants(&self, subject_id: &str, now: i64) -> Result<Vec<GrantView>, sqlx::Error> {
		sqlx::query_as!(
			GrantView,
			"SELECT g.grant_id, c.client_name, g.scope
			 FROM oauth_grant g JOIN oauth_client c ON c.client_id = g.client_id
			 WHERE g.subject_id = ?1 AND g.expires_at > ?2
			 ORDER BY c.client_name, g.grant_id",
			subject_id,
			now
		)
		.fetch_all(&self.pool)
		.await
	}

	/// Disconnect a service: the subject's grant and its tokens. Another
	/// subject's grant id deletes nothing.
	///
	/// # Errors
	/// Propagates any `sqlx` failure.
	pub async fn delete_grant(&self, subject_id: &str, grant_id: &str) -> Result<bool, sqlx::Error> {
		let mut tx = self.pool.begin().await?;
		let deleted = sqlx::query!("DELETE FROM oauth_grant WHERE grant_id = ?1 AND subject_id = ?2", grant_id, subject_id)
			.execute(&mut *tx)
			.await?
			.rows_affected();
		if deleted > 0 {
			sqlx::query!("DELETE FROM oauth_access_token WHERE grant_id = ?1", grant_id).execute(&mut *tx).await?;
		}
		tx.commit().await?;
		Ok(deleted > 0)
	}
}

#[cfg(test)]
mod tests {
	use super::{ClientRow, CreateGrant, NewAccessToken, NewGrant, OAuthRepository, Refresh};
	use crate::{MAX_CLIENTS, MAX_GRANTS_PER_SUBJECT};
	use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};

	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");

	async fn pool() -> SqlitePool {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();
		for subject in ["alice", "bob"] {
			sqlx::query("INSERT INTO account (subject_id, user_handle) VALUES (?1, ?1)")
				.bind(subject)
				.execute(&pool)
				.await
				.unwrap();
		}
		pool
	}

	fn client(id: &str) -> ClientRow {
		ClientRow {
			client_id: id.to_owned(),
			client_name: String::from("Claude"),
			redirect_uris: String::from(r#"["https://claude.ai/api/mcp/auth_callback"]"#),
		}
	}

	fn grant<'a>(grant_id: &'a str, subject_id: &'a str, refresh_hash: &'a [u8], expires_at: i64) -> NewGrant<'a> {
		NewGrant {
			grant_id,
			subject_id,
			client_id: "c1",
			scope: "lessons:read shelf",
			resource: "https://lessons.test/api/v1/mcp",
			refresh_hash,
			refresh_expires_at: expires_at,
		}
	}

	const fn access(token_hash: &[u8], expires_at: i64) -> NewAccessToken<'_> {
		NewAccessToken { token_hash, expires_at }
	}

	#[tokio::test]
	async fn a_grant_hands_out_one_access_token_that_reads_back_until_it_expires() {
		let repo = OAuthRepository::new(pool().await);
		assert!(repo.register_client(&client("c1")).await.unwrap());
		assert_eq!(
			repo.create_grant(&grant("g1", "alice", b"r1", 1_000), &access(b"a1", 100), 0).await.unwrap(),
			CreateGrant::Created
		);

		let row = repo.access_token(b"a1", 50).await.unwrap().unwrap();
		assert_eq!(row.subject_id, "alice");
		assert_eq!(row.scope, "lessons:read shelf");
		assert_eq!(row.resource, "https://lessons.test/api/v1/mcp");
		assert!(repo.access_token(b"a1", 100).await.unwrap().is_none(), "expired");
		assert!(repo.access_token(b"other", 50).await.unwrap().is_none());
	}

	#[tokio::test]
	async fn a_grant_for_a_deleted_account_is_not_written() {
		let pool = pool().await;
		let repo = OAuthRepository::new(pool.clone());
		sqlx::query("DELETE FROM account WHERE subject_id = 'alice'").execute(&pool).await.unwrap();
		assert_eq!(
			repo.create_grant(&grant("g1", "alice", b"r1", 1_000), &access(b"a1", 100), 0).await.unwrap(),
			CreateGrant::NoAccount
		);
		let rows: i64 = sqlx::query_scalar("SELECT (SELECT COUNT(*) FROM oauth_grant) + (SELECT COUNT(*) FROM oauth_access_token)")
			.fetch_one(&pool)
			.await
			.unwrap();
		assert_eq!(rows, 0);
	}

	#[tokio::test]
	async fn refreshing_rotates_and_presenting_the_replaced_token_ends_the_grant() {
		let repo = OAuthRepository::new(pool().await);
		repo.register_client(&client("c1")).await.unwrap();
		repo.create_grant(&grant("g1", "alice", b"r1", 1_000), &access(b"a1", 100), 0).await.unwrap();

		assert_eq!(
			repo.refresh(b"r1", "other-client", b"r2", 2_000, &access(b"a2", 200), 10).await.unwrap(),
			Refresh::Unknown,
			"another client's grant"
		);
		assert_eq!(
			repo.refresh(b"r1", "c1", b"r2", 2_000, &access(b"a2", 200), 10).await.unwrap(),
			Refresh::Rotated {
				scope: String::from("lessons:read shelf")
			}
		);
		assert!(repo.access_token(b"a1", 10).await.unwrap().is_none(), "the old access token ended");
		assert!(repo.access_token(b"a2", 10).await.unwrap().is_some());

		assert_eq!(repo.refresh(b"r1", "c1", b"r3", 3_000, &access(b"a3", 300), 20).await.unwrap(), Refresh::Reused);
		assert!(repo.access_token(b"a2", 20).await.unwrap().is_none(), "reuse ends the grant's tokens");
		assert_eq!(
			repo.refresh(b"r2", "c1", b"r4", 3_000, &access(b"a4", 300), 20).await.unwrap(),
			Refresh::Unknown,
			"and the grant"
		);
		assert!(repo.grants("alice", 20).await.unwrap().is_empty());
	}

	#[tokio::test]
	async fn an_expired_refresh_token_refreshes_nothing() {
		let repo = OAuthRepository::new(pool().await);
		repo.register_client(&client("c1")).await.unwrap();
		repo.create_grant(&grant("g1", "alice", b"r1", 1_000), &access(b"a1", 100), 0).await.unwrap();
		assert_eq!(repo.refresh(b"r1", "c1", b"r2", 2_000, &access(b"a2", 2_000), 1_000).await.unwrap(), Refresh::Unknown);
	}

	#[tokio::test]
	async fn a_subject_lists_and_deletes_only_its_own_grants() {
		let repo = OAuthRepository::new(pool().await);
		repo.register_client(&client("c1")).await.unwrap();
		repo.create_grant(&grant("g1", "alice", b"r1", 1_000), &access(b"a1", 100), 0).await.unwrap();
		repo.create_grant(&grant("g2", "bob", b"r2", 1_000), &access(b"a2", 100), 0).await.unwrap();

		let listed = repo.grants("alice", 0).await.unwrap();
		assert_eq!(listed.len(), 1);
		assert_eq!(listed.first().unwrap().client_name, "Claude");
		assert!(!repo.delete_grant("alice", "g2").await.unwrap(), "bob's grant is not alice's to delete");
		assert!(repo.access_token(b"a2", 0).await.unwrap().is_some());
		assert!(repo.delete_grant("alice", "g1").await.unwrap());
		assert!(repo.access_token(b"a1", 0).await.unwrap().is_none(), "its token went with it");
	}

	#[tokio::test]
	async fn a_subject_holds_a_bounded_number_of_grants() {
		let repo = OAuthRepository::new(pool().await);
		repo.register_client(&client("c1")).await.unwrap();
		let cap = usize::try_from(MAX_GRANTS_PER_SUBJECT).unwrap();
		for n in 0..=cap {
			let id = String::from("g") + &n.to_string();
			let refresh = id.clone() + "r";
			let token = id.clone() + "a";
			let expires = 1_000 + i64::try_from(n).unwrap();
			repo
				.create_grant(&grant(&id, "alice", refresh.as_bytes(), expires), &access(token.as_bytes(), 100), 0)
				.await
				.unwrap();
		}
		let listed = repo.grants("alice", 0).await.unwrap();
		assert_eq!(listed.len(), cap);
		assert!(listed.iter().all(|grant| grant.grant_id != "g0"), "the one closest to expiring went");
		assert!(repo.access_token(b"g0a", 0).await.unwrap().is_none(), "with its token");
	}

	#[tokio::test]
	async fn a_full_client_table_makes_room_only_from_clients_nobody_approved() {
		let pool = pool().await;
		let repo = OAuthRepository::new(pool.clone());
		for n in 0..MAX_CLIENTS {
			repo.register_client(&client(&(String::from("c") + &n.to_string()))).await.unwrap();
		}
		repo.create_grant(&grant("g1", "alice", b"r1", 1_000), &access(b"a1", 100), 0).await.unwrap();
		assert!(repo.register_client(&client("new")).await.unwrap(), "unapproved clients made room");
		assert!(repo.client("c1").await.unwrap().is_some(), "an approved client stays");
		assert!(repo.client("c2").await.unwrap().is_none());
	}
}

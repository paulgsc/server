use crate::model::{content_hash, validate_round, ParsedRound, Problem, RoundEntry, Witness};
use sqlx::{SqliteConnection, SqlitePool};
use std::collections::BTreeMap;

/// The most rounds one manifest lists.
///
/// Bounded in the query and refused, never silently truncated, above it — the
/// shape `curriculum_repo::MANIFEST_CEILING` takes, for the same reasons. An
/// entry is an id, three scalars and a few witness pairs, so a full manifest is
/// well under a megabyte. It bounds the **listed** rounds; the write route
/// refuses to list one past it, so the manifest refusing is a backstop.
pub const MANIFEST_CEILING: i64 = 1_000;

/// The most rounds, listed and retired, one operator listing returns. Retiring
/// never deletes, so the table only grows; over this the listing is refused,
/// never truncated.
pub const OPERATOR_LISTING_CEILING: i64 = 5_000;

/// What [`RoundRepository::upsert`] did to one round.
///
/// No `MetadataChanged`, unlike `curriculum_repo::Change`: a round has no
/// metadata outside its body, so every change is a change of bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
	/// An id this table had never held: version 1, published now.
	Inserted,
	/// The body's bytes changed: version bumped, `published_at` moved, body
	/// and witness rows replaced.
	ContentChanged,
	/// Byte-identical: nothing written.
	Unchanged,
}

/// Why [`RoundRepository::upsert`] wrote nothing.
#[derive(Debug)]
pub enum WriteError {
	/// The key or body failed [`validate_round`]; every problem, by field.
	Invalid(Vec<Problem>),
	Storage(sqlx::Error),
}

impl std::fmt::Display for WriteError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::Invalid(problems) => {
				f.write_str("not a round this server can store:")?;
				for (field, why) in problems {
					write!(f, " {field} {why};")?;
				}
				Ok(())
			}
			Self::Storage(err) => write!(f, "database error: {err}"),
		}
	}
}

impl std::error::Error for WriteError {}

impl From<sqlx::Error> for WriteError {
	fn from(err: sqlx::Error) -> Self {
		Self::Storage(err)
	}
}

/// A listed round whose option set has a member witnessing some proposition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WitnessingRound {
	pub id: String,
	/// Whether that member is the round's admissible one — the proposition is
	/// the round's answer — rather than only a distractor's.
	pub admissible: bool,
}

pub struct RoundRepository {
	pool: SqlitePool,
}

impl RoundRepository {
	#[must_use]
	pub const fn new(pool: SqlitePool) -> Self {
		Self { pool }
	}

	/// How many rounds the table holds, on `conn`.
	///
	/// # Errors
	/// Propagates any `sqlx` failure.
	pub async fn count(conn: &mut SqliteConnection) -> Result<i64, sqlx::Error> {
		sqlx::query_scalar!(r#"SELECT COUNT(*) AS "count!: i64" FROM leetype_round"#).fetch_one(&mut *conn).await
	}

	/// How many rounds are listed: served by the manifest, not retired.
	///
	/// # Errors
	/// Propagates any `sqlx` failure.
	pub async fn listed_count(conn: &mut SqliteConnection) -> Result<i64, sqlx::Error> {
		sqlx::query_scalar!(r#"SELECT COUNT(*) AS "count!: i64" FROM leetype_round WHERE retired_at IS NULL"#)
			.fetch_one(&mut *conn)
			.await
	}

	/// Write one round under `key`, deciding by [`content_hash`] whether it
	/// changed.
	///
	/// Validates first ([`validate_round`]), so no caller can store a body the
	/// edge table was not derived from: the witness rows are rewritten from
	/// this body, wholesale, in the caller's transaction, exactly when its bytes
	/// change. Byte-identical content writes nothing and moves no
	/// `published_at`. A retired round stays retired.
	///
	/// With `dry_run`, validates, decides and reports, and writes nothing.
	///
	/// # Errors
	/// [`WriteError::Invalid`] for a key or body that fails validation;
	/// [`WriteError::Storage`] for any `sqlx` failure.
	pub async fn upsert(conn: &mut SqliteConnection, key: &str, body: &[u8], now: &str, dry_run: bool) -> Result<Change, WriteError> {
		let round = validate_round(key, body).map_err(WriteError::Invalid)?;
		let hash = content_hash(body);
		// Exact: `validate_round` parsed it as JSON, which is UTF-8.
		let text = String::from_utf8_lossy(body);

		let stored = sqlx::query_scalar!("SELECT content_hash FROM leetype_round WHERE id = ?", key)
			.fetch_optional(&mut *conn)
			.await?;
		let change = match stored {
			None => Change::Inserted,
			Some(stored) if stored != hash => Change::ContentChanged,
			Some(_) => Change::Unchanged,
		};
		if dry_run {
			return Ok(change);
		}

		match change {
			Change::Unchanged => return Ok(change),
			Change::Inserted => {
				sqlx::query!(
					"INSERT INTO leetype_round (id, language, published_at, version, content_hash, retired_at, body) VALUES (?1, ?2, ?3, 1, ?4, NULL, ?5)",
					key,
					round.language,
					now,
					hash,
					text,
				)
				.execute(&mut *conn)
				.await?;
			}
			Change::ContentChanged => {
				sqlx::query!(
					"UPDATE leetype_round SET language = ?2, published_at = ?3, version = version + 1, content_hash = ?4, body = ?5 WHERE id = ?1",
					key,
					round.language,
					now,
					hash,
					text,
				)
				.execute(&mut *conn)
				.await?;
			}
		}
		Self::write_witnesses(conn, &round).await?;
		Ok(change)
	}

	/// Replace `round`'s witness rows with the ones its body yields.
	async fn write_witnesses(conn: &mut SqliteConnection, round: &ParsedRound) -> Result<(), sqlx::Error> {
		sqlx::query!("DELETE FROM leetype_round_witness WHERE round_id = ?", round.id).execute(&mut *conn).await?;
		for (index, witness) in round.witnesses.iter().enumerate() {
			let index = i64::try_from(index).map_err(|err| sqlx::Error::Encode(Box::new(err)))?;
			sqlx::query!(
				"INSERT INTO leetype_round_witness (round_id, member_index, proposition_id, admissible) VALUES (?1, ?2, ?3, ?4)",
				round.id,
				index,
				witness.proposition_id,
				witness.admissible,
			)
			.execute(&mut *conn)
			.await?;
		}
		Ok(())
	}

	/// One round's stored bytes and their hash, or `None` for an id this table
	/// does not hold. Retired rounds included.
	///
	/// # Errors
	/// Propagates any `sqlx` failure.
	pub async fn body(&self, key: &str) -> Result<Option<(String, String)>, sqlx::Error> {
		Ok(
			sqlx::query!("SELECT content_hash, body FROM leetype_round WHERE id = ?", key)
				.fetch_optional(&self.pool)
				.await?
				.map(|row| (row.content_hash, row.body)),
		)
	}

	/// Every **listed** round, by id, with its witnesses and without its body:
	/// what the manifest serves. At most `limit`.
	///
	/// # Errors
	/// Propagates any `sqlx` failure.
	pub async fn entries(&self, limit: i64) -> Result<Vec<RoundEntry>, sqlx::Error> {
		// One read transaction, so the rows and their witnesses are one
		// snapshot: a write between two reads could pair a round with another
		// version's witnesses.
		let mut tx = self.pool.begin().await?;
		let rows = sqlx::query_as!(
			EntryRow,
			r#"SELECT id AS "id!", version, published_at, content_hash, retired_at FROM leetype_round WHERE retired_at IS NULL ORDER BY id LIMIT ?"#,
			limit
		)
		.fetch_all(&mut *tx)
		.await?;
		let entries = Self::with_witnesses(&mut tx, rows).await?;
		tx.commit().await?;
		Ok(entries)
	}

	/// Every round, listed and retired, by id, with witnesses and without
	/// bodies: the operator's view. At most `limit`.
	///
	/// # Errors
	/// Propagates any `sqlx` failure.
	pub async fn all_entries(&self, limit: i64) -> Result<Vec<RoundEntry>, sqlx::Error> {
		let mut tx = self.pool.begin().await?;
		let rows = sqlx::query_as!(
			EntryRow,
			r#"SELECT id AS "id!", version, published_at, content_hash, retired_at FROM leetype_round ORDER BY id LIMIT ?"#,
			limit
		)
		.fetch_all(&mut *tx)
		.await?;
		let entries = Self::with_witnesses(&mut tx, rows).await?;
		tx.commit().await?;
		Ok(entries)
	}

	/// One round, listed or retired, with its witnesses and without its body.
	///
	/// # Errors
	/// Propagates any `sqlx` failure.
	pub async fn entry(conn: &mut SqliteConnection, key: &str) -> Result<Option<RoundEntry>, sqlx::Error> {
		let row = sqlx::query_as!(
			EntryRow,
			r#"SELECT id AS "id!", version, published_at, content_hash, retired_at FROM leetype_round WHERE id = ?"#,
			key
		)
		.fetch_optional(&mut *conn)
		.await?;
		match row {
			Some(row) => Ok(Self::with_witnesses(conn, vec![row]).await?.pop()),
			None => Ok(None),
		}
	}

	async fn with_witnesses(conn: &mut SqliteConnection, rows: Vec<EntryRow>) -> Result<Vec<RoundEntry>, sqlx::Error> {
		let ids: Vec<&str> = rows.iter().map(|row| row.id.as_str()).collect();
		let mut witnesses = Self::witnesses_for(conn, &ids).await?;
		Ok(
			rows
				.into_iter()
				.map(|row| RoundEntry {
					witnesses: witnesses.remove(&row.id).unwrap_or_default(),
					id: row.id,
					version: row.version,
					published_at: row.published_at,
					content_hash: row.content_hash,
					retired_at: row.retired_at,
				})
				.collect(),
		)
	}

	/// The witness rows of every round in `ids`, by round, in option order —
	/// one query for the whole set, never one per round.
	///
	/// # Errors
	/// Propagates any `sqlx` failure.
	pub async fn witnesses_for(conn: &mut SqliteConnection, ids: &[&str]) -> Result<BTreeMap<String, Vec<Witness>>, sqlx::Error> {
		if ids.is_empty() {
			return Ok(BTreeMap::new());
		}
		// A bound parameter (a JSON array for `json_each`), not a tracing
		// argument.
		#[allow(clippy::disallowed_methods)]
		let ids = serde_json::to_string(ids).map_err(|err| sqlx::Error::Encode(Box::new(err)))?;
		let rows = sqlx::query!(
			r#"
			SELECT round_id, proposition_id, admissible AS "admissible: bool"
			FROM leetype_round_witness
			WHERE round_id IN (SELECT value FROM json_each(?1))
			ORDER BY round_id, member_index
			"#,
			ids
		)
		.fetch_all(&mut *conn)
		.await?;
		let mut by_round: BTreeMap<String, Vec<Witness>> = BTreeMap::new();
		for row in rows {
			by_round.entry(row.round_id).or_default().push(Witness {
				proposition_id: row.proposition_id,
				admissible: row.admissible,
			});
		}
		Ok(by_round)
	}

	/// The listed rounds with a member witnessing `proposition_id` (`μ`), by
	/// id, at most `limit` — "which rounds witness CW-P5".
	///
	/// A range read of `idx_leetype_round_witness_proposition`, which already
	/// orders by round id, so the grouping and the order need no sort.
	///
	/// # Errors
	/// Propagates any `sqlx` failure.
	pub async fn rounds_witnessing(&self, proposition_id: &str, limit: i64) -> Result<Vec<WitnessingRound>, sqlx::Error> {
		let rows = sqlx::query!(
			r#"
			SELECT w.round_id AS "round_id!", MAX(w.admissible) AS "admissible!: i64"
			FROM leetype_round_witness w JOIN leetype_round r ON r.id = w.round_id
			WHERE w.proposition_id = ?1 AND r.retired_at IS NULL
			GROUP BY w.round_id
			ORDER BY w.round_id
			LIMIT ?2
			"#,
			proposition_id,
			limit
		)
		.fetch_all(&self.pool)
		.await?;
		Ok(
			rows
				.into_iter()
				.map(|row| WitnessingRound {
					id: row.round_id,
					admissible: row.admissible != 0,
				})
				.collect(),
		)
	}

	/// Take `key` out of the manifest, as of `now`, and report whether the
	/// table holds it. Idempotent: retiring a retired round keeps when it was
	/// first retired. Moves no `version` and no `published_at`.
	///
	/// # Errors
	/// Propagates any `sqlx` failure.
	pub async fn retire(conn: &mut SqliteConnection, key: &str, now: &str) -> Result<bool, sqlx::Error> {
		let updated = sqlx::query!("UPDATE leetype_round SET retired_at = COALESCE(retired_at, ?2) WHERE id = ?1", key, now)
			.execute(&mut *conn)
			.await?;
		Ok(updated.rows_affected() == 1)
	}

	/// Put `key` back in the manifest, and report whether the table holds it.
	/// Idempotent, and moves no `version`. The caller owns the
	/// [`MANIFEST_CEILING`] check, in the same transaction.
	///
	/// # Errors
	/// Propagates any `sqlx` failure.
	pub async fn restore(conn: &mut SqliteConnection, key: &str) -> Result<bool, sqlx::Error> {
		let updated = sqlx::query!("UPDATE leetype_round SET retired_at = NULL WHERE id = ?", key).execute(&mut *conn).await?;
		Ok(updated.rows_affected() == 1)
	}
}

/// One `leetype_round` row as `query_as!` reads it, before its witnesses.
struct EntryRow {
	id: String,
	version: i64,
	published_at: String,
	content_hash: String,
	retired_at: Option<String>,
}

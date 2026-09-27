use crate::model::{content_hash, CurriculumEntry, Level, ManifestEntry};
use sqlx::{SqliteConnection, SqlitePool};

/// The most lessons one manifest lists (#276).
///
/// The corpus grows and the manifest lists all of it, so the bound is in the
/// query per #253 — and exceeding it is a refusal, never a silently short
/// manifest, the same shape `activity_repo::CATALOG_CEILING` takes. At a few
/// hundred bytes of metadata per entry this is well under a megabyte, and a
/// corpus that outgrows it needs a paginated manifest, which the client's
/// `TopikManifestSchema` does not describe yet.
///
/// It bounds the **listed** lessons — the ones the manifest serves — not the
/// table: retired rows accumulate, and are the operator listing's to bound
/// ([`OPERATOR_LISTING_CEILING`]). The write route refuses to list a lesson
/// past it, so the manifest refusing is a backstop, not the operator's first
/// sign.
pub const MANIFEST_CEILING: i64 = 1_000;

/// The most lessons, listed and retired, one operator listing returns.
///
/// Retiring never deletes, so the table only grows — by the few lessons of a
/// weekly batch at a time. Over this the listing is refused, never truncated,
/// like the manifest; a corpus that reaches it needs a paginated listing.
pub const OPERATOR_LISTING_CEILING: i64 = 5_000;

/// The largest lesson file the write route accepts, in bytes.
///
/// A lesson is a few conversations and their probes — tens of kilobytes. The
/// ceiling is on the one input this server stores without reading, so a
/// mistaken paste of something else is refused rather than kept forever.
pub const LESSON_BYTES_CEILING: usize = 1024 * 1024;

/// What [`CurriculumRepository::upsert`] did to one lesson.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
	/// A key this table had never held: version 1, published now.
	Inserted,
	/// The lesson file's bytes changed: version bumped, `published_at` moved,
	/// body replaced. The only case that is new material to anybody.
	ContentChanged,
	/// Only manifest metadata changed (a `displayName`, say): written, but
	/// `version`, `published_at` and `content_hash` untouched — a rename is not
	/// new material.
	MetadataChanged,
	/// Byte-identical content and identical metadata: nothing written.
	Unchanged,
}

pub struct CurriculumRepository {
	pool: SqlitePool,
}

impl CurriculumRepository {
	#[must_use]
	pub const fn new(pool: SqlitePool) -> Self {
		Self { pool }
	}

	/// How many lessons the table holds — on `conn`, so the importer can ask
	/// inside the transaction it writes in.
	///
	/// # Errors
	/// Propagates any `sqlx` failure.
	pub async fn count(conn: &mut SqliteConnection) -> Result<i64, sqlx::Error> {
		sqlx::query_scalar!(r#"SELECT COUNT(*) AS "count!: i64" FROM curriculum"#).fetch_one(&mut *conn).await
	}

	/// How many lessons are listed — served by the manifest, not retired.
	///
	/// # Errors
	/// Propagates any `sqlx` failure.
	pub async fn listed_count(conn: &mut SqliteConnection) -> Result<i64, sqlx::Error> {
		sqlx::query_scalar!(r#"SELECT COUNT(*) AS "count!: i64" FROM curriculum WHERE retired_at IS NULL"#)
			.fetch_one(&mut *conn)
			.await
	}

	/// The stored hash, activity, and manifest fields for `key`, if it exists.
	async fn existing(conn: &mut SqliteConnection, key: &str) -> Result<Option<(String, String, ManifestEntry)>, sqlx::Error> {
		let row = sqlx::query!(
			r#"
			SELECT content_hash, activity_id, level, display_name, description, batch_count, total_questions, total_messages, tags
			FROM curriculum WHERE key = ?
			"#,
			key
		)
		.fetch_optional(&mut *conn)
		.await?;

		row
			.map(|row| {
				let tags = row
					.tags
					.as_deref()
					.map(serde_json::from_str::<Vec<String>>)
					.transpose()
					.map_err(|err| sqlx::Error::Decode(Box::new(err)))?;
				Ok((
					row.content_hash,
					row.activity_id,
					ManifestEntry {
						key: key.to_owned(),
						display_name: row.display_name,
						description: row.description,
						batch_count: row.batch_count,
						total_questions: row.total_questions,
						total_messages: row.total_messages,
						difficulty: row.level.as_deref().and_then(Level::parse),
						tags,
					},
				))
			})
			.transpose()
	}

	/// Write one lesson, deciding by [`content_hash`] — never by mtime, file
	/// name, or size — whether its content changed (#275).
	///
	/// Re-importing byte-identical content with identical metadata writes
	/// nothing at all, and in particular does not move `published_at`: a file
	/// that has not changed must not look new, or #277 would announce a no-op
	/// to everyone. Changed bytes are a version bump; changed metadata alone is
	/// written without one.
	///
	/// With `dry_run`, decides and reports but writes nothing. Runs on `conn`
	/// — the importer's transaction — rather than the pool, so a whole import
	/// commits or rolls back as one.
	///
	/// # Errors
	/// Propagates any `sqlx` failure.
	pub async fn upsert(conn: &mut SqliteConnection, activity_id: &str, entry: &ManifestEntry, body: &[u8], now: &str, dry_run: bool) -> Result<Change, sqlx::Error> {
		let hash = content_hash(body);
		let text = String::from_utf8_lossy(body);
		let level = entry.difficulty.map(Level::as_str);
		// Disallowed for tracing; this is the stored `tags` column.
		#[allow(clippy::disallowed_methods)]
		let tags = entry
			.tags
			.as_ref()
			.map(serde_json::to_string)
			.transpose()
			.map_err(|err| sqlx::Error::Encode(Box::new(err)))?;

		// The activity is metadata too: re-running with a corrected
		// `--activity` must write it (a real `chatgpt-codex-connector` finding
		// on #365).
		let change = match Self::existing(conn, &entry.key).await? {
			None => Change::Inserted,
			Some((stored_hash, _, _)) if stored_hash != hash => Change::ContentChanged,
			Some((_, stored_activity, stored)) if stored != *entry || stored_activity != activity_id => Change::MetadataChanged,
			Some(_) => Change::Unchanged,
		};
		if dry_run {
			return Ok(change);
		}

		match change {
			Change::Unchanged => {}
			Change::Inserted => {
				sqlx::query!(
					r#"
					INSERT INTO curriculum
					    (key, activity_id, level, display_name, description, batch_count, total_questions, total_messages, tags,
					     published_at, version, content_hash, body)
					VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 1, ?11, ?12)
					"#,
					entry.key,
					activity_id,
					level,
					entry.display_name,
					entry.description,
					entry.batch_count,
					entry.total_questions,
					entry.total_messages,
					tags,
					now,
					hash,
					text,
				)
				.execute(&mut *conn)
				.await?;
			}
			Change::ContentChanged => {
				sqlx::query!(
					r#"
					UPDATE curriculum
					SET activity_id = ?2, level = ?3, display_name = ?4, description = ?5, batch_count = ?6,
					    total_questions = ?7, total_messages = ?8, tags = ?9,
					    published_at = ?10, version = version + 1, content_hash = ?11, body = ?12
					WHERE key = ?1
					"#,
					entry.key,
					activity_id,
					level,
					entry.display_name,
					entry.description,
					entry.batch_count,
					entry.total_questions,
					entry.total_messages,
					tags,
					now,
					hash,
					text,
				)
				.execute(&mut *conn)
				.await?;
			}
			Change::MetadataChanged => {
				sqlx::query!(
					r#"
					UPDATE curriculum
					SET activity_id = ?2, level = ?3, display_name = ?4, description = ?5, batch_count = ?6,
					    total_questions = ?7, total_messages = ?8, tags = ?9
					WHERE key = ?1
					"#,
					entry.key,
					activity_id,
					level,
					entry.display_name,
					entry.description,
					entry.batch_count,
					entry.total_questions,
					entry.total_messages,
					tags,
				)
				.execute(&mut *conn)
				.await?;
			}
		}
		Ok(change)
	}

	/// One lesson's stored bytes and their hash, or `None` for a key this
	/// table does not hold.
	///
	/// # Errors
	/// Propagates any `sqlx` failure.
	pub async fn body(&self, key: &str) -> Result<Option<(String, String)>, sqlx::Error> {
		Ok(
			sqlx::query!("SELECT content_hash, body FROM curriculum WHERE key = ?", key)
				.fetch_optional(&self.pool)
				.await?
				.map(|row| (row.content_hash, row.body)),
		)
	}

	/// Every **listed** lesson's row, by key, without bodies — what the
	/// manifest serves. Retired lessons are not in it.
	///
	/// # Errors
	/// Propagates any `sqlx` failure.
	pub async fn entries(&self, limit: i64) -> Result<Vec<CurriculumEntry>, sqlx::Error> {
		sqlx::query_as!(
			EntryRow,
			r#"
			SELECT key AS "key!", activity_id, level, display_name, description, batch_count, total_questions, total_messages, tags,
			       published_at, version, content_hash, retired_at
			FROM curriculum WHERE retired_at IS NULL ORDER BY key LIMIT ?
			"#,
			limit
		)
		.fetch_all(&self.pool)
		.await?
		.into_iter()
		.map(EntryRow::into_entry)
		.collect()
	}

	/// Every lesson's row, listed and retired, by key, without bodies — the
	/// operator's view.
	///
	/// # Errors
	/// Propagates any `sqlx` failure.
	pub async fn all_entries(&self, limit: i64) -> Result<Vec<CurriculumEntry>, sqlx::Error> {
		sqlx::query_as!(
			EntryRow,
			r#"
			SELECT key AS "key!", activity_id, level, display_name, description, batch_count, total_questions, total_messages, tags,
			       published_at, version, content_hash, retired_at
			FROM curriculum ORDER BY key LIMIT ?
			"#,
			limit
		)
		.fetch_all(&self.pool)
		.await?
		.into_iter()
		.map(EntryRow::into_entry)
		.collect()
	}

	/// One lesson's row, listed or retired, without its body.
	///
	/// # Errors
	/// Propagates any `sqlx` failure.
	pub async fn entry(conn: &mut SqliteConnection, key: &str) -> Result<Option<CurriculumEntry>, sqlx::Error> {
		sqlx::query_as!(
			EntryRow,
			r#"
			SELECT key AS "key!", activity_id, level, display_name, description, batch_count, total_questions, total_messages, tags,
			       published_at, version, content_hash, retired_at
			FROM curriculum WHERE key = ?
			"#,
			key
		)
		.fetch_optional(&mut *conn)
		.await?
		.map(EntryRow::into_entry)
		.transpose()
	}

	/// Take `key` out of the manifest, as of `now`, and report whether the
	/// table holds it.
	///
	/// Idempotent: retiring a retired lesson keeps the time it was first
	/// retired. Moves no `version` and no `published_at`, so it is never a
	/// publication — see `20260927000100_add_curriculum_retired_at.up.sql`.
	///
	/// # Errors
	/// Propagates any `sqlx` failure.
	pub async fn retire(conn: &mut SqliteConnection, key: &str, now: &str) -> Result<bool, sqlx::Error> {
		let updated = sqlx::query!("UPDATE curriculum SET retired_at = COALESCE(retired_at, ?2) WHERE key = ?1", key, now)
			.execute(&mut *conn)
			.await?;
		Ok(updated.rows_affected() == 1)
	}

	/// Put `key` back in the manifest, and report whether the table holds it.
	///
	/// Idempotent, and like [`Self::retire`] moves no `version`: restoring
	/// unchanged content announces nothing. The caller owns the
	/// [`MANIFEST_CEILING`] check, in the same transaction.
	///
	/// # Errors
	/// Propagates any `sqlx` failure.
	pub async fn restore(conn: &mut SqliteConnection, key: &str) -> Result<bool, sqlx::Error> {
		let updated = sqlx::query!("UPDATE curriculum SET retired_at = NULL WHERE key = ?", key).execute(&mut *conn).await?;
		Ok(updated.rows_affected() == 1)
	}
}

/// One `curriculum` row as `query_as!` reads it, before `level` and `tags`
/// are decoded.
struct EntryRow {
	key: String,
	activity_id: String,
	level: Option<String>,
	display_name: String,
	description: String,
	batch_count: i64,
	total_questions: i64,
	total_messages: i64,
	tags: Option<String>,
	published_at: String,
	version: i64,
	content_hash: String,
	retired_at: Option<String>,
}

impl EntryRow {
	fn into_entry(self) -> Result<CurriculumEntry, sqlx::Error> {
		Ok(CurriculumEntry {
			key: self.key,
			activity_id: self.activity_id,
			level: self.level.as_deref().and_then(Level::parse),
			display_name: self.display_name,
			description: self.description,
			batch_count: self.batch_count,
			total_questions: self.total_questions,
			total_messages: self.total_messages,
			tags: self
				.tags
				.as_deref()
				.map(serde_json::from_str::<Vec<String>>)
				.transpose()
				.map_err(|err| sqlx::Error::Decode(Box::new(err)))?,
			published_at: self.published_at,
			version: self.version,
			content_hash: self.content_hash,
			retired_at: self.retired_at,
		})
	}
}

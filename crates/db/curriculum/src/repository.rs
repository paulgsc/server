use crate::model::{content_hash, CurriculumEntry, Level, ManifestEntry};
use sqlx::SqlitePool;

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

	/// How many lessons the table holds.
	///
	/// # Errors
	/// Propagates any `sqlx` failure.
	pub async fn count(&self) -> Result<i64, sqlx::Error> {
		sqlx::query_scalar!(r#"SELECT COUNT(*) AS "count!: i64" FROM curriculum"#).fetch_one(&self.pool).await
	}

	/// The stored hash and manifest fields for `key`, if it exists.
	async fn existing(&self, key: &str) -> Result<Option<(String, ManifestEntry)>, sqlx::Error> {
		let row = sqlx::query!(
			r#"
			SELECT content_hash, level, display_name, description, batch_count, total_questions, total_messages, tags
			FROM curriculum WHERE key = ?
			"#,
			key
		)
		.fetch_optional(&self.pool)
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
	/// With `dry_run`, decides and reports but writes nothing.
	///
	/// # Errors
	/// Propagates any `sqlx` failure.
	pub async fn upsert(&self, activity_id: &str, entry: &ManifestEntry, body: &[u8], now: &str, dry_run: bool) -> Result<Change, sqlx::Error> {
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

		let change = match self.existing(&entry.key).await? {
			None => Change::Inserted,
			Some((stored_hash, _)) if stored_hash != hash => Change::ContentChanged,
			Some((_, stored)) if stored != *entry => Change::MetadataChanged,
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
				.execute(&self.pool)
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
				.execute(&self.pool)
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
				.execute(&self.pool)
				.await?;
			}
		}
		Ok(change)
	}

	/// Every lesson's manifest-facing row, by key, without bodies.
	///
	/// # Errors
	/// Propagates any `sqlx` failure.
	pub async fn entries(&self, limit: i64) -> Result<Vec<CurriculumEntry>, sqlx::Error> {
		let rows = sqlx::query!(
			r#"
			SELECT key AS "key!", activity_id, level, display_name, description, batch_count, total_questions, total_messages, tags,
			       published_at, version, content_hash
			FROM curriculum ORDER BY key LIMIT ?
			"#,
			limit
		)
		.fetch_all(&self.pool)
		.await?;

		rows
			.into_iter()
			.map(|row| {
				Ok(CurriculumEntry {
					key: row.key,
					activity_id: row.activity_id,
					level: row.level.as_deref().and_then(Level::parse),
					display_name: row.display_name,
					description: row.description,
					batch_count: row.batch_count,
					total_questions: row.total_questions,
					total_messages: row.total_messages,
					tags: row
						.tags
						.as_deref()
						.map(serde_json::from_str::<Vec<String>>)
						.transpose()
						.map_err(|err| sqlx::Error::Decode(Box::new(err)))?,
					published_at: row.published_at,
					version: row.version,
					content_hash: row.content_hash,
				})
			})
			.collect()
	}
}

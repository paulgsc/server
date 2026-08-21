use crate::model::{LayoutMode, SessionRecord, SessionStatus};
use sqlx::{FromRow, SqlitePool};

/// The row as `SQLite` hands it back.
///
/// The three JSON columns arrive as `TEXT` and are parsed on the way out
/// rather than by sqlx, because `layout` needs the absent-versus-explicit-null
/// distinction that a `Json<T>` column type would flatten.
#[derive(FromRow)]
struct SessionRow {
	id: String,
	name: String,
	status: String,
	layout_mode: String,
	total_duration_ms: i64,
	created_at: String,
	updated_at: String,
	started_at: Option<String>,
	completed_at: Option<String>,
	final_elapsed_ms: Option<i64>,
	activities: String,
	scenes: String,
	layout: Option<String>,
}

/// A row this schema did not write.
#[derive(Debug)]
pub enum RowError {
	/// `status` held something outside the five-value vocabulary.
	UnknownStatus(String),
	/// One of the JSON columns did not parse.
	MalformedJson(&'static str, serde_json::Error),
}

impl std::fmt::Display for RowError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::UnknownStatus(raw) => write!(f, "unknown session status: {raw}"),
			Self::MalformedJson(column, err) => write!(f, "malformed JSON in `{column}`: {err}"),
		}
	}
}

impl std::error::Error for RowError {}

fn parse_json<T: serde::de::DeserializeOwned>(column: &'static str, raw: &str) -> Result<T, RowError> {
	serde_json::from_str(raw).map_err(|err| RowError::MalformedJson(column, err))
}

impl TryFrom<SessionRow> for SessionRecord {
	type Error = RowError;

	fn try_from(row: SessionRow) -> Result<Self, Self::Error> {
		Ok(Self {
			status: SessionStatus::parse(&row.status).ok_or_else(|| RowError::UnknownStatus(row.status.clone()))?,
			layout_mode: LayoutMode::parse(&row.layout_mode),
			activities: parse_json("activities", &row.activities)?,
			scenes: parse_json("scenes", &row.scenes)?,
			// SQL NULL is "absent"; the literal text `null` is an explicit null.
			layout: row.layout.map(|raw| parse_json("layout", &raw)).transpose()?,
			id: row.id,
			name: row.name,
			total_duration_ms: row.total_duration_ms,
			created_at: row.created_at,
			updated_at: row.updated_at,
			started_at: row.started_at,
			completed_at: row.completed_at,
			final_elapsed_ms: row.final_elapsed_ms,
		})
	}
}

/// Anything that can go wrong reading or writing a session.
#[derive(Debug)]
pub enum SessionRepoError {
	Sqlx(sqlx::Error),
	Row(RowError),
	/// Serializing one of the JSON columns on the way in.
	Serialize(serde_json::Error),
}

impl std::fmt::Display for SessionRepoError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::Sqlx(err) => write!(f, "{err}"),
			Self::Row(err) => write!(f, "{err}"),
			Self::Serialize(err) => write!(f, "could not serialize session JSON: {err}"),
		}
	}
}

impl std::error::Error for SessionRepoError {}

impl From<sqlx::Error> for SessionRepoError {
	fn from(err: sqlx::Error) -> Self {
		Self::Sqlx(err)
	}
}

impl From<RowError> for SessionRepoError {
	fn from(err: RowError) -> Self {
		Self::Row(err)
	}
}

impl From<serde_json::Error> for SessionRepoError {
	fn from(err: serde_json::Error) -> Self {
		Self::Serialize(err)
	}
}

pub struct SessionRepository {
	pool: SqlitePool,
}

impl SessionRepository {
	#[must_use]
	pub const fn new(pool: SqlitePool) -> Self {
		Self { pool }
	}

	/// Every session, newest first.
	///
	/// Unpaginated, but no longer only on the premise that justified it
	/// originally — "the client paginates this list in memory today, and a
	/// page parameter nobody sends is a contract nobody tests." That was true
	/// when `GET /sessions` was the only caller. It stopped being true when
	/// `file_host`'s engagement waker (`nudge::waker::consider`) started
	/// calling this once per due subject to find a prepared session: the
	/// waker has no pagination of its own to hand a page parameter to, and no
	/// caller upstream of it either — see `docs/study-nudge.md`'s "a read
	/// reachable from the waker declares its own bound" invariant. The
	/// caller-paginates assumption is retracted, not just amended, since it no
	/// longer describes every caller, only the original one. Bounding this
	/// query is #263 (SLI2); #262 (SLI1) only characterises today's cost, in
	/// `nudge::waker`'s test module.
	///
	/// # Errors
	/// Fails on any `sqlx` error, or if a stored row does not parse.
	pub async fn list(&self) -> Result<Vec<SessionRecord>, SessionRepoError> {
		let rows = sqlx::query_as!(
			SessionRow,
			r#"
			SELECT
			    id as "id!", name, status, layout_mode, total_duration_ms,
			    created_at, updated_at, started_at, completed_at, final_elapsed_ms,
			    activities, scenes, layout
			FROM sessions
			ORDER BY created_at DESC
			"#
		)
		.fetch_all(&self.pool)
		.await?;

		rows.into_iter().map(|row| SessionRecord::try_from(row).map_err(Into::into)).collect()
	}

	/// One session, or `None` if there is no such id.
	///
	/// # Errors
	/// Fails on any `sqlx` error, or if the stored row does not parse.
	pub async fn get(&self, id: &str) -> Result<Option<SessionRecord>, SessionRepoError> {
		let row = sqlx::query_as!(
			SessionRow,
			r#"
			SELECT
			    id as "id!", name, status, layout_mode, total_duration_ms,
			    created_at, updated_at, started_at, completed_at, final_elapsed_ms,
			    activities, scenes, layout
			FROM sessions
			WHERE id = ?
			"#,
			id
		)
		.fetch_optional(&self.pool)
		.await?;

		row.map(|row| SessionRecord::try_from(row).map_err(Into::into)).transpose()
	}

	/// Write a record, creating or replacing. The caller owns the record's
	/// timestamps: it knows whether this is a create, an edit, or a migration
	/// carrying a browser's existing `createdAt` across, and this layer should
	/// not overwrite any of those with `now`.
	///
	/// # Errors
	/// Fails on any `sqlx` error, or if the record's JSON does not serialize.
	pub async fn upsert(&self, record: &SessionRecord) -> Result<(), SessionRepoError> {
		let status = record.status.as_str();
		let layout_mode = record.layout_mode.as_str();
		// `serde_json::to_string` is on clippy.toml's disallowed list to keep
		// eager serialization out of tracing calls. These three are the
		// database write itself, which is the one place the string is the
		// point.
		#[allow(clippy::disallowed_methods)]
		let (activities, scenes, layout) = (
			serde_json::to_string(&record.activities)?,
			serde_json::to_string(&record.scenes)?,
			record.layout.as_ref().map(serde_json::to_string).transpose()?,
		);

		sqlx::query!(
			r#"
			INSERT INTO sessions (
			    id, name, status, layout_mode, total_duration_ms,
			    created_at, updated_at, started_at, completed_at, final_elapsed_ms,
			    activities, scenes, layout
			)
			VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
			ON CONFLICT(id) DO UPDATE SET
			    name              = excluded.name,
			    status            = excluded.status,
			    layout_mode       = excluded.layout_mode,
			    total_duration_ms = excluded.total_duration_ms,
			    updated_at        = excluded.updated_at,
			    started_at        = excluded.started_at,
			    completed_at      = excluded.completed_at,
			    final_elapsed_ms  = excluded.final_elapsed_ms,
			    activities        = excluded.activities,
			    scenes            = excluded.scenes,
			    layout            = excluded.layout
			"#,
			record.id,
			record.name,
			status,
			layout_mode,
			record.total_duration_ms,
			record.created_at,
			record.updated_at,
			record.started_at,
			record.completed_at,
			record.final_elapsed_ms,
			activities,
			scenes,
			layout,
		)
		.execute(&self.pool)
		.await?;

		Ok(())
	}

	/// Remove one session. Returns whether a row was actually removed, so a
	/// caller that cares about 404 can tell.
	///
	/// # Errors
	/// Propagates any `sqlx` failure from the underlying statement.
	pub async fn delete(&self, id: &str) -> Result<bool, SessionRepoError> {
		let result = sqlx::query!("DELETE FROM sessions WHERE id = ?", id).execute(&self.pool).await?;
		Ok(result.rows_affected() > 0)
	}

	/// Remove several, in one transaction so a partial delete cannot survive a
	/// failure halfway through.
	///
	/// # Errors
	/// Propagates any `sqlx` failure from the underlying statements.
	pub async fn delete_many(&self, ids: &[String]) -> Result<u64, SessionRepoError> {
		let mut tx = self.pool.begin().await?;
		let mut deleted = 0_u64;

		for id in ids {
			deleted += sqlx::query!("DELETE FROM sessions WHERE id = ?", id).execute(&mut *tx).await?.rows_affected();
		}

		tx.commit().await?;
		Ok(deleted)
	}

	/// Set one status across a group, and return the affected records.
	///
	/// Status is the only field it is coherent to set identically across an
	/// arbitrary group — the client's `updateStatusMany` makes the same
	/// argument, and this is its server half.
	///
	/// # Errors
	/// Fails on any `sqlx` error, or if a stored row does not parse.
	pub async fn set_status_many(&self, ids: &[String], status: SessionStatus, now: &str) -> Result<Vec<SessionRecord>, SessionRepoError> {
		let status_text = status.as_str();
		let mut tx = self.pool.begin().await?;

		for id in ids {
			sqlx::query!("UPDATE sessions SET status = ?, updated_at = ? WHERE id = ?", status_text, now, id)
				.execute(&mut *tx)
				.await?;
		}

		tx.commit().await?;

		let mut updated = Vec::with_capacity(ids.len());
		for id in ids {
			if let Some(record) = self.get(id).await? {
				updated.push(record);
			}
		}

		Ok(updated)
	}

	/// Sessions whose `started_at` or `completed_at` falls on a given local
	/// day, expressed as the half-open UTC instant range `[from, to)` the
	/// caller's timezone maps that day to.
	///
	/// The range is computed by the caller rather than here because "which day
	/// is it" is a single decision that belongs in one place —
	/// `file_host::nudge::clock` — and a second implementation in SQL is a
	/// second place to be wrong. Both indexed columns are covered.
	///
	/// # Errors
	/// Fails on any `sqlx` error, or if a stored row does not parse.
	pub async fn touched_between(&self, from: &str, to: &str) -> Result<Vec<SessionRecord>, SessionRepoError> {
		let rows = sqlx::query_as!(
			SessionRow,
			r#"
			SELECT
			    id as "id!", name, status, layout_mode, total_duration_ms,
			    created_at, updated_at, started_at, completed_at, final_elapsed_ms,
			    activities, scenes, layout
			FROM sessions
			WHERE (started_at   >= ?1 AND started_at   < ?2)
			   OR (completed_at >= ?1 AND completed_at < ?2)
			"#,
			from,
			to
		)
		.fetch_all(&self.pool)
		.await?;

		rows.into_iter().map(|row| SessionRecord::try_from(row).map_err(Into::into)).collect()
	}
}

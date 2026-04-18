use sqlx::types::Json;
use sqlx::{FromRow, SqlitePool};
use ws_events::tabsched::{CaptureSession, CaptureSummary, SkippedTab, TabCapture};

// ── Row type ────────────────────────────────────────────────────────────────

#[derive(FromRow)]
struct CaptureSessionRow {
	#[allow(dead_code)]
	pub id: i64,
	pub session_id: String,
	pub captured_at: String,
	pub extension_version: String,
	pub total_open_tabs: i64,
	pub captures: Json<Vec<TabCapture>>,
	pub skipped: Json<Vec<SkippedTab>>,
}

impl From<CaptureSessionRow> for CaptureSession {
	fn from(row: CaptureSessionRow) -> Self {
		CaptureSession {
			session_id: row.session_id,
			captured_at: row.captured_at,
			extension_version: row.extension_version,
			total_open_tabs: row.total_open_tabs as u64,
			captures: row.captures.0,
			skipped: row.skipped.0,
		}
	}
}

// ── Stored row ──────────────────────────────────────────────────────────────

pub struct StoredSession {
	pub rowid: i64,
	pub session: CaptureSession,
}

// ── Repository ──────────────────────────────────────────────────────────────

pub struct CaptureSessionRepository {
	pool: SqlitePool,
}

impl CaptureSessionRepository {
	pub fn new(pool: SqlitePool) -> Self {
		Self { pool }
	}

	// ── Single ────────────────────────────────────────────────────────────

	pub async fn create(&self, session: CaptureSession) -> Result<StoredSession, sqlx::Error> {
		let total_open_tabs = session.total_open_tabs as i64;
		let captures = Json(&session.captures);
		let skipped = Json(&session.skipped);
		let rowid = sqlx::query!(
			r#"
            INSERT INTO capture_sessions
                (session_id, captured_at, extension_version, total_open_tabs, captures, skipped)
            VALUES (?, ?, ?, ?, ?, ?)
            "#,
			session.session_id,
			session.captured_at,
			session.extension_version,
			total_open_tabs,
			captures,
			skipped,
		)
		.execute(&self.pool)
		.await?
		.last_insert_rowid();

		Ok(StoredSession { rowid, session })
	}

	pub async fn get_all(&self) -> Result<Vec<CaptureSession>, sqlx::Error> {
		let rows = sqlx::query_as!(
			CaptureSessionRow,
			r#"
            SELECT id as "id!", session_id, captured_at, extension_version,
                   total_open_tabs as "total_open_tabs!", captures as "captures: Json<Vec<TabCapture>>",
                   skipped  as "skipped: Json<Vec<SkippedTab>>"
            FROM capture_sessions
            ORDER BY captured_at DESC
            "#
		)
		.fetch_all(&self.pool)
		.await?;

		Ok(rows.into_iter().map(Into::into).collect())
	}

	pub async fn get_by_session_id(&self, session_id: &str) -> Result<Option<CaptureSession>, sqlx::Error> {
		let row = sqlx::query_as!(
			CaptureSessionRow,
			r#"
            SELECT id as "id!", session_id, captured_at, extension_version,
                   total_open_tabs as "total_open_tabs!", captures as "captures: Json<Vec<TabCapture>>",
                   skipped  as "skipped: Json<Vec<SkippedTab>>"
            FROM capture_sessions
            WHERE session_id = ?
            "#,
			session_id
		)
		.fetch_optional(&self.pool)
		.await?;

		Ok(row.map(Into::into))
	}

	pub async fn get_by_id(&self, id: i64) -> Result<Option<CaptureSession>, sqlx::Error> {
		let row = sqlx::query_as!(
			CaptureSessionRow,
			r#"
            SELECT id, session_id, captured_at, extension_version,
                   total_open_tabs, captures as "captures: Json<Vec<TabCapture>>",
                   skipped  as "skipped: Json<Vec<SkippedTab>>"
            FROM capture_sessions
            WHERE id = ?
            "#,
			id
		)
		.fetch_optional(&self.pool)
		.await?;

		Ok(row.map(Into::into))
	}

	pub async fn update_by_session_id(&self, session_id: &str, session: CaptureSession) -> Result<Option<CaptureSession>, sqlx::Error> {
		let total_open_tabs = session.total_open_tabs as i64;
		let captures = Json(&session.captures);
		let skipped = Json(&session.skipped);
		let rows_affected = sqlx::query!(
			r#"
            UPDATE capture_sessions
            SET captured_at = ?,
                extension_version = ?,
                total_open_tabs = ?,
                captures = ?,
                skipped = ?
            WHERE session_id = ?
            "#,
			session.captured_at,
			session.extension_version,
			total_open_tabs,
			captures,
			skipped,
			session_id,
		)
		.execute(&self.pool)
		.await?
		.rows_affected();

		if rows_affected == 0 {
			return Ok(None);
		}

		self.get_by_session_id(session_id).await
	}

	pub async fn delete_by_session_id(&self, session_id: &str) -> Result<bool, sqlx::Error> {
		let rows_affected = sqlx::query!("DELETE FROM capture_sessions WHERE session_id = ?", session_id)
			.execute(&self.pool)
			.await?
			.rows_affected();

		Ok(rows_affected > 0)
	}

	// ── Batch ─────────────────────────────────────────────────────────────

	pub async fn batch_create(&self, sessions: Vec<CaptureSession>) -> Result<Vec<CaptureSession>, sqlx::Error> {
		let mut tx = self.pool.begin().await?;
		let mut created = Vec::with_capacity(sessions.len());

		for session in sessions {
			let total_open_tabs = session.total_open_tabs as i64;
			let captures = Json(&session.captures);
			let skipped = Json(&session.skipped);
			sqlx::query!(
				r#"
                INSERT INTO capture_sessions
                    (session_id, captured_at, extension_version, total_open_tabs, captures, skipped)
                VALUES (?, ?, ?, ?, ?, ?)
                ON CONFLICT(session_id) DO UPDATE SET
                    captured_at       = excluded.captured_at,
                    extension_version = excluded.extension_version,
                    total_open_tabs   = excluded.total_open_tabs,
                    captures          = excluded.captures,
                    skipped           = excluded.skipped
                "#,
				session.session_id,
				session.captured_at,
				session.extension_version,
				total_open_tabs,
				captures,
				skipped,
			)
			.execute(&mut *tx)
			.await?;

			created.push(session);
		}

		tx.commit().await?;
		Ok(created)
	}

	pub async fn batch_delete(&self, session_ids: Vec<String>) -> Result<u64, sqlx::Error> {
		let mut tx = self.pool.begin().await?;
		let mut deleted = 0;

		for session_id in &session_ids {
			let n = sqlx::query!("DELETE FROM capture_sessions WHERE session_id = ?", session_id)
				.execute(&mut *tx)
				.await?
				.rows_affected();
			deleted += n;
		}

		tx.commit().await?;
		Ok(deleted)
	}

	// ── Query ─────────────────────────────────────────────────────────────

	pub async fn get_by_date(&self, date: &str) -> Result<Vec<CaptureSession>, sqlx::Error> {
		let date_str = format!("{}%", date);
		let rows = sqlx::query_as!(
			CaptureSessionRow,
			r#"
	    SELECT id as "id!", session_id, captured_at, extension_version,
		   total_open_tabs as "total_open_tabs!", captures as "captures: Json<Vec<TabCapture>>",
		   skipped  as "skipped: Json<Vec<SkippedTab>>"
	    FROM capture_sessions
	    WHERE captured_at LIKE ?
	    ORDER BY captured_at DESC
	    "#,
			date_str
		)
		.fetch_all(&self.pool)
		.await?;

		Ok(rows.into_iter().map(Into::into).collect())
	}

	pub async fn get_summaries(&self) -> Result<Vec<CaptureSummary>, sqlx::Error> {
		let all = self.get_all().await?;
		Ok(all.iter().map(CaptureSummary::from).collect())
	}
}

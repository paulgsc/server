use sqlx::types::Json;
use sqlx::{FromRow, SqlitePool};
use ws_events::tabsched::{ExtractedContent, TabCapture, TabSummary};

// ── Row type ─────────────────────────────────────────────────────────────────

#[derive(FromRow)]
struct TabRow {
	tab_id: i64,
	url: String,
	tab_title: String,
	domain: String,
	captured_at: String,
	extractor: String,
	content: Json<ExtractedContent>,
	extraction_ok: bool,
	extraction_error: Option<String>,
	#[allow(dead_code)]
	last_seen_at: String,
	#[allow(dead_code)]
	created_at: String,
}

impl From<TabRow> for TabCapture {
	fn from(row: TabRow) -> Self {
		TabCapture {
			tab_id: row.tab_id,
			url: row.url,
			tab_title: row.tab_title,
			domain: ws_events::tabsched::Domain(row.domain),
			captured_at: row.captured_at,
			extractor: row.extractor,
			content: row.content.0,
			extraction_ok: row.extraction_ok,
			extraction_error: row.extraction_error,
		}
	}
}

// ── Repository ────────────────────────────────────────────────────────────────

pub struct TabRepository {
	pool: SqlitePool,
}

impl TabRepository {
	pub fn new(pool: SqlitePool) -> Self {
		Self { pool }
	}

	// ── Single ────────────────────────────────────────────────────────────

	/// Upsert a single tab. On conflict (same tab_id) → full replace.
	/// This is the canonical write operation; "create" and "update" are
	/// the same thing from the caller's perspective.
	pub async fn upsert(&self, tab: TabCapture) -> Result<TabCapture, sqlx::Error> {
		let content = Json(&tab.content);
		let domain = tab.domain.0.as_str();

		sqlx::query!(
			r#"
            INSERT INTO tabs (
                tab_id, url, tab_title, domain,
                captured_at, extractor,
                content, extraction_ok, extraction_error,
                last_seen_at, created_at
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, datetime('now'))
            ON CONFLICT(tab_id) DO UPDATE SET
                url               = excluded.url,
                tab_title         = excluded.tab_title,
                domain            = excluded.domain,
                captured_at       = excluded.captured_at,
                extractor         = excluded.extractor,
                content           = excluded.content,
                extraction_ok     = excluded.extraction_ok,
                extraction_error  = excluded.extraction_error,
                last_seen_at      = excluded.last_seen_at
            "#,
			tab.tab_id,
			tab.url,
			tab.tab_title,
			domain,
			tab.captured_at,
			tab.extractor,
			content,
			tab.extraction_ok,
			tab.extraction_error,
			tab.captured_at, // last_seen_at = captured_at on insert
		)
		.execute(&self.pool)
		.await?;

		// Return the tab as-is; the DB row is deterministic from the input.
		Ok(tab)
	}

	pub async fn get_by_tab_id(&self, tab_id: i64) -> Result<Option<TabCapture>, sqlx::Error> {
		let row = sqlx::query_as!(
			TabRow,
			r#"
            SELECT
                tab_id, url, tab_title, domain,
                captured_at, extractor,
                content as "content: Json<ExtractedContent>",
                extraction_ok as "extraction_ok: bool",
                extraction_error,
                last_seen_at, created_at
            FROM tabs
            WHERE tab_id = ?
            "#,
			tab_id
		)
		.fetch_optional(&self.pool)
		.await?;

		Ok(row.map(Into::into))
	}

	pub async fn get_all(&self) -> Result<Vec<TabCapture>, sqlx::Error> {
		let rows = sqlx::query_as!(
			TabRow,
			r#"
            SELECT
                tab_id as "tab_id!", url, tab_title, domain,
                captured_at, extractor,
                content as "content: Json<ExtractedContent>",
                extraction_ok as "extraction_ok: bool",
                extraction_error,
                last_seen_at, created_at
            FROM tabs
            ORDER BY last_seen_at DESC
            "#
		)
		.fetch_all(&self.pool)
		.await?;

		Ok(rows.into_iter().map(Into::into).collect())
	}

	pub async fn delete_by_tab_id(&self, tab_id: i64) -> Result<bool, sqlx::Error> {
		let rows_affected = sqlx::query!("DELETE FROM tabs WHERE tab_id = ?", tab_id).execute(&self.pool).await?.rows_affected();

		Ok(rows_affected > 0)
	}

	// ── Batch ─────────────────────────────────────────────────────────────

	/// Primary write path. Upserts each tab independently within a single
	/// transaction. Partial failures roll back the entire batch.
	pub async fn batch_upsert(&self, tabs: Vec<TabCapture>) -> Result<u64, sqlx::Error> {
		let mut tx = self.pool.begin().await?;
		let count = tabs.len() as u64;

		for tab in tabs {
			let content = Json(&tab.content);
			let domain = tab.domain.0.as_str();

			sqlx::query!(
				r#"
                INSERT INTO tabs (
                    tab_id, url, tab_title, domain,
                    captured_at, extractor,
                    content, extraction_ok, extraction_error,
                    last_seen_at, created_at
                )
                VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, datetime('now'))
                ON CONFLICT(tab_id) DO UPDATE SET
                    url               = excluded.url,
                    tab_title         = excluded.tab_title,
                    domain            = excluded.domain,
                    captured_at       = excluded.captured_at,
                    extractor         = excluded.extractor,
                    content           = excluded.content,
                    extraction_ok     = excluded.extraction_ok,
                    extraction_error  = excluded.extraction_error,
                    last_seen_at      = excluded.last_seen_at
                "#,
				tab.tab_id,
				tab.url,
				tab.tab_title,
				domain,
				tab.captured_at,
				tab.extractor,
				content,
				tab.extraction_ok,
				tab.extraction_error,
				tab.captured_at,
			)
			.execute(&mut *tx)
			.await?;
		}

		tx.commit().await?;
		Ok(count)
	}

	pub async fn batch_delete(&self, tab_ids: Vec<i64>) -> Result<u64, sqlx::Error> {
		let mut tx = self.pool.begin().await?;
		let mut deleted = 0u64;

		for tab_id in &tab_ids {
			let n = sqlx::query!("DELETE FROM tabs WHERE tab_id = ?", tab_id).execute(&mut *tx).await?.rows_affected();
			deleted += n;
		}

		tx.commit().await?;
		Ok(deleted)
	}

	// ── Maintenance ───────────────────────────────────────────────────────

	/// Hard-delete tabs not seen within the given TTL window (in days).
	/// Call periodically; appropriate TTL depends on your suspension cadence.
	/// Conservative default: 30 days.
	pub async fn prune_stale(&self, older_than_days: i64) -> Result<u64, sqlx::Error> {
		let rows_affected = sqlx::query!(
			r#"
            DELETE FROM tabs
            WHERE last_seen_at < datetime('now', printf('-%d days', ?))
            "#,
			older_than_days
		)
		.execute(&self.pool)
		.await?
		.rows_affected();

		Ok(rows_affected)
	}

	// ── Query ─────────────────────────────────────────────────────────────

	pub async fn get_summaries(&self) -> Result<Vec<TabSummary>, sqlx::Error> {
		let rows = sqlx::query!(
			r#"
            SELECT
                tab_id as "tab_id!",
                url,
                tab_title,
                domain,
                last_seen_at,
                extraction_ok as "extraction_ok: bool"
            FROM tabs
            ORDER BY last_seen_at DESC
            "#
		)
		.fetch_all(&self.pool)
		.await?;

		Ok(
			rows
				.into_iter()
				.map(|r| TabSummary {
					tab_id: r.tab_id,
					url: r.url,
					tab_title: r.tab_title,
					domain: r.domain,
					last_seen_at: r.last_seen_at,
					extraction_ok: r.extraction_ok,
				})
				.collect(),
		)
	}

	/// Reconciliation: given the set of tab_ids currently known to the
	/// extension, return the ids that exist in the DB but were not reported.
	/// These are candidates for pruning (closed/crashed tabs).
	pub async fn find_absent(&self, active_tab_ids: &[i64]) -> Result<Vec<i64>, sqlx::Error> {
		// SQLite has no array binding; fetch all and diff in Rust.
		let all_ids: Vec<i64> = sqlx::query_scalar!(r#"SELECT tab_id as "tab_id!" FROM tabs"#).fetch_all(&self.pool).await?;

		let active_set: std::collections::HashSet<i64> = active_tab_ids.iter().copied().collect();
		Ok(all_ids.into_iter().filter(|id| !active_set.contains(id)).collect())
	}
}

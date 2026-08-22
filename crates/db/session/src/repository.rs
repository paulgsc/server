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
	/// `upsert` targeted an id that already exists under a different subject.
	///
	/// Not folded into `Sqlx`: nothing failed at the database layer — the
	/// `ON CONFLICT ... WHERE` guard did exactly what it was asked and left
	/// the row untouched. Surfacing that as a distinct, named outcome is what
	/// makes "silently no-op" impossible for a caller to mistake for success;
	/// see `upsert`'s own doc comment for the mechanism.
	SubjectMismatch {
		id: String,
	},
}

impl std::fmt::Display for SessionRepoError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::Sqlx(err) => write!(f, "{err}"),
			Self::Row(err) => write!(f, "{err}"),
			Self::Serialize(err) => write!(f, "could not serialize session JSON: {err}"),
			Self::SubjectMismatch { id } => write!(f, "session `{id}` exists under a different subject; refusing to move it"),
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

	/// Every session belonging to one subject, newest first.
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
	/// longer describes every caller, only the original one.
	///
	/// `subject_id` (#260/SUB2) narrows the scan to one person's rows but does
	/// not bound its size — a subject with thousands of sessions still reads
	/// all of them. That bound is #263 (SLI2), which replaces the waker's
	/// caller with a purpose-built `first_prepared` query instead of tightening
	/// this one; #262 (SLI1) characterises today's cost, in `nudge::waker`'s
	/// test module.
	///
	/// # Errors
	/// Fails on any `sqlx` error, or if a stored row does not parse.
	pub async fn list(&self, subject_id: &str) -> Result<Vec<SessionRecord>, SessionRepoError> {
		let rows = sqlx::query_as!(
			SessionRow,
			r#"
			SELECT
			    id as "id!", name, status, layout_mode, total_duration_ms,
			    created_at, updated_at, started_at, completed_at, final_elapsed_ms,
			    activities, scenes, layout
			FROM sessions
			WHERE subject_id = ?
			ORDER BY created_at DESC
			"#,
			subject_id
		)
		.fetch_all(&self.pool)
		.await?;

		rows.into_iter().map(|row| SessionRecord::try_from(row).map_err(Into::into)).collect()
	}

	/// The one prepared session — `scheduled`, `paused`, or `draft` — this
	/// subject should be offered next, or `None` if they have none.
	///
	/// Replaces `list(subject).iter().find(|s| matches!(s.status, ...))`, the
	/// shape #262 (SLI1) characterised: that reads every row this subject
	/// owns and searches it in Rust.
	///
	/// Two ordering decisions, made deliberately rather than defaulted into:
	///
	/// - **Recency, not insertion order.** The `list`-based code this
	///   replaces took whichever candidate its `ORDER BY created_at DESC`
	///   happened to place first — an accident of insertion order, not a
	///   considered ranking. This orders by `updated_at DESC` instead,
	///   matching what `idx_sessions_status`'s own comment already claims
	///   the ranking is ("candidate ranking scans by status and breaks ties
	///   on `updated_at`"). That is a real, if small, behaviour change from
	///   what is live today.
	/// - **A paused session outranks an untouched draft.** `paused` first,
	///   then `scheduled`, then `draft`, ahead of recency rather than tied
	///   with it. Resuming something this subject already started is more
	///   relevant than surfacing a draft they never opened — even one
	///   touched more recently, e.g. by an unrelated auto-save. `updated_at
	///   DESC` remains the tiebreak within one status.
	///
	/// **Three bounded probes, not one sorted scan.** A single query with
	/// `WHERE status IN (...) ORDER BY CASE status WHEN 'paused' THEN 0 ...
	/// END, updated_at DESC LIMIT 1` was the first shape tried here, and
	/// `LIMIT 1` hides why it is wrong: `idx_sessions_status`
	/// (`(subject_id, status, updated_at DESC)`) can serve `WHERE subject_id
	/// = ? AND status = ?` in already-sorted order for *one* status, but not
	/// a `CASE`-based priority across three at once — `EXPLAIN QUERY PLAN`
	/// on that shape reports `USE TEMP B-TREE FOR ORDER BY`, meaning every
	/// paused, scheduled, and draft row this subject owns has to be read and
	/// sorted before `LIMIT 1` can pick a winner. Returning one row is not
	/// the same as reading one row; the read cost would still grow with how
	/// many prepared sessions this subject has — narrower than #262's
	/// original defect (scoped to three statuses, one subject, instead of
	/// five statuses, every subject) but the same shape of unbounded.
	///
	/// So this issues up to three independent, single-status probes
	/// instead — `WHERE subject_id = ? AND status = ? ORDER BY updated_at
	/// DESC LIMIT 1`, one per status in priority order, returning on the
	/// first hit. Each probe alone *is* served entirely by the index: an
	/// equality on both leading columns leaves `updated_at DESC` already in
	/// index order, so no sort step exists to show up in `EXPLAIN QUERY
	/// PLAN`. The true bound is "at most three index seeks", independent of
	/// how many sessions in any status this subject owns.
	///
	/// # Errors
	/// Fails on any `sqlx` error.
	pub async fn first_prepared(&self, subject_id: &str) -> Result<Option<String>, SessionRepoError> {
		for status in ["paused", "scheduled", "draft"] {
			let id = sqlx::query_scalar!(
				r#"
				SELECT id as "id!"
				FROM sessions
				WHERE subject_id = ? AND status = ?
				ORDER BY updated_at DESC
				LIMIT 1
				"#,
				subject_id,
				status
			)
			.fetch_optional(&self.pool)
			.await?;

			if id.is_some() {
				return Ok(id);
			}
		}

		Ok(None)
	}

	/// One session, or `None` if there is no such id **owned by this subject**.
	///
	/// An id belonging to another subject returns `None`, the same as an id
	/// that does not exist at all — deliberately indistinguishable from the
	/// caller's side. An id is not a capability: if a foreign id returned a
	/// distinguishable "exists, but not yours" outcome, that difference would
	/// itself leak whether the id is in use, which is exactly the kind of
	/// answer this method should not be able to give.
	///
	/// # Errors
	/// Fails on any `sqlx` error, or if the stored row does not parse.
	pub async fn get(&self, subject_id: &str, id: &str) -> Result<Option<SessionRecord>, SessionRepoError> {
		let row = sqlx::query_as!(
			SessionRow,
			r#"
			SELECT
			    id as "id!", name, status, layout_mode, total_duration_ms,
			    created_at, updated_at, started_at, completed_at, final_elapsed_ms,
			    activities, scenes, layout
			FROM sessions
			WHERE id = ? AND subject_id = ?
			"#,
			id,
			subject_id
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
	/// `subject_id` is not a `SessionRecord` field: the record type mirrors
	/// the client's wire shape field-for-field (see this crate's docs), and
	/// ownership is a server-only concept the client never sends. It is
	/// written on `INSERT` and, deliberately, **never on the `UPDATE` half**
	/// of the upsert — `subject_id` does not appear in the `SET` list, so an
	/// edit of an owned row cannot change who owns it even if the caller
	/// passed a different one by mistake.
	///
	/// A conflict on an id owned by a *different* subject is refused outright
	/// rather than silently skipped: the `ON CONFLICT ... WHERE` guard makes
	/// the `DO UPDATE` a no-op in that case, which `SQLite` reports as zero rows
	/// affected. Zero is otherwise unreachable here — a fresh id always
	/// inserts one row, and a conflict on an id this subject owns always
	/// updates one — so it uniquely identifies the mismatch, and this method
	/// turns it into [`SessionRepoError::SubjectMismatch`] instead of letting
	/// a caller read "zero rows changed" as "nothing to do."
	///
	/// # Errors
	/// Fails on any `sqlx` error, if the record's JSON does not serialize, or
	/// with [`SessionRepoError::SubjectMismatch`] if `id` already belongs to
	/// another subject.
	pub async fn upsert(&self, subject_id: &str, record: &SessionRecord) -> Result<(), SessionRepoError> {
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

		let result = sqlx::query!(
			r#"
			INSERT INTO sessions (
			    id, subject_id, name, status, layout_mode, total_duration_ms,
			    created_at, updated_at, started_at, completed_at, final_elapsed_ms,
			    activities, scenes, layout
			)
			VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
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
			WHERE sessions.subject_id = excluded.subject_id
			"#,
			record.id,
			subject_id,
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

		if result.rows_affected() == 0 {
			return Err(SessionRepoError::SubjectMismatch { id: record.id.clone() });
		}

		Ok(())
	}

	/// Remove one session **owned by this subject**. Returns whether a row was
	/// actually removed, so a caller that cares about 404 can tell.
	///
	/// A foreign id is a no-op, not a deletion — the `WHERE` clause simply
	/// matches nothing, and the caller sees `Ok(false)`, the same outcome as
	/// an id that never existed.
	///
	/// # Errors
	/// Propagates any `sqlx` failure from the underlying statement.
	pub async fn delete(&self, subject_id: &str, id: &str) -> Result<bool, SessionRepoError> {
		let result = sqlx::query!("DELETE FROM sessions WHERE id = ? AND subject_id = ?", id, subject_id)
			.execute(&self.pool)
			.await?;
		Ok(result.rows_affected() > 0)
	}

	/// Remove several **owned by this subject**, in one transaction so a
	/// partial delete cannot survive a failure halfway through.
	///
	/// A foreign id among `ids` is skipped, not deleted, and does not count
	/// toward the returned total — the same scoping as [`Self::delete`],
	/// applied per row.
	///
	/// # Errors
	/// Propagates any `sqlx` failure from the underlying statements.
	pub async fn delete_many(&self, subject_id: &str, ids: &[String]) -> Result<u64, SessionRepoError> {
		let mut tx = self.pool.begin().await?;
		let mut deleted = 0_u64;

		for id in ids {
			deleted += sqlx::query!("DELETE FROM sessions WHERE id = ? AND subject_id = ?", id, subject_id)
				.execute(&mut *tx)
				.await?
				.rows_affected();
		}

		tx.commit().await?;
		Ok(deleted)
	}

	/// Set one status across a group **owned by this subject**, and return the
	/// affected records.
	///
	/// Status is the only field it is coherent to set identically across an
	/// arbitrary group — the client's `updateStatusMany` makes the same
	/// argument, and this is its server half.
	///
	/// A foreign id among `ids` is left untouched by the `UPDATE` and then
	/// excluded by [`Self::get`]'s own subject scoping, so it is silently
	/// absent from the returned `Vec` rather than reported as an error —
	/// consistent with `ids` already being allowed to name ids that do not
	/// exist at all. The length of the returned `Vec` is how many of `ids`
	/// were actually owned and updated.
	///
	/// # Errors
	/// Fails on any `sqlx` error, or if a stored row does not parse.
	pub async fn set_status_many(&self, subject_id: &str, ids: &[String], status: SessionStatus, now: &str) -> Result<Vec<SessionRecord>, SessionRepoError> {
		let status_text = status.as_str();
		let mut tx = self.pool.begin().await?;

		for id in ids {
			sqlx::query!(
				"UPDATE sessions SET status = ?, updated_at = ? WHERE id = ? AND subject_id = ?",
				status_text,
				now,
				id,
				subject_id
			)
			.execute(&mut *tx)
			.await?;
		}

		tx.commit().await?;

		let mut updated = Vec::with_capacity(ids.len());
		for id in ids {
			if let Some(record) = self.get(subject_id, id).await? {
				updated.push(record);
			}
		}

		Ok(updated)
	}

	/// Sessions **owned by this subject** whose `started_at` or `completed_at`
	/// falls on a given local day, expressed as the half-open UTC instant
	/// range `[from, to)` the caller's timezone maps that day to.
	///
	/// The range is computed by the caller rather than here because "which day
	/// is it" is a single decision that belongs in one place —
	/// `file_host::nudge::clock` — and a second implementation in SQL is a
	/// second place to be wrong. `idx_sessions_started_at`/`_completed_at`
	/// cover the time predicate; neither leads with `subject_id`, so this scan
	/// costs an extra filter pass rather than an extra index seek until this
	/// method gains a caller that makes tuning it worthwhile.
	///
	/// # Errors
	/// Fails on any `sqlx` error, or if a stored row does not parse.
	pub async fn touched_between(&self, subject_id: &str, from: &str, to: &str) -> Result<Vec<SessionRecord>, SessionRepoError> {
		let rows = sqlx::query_as!(
			SessionRow,
			r#"
			SELECT
			    id as "id!", name, status, layout_mode, total_duration_ms,
			    created_at, updated_at, started_at, completed_at, final_elapsed_ms,
			    activities, scenes, layout
			FROM sessions
			WHERE subject_id = ?3
			  AND ((started_at   >= ?1 AND started_at   < ?2)
			   OR  (completed_at >= ?1 AND completed_at < ?2))
			"#,
			from,
			to,
			subject_id
		)
		.fetch_all(&self.pool)
		.await?;

		rows.into_iter().map(|row| SessionRecord::try_from(row).map_err(Into::into)).collect()
	}
}

/// #260 (SUB2): every method above is scoped to a subject; these tests are
/// what "scoped" means made concrete — a foreign subject's calls must not be
/// able to read, mutate, or move a row they do not own.
///
/// Run against a fresh in-memory database per test rather than the
/// externally-applied `DATABASE_URL` scratch database `cargo test` already
/// needs for `sqlx::query!` to compile: that database is one file shared by
/// every crate's tests in one `rust_ci` run, and cross-subject isolation is
/// exactly the kind of property a shared, accumulating table would make
/// unreliable to assert. `sqlx::migrate!` embeds the same `.up.sql`/`.down.sql`
/// pairs this workspace already has at compile time — the same choice #262
/// made in `nudge::waker`'s test module (`apps/servers/file_host`), reused
/// here rather than reinvented. This crate has no `AppState`/waker machinery
/// to stand up, so unlike that test, a plain `#[tokio::test]` is enough: there
/// is no synchronous metrics-recorder API here to wrap the runtime around.
#[cfg(test)]
mod tests {
	use super::{SessionRepoError, SessionRepository};
	use crate::model::{LayoutMode, SessionRecord, SessionStatus};
	use sqlx::sqlite::SqlitePoolOptions;
	use sqlx::SqlitePool;

	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");

	async fn pool() -> SqlitePool {
		// One connection: SQLite's `:memory:` database is private to the
		// connection that opened it, so a pool free to open a second one
		// could silently hand a query an empty, unmigrated schema.
		let pool = SqlitePoolOptions::new()
			.max_connections(1)
			.connect("sqlite::memory:")
			.await
			.expect("open an in-memory sqlite database");
		MIGRATOR.run(&pool).await.expect("run the workspace migration history");
		pool
	}

	fn fixture(id: &str) -> SessionRecord {
		let mut name = id.to_owned();
		name.push_str(" name");
		SessionRecord {
			id: id.to_owned(),
			name,
			status: SessionStatus::Draft,
			activities: Vec::new(),
			scenes: Vec::new(),
			layout_mode: LayoutMode::Basic,
			layout: None,
			total_duration_ms: 0,
			created_at: "2026-01-01T00:00:00Z".to_owned(),
			updated_at: "2026-01-01T00:00:00Z".to_owned(),
			started_at: None,
			completed_at: None,
			final_elapsed_ms: None,
		}
	}

	#[tokio::test]
	async fn list_returns_only_this_subjects_sessions() {
		let pool = pool().await;
		let repo = SessionRepository::new(pool);

		repo.upsert("subject-a", &fixture("session-a")).await.expect("seed subject-a's session");
		repo.upsert("subject-b", &fixture("session-b")).await.expect("seed subject-b's session");

		let a_sessions = repo.list("subject-a").await.expect("list subject-a's sessions");
		assert_eq!(a_sessions.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), vec!["session-a"]);
	}

	#[tokio::test]
	async fn get_returns_none_for_a_well_formed_id_owned_by_a_different_subject() {
		let pool = pool().await;
		let repo = SessionRepository::new(pool);
		repo.upsert("subject-a", &fixture("session-1")).await.expect("seed subject-a's session");

		assert!(
			repo.get("subject-b", "session-1").await.expect("query should not fail").is_none(),
			"a foreign subject must not be able to read the row"
		);
		assert!(
			repo.get("subject-a", "session-1").await.expect("query should not fail").is_some(),
			"the owning subject must still be able to read it"
		);
	}

	#[tokio::test]
	async fn delete_cannot_remove_a_row_owned_by_another_subject() {
		let pool = pool().await;
		let repo = SessionRepository::new(pool);
		repo.upsert("subject-a", &fixture("session-1")).await.expect("seed subject-a's session");

		let removed = repo.delete("subject-b", "session-1").await.expect("query should not fail");
		assert!(!removed, "a foreign subject's delete must report nothing removed");
		assert!(
			repo.get("subject-a", "session-1").await.expect("query should not fail").is_some(),
			"the row must survive a foreign delete attempt"
		);
	}

	#[tokio::test]
	async fn delete_many_skips_foreign_ids_and_reports_only_the_owned_count() {
		let pool = pool().await;
		let repo = SessionRepository::new(pool);
		repo.upsert("subject-a", &fixture("session-a")).await.expect("seed subject-a's session");
		repo.upsert("subject-b", &fixture("session-b")).await.expect("seed subject-b's session");

		let deleted = repo
			.delete_many("subject-a", &["session-a".to_owned(), "session-b".to_owned()])
			.await
			.expect("query should not fail");

		assert_eq!(deleted, 1, "only the id subject-a actually owns should count");
		assert!(
			repo.get("subject-b", "session-b").await.expect("query should not fail").is_some(),
			"subject-b's session must survive"
		);
	}

	#[tokio::test]
	async fn upsert_refuses_to_move_an_existing_row_to_a_different_subject() {
		let pool = pool().await;
		let repo = SessionRepository::new(pool);
		repo.upsert("subject-a", &fixture("session-1")).await.expect("seed subject-a's session");

		let hijack_attempt = repo.upsert("subject-b", &fixture("session-1")).await;
		assert!(
			matches!(hijack_attempt, Err(SessionRepoError::SubjectMismatch { .. })),
			"an upsert against an id owned by another subject must fail, not silently move it: got {hijack_attempt:?}"
		);

		assert!(
			repo.get("subject-a", "session-1").await.expect("query should not fail").is_some(),
			"the row must still belong to its original owner"
		);
		assert!(
			repo.get("subject-b", "session-1").await.expect("query should not fail").is_none(),
			"the failed attempt must not have moved the row"
		);
	}

	#[tokio::test]
	async fn set_status_many_updates_only_the_ids_this_subject_owns() {
		let pool = pool().await;
		let repo = SessionRepository::new(pool);
		repo.upsert("subject-a", &fixture("session-a")).await.expect("seed subject-a's session");
		repo.upsert("subject-b", &fixture("session-b")).await.expect("seed subject-b's session");

		let updated = repo
			.set_status_many(
				"subject-a",
				&["session-a".to_owned(), "session-b".to_owned()],
				SessionStatus::Scheduled,
				"2026-01-02T00:00:00Z",
			)
			.await
			.expect("query should not fail");

		assert_eq!(updated.len(), 1, "only the id subject-a actually owns should be reported as updated");
		assert_eq!(updated[0].id, "session-a");

		let foreign_after = repo
			.get("subject-b", "session-b")
			.await
			.expect("query should not fail")
			.expect("subject-b's session still exists");
		assert!(
			matches!(foreign_after.status, SessionStatus::Draft),
			"a foreign id in the batch must not have its status changed"
		);
	}

	fn fixture_with_status(id: &str, status: SessionStatus, updated_at: &str) -> SessionRecord {
		SessionRecord {
			status,
			updated_at: updated_at.to_owned(),
			..fixture(id)
		}
	}

	/// #263 (SLI2): pins the status-priority decision in `first_prepared`'s
	/// own doc comment — a `paused` session is offered over a `scheduled` or
	/// `draft` one even when the others were touched more recently, and a
	/// `completed` session is never a candidate at all.
	#[tokio::test]
	async fn first_prepared_prefers_a_paused_session_over_scheduled_or_draft_regardless_of_recency() {
		let pool = pool().await;
		let repo = SessionRepository::new(pool);

		repo
			.upsert("subject-a", &fixture_with_status("session-draft", SessionStatus::Draft, "2026-01-04T00:00:00Z"))
			.await
			.expect("seed a draft session");
		repo
			.upsert("subject-a", &fixture_with_status("session-scheduled", SessionStatus::Scheduled, "2026-01-03T00:00:00Z"))
			.await
			.expect("seed a scheduled session");
		repo
			.upsert("subject-a", &fixture_with_status("session-paused", SessionStatus::Paused, "2026-01-01T00:00:00Z"))
			.await
			.expect("seed a paused session, touched least recently of the three");
		repo
			.upsert("subject-a", &fixture_with_status("session-completed", SessionStatus::Completed, "2026-01-05T00:00:00Z"))
			.await
			.expect("seed a completed session, touched most recently of all four");

		let prepared = repo.first_prepared("subject-a").await.expect("query should not fail");
		assert_eq!(
			prepared,
			Some("session-paused".to_owned()),
			"a paused session should win even though it is the least recently touched non-completed candidate"
		);
	}

	/// #263 (SLI2): within one status, the tiebreak is `updated_at DESC`.
	#[tokio::test]
	async fn first_prepared_breaks_a_tie_within_one_status_by_most_recently_updated() {
		let pool = pool().await;
		let repo = SessionRepository::new(pool);

		repo
			.upsert("subject-a", &fixture_with_status("session-older", SessionStatus::Draft, "2026-01-01T00:00:00Z"))
			.await
			.expect("seed the older draft");
		repo
			.upsert("subject-a", &fixture_with_status("session-newer", SessionStatus::Draft, "2026-01-02T00:00:00Z"))
			.await
			.expect("seed the more recently touched draft");

		let prepared = repo.first_prepared("subject-a").await.expect("query should not fail");
		assert_eq!(prepared, Some("session-newer".to_owned()), "the more recently updated draft should win the tie");
	}

	/// #263 (SLI2): the same isolation `get`/`list`/etc. already have,
	/// carried over to the new query.
	#[tokio::test]
	async fn first_prepared_never_returns_a_different_subjects_session() {
		let pool = pool().await;
		let repo = SessionRepository::new(pool);

		repo
			.upsert("subject-b", &fixture_with_status("session-b", SessionStatus::Paused, "2026-01-01T00:00:00Z"))
			.await
			.expect("seed subject-b's paused session");

		assert_eq!(
			repo.first_prepared("subject-a").await.expect("query should not fail"),
			None,
			"a foreign subject's prepared session must never be returned"
		);
	}

	#[tokio::test]
	async fn first_prepared_returns_none_when_the_subject_has_no_candidate() {
		let pool = pool().await;
		let repo = SessionRepository::new(pool);
		repo
			.upsert("subject-a", &fixture_with_status("session-done", SessionStatus::Completed, "2026-01-01T00:00:00Z"))
			.await
			.expect("seed a completed session, which is not a candidate");

		assert_eq!(repo.first_prepared("subject-a").await.expect("query should not fail"), None);
	}
}

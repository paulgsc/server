use crate::model::{LayoutMode, SessionOrigin, SessionRecord, SessionStatus};
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
	origin: String,
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
	/// `origin` held something outside `"user"`/`"system"` — see
	/// `SessionOrigin::parse`'s own doc comment for why this is refused
	/// rather than read as `user`.
	UnknownOrigin(String),
	/// One of the JSON columns did not parse.
	MalformedJson(&'static str, serde_json::Error),
}

impl std::fmt::Display for RowError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::UnknownStatus(raw) => write!(f, "unknown session status: {raw}"),
			Self::UnknownOrigin(raw) => write!(f, "unknown session origin: {raw}"),
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
			origin: SessionOrigin::parse(&row.origin).ok_or_else(|| RowError::UnknownOrigin(row.origin.clone()))?,
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
			    id as "id!", name, status, origin, layout_mode, total_duration_ms,
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

	/// Atomically ensure this subject has at most one un-started `system`
	/// proposal, either inserting `record` fresh or refreshing an existing
	/// one in place — #284 (RCM7)'s own chosen strategy, argued in full in
	/// `docs/study-nudge.md`'s "Never stack proposals" section.
	///
	/// **The predicate is `origin = 'system' AND started_at IS NULL`, not
	/// "anything prepared".** This deliberately replaces the coarser
	/// `provision_if_absent` guard RCM2 (#279) originally wrote (`status IN
	/// ('paused', 'scheduled', 'draft')`, any origin) — that predicate
	/// predates `origin` (#283/RCM6) existing at all, and #284's own issue
	/// text is explicit about why it is not enough: a `system` session the
	/// person *did* start is history now, not a proposal, and must never be
	/// touched by this rule; a `system` session promoted to `user` by editing
	/// falls out of the rule automatically. Both fall out for free once the
	/// predicate is this one, because both cases make the row stop matching
	/// it — a started row fails `started_at IS NULL` forever (`UpdateSession`
	/// has no way to clear `started_at` back to `None`), and a promoted row
	/// fails `origin = 'system'` forever (`upsert`'s own one-way guard).
	///
	/// **Enforced as a real constraint**, `idx_sessions_one_unstarted_system_
	/// proposal` (`20260910000100_at_most_one_unstarted_system_proposal.up.sql`),
	/// not just application-level care — the same reasoning `upsert`'s own
	/// `SubjectMismatch` guard already applies to a different invariant.
	/// `ON CONFLICT (subject_id) WHERE origin = 'system' AND started_at IS
	/// NULL AND status IN ('paused', 'scheduled', 'draft')` targets that
	/// index by name; `SQLite` requires an `ON CONFLICT` target's `WHERE`
	/// clause to match a real partial index verbatim, so this statement and
	/// that migration must always agree.
	///
	/// **The `status IN (...)` clause is load-bearing, not redundant with
	/// `started_at IS NULL`** — a real `chatgpt-codex-connector` finding on
	/// `#335` caught its absence. `set_status_many` (`PATCH /sessions/status`)
	/// writes only `status` and `updated_at`, so it can move a `system`,
	/// never-started proposal straight to `active` or `completed` while
	/// `started_at` stays `NULL` — a real, client-reachable path, not a
	/// hypothetical. Without this clause, such a row would still satisfy the
	/// conflict target forever (`first_prepared` can never surface it back
	/// out, since `active`/`completed` aren't in its own tracked set),
	/// permanently starving the subject of any future proposal and standing
	/// as a live target this statement would silently overwrite. Restricting
	/// the clause to the same three statuses `first_prepared` already tracks
	/// closes both: such a row falls out of the conflict target (a fresh
	/// proposal inserts alongside it instead of overwriting it) exactly as it
	/// already falls out of `idx_sessions_one_unstarted_system_proposal`'s
	/// own predicate. See that migration's own comment for the matching
	/// argument on the index side, and `docs/study-nudge.md`'s "Never stack
	/// proposals" section for the full account.
	///
	/// **One statement, not read-then-write.** The same race `provision_if_
	/// absent`'s own doc comment named (a `chatgpt-codex-connector` review on
	/// #313: a concurrent waker pass, or the subject's own `POST /sessions`
	/// call, landing a competing row in the window between a read and a
	/// write) is closed the same way here — `SQLite`'s own per-statement
	/// write serialization decides which of two racing calls actually
	/// lands, and the loser's content becomes the `DO UPDATE`, not a second
	/// row. Bounded per #253: an equality probe against a partial index,
	/// not a scan — the same discipline `first_prepared`'s three single-status
	/// probes already established for the neighbouring query.
	///
	/// **The `WHERE NOT EXISTS` guard is still needed alongside the partial
	/// index — the two protect against different races.** The partial index
	/// stops a second *system* proposal from ever coexisting with an
	/// un-started one; it says nothing about a **foreign** prepared session —
	/// a real person's own `paused`/`scheduled`/`draft` row — landing in the
	/// window between `consider`'s `first_prepared` read (which found
	/// nothing) and this write. Without this guard, that race would insert a
	/// system proposal *alongside* the person's own fresh draft, and
	/// `first_prepared`'s status priority would then surface the system
	/// proposal over it — exactly the race `provision_if_absent`'s own
	/// `WHERE NOT EXISTS` used to close, and a real `chatgpt-codex-connector`
	/// finding on `#335` caught its absence here. The subquery excludes rows
	/// that already match the partial index's own predicate — a pre-existing
	/// `system`/un-started proposal is not "foreign," it is exactly the row
	/// this statement is allowed to refresh via the `ON CONFLICT` branch —
	/// so the two clauses agree rather than fight over the same row.
	///
	/// **What refreshing touches — `name`, `activities`, and
	/// `total_duration_ms`. Deliberately not `updated_at`.** Every other
	/// write path in this codebase (`upsert`, and therefore every real
	/// `PATCH /sessions/:id`) advances `updated_at` unconditionally, so it
	/// already means exactly what `20260805000300_create_sessions.up.sql`'s
	/// own column comment says — "editing only" — for every row except this
	/// one write path. A machine refresh is not a person editing anything,
	/// so it must not be able to produce the same signal a real edit does:
	/// see `refresh_stale_proposal`'s own doc comment (`nudge/waker.rs`) for
	/// why `created_at == updated_at` staying true across any number of
	/// refreshes is exactly the property that lets it tell "only ever
	/// touched by the waker" apart from "a person edited this," which is
	/// otherwise unrecoverable before PRO1 ships `origin` promotion into the
	/// live client's edit path at all (a real `chatgpt-codex-connector`
	/// finding on `#335` — see that section of `docs/study-nudge.md` for the
	/// full argument for why this matters here specifically). `id` and
	/// `created_at` are absent from the `DO UPDATE SET` list for the reasons
	/// already established elsewhere: preserving `id` is the whole point of
	/// "refresh in place" (a notification issued before a refresh still has
	/// to resolve afterwards), and `created_at` follows `upsert`'s own
	/// precedent of never letting a write move it.
	/// `status`/`origin`/`layout_mode`/`scenes`/`layout`/`started_at`/
	/// `completed_at`/`final_elapsed_ms` are left untouched too — the
	/// conflicting row, by construction of the partial index predicate it
	/// matched, already holds the only values `materialize_provisioned_
	/// session` ever writes for them (`scheduled`, `system`, `basic`, `[]`,
	/// `NULL`, `NULL`, `NULL`, `NULL`), so there is nothing for a refresh to
	/// change there.
	///
	/// **The `DO UPDATE ... WHERE sessions.created_at = sessions.updated_at`
	/// clause is this method's own safety net, not merely a restatement of a
	/// check some caller already made — a sixth real `chatgpt-codex-connector`
	/// finding on `#335` caught its absence.** The `NothingToSay` arm's own
	/// call site has no fresh read of the conflicting row to check against:
	/// it discovers a conflict only through this very statement, against
	/// whatever row a *different* concurrent waker pass inserted after this
	/// pass's own `first_prepared` read already returned nothing. A person
	/// can edit that other pass's freshly-inserted proposal (still `system`,
	/// `updated_at` moved, the same pre-PRO1 gap `refresh_if_untouched` was
	/// built for) in the window before this delayed pass's write lands, and
	/// an unconditional `DO UPDATE` would silently overwrite it — exactly
	/// the same class of race `refresh_if_untouched` closes for its own
	/// caller, recurring here because that fix only ever touched
	/// `refresh_stale_proposal`'s call site, not this one. `SQLite`'s own
	/// `DO UPDATE ... WHERE` re-checks the condition atomically, in the same
	/// statement as the conflict resolution itself: when it fails, the row
	/// is left completely untouched and nothing is inserted either — verified
	/// directly, not just read from the documentation, since this repo has no
	/// existing use of this `SQLite` upsert clause to point to as precedent.
	///
	/// # Errors
	/// Fails on any `sqlx` error, or if the record's JSON does not serialize.
	pub async fn provision_or_refresh(&self, subject_id: &str, record: &SessionRecord) -> Result<(), SessionRepoError> {
		let status = record.status.as_str();
		let origin = record.origin.as_str();
		let layout_mode = record.layout_mode.as_str();
		#[allow(clippy::disallowed_methods)]
		let (activities, scenes, layout) = (
			serde_json::to_string(&record.activities)?,
			serde_json::to_string(&record.scenes)?,
			record.layout.as_ref().map(serde_json::to_string).transpose()?,
		);

		sqlx::query!(
			r#"
			INSERT INTO sessions (
			    id, subject_id, name, status, origin, layout_mode, total_duration_ms,
			    created_at, updated_at, started_at, completed_at, final_elapsed_ms,
			    activities, scenes, layout
			)
			SELECT ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?
			WHERE NOT EXISTS (
			    SELECT 1 FROM sessions
			    WHERE subject_id = ?
			      AND status IN ('paused', 'scheduled', 'draft')
			      AND (origin != 'system' OR started_at IS NOT NULL)
			)
			ON CONFLICT (subject_id) WHERE origin = 'system' AND started_at IS NULL AND status IN ('paused', 'scheduled', 'draft')
			DO UPDATE SET
			    name              = excluded.name,
			    activities        = excluded.activities,
			    total_duration_ms = excluded.total_duration_ms
			WHERE sessions.created_at = sessions.updated_at
			"#,
			record.id,
			subject_id,
			record.name,
			status,
			origin,
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
			subject_id,
		)
		.execute(&self.pool)
		.await?;

		Ok(())
	}

	/// Refresh exactly one already-identified un-started `system` proposal's
	/// content in place — `name`, `activities`, `total_duration_ms` — but
	/// only if it is, at the instant of this write, still in the same
	/// untouched state the caller last observed. No insert fallback: this is
	/// not "provision or refresh," it is "refresh this one row, or do
	/// nothing."
	///
	/// **Why this exists alongside [`Self::provision_or_refresh`].** That
	/// method's own `ON CONFLICT` refresh is safe for its one caller
	/// (`nudge::waker::consider`'s `NothingToSay` arm) because a row it could
	/// race against is, by construction, always another concurrent waker
	/// pass's own fresh candidate — `create_session`/`duplicate_session` can
	/// never write `origin = 'system'` at all, so a person's own action can
	/// never be the thing on the other side of *that* race. `nudge::waker::
	/// refresh_stale_proposal`'s situation is different: it reads a specific,
	/// already-existing row well before this write — across a full
	/// `engine.evaluate` call and an admission check — and a real
	/// `chatgpt-codex-connector` finding on `#335` caught that a person's own
	/// `PATCH`/`DELETE` landing in that gap is entirely possible. `read, then
	/// decide, then write` has a window no amount of care in the reading half
	/// can close; only re-checking the same predicate atomically, in the
	/// same statement as the write, actually closes it.
	///
	/// **The `WHERE` clause is the entire safety argument, not the read that
	/// preceded this call.** It re-verifies `origin = 'system' AND started_at
	/// IS NULL AND created_at = updated_at` at the moment of the write,
	/// scoped to the one `id` the caller already knows. Three ways the row
	/// could have changed since the caller's own read, and what each one
	/// does here:
	/// - **Edited** (a rename, even one that never sends `origin` — the
	///   documented pre-PRO1 gap): `updated_at` moved away from `created_at`,
	///   the `WHERE` clause no longer matches, zero rows are affected, the
	///   edit survives untouched.
	/// - **Started, or promoted to `user`**: `started_at` is no longer `NULL`
	///   or `origin` is no longer `'system'`; same outcome, zero rows
	///   affected.
	/// - **Deleted**: the `id` no longer exists at all; same outcome.
	///
	/// In every case, returning `false` rather than falling back to an insert
	/// is what makes this safe where `provision_or_refresh` would not be:
	/// inserting here would create a second, orphaned system proposal under
	/// a brand-new id while whatever `StudyAction` `consider` already
	/// selected keeps pointing at the *old* one — exactly the failure mode
	/// the finding named. The caller (`refresh_stale_proposal`) treats
	/// `false` as "nothing to do," the same as any other outcome that leaves
	/// the existing row exactly as it was.
	///
	/// Bounded per #253: one indexed lookup by primary key, not a scan.
	///
	/// # Errors
	/// Fails on any `sqlx` error, or if `activities` does not serialize.
	pub async fn refresh_if_untouched(&self, subject_id: &str, existing_id: &str, candidate: &SessionRecord) -> Result<bool, SessionRepoError> {
		#[allow(clippy::disallowed_methods)]
		let activities = serde_json::to_string(&candidate.activities)?;

		let result = sqlx::query!(
			r#"
			UPDATE sessions
			SET name = ?, activities = ?, total_duration_ms = ?
			WHERE id = ?
			  AND subject_id = ?
			  AND origin = 'system'
			  AND started_at IS NULL
			  AND created_at = updated_at
			"#,
			candidate.name,
			activities,
			candidate.total_duration_ms,
			existing_id,
			subject_id,
		)
		.execute(&self.pool)
		.await?;

		Ok(result.rows_affected() > 0)
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
			    id as "id!", name, status, origin, layout_mode, total_duration_ms,
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
	/// **`origin` moves in one direction: `system → user`, never `user →
	/// system`** (`#283`). Unlike `subject_id`, `origin` *is* in the `SET`
	/// list — a legitimate promotion has to reach the stored row somehow —
	/// but the value written is `CASE WHEN sessions.origin = 'user' THEN
	/// 'user' ELSE excluded.origin END` rather than a bare `excluded.origin`:
	/// once a row is `user`, no later write can move it back to `system`,
	/// regardless of what `record.origin` says. A proposal a person edited
	/// stays theirs even if some future caller re-sends the original
	/// `system` value by mistake; there is deliberately no error for this
	/// case (unlike [`SessionRepoError::SubjectMismatch`]) — silently
	/// keeping the stronger claim is the same shape of policy as
	/// `origin`'s own one-way promotion, not a caller mistake worth
	/// surfacing.
	///
	/// # Errors
	/// Fails on any `sqlx` error, if the record's JSON does not serialize, or
	/// with [`SessionRepoError::SubjectMismatch`] if `id` already belongs to
	/// another subject.
	pub async fn upsert(&self, subject_id: &str, record: &SessionRecord) -> Result<(), SessionRepoError> {
		let status = record.status.as_str();
		let origin = record.origin.as_str();
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
			    id, subject_id, name, status, origin, layout_mode, total_duration_ms,
			    created_at, updated_at, started_at, completed_at, final_elapsed_ms,
			    activities, scenes, layout
			)
			VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
			ON CONFLICT(id) DO UPDATE SET
			    name              = excluded.name,
			    status            = excluded.status,
			    origin            = CASE WHEN sessions.origin = 'user' THEN 'user' ELSE excluded.origin END,
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
			origin,
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
			    id as "id!", name, status, origin, layout_mode, total_duration_ms,
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
	use crate::model::{LayoutMode, SessionOrigin, SessionRecord, SessionStatus};
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
			origin: SessionOrigin::User,
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

	/// A `materialize_provisioned_session`-shaped fixture: `system`-origin,
	/// `Scheduled`, never started — exactly the row `provision_or_refresh`'s
	/// partial-index predicate (`origin = 'system' AND started_at IS NULL`)
	/// targets, unlike plain `fixture` (`user`-origin, `Draft`).
	fn system_proposal_fixture(id: &str, name: &str, total_duration_ms: i64, stamp: &str) -> SessionRecord {
		SessionRecord {
			name: name.to_owned(),
			status: SessionStatus::Scheduled,
			origin: SessionOrigin::System,
			total_duration_ms,
			created_at: stamp.to_owned(),
			updated_at: stamp.to_owned(),
			..fixture(id)
		}
	}

	/// #284 (RCM7)'s core acceptance criterion: "runs five consecutive
	/// eligible passes and counts one." Each call below stands in for one
	/// waker pass over a subject who keeps ignoring the same proposal — a
	/// fresh id and fresh content every time, exactly like five different
	/// `materialize_provisioned_session` outputs on five different days.
	///
	/// Two things have to both be true, not just "no duplicate row": the
	/// *id* the first call wrote must be the one still live (RCM8's deep
	/// links depend on it), and the *content* must be the *last* call's, not
	/// the first's — proving this is a refresh, not a silent no-op that
	/// would leave a five-day-stale proposal exactly as #284's own issue
	/// text warns against.
	#[tokio::test]
	async fn provision_or_refresh_collapses_five_consecutive_calls_into_one_row_with_the_first_id_and_the_last_content() {
		let pool = pool().await;
		let repo = SessionRepository::new(pool);

		// One tuple per simulated day: (id, name, total_duration_ms, stamp) —
		// spelled out rather than built with `format!` (disallowed by
		// `clippy.toml` — eager allocation ahead of what would otherwise be a
		// tracing call), matching every other fixture in this module, which
		// only ever takes static ids.
		let passes = [
			("session-day-0", "proposal for day 0", 0_i64, "2026-01-01T00:00:00Z"),
			("session-day-1", "proposal for day 1", 60_000, "2026-01-02T00:00:00Z"),
			("session-day-2", "proposal for day 2", 120_000, "2026-01-03T00:00:00Z"),
			("session-day-3", "proposal for day 3", 180_000, "2026-01-04T00:00:00Z"),
			("session-day-4", "proposal for day 4", 240_000, "2026-01-05T00:00:00Z"),
		];
		for (id, name, total_duration_ms, stamp) in passes {
			let candidate = system_proposal_fixture(id, name, total_duration_ms, stamp);
			repo.provision_or_refresh("subject-a", &candidate).await.unwrap();
		}

		let sessions = repo.list("subject-a").await.unwrap();
		assert_eq!(sessions.len(), 1, "five eligible passes over an ignored proposal must collapse to one row, not stack");

		let survivor = &sessions[0];
		assert_eq!(
			survivor.id, "session-day-0",
			"the first call's id must still be live, so an old notification's deep link keeps resolving"
		);
		assert_eq!(
			survivor.name, "proposal for day 4",
			"the content must be the last call's, not the first's stale one — this is the refresh, not a no-op"
		);
		assert_eq!(survivor.total_duration_ms, 240_000);
		assert_eq!(
			survivor.created_at, "2026-01-01T00:00:00Z",
			"created_at follows upsert's own precedent: a refresh never moves it"
		);
		assert_eq!(
			survivor.updated_at, "2026-01-01T00:00:00Z",
			"a machine refresh must not touch updated_at either -- a real chatgpt-codex-connector finding on #335 established that \
			 created_at == updated_at surviving every refresh is what lets refresh_stale_proposal tell 'only ever touched by the \
			 waker' apart from 'a person edited this,' which matters because origin alone cannot, before PRO1 ships"
		);
	}

	/// A fourth real `chatgpt-codex-connector` finding on `#335`, P1:
	/// `refresh_stale_proposal` reads a row, decides it is safe to refresh,
	/// then writes — a real window for a concurrent `PATCH`/`DELETE` to land
	/// in between. `refresh_if_untouched` closes it by re-checking the
	/// identical predicate atomically, in the same statement as the write,
	/// rather than trusting a decision made from an now-possibly-stale read.
	/// This pins the base case: the row is exactly as last read, so the
	/// refresh applies and reports `true`.
	#[tokio::test]
	async fn refresh_if_untouched_applies_when_the_row_is_still_untouched() {
		let pool = pool().await;
		let repo = SessionRepository::new(pool);

		let original = system_proposal_fixture("session-1", "original", 100, "2026-01-01T00:00:00Z");
		repo.upsert("subject-a", &original).await.unwrap();

		let candidate = system_proposal_fixture("session-fresh-candidate", "refreshed", 999, "2026-01-02T00:00:00Z");
		let applied = repo.refresh_if_untouched("subject-a", "session-1", &candidate).await.unwrap();
		assert!(applied, "an untouched row must be refreshed");

		let after = repo.get("subject-a", "session-1").await.unwrap().unwrap();
		assert_eq!(after.name, "refreshed");
		assert_eq!(after.total_duration_ms, 999);
		assert_eq!(after.created_at, "2026-01-01T00:00:00Z", "refresh_if_untouched must not move created_at either");
		assert_eq!(after.updated_at, "2026-01-01T00:00:00Z", "refresh_if_untouched must not move updated_at either");
	}

	/// The race itself: simulates a person's `PATCH` landing in the gap
	/// between `refresh_stale_proposal`'s own read and this write — the
	/// exact scenario the finding named — by editing the row (which bumps
	/// `updated_at`, exactly as `update_session` always does) *after* the
	/// caller would have read it but *before* the refresh statement runs.
	/// The atomic `WHERE` clause must catch this even though nothing in
	/// this test re-reads the row first, proving the safety does not depend
	/// on the caller's own read being fresh.
	#[tokio::test]
	async fn refresh_if_untouched_no_ops_when_a_concurrent_edit_landed_first() {
		let pool = pool().await;
		let repo = SessionRepository::new(pool);

		let original = system_proposal_fixture("session-1", "original", 100, "2026-01-01T00:00:00Z");
		repo.upsert("subject-a", &original).await.unwrap();

		// The concurrent edit: same row, renamed, updated_at bumped -- the
		// live client never sends `origin`, so it stays `system`.
		let edited = SessionRecord {
			name: "person's own rename".to_owned(),
			updated_at: "2026-01-01T00:05:00Z".to_owned(),
			..original
		};
		repo.upsert("subject-a", &edited).await.unwrap();

		let candidate = system_proposal_fixture("session-fresh-candidate", "would-be refresh", 999, "2026-01-02T00:00:00Z");
		let applied = repo.refresh_if_untouched("subject-a", "session-1", &candidate).await.unwrap();
		assert!(
			!applied,
			"a concurrently edited row must not be refreshed, even though the caller's own earlier read could not have known that"
		);

		let after = repo.get("subject-a", "session-1").await.unwrap().unwrap();
		assert_eq!(after.name, "person's own rename", "the concurrent edit must survive completely untouched");
	}

	/// The other half of the same finding: a concurrent `DELETE` (or a
	/// promotion/start that moved the row out of the predicate entirely)
	/// must also no-op rather than fall back to inserting a new row under a
	/// different id -- `refresh_if_untouched` has no insert path at all, so
	/// there is nothing for it to fall back to.
	#[tokio::test]
	async fn refresh_if_untouched_no_ops_when_the_row_no_longer_exists() {
		let pool = pool().await;
		let repo = SessionRepository::new(pool);

		let candidate = system_proposal_fixture("session-fresh-candidate", "would-be refresh", 999, "2026-01-02T00:00:00Z");
		let applied = repo.refresh_if_untouched("subject-a", "session-deleted", &candidate).await.unwrap();
		assert!(!applied, "refreshing an id that no longer exists must no-op, not insert a new row under a different id");

		assert_eq!(repo.list("subject-a").await.unwrap().len(), 0, "no orphaned row must appear");
	}

	/// #284's named edge: "a system session the person did start is history
	/// now, not a proposal — this rule must never touch it." A started
	/// system session no longer matches `started_at IS NULL`, so it must not
	/// block — or be overwritten by — a brand new proposal.
	#[tokio::test]
	async fn provision_or_refresh_never_touches_a_started_system_session() {
		let pool = pool().await;
		let repo = SessionRepository::new(pool);

		let started = SessionRecord {
			started_at: Some("2026-01-01T12:00:00Z".to_owned()),
			status: SessionStatus::Active,
			..system_proposal_fixture("session-started", "already opened", 300_000, "2026-01-01T00:00:00Z")
		};
		repo.upsert("subject-a", &started).await.unwrap();

		let fresh = system_proposal_fixture("session-fresh", "a brand new proposal", 600_000, "2026-01-02T00:00:00Z");
		repo.provision_or_refresh("subject-a", &fresh).await.unwrap();

		let sessions = repo.list("subject-a").await.unwrap();
		assert_eq!(sessions.len(), 2, "the started session and the new proposal must coexist, not collapse into one");

		let started_after = sessions.iter().find(|s| s.id == "session-started").unwrap();
		assert_eq!(started_after.name, "already opened", "a started system session must never be refreshed");
		assert_eq!(started_after.started_at.as_deref(), Some("2026-01-01T12:00:00Z"));
	}

	/// A fifth real `chatgpt-codex-connector` finding on `#335`, P1:
	/// `set_status_many` (`PATCH /sessions/status`) writes only `status` and
	/// `updated_at`, so it can move a `system`-origin, never-started proposal
	/// straight to `active` or `completed` while `started_at` stays `NULL` --
	/// a real, client-reachable path. Without `status IN (...)` in the
	/// conflict target, such a row would permanently occupy this subject's
	/// one slot (`first_prepared` can never surface it back out, since
	/// `active`/`completed` aren't in its own tracked set) and stand as a
	/// live target this statement would silently overwrite. This pins both
	/// halves of the fix: a fresh proposal is not blocked by such a row, and
	/// that row's own content survives completely untouched.
	#[tokio::test]
	async fn provision_or_refresh_never_touches_or_is_blocked_by_a_set_status_many_anomaly() {
		let pool = pool().await;
		let repo = SessionRepository::new(pool);

		// The anomaly itself: origin still `system`, `started_at` still
		// `NULL`, but `status` moved to `active` -- exactly what
		// `set_status_many` alone can produce, bypassing the lifecycle
		// fields a real `Start` action would also set.
		let anomaly = SessionRecord {
			status: SessionStatus::Active,
			updated_at: "2026-01-01T00:05:00Z".to_owned(),
			..system_proposal_fixture("session-anomaly", "orphaned by set_status_many", 300_000, "2026-01-01T00:00:00Z")
		};
		repo.upsert("subject-a", &anomaly).await.unwrap();

		let fresh = system_proposal_fixture("session-fresh", "a brand new proposal", 600_000, "2026-01-02T00:00:00Z");
		repo.provision_or_refresh("subject-a", &fresh).await.unwrap();

		let sessions = repo.list("subject-a").await.unwrap();
		assert_eq!(
			sessions.len(),
			2,
			"the anomaly and the new proposal must coexist, not collapse into one -- the subject must not be starved of future proposals"
		);

		let anomaly_after = sessions.iter().find(|s| s.id == "session-anomaly").unwrap();
		assert_eq!(
			anomaly_after.name, "orphaned by set_status_many",
			"an active/completed row must never be silently overwritten by a refresh"
		);
		assert!(matches!(anomaly_after.status, SessionStatus::Active));
	}

	/// A sixth real `chatgpt-codex-connector` finding on `#335`, P1: the
	/// `NothingToSay` arm's own call site has no fresh read of the
	/// conflicting row to check against -- it discovers a conflict only
	/// through this statement, against whatever another concurrent waker
	/// pass already inserted after this pass's own `first_prepared` read
	/// found nothing. Simulates that interleaving directly: an insert (the
	/// other pass), a person's edit landing on it (still `system`, no
	/// `origin` field, exactly the pre-PRO1 gap), then a second,
	/// independent `provision_or_refresh` call (the delayed pass) -- which
	/// must leave the edit completely untouched rather than overwriting it,
	/// the same guarantee `refresh_if_untouched` already gives its own
	/// caller.
	#[tokio::test]
	async fn provision_or_refresh_never_overwrites_a_conflicting_row_a_person_edited_since_it_was_inserted() {
		let pool = pool().await;
		let repo = SessionRepository::new(pool);

		// The first (concurrent) pass's insert.
		let first_pass = system_proposal_fixture("session-1", "first pass's proposal", 300_000, "2026-01-01T00:00:00Z");
		repo.provision_or_refresh("subject-a", &first_pass).await.unwrap();

		// The person edits it before the second pass's write lands --
		// `origin` stays `system` (today's client never sends it), only
		// `updated_at` moves, exactly as `update_session` always does.
		let edited = SessionRecord {
			name: "person's own rename".to_owned(),
			updated_at: "2026-01-01T00:05:00Z".to_owned(),
			..first_pass
		};
		repo.upsert("subject-a", &edited).await.unwrap();

		// The second (delayed) pass's own conflicting write.
		let second_pass = system_proposal_fixture("session-2", "second pass's proposal", 999_000, "2026-01-02T00:00:00Z");
		repo.provision_or_refresh("subject-a", &second_pass).await.unwrap();

		let sessions = repo.list("subject-a").await.unwrap();
		assert_eq!(sessions.len(), 1, "the two racing passes must still collapse to one row, not two");
		assert_eq!(sessions[0].id, "session-1", "the first pass's id must survive -- it is the row that actually exists");
		assert_eq!(
			sessions[0].name, "person's own rename",
			"the person's edit must survive a second pass's conflicting write completely untouched"
		);
	}

	/// A real `chatgpt-codex-connector` finding on `#335`: the partial index
	/// alone only stops a second *system* proposal from coexisting with an
	/// un-started one — it says nothing about a **foreign** prepared session
	/// (a real person's own `paused`/`scheduled`/`draft` row) landing beside
	/// a freshly-provisioned system proposal. Without the `WHERE NOT EXISTS`
	/// guard restored alongside the `ON CONFLICT`, a concurrent `POST
	/// /sessions` in the race window `provision_if_absent`'s own doc comment
	/// already named (#313) would let a system proposal land next to it, and
	/// `first_prepared`'s status priority would then surface the *wrong*
	/// session. This pins the restored guard directly: a foreign draft
	/// already sitting there must block the write outright, not just avoid
	/// colliding with it.
	#[tokio::test]
	async fn provision_or_refresh_is_blocked_by_a_foreign_prepared_session() {
		let pool = pool().await;
		let repo = SessionRepository::new(pool);

		repo.upsert("subject-a", &fixture("session-user-draft")).await.unwrap();

		let candidate = system_proposal_fixture("session-system", "a proposal", 300_000, "2026-01-01T00:00:00Z");
		repo.provision_or_refresh("subject-a", &candidate).await.unwrap();

		let sessions = repo.list("subject-a").await.unwrap();
		assert_eq!(
			sessions.len(),
			1,
			"a foreign prepared session must block provisioning outright, not merely avoid overwriting it"
		);
		assert_eq!(sessions[0].id, "session-user-draft", "the person's own draft must be the only row, untouched");
	}

	/// #284's other named edge, corrected after the finding above: "a system
	/// session promoted to user by editing falls out of this rule
	/// automatically" is true of the **uniqueness** invariant (the partial
	/// index no longer covers a `user`-origin row at all), not of the
	/// foreign-prepared-session guard, which tracks *status*, not origin —
	/// and correctly so: while a promoted session still sits in a prepared
	/// status, `first_prepared` already surfaces it and `consider` never
	/// reaches `provision_or_refresh` in the first place (the previous test
	/// pins that this method independently refuses to race past it either).
	/// Once the promoted session leaves every prepared status — here,
	/// completed, the same as any real finished session — it stops being
	/// foreign to this guard too, and a fresh proposal is free to land.
	#[tokio::test]
	async fn provision_or_refresh_is_not_blocked_by_a_promoted_session_that_is_no_longer_prepared() {
		let pool = pool().await;
		let repo = SessionRepository::new(pool);

		let proposal = system_proposal_fixture("session-edited", "Suggested for you", 300_000, "2026-01-01T00:00:00Z");
		repo.upsert("subject-a", &proposal).await.unwrap();

		// A person renames it (PRO1's "what counts as an edit" promotes
		// origin to `user`, exactly as `update_session` would send it), then
		// finishes it — completed is no longer a "prepared" status at all.
		let completed = SessionRecord {
			name: "My own session".to_owned(),
			origin: SessionOrigin::User,
			status: SessionStatus::Completed,
			started_at: Some("2026-01-02T00:00:00Z".to_owned()),
			completed_at: Some("2026-01-02T00:30:00Z".to_owned()),
			updated_at: "2026-01-02T00:30:00Z".to_owned(),
			..proposal
		};
		repo.upsert("subject-a", &completed).await.unwrap();

		let fresh = system_proposal_fixture("session-fresh", "a brand new proposal", 600_000, "2026-01-03T00:00:00Z");
		repo.provision_or_refresh("subject-a", &fresh).await.unwrap();

		let sessions = repo.list("subject-a").await.unwrap();
		assert_eq!(sessions.len(), 2, "the completed, promoted session and the new proposal must coexist, not block each other");

		let promoted_after = sessions.iter().find(|s| s.id == "session-edited").unwrap();
		assert_eq!(
			promoted_after.name, "My own session",
			"a session a person took ownership of and finished must never be touched by the waker"
		);
		assert!(matches!(promoted_after.origin, SessionOrigin::User));

		let fresh_after = sessions.iter().find(|s| s.id == "session-fresh").unwrap();
		assert_eq!(fresh_after.name, "a brand new proposal");
	}

	#[tokio::test]
	async fn provision_or_refresh_is_scoped_per_subject_not_global() {
		let pool = pool().await;
		let repo = SessionRepository::new(pool);

		repo
			.provision_or_refresh("subject-a", &system_proposal_fixture("session-a", "a", 0, "2026-01-01T00:00:00Z"))
			.await
			.unwrap();
		repo
			.provision_or_refresh("subject-b", &system_proposal_fixture("session-b", "b", 0, "2026-01-01T00:00:00Z"))
			.await
			.unwrap();

		assert_eq!(repo.list("subject-a").await.unwrap().len(), 1);
		assert_eq!(
			repo.list("subject-b").await.unwrap().len(),
			1,
			"subject-a's proposal must not count against subject-b's check"
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

	/// `#283`'s own acceptance criterion: `system → user` is a real
	/// promotion, `user → system` is refused silently at the repository
	/// layer regardless of what the caller sends.
	#[tokio::test]
	async fn upsert_allows_a_system_origin_to_become_user_but_never_the_reverse() {
		let pool = pool().await;
		let repo = SessionRepository::new(pool);

		let system_record = SessionRecord {
			origin: SessionOrigin::System,
			..fixture("session-1")
		};
		repo.upsert("subject-a", &system_record).await.unwrap();

		let promoted = SessionRecord {
			origin: SessionOrigin::User,
			..system_record.clone()
		};
		repo.upsert("subject-a", &promoted).await.unwrap();
		let after_promotion = repo.get("subject-a", "session-1").await.unwrap().unwrap();
		assert!(
			matches!(after_promotion.origin, SessionOrigin::User),
			"system → user must be a real transition, not silently ignored"
		);

		let demotion_attempt = SessionRecord {
			origin: SessionOrigin::System,
			..promoted
		};
		repo.upsert("subject-a", &demotion_attempt).await.unwrap();
		let after_demotion_attempt = repo.get("subject-a", "session-1").await.unwrap().unwrap();
		assert!(
			matches!(after_demotion_attempt.origin, SessionOrigin::User),
			"user → system must be refused at the repository layer, even though the write itself succeeds"
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

//! What home keeps for good of one subject, counted (paulgsc/some-ui#2519,
//! MY-DATA MD2b): for each kind of record the owner never deletes, how many
//! home holds and since when.
//!
//! The phone's My data page reads it (`GET /api/v1/subjects/me/kept`,
//! `file_host`'s `handlers::subjects`) to say, beside each kind it keeps for
//! good, how many home keeps. The kinds are the page's list
//! (`KEPT_FOR_GOOD` in paulgsc/some-ui `apps/www/src/lib/my-data/words.ts`),
//! each read from the table that is its record ([`Record`] names it).
//!
//! - **Counts and one time only.** Each kind is a `COUNT` and a `MIN` of a
//!   time column. No body, text, title, key or id is read out, so nothing the
//!   owner wrote or said leaves through it.
//! - **Per subject, and only per subject.** Every part of the one statement
//!   takes the subject and filters on it, on a key or index whose first
//!   column is `subject_id`: a range read of that subject's rows, never a
//!   scan of anyone else's.
//! - **Bounded by the kinds.** The answer is at most one row per [`Record`],
//!   however much the subject keeps.
//! - **No sizes.** None of these tables stores a row's size, so a size would
//!   read every body the subject holds. Room per table is `storage_sample`'s,
//!   across subjects.
//! - **Reads only.** Its one caller is `file_host`'s `handlers::subjects`.

use sqlx::SqliteConnection;

/// A kind of record home keeps for good, in the page's order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Record {
	/// `sessions`: each study session home holds; since its earliest
	/// `created_at`.
	Sessions,
	/// `written_reflection`: each typed reflection; since the earliest `at`.
	Reflections,
	/// `idea_revision`: each idea, however many revisions; since the
	/// earliest `created_at`.
	Ideas,
	/// `activity_outcome`: each block played; since the earliest
	/// `started_at`.
	Practice,
	/// `korean_entry`: each entry of the Korean record (a word set, a pace,
	/// a lens line, a day), however many revisions; since the earliest
	/// `written_at`.
	Korean,
	/// `jobs_event`: each event of the Jobs record; since the earliest `at`.
	Jobs,
	/// `aph_entry`: each self entry, however many revisions; since the
	/// earliest `written_at`.
	SelfEntries,
	/// `leetype_session_event`: each `LeetType` session; since its earliest
	/// event's `at`.
	Leetype,
	/// `rust_round`: each Rust Round; since the earliest `received_at`.
	RustRounds,
	/// `drama_prompt`: each ask for the next scene; since the moment the
	/// earliest was asked for (its stamp).
	SceneAsks,
	/// `studio_sitting_event`: each Studio sitting; since its earliest
	/// event's `at`.
	StudioLog,
	/// `studio_video_queued`: each video queued on home, however many
	/// revisions; since the earliest `queued_at`.
	QueuedVideos,
	/// `rust_week_final` and RR14's `rust_week_sent`: each week of Rust
	/// Rounds whose send is kept for good, a week in both counted once; since
	/// the earliest `received_at`. A draft is not kept for good, so not here.
	RustWeeks,
	/// `rust_round_log`: each card-era Rust Rounds session, however many
	/// revisions; since the earliest `started_at` (stored in UTC, so the
	/// text minimum is the earliest in time).
	RustRoundLog,
}

impl Record {
	/// Every kind, in the page's order.
	pub const ALL: [Self; 14] = [
		Self::Sessions,
		Self::Reflections,
		Self::Ideas,
		Self::Practice,
		Self::Korean,
		Self::Jobs,
		Self::SelfEntries,
		Self::Leetype,
		Self::RustRounds,
		Self::SceneAsks,
		Self::StudioLog,
		Self::QueuedVideos,
		Self::RustWeeks,
		Self::RustRoundLog,
	];

	/// The name the answer gives the kind.
	#[must_use]
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::Sessions => "sessions",
			Self::Reflections => "reflections",
			Self::Ideas => "ideas",
			Self::Practice => "practice",
			Self::Korean => "korean",
			Self::Jobs => "jobs",
			Self::SelfEntries => "self",
			Self::Leetype => "leetype",
			Self::RustRounds => "rust-rounds",
			Self::SceneAsks => "scene-asks",
			Self::StudioLog => "studio-log",
			Self::QueuedVideos => "queued-videos",
			Self::RustWeeks => "rust-weeks",
			Self::RustRoundLog => "rust-round-log",
		}
	}

	fn named(name: &str) -> Option<Self> {
		Self::ALL.into_iter().find(|record| record.as_str() == name)
	}
}

/// How many of one kind home keeps for a subject, and since when.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Held {
	pub record: Record,
	/// How many: rows, or for a kind kept as revisions or events, the
	/// records they make up (see [`Record`]).
	pub count: i64,
	/// The earliest time the kind's records name, ISO-8601 UTC as stored.
	pub since: String,
}

/// What `subject` keeps for good on home, by kind, in [`Record::ALL`]'s
/// order; a kind it keeps none of is not listed.
///
/// One statement, so its parts agree.
///
/// # Errors
/// Any `sqlx` failure.
pub async fn kept(conn: &mut SqliteConnection, subject: &str) -> Result<Vec<Held>, sqlx::Error> {
	let rows = sqlx::query!(
		r#"SELECT 'sessions' AS "kind!: String", COUNT(*) AS "count!: i64", MIN(created_at) AS "since?: String"
		     FROM sessions WHERE subject_id = ?1
		   UNION ALL
		   SELECT 'reflections', COUNT(*), MIN(at) FROM written_reflection WHERE subject_id = ?1
		   UNION ALL
		   SELECT 'ideas', COUNT(DISTINCT idea_id), MIN(created_at) FROM idea_revision WHERE subject_id = ?1
		   UNION ALL
		   SELECT 'practice', COUNT(*), MIN(started_at) FROM activity_outcome WHERE subject_id = ?1
		   UNION ALL
		   SELECT 'korean',
		          (SELECT COUNT(*) FROM (SELECT 1 FROM korean_entry WHERE subject_id = ?1 GROUP BY kind, entry_key)),
		          (SELECT MIN(written_at) FROM korean_entry WHERE subject_id = ?1)
		   UNION ALL
		   SELECT 'jobs', COUNT(*), MIN(at) FROM jobs_event WHERE subject_id = ?1
		   UNION ALL
		   SELECT 'self', COUNT(DISTINCT entry_id), MIN(written_at) FROM aph_entry WHERE subject_id = ?1
		   UNION ALL
		   SELECT 'leetype', COUNT(DISTINCT session_id), MIN(at) FROM leetype_session_event WHERE subject_id = ?1
		   UNION ALL
		   SELECT 'rust-rounds', COUNT(*), MIN(received_at) FROM rust_round WHERE subject_id = ?1
		   UNION ALL
		   SELECT 'scene-asks', COUNT(*), MIN(stamp) FROM drama_prompt WHERE subject_id = ?1
		   UNION ALL
		   SELECT 'studio-log', COUNT(DISTINCT sitting_id), MIN(at) FROM studio_sitting_event WHERE subject_id = ?1
		   UNION ALL
		   SELECT 'queued-videos', COUNT(DISTINCT video_id), MIN(queued_at) FROM studio_video_queued WHERE subject_id = ?1
		   UNION ALL
		   SELECT 'rust-weeks', COUNT(DISTINCT week_id), MIN(received_at) FROM (
		       SELECT week_id, received_at FROM rust_week_final WHERE subject_id = ?1
		       UNION ALL
		       SELECT week_id, received_at FROM rust_week_sent WHERE subject_id = ?1)
		   UNION ALL
		   SELECT 'rust-round-log', COUNT(DISTINCT session_id), MIN(started_at) FROM rust_round_log WHERE subject_id = ?1"#,
		subject
	)
	.fetch_all(&mut *conn)
	.await?;
	let mut held: Vec<Held> = rows
		.into_iter()
		.filter_map(|row| {
			let record = Record::named(&row.kind)?;
			let since = row.since?;
			let since = if record == Record::SceneAsks { stamp_moment(&since)? } else { since };
			(row.count > 0).then_some(Held { record, count: row.count, since })
		})
		.collect();
	held.sort_by_key(|kind| kind.record);
	Ok(held)
}

/// A drama prompt's stamp (`drama-20261031T071503120Z`, `drama_prompt_repo`'s
/// `is_stamp`) as the moment it names, `2026-10-31T07:15:03.120Z`; `None`
/// for one that is not a stamp.
fn stamp_moment(stamp: &str) -> Option<String> {
	let digits = stamp.strip_prefix("drama-")?.strip_suffix('Z')?;
	let (date, time) = digits.split_once('T')?;
	if date.len() != 8 || time.len() != 9 || !date.bytes().chain(time.bytes()).all(|b| b.is_ascii_digit()) {
		return None;
	}
	let part = |text: &str, from: usize, to: usize| text.get(from..to).map(str::to_owned);
	let pieces = [
		part(date, 0, 4)?,
		String::from("-"),
		part(date, 4, 6)?,
		String::from("-"),
		part(date, 6, 8)?,
		String::from("T"),
		part(time, 0, 2)?,
		String::from(":"),
		part(time, 2, 4)?,
		String::from(":"),
		part(time, 4, 6)?,
		String::from("."),
		part(time, 6, 9)?,
		String::from("Z"),
	];
	Some(pieces.concat())
}

#[cfg(test)]
mod tests {
	use super::{kept, stamp_moment, Held, Record};
	use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};

	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");
	const A: &str = "subject-a";
	const B: &str = "subject-b";

	async fn pool() -> SqlitePool {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();
		for subject in [A, B] {
			sqlx::query("INSERT INTO account (subject_id, user_handle) VALUES (?1, ?2)")
				.bind(subject)
				.bind(subject.as_bytes())
				.execute(&pool)
				.await
				.unwrap();
		}
		pool
	}

	/// Runs `sql` once per subject in `subjects`, binding the subject as `?1`.
	async fn each(pool: &SqlitePool, subjects: &[&str], sql: &str) {
		for subject in subjects {
			sqlx::query(sql).bind(subject).execute(pool).await.unwrap();
		}
	}

	/// One or two records of every kind for A (revisions and events counted
	/// as the records they make up), one of some kinds for B.
	async fn seed(pool: &SqlitePool) {
		each(
			pool,
			&[A, B],
			"INSERT INTO sessions (id, subject_id, name, status, origin, layout_mode, total_duration_ms, created_at, updated_at, activities, scenes)
			 VALUES (?1 || '-s1', ?1, 'n', 'completed', 'user', 'single', 0, '2026-09-02T10:00:00.000Z', '2026-09-02T10:00:00.000Z', '[]', '[]')",
		)
		.await;
		each(
			pool,
			&[A],
			"INSERT INTO sessions (id, subject_id, name, status, origin, layout_mode, total_duration_ms, created_at, updated_at, activities, scenes)
			 VALUES ('a-s2', ?1, 'n', 'completed', 'user', 'single', 0, '2026-09-01T10:00:00.000Z', '2026-09-01T10:00:00.000Z', '[]', '[]')",
		)
		.await;
		each(
			pool,
			&[A, B],
			"INSERT INTO written_reflection (subject_id, reflection_id, session_id, at, text, written_at)
			 VALUES (?1, 'r1', 's1', '2026-09-03T10:00:00.000Z', 'words', '2026-09-03T10:00:01.000Z')",
		)
		.await;
		for revision in [1, 2] {
			sqlx::query(
				"INSERT INTO idea_revision (subject_id, idea_id, revision, created_at, sent_at, title, topic, format, keywords, questions, stored_at)
				 VALUES (?1, 'i1', ?2, '2026-09-04T10:00:00.000Z', ?3, 't', 'x', 'short', '[]', 1, ?3)",
			)
			.bind(A)
			.bind(revision)
			.bind(if revision == 1 { "2026-09-04T10:00:00.000Z" } else { "2026-09-05T10:00:00.000Z" })
			.execute(pool)
			.await
			.unwrap();
		}
		each(
			pool,
			&[A, B],
			"INSERT INTO activity_outcome (subject_id, session_id, activity_id, block_index, started_at, ended_at, planned_ms, elapsed_ms, outcome)
			 VALUES (?1, ?1 || '-s1', 'honeycomb', 0, '2026-09-06T10:00:00.000Z', '2026-09-06T10:05:00.000Z', 1, 1, 'completed')",
		)
		.await;
		seed_logs(pool).await;
	}

	/// The seed's logs and records kept as revisions or events.
	async fn seed_logs(pool: &SqlitePool) {
		for (kind, key, revision, at) in [
			("words", "set-1", 1, "2026-09-08T10:00:00.000Z"),
			("words", "set-1", 2, "2026-09-09T10:00:00.000Z"),
			("lens", "set-1", 1, "2026-09-07T10:00:00.000Z"),
		] {
			sqlx::query("INSERT INTO korean_entry (subject_id, kind, entry_key, revision, body, sha256, written_at) VALUES (?1, ?2, ?3, ?4, '{}', 'h', ?5)")
				.bind(A)
				.bind(kind)
				.bind(key)
				.bind(revision)
				.bind(at)
				.execute(pool)
				.await
				.unwrap();
		}
		each(
			pool,
			&[A, B],
			"INSERT INTO jobs_event (subject_id, event_id, at, body, written_at) VALUES (?1, 'j1', '2026-09-10T10:00:00.000Z', '{}', '2026-09-10T10:00:01.000Z')",
		)
		.await;
		for revision in [1, 2] {
			sqlx::query(
				"INSERT INTO aph_entry (subject_id, entry_id, revision, day, checkpoint, plain, body, written_at)
				 VALUES (?1, 'e1', ?2, '2026-09-11', '7', 0, '{}', ?3)",
			)
			.bind(A)
			.bind(revision)
			.bind(if revision == 1 { "2026-09-11T10:00:00.000Z" } else { "2026-09-12T10:00:00.000Z" })
			.execute(pool)
			.await
			.unwrap();
		}
		for (session, event, at) in [
			("l1", "e1", "2026-09-13T10:00:00.000Z"),
			("l1", "e2", "2026-09-13T10:05:00.000Z"),
			("l2", "e1", "2026-09-14T10:00:00.000Z"),
		] {
			sqlx::query("INSERT INTO leetype_session_event (subject_id, session_id, event_id, at, type, body, written_at) VALUES (?1, ?2, ?3, ?4, 'started', '{}', ?4)")
				.bind(A)
				.bind(session)
				.bind(event)
				.bind(at)
				.execute(pool)
				.await
				.unwrap();
		}
		each(
			pool,
			&[A, B],
			"INSERT INTO rust_round (subject_id, id, body, content_hash, received_at) VALUES (?1, 'rr1', '{}', 'h', '2026-09-15T10:00:00.000Z')",
		)
		.await;
		each(
			pool,
			&[A, B],
			"INSERT INTO drama_prompt (subject_id, stamp, text, written_at) VALUES (?1, 'drama-20260916T071503120Z', 'ask', '2026-09-16T08:00:00.000Z')",
		)
		.await;
		for (sitting, event, at) in [("st1", "e1", "2026-09-17T10:00:00.000Z"), ("st1", "e2", "2026-09-17T10:30:00.000Z")] {
			sqlx::query("INSERT INTO studio_sitting_event (subject_id, sitting_id, event_id, at, type, kind, body, written_at) VALUES (?1, ?2, ?3, ?4, 'opened', 'x', '{}', ?4)")
				.bind(A)
				.bind(sitting)
				.bind(event)
				.bind(at)
				.execute(pool)
				.await
				.unwrap();
		}
		for (video, rev, at) in [("v1", 1, "2026-09-18T10:00:00.000Z"), ("v1", 2, "2026-09-19T10:00:00.000Z")] {
			sqlx::query(
				"INSERT INTO studio_video_queued (subject_id, video_id, rev, title, queued_at, template_kind, template_id, template_version, dna, dna_sha256, stored_at)
				 VALUES (?1, ?2, ?3, 't', ?4, 'show-format', 'playoff-delta', 1, '{}', 'h', ?4)",
			)
			.bind(A)
			.bind(video)
			.bind(rev)
			.bind(at)
			.execute(pool)
			.await
			.unwrap();
		}
		seed_rust_weeks(pool).await;
	}

	/// Two weeks of Rust Rounds kept for good (one RR14 kept and made final
	/// too, counted once; one only final), a draft that is not kept for good,
	/// and two card-era sessions, one in two revisions.
	async fn seed_rust_weeks(pool: &SqlitePool) {
		for statement in [
			"INSERT INTO rust_week_sent (subject_id, week_id, sent_at, body, received_at) VALUES (?1, '2026-w39', 'x', '{}', '2026-09-20T10:00:00.000Z')",
			"INSERT INTO rust_week_final (subject_id, week_id, sent_at, body, received_at) VALUES (?1, '2026-w39', 'x', '{}', '2026-09-28T08:00:00.000Z')",
			"INSERT INTO rust_week_final (subject_id, week_id, sent_at, body, received_at) VALUES (?1, '2026-w40', 'x', '{}', '2026-10-05T08:00:00.000Z')",
			"INSERT INTO rust_week_draft (subject_id, week_id, sent_at, body, received_at) VALUES (?1, '2026-w41', 'x', '{}', '2026-09-01T00:00:00.000Z')",
		] {
			sqlx::query(statement).bind(A).execute(pool).await.unwrap();
		}
		for (session, revision, started) in [
			("s1", 1, "2026-09-21T10:00:00.000Z"),
			("s1", 2, "2026-09-21T10:00:00.000Z"),
			("s2", 1, "2026-09-22T10:00:00.000Z"),
		] {
			sqlx::query("INSERT INTO rust_round_log (subject_id, session_id, revision, started_at, body, sha256, written_at) VALUES (?1, ?2, ?3, ?4, '{}', 'h', ?4)")
				.bind(A)
				.bind(session)
				.bind(revision)
				.bind(started)
				.execute(pool)
				.await
				.unwrap();
		}
	}

	fn held(record: Record, count: i64, since: &str) -> Held {
		Held {
			record,
			count,
			since: since.to_owned(),
		}
	}

	#[tokio::test]
	async fn home_counts_each_kind_it_keeps_for_good_and_since_when_for_its_subject_only() {
		let pool = pool().await;
		seed(&pool).await;
		let mut conn = pool.acquire().await.unwrap();

		assert_eq!(
			kept(&mut conn, A).await.unwrap(),
			vec![
				held(Record::Sessions, 2, "2026-09-01T10:00:00.000Z"),
				held(Record::Reflections, 1, "2026-09-03T10:00:00.000Z"),
				held(Record::Ideas, 1, "2026-09-04T10:00:00.000Z"),
				held(Record::Practice, 1, "2026-09-06T10:00:00.000Z"),
				held(Record::Korean, 2, "2026-09-07T10:00:00.000Z"),
				held(Record::Jobs, 1, "2026-09-10T10:00:00.000Z"),
				held(Record::SelfEntries, 1, "2026-09-11T10:00:00.000Z"),
				held(Record::Leetype, 2, "2026-09-13T10:00:00.000Z"),
				held(Record::RustRounds, 1, "2026-09-15T10:00:00.000Z"),
				held(Record::SceneAsks, 1, "2026-09-16T07:15:03.120Z"),
				held(Record::StudioLog, 1, "2026-09-17T10:00:00.000Z"),
				held(Record::QueuedVideos, 1, "2026-09-18T10:00:00.000Z"),
				held(Record::RustWeeks, 2, "2026-09-20T10:00:00.000Z"),
				held(Record::RustRoundLog, 2, "2026-09-21T10:00:00.000Z"),
			],
			"every kind, revisions and events counted as their records, B's in none"
		);
		let other: Vec<Record> = kept(&mut conn, B).await.unwrap().into_iter().map(|kind| kind.record).collect();
		assert_eq!(
			other,
			[Record::Sessions, Record::Reflections, Record::Practice, Record::Jobs, Record::RustRounds, Record::SceneAsks],
			"a kind the subject keeps none of is not listed"
		);
		assert!(kept(&mut conn, "subject-c").await.unwrap().is_empty());
	}

	#[test]
	fn a_stamp_reads_as_its_moment_and_anything_else_as_none() {
		assert_eq!(stamp_moment("drama-20261031T071503120Z").as_deref(), Some("2026-10-31T07:15:03.120Z"));
		for refused in ["drama-2026103T071503120Z", "20261031T071503120Z", "drama-20261031T07150312xZ", "drama-20261031T071503120"] {
			assert_eq!(stamp_moment(refused), None, "{refused}");
		}
	}

	#[test]
	fn every_kind_reads_back_by_its_name() {
		for record in Record::ALL {
			assert_eq!(Record::named(record.as_str()), Some(record));
		}
		assert_eq!(Record::named("voice"), None);
	}
}

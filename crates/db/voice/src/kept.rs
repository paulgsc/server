//! What home keeps of the owner's voice notes, as counts and sizes.
//!
//! The phone's My data page (paulgsc/some-ui#2476, MY-DATA MD2) reads it to
//! say how many of each kind home holds, how much room they take, since
//! when, how many were set aside, and how the room grew week by week.
//!
//! - **Descriptions only.** Every read here is of `voice_note` and
//!   `voice_note_set_aside`, and [`newest`] reads whether an idea's answer
//!   names a note (`idea_answer`), to say whether My data may offer to delete
//!   it; none reads `voice_note_audio`, a transcript, a
//!   `ref` or a bite, so what was said never leaves through it.
//! - **Per subject, and only per subject.** Every query takes the subject and
//!   filters on it.
//! - **Bounded.** [`kinds`] answers at most one row per [`Of`], [`weeks`] one
//!   row per week since the subject's first note reached home, and [`newest`]
//!   at most [`NEWEST`] notes.
//! - **Reads only.** Its one caller is `file_host`'s `handlers::voice`
//!   (`GET /api/v1/voice-notes/kept`).

use crate::{
	delete::{deletable, Deletable},
	Of, VoiceError,
};
use serde::Serialize;
use sqlx::SqliteConnection;

/// The most notes [`newest`] answers.
pub const NEWEST: i64 = 50;

/// What home keeps of one kind of note.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KindKept {
	/// What the notes were said for.
	pub of: Of,
	/// How many home holds.
	pub notes: i64,
	/// Their audio's bytes, summed from each note's `size`.
	pub bytes: i64,
	/// How many of them the owner set aside (they are kept too).
	pub set_aside: i64,
	/// When the earliest of them was recorded, ISO-8601 UTC.
	pub since: String,
}

/// What reached home in one week.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WeekKept {
	/// The week's Monday (UTC), `YYYY-MM-DD`.
	pub week: String,
	/// How many notes reached home that week.
	pub notes: i64,
	/// Their audio's bytes.
	pub bytes: i64,
}

/// One note as My data lists it: no `ref`, no audio, nothing said.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteKept {
	/// The phone's id for the note.
	pub id: String,
	/// When the phone recorded it, ISO-8601 UTC.
	pub recorded_at: String,
	/// When it reached home, ISO-8601 UTC.
	pub stored_at: String,
	/// Its audio's bytes.
	pub size: i64,
	/// When the owner set it aside, if they did.
	pub set_aside_at: Option<String>,
	/// My data may offer to delete it: its kind is one the owner switched on
	/// ([`crate::delete::SWITCHED_ON`]), and it is not the work.
	pub deletable: bool,
}

/// What `subject` keeps on home, by kind, in [`Of`]'s order.
///
/// # Errors
/// [`VoiceError::Unreadable`] for a stored kind this crate would not have
/// written, and [`VoiceError::Storage`] for any `sqlx` failure.
pub async fn kinds(conn: &mut SqliteConnection, subject: &str) -> Result<Vec<KindKept>, VoiceError> {
	let rows = sqlx::query!(
		r#"SELECT note.of_kind AS "of_kind!: String",
		          COUNT(*) AS "notes!: i64",
		          SUM(note.size) AS "bytes!: i64",
		          COUNT(aside.note_id) AS "set_aside!: i64",
		          MIN(note.recorded_at) AS "since!: String"
		   FROM voice_note AS note
		   LEFT JOIN voice_note_set_aside AS aside ON aside.subject_id = note.subject_id AND aside.note_id = note.note_id
		   WHERE note.subject_id = ?1
		   GROUP BY note.of_kind"#,
		subject
	)
	.fetch_all(&mut *conn)
	.await?;
	let mut kinds = rows
		.into_iter()
		.map(|row| {
			Ok(KindKept {
				of: Of::named(&row.of_kind).ok_or(VoiceError::Unreadable)?,
				notes: row.notes,
				bytes: row.bytes,
				set_aside: row.set_aside,
				since: row.since,
			})
		})
		.collect::<Result<Vec<_>, VoiceError>>()?;
	kinds.sort_by_key(|kind| Of::ALL.iter().position(|of| *of == kind.of));
	Ok(kinds)
}

/// What reached home for `subject`, week by week (weeks start on Monday,
/// UTC), oldest first. A week nothing reached home is not listed.
///
/// # Errors
/// [`VoiceError::Storage`] for any `sqlx` failure.
pub async fn weeks(conn: &mut SqliteConnection, subject: &str) -> Result<Vec<WeekKept>, VoiceError> {
	// `weekday 0` moves to the coming Sunday (or stays on one), so six days
	// back is that week's Monday.
	let rows = sqlx::query!(
		r#"SELECT date(stored_at, 'weekday 0', '-6 days') AS "week!: String",
		          COUNT(*) AS "notes!: i64",
		          SUM(size) AS "bytes!: i64"
		   FROM voice_note
		   WHERE subject_id = ?1
		   GROUP BY 1
		   ORDER BY 1"#,
		subject
	)
	.fetch_all(&mut *conn)
	.await?;
	Ok(
		rows
			.into_iter()
			.map(|row| WeekKept {
				week: row.week,
				notes: row.notes,
				bytes: row.bytes,
			})
			.collect(),
	)
}

/// `subject`'s newest notes said for `of`, newest recorded first, at most
/// [`NEWEST`].
///
/// # Errors
/// [`VoiceError::Storage`] for any `sqlx` failure.
pub async fn newest(conn: &mut SqliteConnection, subject: &str, of: Of, on: &[Deletable]) -> Result<Vec<NoteKept>, VoiceError> {
	let of = of.as_str();
	let rows = sqlx::query!(
		r#"SELECT note.note_id, note.of_ref, note.recorded_at, note.stored_at, note.size,
		          aside.set_aside_at AS "set_aside_at?: String",
		          EXISTS (SELECT 1 FROM idea_answer AS answer WHERE answer.subject_id = note.subject_id AND answer.note_id = note.note_id)
		            AS "answers!: bool"
		   FROM voice_note AS note
		   LEFT JOIN voice_note_set_aside AS aside ON aside.subject_id = note.subject_id AND aside.note_id = note.note_id
		   WHERE note.subject_id = ?1 AND note.of_kind = ?2
		   ORDER BY note.recorded_at DESC, note.note_id DESC
		   LIMIT ?3"#,
		subject,
		of,
		NEWEST
	)
	.fetch_all(&mut *conn)
	.await?;
	Ok(
		rows
			.into_iter()
			.map(|row| NoteKept {
				id: row.note_id,
				recorded_at: row.recorded_at,
				stored_at: row.stored_at,
				size: row.size,
				set_aside_at: row.set_aside_at,
				deletable: deletable(of, &row.of_ref, row.answers).is_some_and(|kind| on.contains(&kind)),
			})
			.collect(),
	)
}

#[cfg(test)]
mod tests {
	use super::{kinds, newest, weeks, KindKept, WeekKept, NEWEST};
	use crate::{delete::Deletable, set_aside::set_aside, Of};
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

	/// Stores `subject`'s note `id`, said for `of` and recorded at `at`, of
	/// `size` bytes, reaching home at `now`, as `put_note` would.
	async fn note(pool: &SqlitePool, subject: &str, id: &str, of: &str, at: &str, size: i64, now: &str) {
		sqlx::query(
			"INSERT INTO voice_note (subject_id, note_id, of_kind, of_ref, recorded_at, content_type, size, sha256, stored_at)
			 VALUES (?1, ?2, ?3, 'ref', ?4, 'audio/webm', ?5, 's', ?6)",
		)
		.bind(subject)
		.bind(id)
		.bind(of)
		.bind(at)
		.bind(size)
		.bind(now)
		.execute(pool)
		.await
		.unwrap();
	}

	#[tokio::test]
	async fn home_counts_each_kind_its_room_and_what_was_set_aside_for_its_subject_only() {
		let pool = pool().await;
		note(&pool, A, "s1", "studio", "2026-10-01T10:00:00Z", 300, "2026-10-01T10:00:01.000Z").await;
		note(&pool, A, "s2", "studio", "2026-09-12T10:00:00Z", 200, "2026-10-02T10:00:01.000Z").await;
		note(&pool, A, "r1", "reflection", "2026-10-03T10:00:00Z", 50, "2026-10-03T10:00:01.000Z").await;
		note(&pool, B, "x1", "studio", "2026-08-01T10:00:00Z", 999, "2026-08-01T10:00:01.000Z").await;
		let mut conn = pool.acquire().await.unwrap();
		set_aside(&mut conn, A, "s1", "2026-10-04T10:00:00.000Z", "2026-10-04T10:00:01.000Z").await.unwrap();

		assert_eq!(
			kinds(&mut conn, A).await.unwrap(),
			vec![
				KindKept {
					of: Of::Reflection,
					notes: 1,
					bytes: 50,
					set_aside: 0,
					since: "2026-10-03T10:00:00Z".to_owned(),
				},
				KindKept {
					of: Of::Studio,
					notes: 2,
					bytes: 500,
					set_aside: 1,
					since: "2026-09-12T10:00:00Z".to_owned(),
				},
			],
			"in Of's order, with B's take in neither"
		);
		assert!(kinds(&mut conn, "subject-c").await.unwrap().is_empty());
	}

	#[tokio::test]
	async fn what_reached_home_is_summed_by_the_week_it_arrived() {
		let pool = pool().await;
		// 2026-10-05 is a Monday, 2026-10-11 the Sunday that ends its week.
		note(&pool, A, "n1", "studio", "2026-09-01T10:00:00Z", 100, "2026-10-05T00:00:00.000Z").await;
		note(&pool, A, "n2", "lens", "2026-10-11T10:00:00Z", 20, "2026-10-11T23:59:59.000Z").await;
		note(&pool, A, "n3", "studio", "2026-10-12T10:00:00Z", 7, "2026-10-12T00:00:00.000Z").await;
		note(&pool, B, "n4", "studio", "2026-10-12T10:00:00Z", 999, "2026-10-12T00:00:00.000Z").await;
		let mut conn = pool.acquire().await.unwrap();

		assert_eq!(
			weeks(&mut conn, A).await.unwrap(),
			vec![
				WeekKept {
					week: "2026-10-05".to_owned(),
					notes: 2,
					bytes: 120,
				},
				WeekKept {
					week: "2026-10-12".to_owned(),
					notes: 1,
					bytes: 7,
				},
			]
		);
	}

	#[tokio::test]
	async fn a_kinds_newest_notes_come_first_and_stop_at_the_cap() {
		let pool = pool().await;
		for i in 0..=NEWEST {
			let id = String::from("t") + &i.to_string();
			let at = String::from("2026-10-01T10:") + &format_two(i) + ":00Z";
			note(&pool, A, &id, "studio", &at, 10, "2026-10-02T00:00:00.000Z").await;
		}
		note(&pool, A, "r1", "reflection", "2026-10-09T10:00:00Z", 10, "2026-10-09T10:00:01.000Z").await;
		note(&pool, B, "b1", "studio", "2026-12-01T10:00:00Z", 10, "2026-12-01T10:00:01.000Z").await;
		let mut conn = pool.acquire().await.unwrap();
		set_aside(&mut conn, A, "t50", "2026-10-03T10:00:00.000Z", "2026-10-03T10:00:01.000Z").await.unwrap();

		let listed = newest(&mut conn, A, Of::Studio, &[]).await.unwrap();
		assert_eq!(listed.len(), usize::try_from(NEWEST).unwrap());
		assert_eq!(listed[0].id, "t50", "the newest recorded first, B's newer take not among them");
		assert_eq!(listed[0].set_aside_at.as_deref(), Some("2026-10-03T10:00:00.000Z"));
		assert_eq!(listed[0].size, 10);
		assert_eq!(listed[1].set_aside_at, None);
		assert!(listed.iter().all(|kept| kept.id.starts_with('t')), "only the kind asked for");
		assert!(!listed.iter().any(|kept| kept.id == "t0"), "the oldest past the cap is left out");
		assert!(listed.iter().all(|kept| !kept.deletable), "nothing is deletable while takes are switched off");
	}

	/// With takes switched on, a take is listed as deletable and an idea's
	/// spoken answer, the work, is not.
	#[tokio::test]
	async fn a_take_is_listed_as_deletable_only_when_takes_are_switched_on() {
		let pool = pool().await;
		note(&pool, A, "take", "studio", "2026-10-01T10:00:00Z", 10, "2026-10-02T00:00:00.000Z").await;
		note(&pool, A, "answer", "studio", "2026-10-01T11:00:00Z", 10, "2026-10-02T00:00:00.000Z").await;
		sqlx::query(
			"INSERT INTO voice_note (subject_id, note_id, of_kind, of_ref, recorded_at, content_type, size, sha256, stored_at)
			 VALUES (?1, 'spoken', 'studio', 'idea:i1:why', '2026-10-01T09:00:00Z', 'audio/webm', 10, 's', '2026-10-02T00:00:00.000Z')",
		)
		.bind(A)
		.execute(&pool)
		.await
		.unwrap();
		for statement in [
			"INSERT INTO idea_revision (subject_id, idea_id, revision, created_at, sent_at, title, topic, format, keywords, questions, stored_at)
			 VALUES (?1, 'i', 1, 'then', 'then', 't', 'Studio', 'short', '[]', 1, 'now')",
			"INSERT INTO idea_answer (subject_id, idea_id, revision, question, note_id, typed) VALUES (?1, 'i', 1, 'what', 'answer', NULL)",
		] {
			sqlx::query(statement).bind(A).execute(&pool).await.unwrap();
		}
		let mut conn = pool.acquire().await.unwrap();
		let listed = newest(&mut conn, A, Of::Studio, &[Deletable::StudioTake]).await.unwrap();
		let marked: Vec<(&str, bool)> = listed.iter().map(|kept| (kept.id.as_str(), kept.deletable)).collect();
		assert_eq!(marked, [("answer", false), ("take", true), ("spoken", false)]);
	}

	/// `i` as two digits, without `format!` (banned repo-wide).
	fn format_two(i: i64) -> String {
		let digits = i.to_string();
		if digits.len() < 2 {
			String::from("0") + &digits
		} else {
			digits
		}
	}
}

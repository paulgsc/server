//! What home keeps of the owner's voice notes, as counts and sizes.
//!
//! The phone's My data page (paulgsc/some-ui#2476, MY-DATA MD2) reads it to
//! say how many of each kind home holds, how much room they take, since
//! when, how many were set aside, and how the room grew week by week.
//!
//! - **Descriptions, and the start of what home heard.** Every read here is
//!   of `voice_note`, `voice_note_set_aside` and, for [`weeks`], the receipts
//!   in `voice_note_deletion`. [`newest`] also reads whether an idea's answer
//!   names a note (`idea_answer`), to say whether My data may offer to delete
//!   it, and each note's chosen ending in `voice_transcript`, to say how long
//!   it is and its first words ([`HEARD_CHARS`] or so), so the owner can tell
//!   what they would delete (the owner, MY-DATA, paulgsc/some-ui#2473,
//!   2026-11-13).
//!   That ending is the one `GET /voice-notes/:id/heard` answers
//!   ([`crate::transcript::heard`]), which already gives a note's words to its
//!   own subject: what was said reaches only the owner's paired phone through
//!   it. None reads `voice_note_audio`, a word's time, a `ref` or a bite;
//!   whether a note plays back is its kind's ([`crate::PLAYED_BACK`]).
//! - **Per subject, and only per subject.** Every query takes the subject and
//!   filters on it.
//! - **Bounded.** [`kinds`] answers at most one row per [`Of`], [`weeks`] one
//!   row per week since the subject's first note reached home, and [`newest`]
//!   at most [`NEWEST`] notes and the one it is asked for.
//! - **Reads only.** Its one caller is `file_host`'s `handlers::voice`
//!   (`GET /api/v1/voice-notes/kept`).

use crate::{
	delete::{deletable, Deletable},
	Of, VoiceError, PLAYED_BACK,
};
use serde::Serialize;
use sqlx::SqliteConnection;

/// The most notes [`newest`] answers.
pub const NEWEST: i64 = 50;

/// About how many characters of what home heard [`newest`] answers per note:
/// the words that fit, cut at the last space before it, or at it when a word
/// runs past it.
pub const HEARD_CHARS: usize = 80;

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

/// What reached home in one week, and what the owner deleted from it then.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WeekKept {
	/// The week's Monday (UTC), `YYYY-MM-DD`.
	pub week: String,
	/// How many notes reached home that week, those deleted since included.
	pub notes: i64,
	/// Their audio's bytes.
	pub bytes: i64,
	/// How many notes the owner deleted that week, from My data.
	pub left_notes: i64,
	/// Their audio's bytes.
	pub left_bytes: i64,
}

/// One note as My data lists it: no `ref` and no audio; of what was said,
/// only how long it ran and its first words.
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
	/// ([`crate::delete::SWITCHED_ON`]).
	pub deletable: bool,
	/// How long it runs, in milliseconds, as home heard it, words or silence;
	/// null while home has heard it neither as words nor as silence, and when
	/// its audio was unreadable.
	pub duration_ms: Option<i64>,
	/// The first words home heard in it, about [`HEARD_CHARS`] characters;
	/// null while home has not heard it, and when it was silent or heard only
	/// whitespace.
	pub heard: Option<String>,
	/// Whether [`NoteKept::heard`] stops short of what home heard.
	pub heard_cut: bool,
	/// Whether `GET /voice-notes/:id/audio` plays it back: its kind is in
	/// [`crate::PLAYED_BACK`].
	pub plays_back: bool,
}

/// One `voice_note` row as [`newest`] reads it, with its chosen ending.
struct Row {
	note_id: String,
	of_ref: String,
	recorded_at: String,
	stored_at: String,
	size: i64,
	set_aside_at: Option<String>,
	answers: bool,
	outcome: Option<String>,
	duration_ms: Option<i64>,
	text: Option<String>,
}

impl Row {
	fn kept(self, of: Of, on: &[Deletable]) -> Result<NoteKept, VoiceError> {
		let (heard, heard_cut) = match self.outcome.as_deref() {
			Some("heard") => {
				let (start, cut) = first_words(self.text.as_deref().ok_or(VoiceError::Unreadable)?);
				// Words that are only whitespace say nothing, as a silence does.
				(Some(start).filter(|start| !start.is_empty()), cut)
			}
			_ => (None, false),
		};
		Ok(NoteKept {
			deletable: deletable(of.as_str(), &self.of_ref, self.answers).is_some_and(|kind| on.contains(&kind)),
			id: self.note_id,
			recorded_at: self.recorded_at,
			stored_at: self.stored_at,
			size: self.size,
			set_aside_at: self.set_aside_at,
			duration_ms: self.duration_ms,
			heard,
			heard_cut,
			plays_back: PLAYED_BACK.contains(&of),
		})
	}
}

/// `text`'s first words, about [`HEARD_CHARS`] characters, and whether they
/// stop short of it.
///
/// Cut at the last whitespace within [`HEARD_CHARS`] characters, or at
/// [`HEARD_CHARS`] when there is none (a language written without spaces, or
/// one long word); never inside a character.
fn first_words(text: &str) -> (String, bool) {
	let text = text.trim();
	let Some((end, _)) = text.char_indices().nth(HEARD_CHARS) else {
		return (text.to_owned(), false);
	};
	let head = &text[..end];
	let start = if text[end..].starts_with(char::is_whitespace) {
		head
	} else {
		head.rfind(char::is_whitespace).map_or(head, |space| &head[..space])
	};
	(start.trim_end().to_owned(), true)
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
/// UTC), oldest first.
///
/// A note the owner later deleted still counts in the week it reached home,
/// and leaves in the week it was deleted, both read from its receipt, so a
/// delete never rewrites a past week (the owner, paulgsc/some-ui#2477) and
/// the weeks' running total is what home holds. A week nothing reached home
/// and nothing left it is not listed.
///
/// # Errors
/// [`VoiceError::Storage`] for any `sqlx` failure.
pub async fn weeks(conn: &mut SqliteConnection, subject: &str) -> Result<Vec<WeekKept>, VoiceError> {
	// `weekday 0` moves to the coming Sunday (or stays on one), so six days
	// back is that week's Monday.
	let rows = sqlx::query!(
		r#"SELECT date(at, 'weekday 0', '-6 days') AS "week!: String",
		          SUM(arrived) AS "notes!: i64",
		          SUM(arrived * size) AS "bytes!: i64",
		          SUM(1 - arrived) AS "left_notes!: i64",
		          SUM((1 - arrived) * size) AS "left_bytes!: i64"
		   FROM (SELECT stored_at AS at, size, 1 AS arrived FROM voice_note WHERE subject_id = ?1
		         UNION ALL
		         SELECT stored_at, size, 1 FROM voice_note_deletion WHERE subject_id = ?1
		         UNION ALL
		         SELECT deleted_at, size, 0 FROM voice_note_deletion WHERE subject_id = ?1)
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
				left_notes: row.left_notes,
				left_bytes: row.left_bytes,
			})
			.collect(),
	)
}

/// `subject`'s newest notes said for `of`, newest recorded first, at most
/// [`NEWEST`].
///
/// After them comes the note `also` names, when it is one of the subject's
/// said for `of` but older: the note another screen's "Delete…" opens My
/// data on (paulgsc/some-ui#2579).
///
/// # Errors
/// [`VoiceError::Unreadable`] for a heard transcript with no text, which this
/// crate never writes, and [`VoiceError::Storage`] for any `sqlx` failure.
pub async fn newest(conn: &mut SqliteConnection, subject: &str, of: Of, on: &[Deletable], also: Option<&str>) -> Result<Vec<NoteKept>, VoiceError> {
	let kind = of;
	let of = of.as_str();
	// The ending is `transcript::heard`'s choice, as `takes::library`'s is.
	let rows = sqlx::query_as!(
		Row,
		r#"SELECT note.note_id, note.of_ref, note.recorded_at, note.stored_at, note.size,
		          aside.set_aside_at AS "set_aside_at?: String",
		          EXISTS (SELECT 1 FROM idea_answer AS answer WHERE answer.subject_id = note.subject_id AND answer.note_id = note.note_id)
		            AS "answers!: bool",
		          heard.outcome AS "outcome?: String", heard.duration_ms AS "duration_ms?: i64", heard.text AS "text?: String"
		   FROM voice_note AS note
		   LEFT JOIN voice_note_set_aside AS aside ON aside.subject_id = note.subject_id AND aside.note_id = note.note_id
		   LEFT JOIN voice_transcript AS heard ON heard.rowid = (
		     SELECT hearing.rowid FROM voice_transcript AS hearing
		     WHERE hearing.subject_id = note.subject_id AND hearing.note_id = note.note_id
		       AND hearing.outcome IN ('heard', 'silent')
		     ORDER BY hearing.outcome = 'heard' DESC, hearing.heard_at DESC, hearing.model DESC
		     LIMIT 1
		   )
		   WHERE note.subject_id = ?1 AND note.of_kind = ?2
		   ORDER BY note.recorded_at DESC, note.note_id DESC
		   LIMIT ?3"#,
		subject,
		of,
		NEWEST
	)
	.fetch_all(&mut *conn)
	.await?;
	let mut notes = rows.into_iter().map(|row| row.kept(kind, on)).collect::<Result<Vec<_>, _>>()?;
	if let Some(also) = also.filter(|also| !notes.iter().any(|note| note.id == *also)) {
		let row = sqlx::query_as!(
			Row,
			r#"SELECT note.note_id, note.of_ref, note.recorded_at, note.stored_at, note.size,
			          aside.set_aside_at AS "set_aside_at?: String",
			          EXISTS (SELECT 1 FROM idea_answer AS answer WHERE answer.subject_id = note.subject_id AND answer.note_id = note.note_id)
			            AS "answers!: bool",
			          heard.outcome AS "outcome?: String", heard.duration_ms AS "duration_ms?: i64", heard.text AS "text?: String"
			   FROM voice_note AS note
			   LEFT JOIN voice_note_set_aside AS aside ON aside.subject_id = note.subject_id AND aside.note_id = note.note_id
			   LEFT JOIN voice_transcript AS heard ON heard.rowid = (
			     SELECT hearing.rowid FROM voice_transcript AS hearing
			     WHERE hearing.subject_id = note.subject_id AND hearing.note_id = note.note_id
			       AND hearing.outcome IN ('heard', 'silent')
			     ORDER BY hearing.outcome = 'heard' DESC, hearing.heard_at DESC, hearing.model DESC
			     LIMIT 1
			   )
			   WHERE note.subject_id = ?1 AND note.of_kind = ?2 AND note.note_id = ?3"#,
			subject,
			of,
			also
		)
		.fetch_optional(&mut *conn)
		.await?;
		if let Some(row) = row {
			notes.push(row.kept(kind, on)?);
		}
	}
	Ok(notes)
}

#[cfg(test)]
mod tests {
	use super::{first_words, kinds, newest, weeks, KindKept, WeekKept, HEARD_CHARS, NEWEST};
	use crate::{
		delete::{delete, Deletable, Deleted, SWITCHED_ON},
		set_aside::set_aside,
		transcript::{heard, Hearing},
		Of,
	};
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

	/// A note deleted since still counts in the week it reached home, and
	/// leaves in the week it was deleted.
	#[tokio::test]
	async fn what_reached_home_is_summed_by_the_week_it_arrived() {
		let pool = pool().await;
		// 2026-10-05 is a Monday, 2026-10-11 the Sunday that ends its week.
		note(&pool, A, "n1", "studio", "2026-09-01T10:00:00Z", 100, "2026-10-05T00:00:00.000Z").await;
		note(&pool, A, "n2", "lens", "2026-10-11T10:00:00Z", 20, "2026-10-11T23:59:59.000Z").await;
		note(&pool, A, "n3", "studio", "2026-10-12T10:00:00Z", 7, "2026-10-12T00:00:00.000Z").await;
		note(&pool, B, "n4", "studio", "2026-10-12T10:00:00Z", 999, "2026-10-12T00:00:00.000Z").await;
		note(&pool, A, "n5", "studio", "2026-10-06T10:00:00Z", 3, "2026-10-06T10:00:01.000Z").await;
		note(&pool, B, "n5", "studio", "2026-10-06T10:00:00Z", 999, "2026-10-06T10:00:01.000Z").await;
		let mut conn = pool.acquire().await.unwrap();
		for subject in [A, B] {
			let gone = delete(&mut conn, subject, "n5", SWITCHED_ON, "2026-11-04T10:00:00.000Z").await.unwrap();
			assert!(matches!(gone, Deleted::Gone(_)));
		}

		assert_eq!(
			weeks(&mut conn, A).await.unwrap(),
			vec![
				WeekKept {
					week: "2026-10-05".to_owned(),
					notes: 3,
					bytes: 123,
					left_notes: 0,
					left_bytes: 0,
				},
				WeekKept {
					week: "2026-10-12".to_owned(),
					notes: 1,
					bytes: 7,
					left_notes: 0,
					left_bytes: 0,
				},
				WeekKept {
					week: "2026-11-02".to_owned(),
					notes: 0,
					bytes: 0,
					left_notes: 1,
					left_bytes: 3,
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

		let listed = newest(&mut conn, A, Of::Studio, &[], None).await.unwrap();
		assert_eq!(listed.len(), usize::try_from(NEWEST).unwrap());
		assert_eq!(listed[0].id, "t50", "the newest recorded first, B's newer take not among them");
		assert_eq!(listed[0].set_aside_at.as_deref(), Some("2026-10-03T10:00:00.000Z"));
		assert_eq!(listed[0].size, 10);
		assert_eq!(listed[1].set_aside_at, None);
		assert!(listed.iter().all(|kept| kept.id.starts_with('t')), "only the kind asked for");
		assert!(!listed.iter().any(|kept| kept.id == "t0"), "the oldest past the cap is left out");
		assert!(listed.iter().all(|kept| !kept.deletable), "nothing is deletable while takes are switched off");
		let asked = newest(&mut conn, A, Of::Studio, &[], Some("t0")).await.unwrap();
		assert_eq!(asked.len(), listed.len() + 1);
		assert_eq!(asked.last().map(|kept| kept.id.as_str()), Some("t0"), "the note asked for, after the newest");
		for (also, why) in [("t50", "already listed"), ("r1", "another kind"), ("b1", "another subject's")] {
			assert_eq!(newest(&mut conn, A, Of::Studio, &[], Some(also)).await.unwrap().len(), listed.len(), "{why}");
		}
	}

	/// A note is listed as deletable only when its kind is switched on: with
	/// takes alone, a take is and an idea's spoken answer is not; with both,
	/// both are.
	#[tokio::test]
	async fn a_note_is_listed_as_deletable_only_when_its_kind_is_switched_on() {
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
		let listed = newest(&mut conn, A, Of::Studio, &[Deletable::StudioTake], None).await.unwrap();
		let marked: Vec<(&str, bool)> = listed.iter().map(|kept| (kept.id.as_str(), kept.deletable)).collect();
		assert_eq!(marked, [("answer", false), ("take", true), ("spoken", false)]);
		let listed = newest(&mut conn, A, Of::Studio, SWITCHED_ON, None).await.unwrap();
		assert!(listed.iter().all(|kept| kept.deletable), "with answers on too, every studio note is");
	}

	/// Keeps `model`'s ending of `subject`'s note `id`, as the transcriber
	/// would: `text` when it heard something.
	async fn ending(pool: &SqlitePool, (subject, id): (&str, &str), model: &str, outcome: &str, duration_ms: Option<i64>, text: Option<&str>, at: &str) {
		let language = text.map(|_| "en");
		sqlx::query(
			"INSERT INTO voice_transcript (subject_id, note_id, model, engine, attempt, outcome, language, language_p, duration_ms, text, took_ms, heard_at)
			 VALUES (?1, ?2, ?3, 'whisper.cpp', 1, ?4, ?5, ?6, ?7, ?8, 5, ?9)",
		)
		.bind(subject)
		.bind(id)
		.bind(model)
		.bind(outcome)
		.bind(language)
		.bind(language.map(|_| 0.9))
		.bind(duration_ms)
		.bind(text)
		.bind(at)
		.execute(pool)
		.await
		.unwrap();
	}

	#[test]
	fn what_was_heard_is_cut_at_the_last_space_within_the_limit() {
		assert_eq!(first_words("  a short note \n"), ("a short note".to_owned(), false), "trimmed, not cut");
		let exact = "x".repeat(HEARD_CHARS);
		assert_eq!(first_words(&exact), (exact.clone(), false), "exactly the limit is whole");

		let long = "the quick brown fox jumps over the lazy dog and keeps on running until it reaches the river bank";
		let (start, cut) = first_words(long);
		assert!(cut);
		assert_eq!(start, "the quick brown fox jumps over the lazy dog and keeps on running until it");
		assert!(long.starts_with(&start) && start.chars().count() <= HEARD_CHARS);

		let at_a_space = String::from(&"y".repeat(HEARD_CHARS)) + " and more";
		assert_eq!(first_words(&at_a_space), ("y".repeat(HEARD_CHARS), true), "a space right at the limit keeps the whole word");

		let no_space = "가".repeat(HEARD_CHARS + 5);
		assert_eq!(first_words(&no_space), ("가".repeat(HEARD_CHARS), true), "no space: cut at the limit, on a character");
	}

	/// Each note says how long it ran and its first words from the ending
	/// `GET /voice-notes/:id/heard` answers: words over silence, then the
	/// newest; nothing while unheard, failed or silent, and never another
	/// subject's.
	#[tokio::test]
	async fn each_note_says_how_long_it_ran_and_what_home_heard_first() {
		let pool = pool().await;
		let long = "I want to remember why the second take of the opening shot felt slower than the first one did";
		for (id, at) in [
			("heard", "2026-10-01T10:05:00Z"),
			("silent", "2026-10-01T10:04:00Z"),
			("failed", "2026-10-01T10:03:00Z"),
			("waiting", "2026-10-01T10:02:00Z"),
			("better", "2026-10-01T10:01:00Z"),
			("blank", "2026-10-01T10:00:30Z"),
			("old", "2026-09-01T10:00:00Z"),
		] {
			note(&pool, A, id, "reflection", at, 10, "2026-10-02T00:00:00.000Z").await;
		}
		note(&pool, B, "waiting", "reflection", "2026-10-01T10:02:00Z", 10, "2026-10-02T00:00:00.000Z").await;
		ending(&pool, (A, "heard"), "small", "heard", Some(4200), Some(long), "2026-10-02T01:00:00.000Z").await;
		ending(&pool, (A, "silent"), "small", "silent", Some(900), None, "2026-10-02T01:00:00.000Z").await;
		ending(&pool, (A, "failed"), "small", "unreadable", None, None, "2026-10-02T01:00:00.000Z").await;
		ending(&pool, (B, "waiting"), "small", "heard", Some(1), Some("B's words"), "2026-10-02T01:00:00.000Z").await;
		// Heard by one model, later found silent by another, then heard by a
		// third: the newest that heard words is the one answered.
		ending(&pool, (A, "better"), "small", "heard", Some(3000), Some("first try"), "2026-10-02T01:00:00.000Z").await;
		ending(&pool, (A, "better"), "medium", "silent", Some(3001), None, "2026-10-03T01:00:00.000Z").await;
		ending(&pool, (A, "better"), "large", "heard", Some(3002), Some("  better words "), "2026-10-04T01:00:00.000Z").await;
		ending(&pool, (A, "blank"), "small", "heard", Some(5), Some("   "), "2026-10-02T01:00:00.000Z").await;
		ending(&pool, (A, "old"), "small", "heard", Some(10), Some("asked for by name"), "2026-10-02T01:00:00.000Z").await;
		sqlx::query("INSERT INTO voice_note (subject_id, note_id, of_kind, of_ref, recorded_at, content_type, size, sha256, stored_at) WITH RECURSIVE n(value) AS (SELECT 1 UNION ALL SELECT value + 1 FROM n WHERE value < ?2) SELECT ?1, 'n' || value, 'reflection', 'ref', '2026-09-15T10:00:00Z', 'audio/webm', 1, 's', 'now' FROM n")
			.bind(A)
			.bind(NEWEST - 5)
			.execute(&pool)
			.await
			.unwrap();
		let mut conn = pool.acquire().await.unwrap();

		let listed = newest(&mut conn, A, Of::Reflection, &[], Some("old")).await.unwrap();
		let said: Vec<(&str, Option<i64>, Option<&str>, bool)> = listed
			.iter()
			.filter(|kept| !kept.id.starts_with('n'))
			.map(|kept| (kept.id.as_str(), kept.duration_ms, kept.heard.as_deref(), kept.heard_cut))
			.collect();
		assert_eq!(
			said,
			[
				(
					"heard",
					Some(4200),
					Some("I want to remember why the second take of the opening shot felt slower than the"),
					true
				),
				("silent", Some(900), None, false),
				("failed", None, None, false),
				("waiting", None, None, false),
				("better", Some(3002), Some("better words"), false),
				("blank", Some(5), None, false),
				("old", Some(10), Some("asked for by name"), false),
			]
		);
		assert_eq!(
			listed.last().map(|kept| kept.id.as_str()),
			Some("old"),
			"the note asked for, past the cap, carries its words too"
		);
		assert!(listed.iter().all(|kept| kept.plays_back), "every kind plays back from home");
		for kept in &listed {
			match heard(&mut conn, A, &kept.id).await.unwrap() {
				Hearing::Heard(transcript) if !transcript.text.trim().is_empty() => {
					assert_eq!(kept.heard, Some(first_words(&transcript.text).0), "{}: the words /heard answers", kept.id);
				}
				_ => assert_eq!(kept.heard, None, "{}", kept.id),
			}
		}
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

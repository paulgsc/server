//! Home's take library: every Studio take the owner keeps, with its words.
//!
//! The story is paulgsc/some-ui#2590 (ST32), on the owner's decision 8 of
//! 2026-11-04 on paulgsc/some-ui#2248: takes are a library on home, each
//! with an ID, transcribed, kept and played from home, searchable, and
//! attached to a scene by its ID.
//!
//! A take is a note said for [`Of::Studio`]: a scene's take
//! (`<project>:<shot>:<take>`), an idea's spoken answer
//! (`idea:<idea>:<question>`), or a take recorded with no scene at all. The
//! library is all of them; the `ref` says which, as the phone named it.
//!
//! - **Reads only, per subject.** One statement over `voice_note`,
//!   `voice_note_set_aside` and `voice_transcript`, filtered on the subject.
//!   It reads no audio: playing a take is `GET /voice-notes/:id/audio`'s
//!   (`crate::audio_head`, `crate::audio_body`).
//! - **The words are the ones `GET /voice-notes/:id/heard` answers**
//!   ([`crate::transcript::heard`]): the newest transcript that heard
//!   something, by `heard_at` then model, or else the newest silent ending as
//!   empty text, so the library and a take's own read never disagree.
//! - **A search by words, for now.** [`Search`] keeps a take whose words hold
//!   every term asked, compared in Unicode lowercase, so Korean and English
//!   both match. Search by meaning is the index of
//!   docs/take-library.md, "Where the meaning index lives", which is not
//!   built yet; [`By`] says which search answered, so the phone never says
//!   meaning when it searched words.
//! - **Cost.** Each read walks the subject's takes once (every `studio`
//!   note, and its chosen words by an indexed lookup per take), then keeps
//!   at most [`NEWEST`]. That grows
//!   with how many takes the owner keeps, one owner's library, never with
//!   other subjects' or other kinds' notes, and it is only ever asked by the
//!   owner's own phone, one read per screen opened or search said.
//! - **Its one caller** is `file_host`'s `handlers::voice`
//!   (`GET /api/v1/voice-notes/takes`).

use crate::{Of, VoiceError};
use serde::Serialize;
use sqlx::SqliteConnection;

/// The most takes [`library`] answers.
pub const NEWEST: usize = 50;

/// The longest search, in characters.
pub const MAX_SEARCH_CHARS: usize = 200;

/// A search of the library, as [`Search::parse`] reads one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Search {
	/// Each term asked, in Unicode lowercase; never empty.
	terms: Vec<String>,
}

impl Search {
	/// `asked` as a search: its whitespace-separated terms, in lowercase.
	///
	/// `None` for a search with no term, and for one longer than
	/// [`MAX_SEARCH_CHARS`].
	#[must_use]
	pub fn parse(asked: &str) -> Option<Self> {
		if asked.chars().count() > MAX_SEARCH_CHARS {
			return None;
		}
		let terms: Vec<String> = asked.split_whitespace().map(str::to_lowercase).collect();
		(!terms.is_empty()).then_some(Self { terms })
	}

	/// Whether `words` hold every term of the search.
	fn finds(&self, words: &str) -> bool {
		let words = words.to_lowercase();
		self.terms.iter().all(|term| words.contains(term.as_str()))
	}
}

/// How the library chose the takes it answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum By {
	/// No search: the newest takes.
	Newest,
	/// A search kept the takes whose words hold every term.
	Words,
}

/// What home heard in a take, as `GET /voice-notes/:id/heard` says it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Heard {
	/// What was heard; empty when home heard the take and found it silent.
	pub text: String,
	/// Whisper's code for the language detected; none for a silent take.
	pub language: Option<String>,
	/// The model file it was heard with.
	pub model: String,
	/// When that hearing ended, ISO-8601 UTC.
	pub heard_at: String,
}

/// One take, as the library lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Take {
	/// The phone's id for the take, the one a scene names it by.
	pub id: String,
	/// What the take was said for, as the phone named it: a scene's take,
	/// an idea's answer, or a take with no scene.
	#[serde(rename = "ref")]
	pub of_ref: String,
	/// When the phone recorded it, ISO-8601 UTC.
	pub recorded_at: String,
	/// When it reached home, ISO-8601 UTC.
	pub stored_at: String,
	/// Its audio's bytes.
	pub size: i64,
	/// When the owner set it aside, if they did; it is kept either way.
	pub set_aside_at: Option<String>,
	/// What home heard in it; null while home has not heard it yet.
	pub heard: Option<Heard>,
}

/// The library's answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Library {
	/// The takes chosen, newest recorded first, at most [`NEWEST`].
	pub takes: Vec<Take>,
	/// How many takes home holds for the subject.
	pub total: usize,
	/// With a search, how many takes it found, which may be more than it
	/// lists; null without one.
	pub matched: Option<usize>,
	/// How the takes were chosen.
	pub by: By,
}

/// `subject`'s take library, or the part of it `search` finds.
///
/// Newest recorded first (then newest to reach home, then by id), at most
/// [`NEWEST`].
///
/// # Errors
/// [`VoiceError::Unreadable`] for a heard transcript with no text, which this
/// crate never writes, and [`VoiceError::Storage`] for any `sqlx` failure.
pub async fn library(conn: &mut SqliteConnection, subject: &str, search: Option<&Search>) -> Result<Library, VoiceError> {
	let studio = Of::Studio.as_str();
	// The chosen hearing is `transcript::heard`'s order: a heard ending before
	// a silent one, then the newest, then the model, so both reads agree. It
	// is looked up per take by the transcripts' primary key, so no other
	// kind's transcript is read.
	let rows = sqlx::query!(
		r#"SELECT note.note_id AS "id!: String", note.of_ref AS "of_ref!: String",
		          note.recorded_at AS "recorded_at!: String", note.stored_at AS "stored_at!: String",
		          note.size AS "size!: i64", aside.set_aside_at AS "set_aside_at?: String",
		          heard.outcome AS "outcome?: String", heard.text AS "text?: String",
		          heard.language AS "language?: String", heard.model AS "model?: String",
		          heard.heard_at AS "heard_at?: String"
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
		   ORDER BY note.recorded_at DESC, note.stored_at DESC, note.note_id DESC"#,
		subject,
		studio
	)
	.fetch_all(&mut *conn)
	.await?;
	let total = rows.len();
	let mut found = 0;
	let mut takes = Vec::new();
	for row in rows {
		let heard = match (row.outcome, row.model, row.heard_at) {
			(Some(outcome), Some(model), Some(heard_at)) => Some(Heard {
				text: if outcome == "heard" { row.text.ok_or(VoiceError::Unreadable)? } else { String::new() },
				language: row.language,
				model,
				heard_at,
			}),
			_ => None,
		};
		if let Some(search) = search {
			if !heard.as_ref().is_some_and(|heard| search.finds(&heard.text)) {
				continue;
			}
			found += 1;
		}
		if takes.len() < NEWEST {
			takes.push(Take {
				id: row.id,
				of_ref: row.of_ref,
				recorded_at: row.recorded_at,
				stored_at: row.stored_at,
				size: row.size,
				set_aside_at: row.set_aside_at,
				heard,
			});
		}
	}
	Ok(Library {
		takes,
		total,
		matched: search.map(|_| found),
		by: if search.is_some() { By::Words } else { By::Newest },
	})
}

#[cfg(test)]
mod tests {
	use super::{library, By, Heard, Search, MAX_SEARCH_CHARS, NEWEST};
	use crate::set_aside::set_aside;
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

	/// Stores `subject`'s note `id`, said for `of` with `of_ref`, recorded at
	/// `at`, as `put_note` would.
	async fn note(pool: &SqlitePool, subject: &str, id: &str, of: &str, of_ref: &str, at: &str) {
		sqlx::query(
			"INSERT INTO voice_note (subject_id, note_id, of_kind, of_ref, recorded_at, content_type, size, sha256, stored_at)
			 VALUES (?1, ?2, ?3, ?4, ?5, 'audio/webm', 10, 's', '2026-11-01T00:00:00.000Z')",
		)
		.bind(subject)
		.bind(id)
		.bind(of)
		.bind(of_ref)
		.bind(at)
		.execute(pool)
		.await
		.unwrap();
	}

	/// Stores a hearing of `subject`'s note `id` by `model`, ending at `at`:
	/// `text` heard, or silence for `None`.
	async fn hearing(pool: &SqlitePool, subject: &str, id: &str, model: &str, text: Option<&str>, at: &str) {
		let (outcome, language, p) = match text {
			Some(_) => ("heard", Some("en"), Some(0.9)),
			None => ("silent", None, None),
		};
		sqlx::query(
			"INSERT INTO voice_transcript
			   (subject_id, note_id, model, engine, attempt, outcome, language, language_p, duration_ms, text, took_ms, heard_at)
			 VALUES (?1, ?2, ?3, 'whisper.cpp', 1, ?4, ?5, ?6, 1000, ?7, 5, ?8)",
		)
		.bind(subject)
		.bind(id)
		.bind(model)
		.bind(outcome)
		.bind(language)
		.bind(p)
		.bind(text)
		.bind(at)
		.execute(pool)
		.await
		.unwrap();
	}

	#[test]
	fn a_search_is_its_terms_in_lowercase_and_has_one_at_least() {
		assert_eq!(
			Search::parse("  The BRAND  intro "),
			Some(Search {
				terms: vec!["the".into(), "brand".into(), "intro".into()]
			})
		);
		assert_eq!(Search::parse(" \t "), None, "no term is no search");
		assert_eq!(Search::parse(""), None);
		assert!(Search::parse(&"a".repeat(MAX_SEARCH_CHARS)).is_some());
		assert_eq!(Search::parse(&"a".repeat(MAX_SEARCH_CHARS + 1)), None, "too long");
		let search = Search::parse("브랜드 intro").unwrap();
		assert!(search.finds("오늘은 브랜드 INTRO를 이야기해요"), "Korean and English, any case");
		assert!(!search.finds("오늘은 브랜드만"), "every term must be there");
	}

	#[tokio::test]
	async fn the_library_is_every_studio_take_of_the_subject_newest_first_with_its_words() {
		let pool = pool().await;
		note(&pool, A, "scene", "studio", "ep04:s3:2", "2026-11-01T10:00:00Z").await;
		note(&pool, A, "answer", "studio", "idea:i1:why", "2026-11-02T10:00:00Z").await;
		note(&pool, A, "loose", "studio", "library:t1", "2026-11-03T10:00:00Z").await;
		note(&pool, A, "reflection", "reflection", "r", "2026-11-04T10:00:00Z").await;
		note(&pool, B, "theirs", "studio", "ep01:s1:1", "2026-11-05T10:00:00Z").await;
		hearing(&pool, A, "scene", "small", Some("the brand intro"), "2026-11-01T10:05:00Z").await;
		hearing(&pool, A, "answer", "small", None, "2026-11-02T10:05:00Z").await;
		let mut conn = pool.acquire().await.unwrap();
		set_aside(&mut conn, A, "answer", "2026-11-02T11:00:00.000Z", "2026-11-02T11:00:01.000Z").await.unwrap();

		let all = library(&mut conn, A, None).await.unwrap();
		let listed: Vec<(&str, &str)> = all.takes.iter().map(|take| (take.id.as_str(), take.of_ref.as_str())).collect();
		assert_eq!(
			listed,
			[("loose", "library:t1"), ("answer", "idea:i1:why"), ("scene", "ep04:s3:2")],
			"no other kind, no other subject"
		);
		assert_eq!((all.total, all.matched, all.by), (3, None, By::Newest));
		assert_eq!(all.takes[0].heard, None, "not heard yet");
		assert_eq!(all.takes[1].heard.as_ref().map(|heard| heard.text.as_str()), Some(""), "heard, and silent");
		assert_eq!(all.takes[1].set_aside_at.as_deref(), Some("2026-11-02T11:00:00.000Z"), "set aside, and still listed");
		assert_eq!(
			all.takes[2].heard,
			Some(Heard {
				text: "the brand intro".to_owned(),
				language: Some("en".to_owned()),
				model: "small".to_owned(),
				heard_at: "2026-11-01T10:05:00Z".to_owned(),
			})
		);
		assert!(library(&mut conn, "subject-c", None).await.unwrap().takes.is_empty());
	}

	#[tokio::test]
	async fn the_words_are_the_ones_the_takes_own_read_answers() {
		let pool = pool().await;
		note(&pool, A, "t", "studio", "ep04:s1:1", "2026-11-01T10:00:00Z").await;
		hearing(&pool, A, "t", "small", Some("an older hearing"), "2026-11-01T10:05:00Z").await;
		hearing(&pool, A, "t", "medium", Some("the newer hearing"), "2026-11-01T11:00:00Z").await;
		hearing(&pool, A, "t", "large", None, "2026-11-01T12:00:00Z").await;
		let mut conn = pool.acquire().await.unwrap();

		let words = library(&mut conn, A, None).await.unwrap().takes[0].heard.clone().unwrap();
		let crate::transcript::Hearing::Heard(own) = crate::transcript::heard(&mut conn, A, "t").await.unwrap() else {
			panic!("heard");
		};
		assert_eq!(words.text, "the newer hearing", "a later silence does not hide what was heard");
		assert_eq!((words.text, words.model, words.heard_at), (own.text, own.model, own.heard_at));
	}

	#[tokio::test]
	async fn a_search_keeps_the_takes_whose_words_hold_every_term_and_counts_them_all() {
		let pool = pool().await;
		for i in 0..=NEWEST {
			let id = String::from("t") + &i.to_string();
			let at = String::from("2026-10-01T10:") + &two(i) + ":00Z";
			note(&pool, A, &id, "studio", "ep:s:1", &at).await;
			hearing(&pool, A, &id, "small", Some("we talk about the Brand"), "2026-10-02T00:00:00Z").await;
		}
		note(&pool, A, "other", "studio", "ep:s:2", "2026-12-01T10:00:00Z").await;
		hearing(&pool, A, "other", "small", Some("the intro only"), "2026-12-02T00:00:00Z").await;
		note(&pool, A, "waiting", "studio", "ep:s:3", "2026-12-03T10:00:00Z").await;
		let mut conn = pool.acquire().await.unwrap();

		let found = library(&mut conn, A, Search::parse("brand TALK").as_ref()).await.unwrap();
		assert_eq!(found.by, By::Words);
		assert_eq!(found.total, NEWEST + 3, "every take home holds");
		assert_eq!(found.matched, Some(NEWEST + 1), "every take found, past the ones listed");
		assert_eq!(found.takes.len(), NEWEST);
		assert_eq!(found.takes[0].id, "t50", "the newest found first");
		assert!(
			found.takes.iter().all(|take| take.id.starts_with('t')),
			"neither the take without the terms nor the one not yet heard"
		);

		let none = library(&mut conn, A, Search::parse("nowhere").as_ref()).await.unwrap();
		assert_eq!((none.takes.len(), none.matched), (0, Some(0)));
	}

	/// `i` as two digits, without `format!` (banned repo-wide).
	fn two(i: usize) -> String {
		let digits = i.to_string();
		if digits.len() < 2 {
			String::from("0") + &digits
		} else {
			digits
		}
	}
}

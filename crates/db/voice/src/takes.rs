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
//!   `voice_note_set_aside`, `voice_transcript`, and `studio_take_move` and
//!   `idea_answer` for where each take is on the conveyor
//!   ([`crate::conveyor`]), filtered on the subject.
//!   It reads no audio: playing a take is `GET /voice-notes/:id/audio`'s
//!   (`crate::audio_head`, `crate::audio_body`).
//! - **The words are the ones `GET /voice-notes/:id/heard` answers**
//!   ([`crate::transcript::heard`]): the newest transcript that heard
//!   something, by `heard_at` then model, or else the newest silent ending as
//!   empty text, so the library and a take's own read never disagree.
//! - **A search by meaning, or by words.** With the asked words' vector
//!   ([`Search::with_meaning`]), a search ranks the takes by the cosine of
//!   that vector with each take's in the meaning index ([`crate::meaning`]),
//!   best first, and finds those it points the same way as (a cosine above
//!   zero). It does so only when every take home heard words in has a
//!   vector under that model: off, or with a take not indexed yet, or with
//!   asked words the model knows nothing of, [`Search`] keeps a take whose
//!   words hold every term asked, compared in Unicode lowercase, so Korean
//!   and English both match. [`By`] says which search answered, so the phone
//!   never says meaning when it searched words. No vector is answered.
//! - **Cost.** One statement walks the subject's note descriptions (no
//!   audio), keeps the `studio` ones, and reads each one's chosen words by
//!   the transcripts' key; a search by meaning reads the subject's vectors
//!   under one model too, 1 KiB a take; then at most [`NEWEST`] are kept. It reads no
//!   other kind's transcript, and only the owner's own phone asks it
//!   (docs/take-library.md, "Cost").
//! - **Its one caller** is `file_host`'s `handlers::voice`
//!   (`GET /api/v1/voice-notes/takes`).

use crate::{conveyor::Station, meaning, transcript::Transcript, Of, VoiceError};
use serde::Serialize;
use sqlx::SqliteConnection;
use std::collections::HashMap;

/// The most takes [`library`] answers.
pub const NEWEST: usize = 50;

/// The longest search, in characters.
pub const MAX_SEARCH_CHARS: usize = 200;

/// A search of the library, as [`Search::parse`] reads one.
#[derive(Debug, Clone, PartialEq)]
pub struct Search {
	/// Each term asked, in Unicode lowercase; never empty.
	terms: Vec<String>,
	/// The asked words' vector and the model that made it, to search by
	/// meaning; none to search by words only.
	meaning: Option<Asked>,
}

/// The asked words' vector, by the model whose index it is compared with.
#[derive(Debug, Clone, PartialEq)]
struct Asked {
	model: String,
	vector: Vec<f32>,
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
		(!terms.is_empty()).then_some(Self { terms, meaning: None })
	}

	/// The same search, by meaning where it can be: `vector` is the asked
	/// words' vector by `model` ([`meaning::Embed`]).
	#[must_use]
	pub fn with_meaning(self, model: &str, vector: Vec<f32>) -> Self {
		Self {
			meaning: Some(Asked { model: model.to_owned(), vector }),
			..self
		}
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
	/// A search ranked the takes by how near their meaning is to the asked
	/// words', best first.
	Meaning,
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
	pub heard: Option<Transcript>,
	/// Where the take is on the conveyor ([`crate::conveyor`]); null for an
	/// idea's answer or a take let go of, which are not on it.
	pub station: Option<Station>,
	/// When its newest move put it there; null in the oven, or for an
	/// idea's answer or a take let go of.
	pub station_at: Option<String>,
	/// Its newest move's number, 0 in the oven, what a move of it names;
	/// null off the conveyor.
	pub station_seq: Option<i64>,
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
/// [`NEWEST`]; by meaning, best first, then in that order.
///
/// # Errors
/// [`VoiceError::Unreadable`] for a heard transcript with no text or a
/// stored vector of the wrong size, which this crate never writes, and
/// [`VoiceError::Storage`] for any `sqlx` failure.
pub async fn library(conn: &mut SqliteConnection, subject: &str, search: Option<&Search>) -> Result<Library, VoiceError> {
	let studio = Of::Studio.as_str();
	// `transcript::heard`'s ORDER BY, kept equal by
	// `the_words_are_the_ones_the_takes_own_read_answers`.
	let rows = sqlx::query!(
		r#"SELECT note.note_id AS "id!: String", note.of_ref AS "of_ref!: String",
		          note.recorded_at AS "recorded_at!: String", note.stored_at AS "stored_at!: String",
		          note.size AS "size!: i64", aside.set_aside_at AS "set_aside_at?: String",
		          heard.outcome AS "outcome?: String", heard.text AS "text?: String",
		          heard.language AS "language?: String", heard.model AS "model?: String",
		          heard.heard_at AS "heard_at?: String",
		          moved.to_station AS "to_station?: String", moved.moved_at AS "moved_at?: String", moved.seq AS "seq?: i64",
		          EXISTS (SELECT 1 FROM idea_answer AS answer WHERE answer.subject_id = note.subject_id AND answer.note_id = note.note_id) AS "answers!: bool"
		   FROM voice_note AS note
		   LEFT JOIN voice_note_set_aside AS aside ON aside.subject_id = note.subject_id AND aside.note_id = note.note_id
		   LEFT JOIN studio_take_move AS moved ON moved.subject_id = note.subject_id AND moved.note_id = note.note_id
		     AND moved.seq = (SELECT MAX(later.seq) FROM studio_take_move AS later WHERE later.subject_id = note.subject_id AND later.note_id = note.note_id)
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
	let mut all = Vec::with_capacity(total);
	for row in rows {
		let heard = match (row.outcome, row.model, row.heard_at) {
			(Some(outcome), Some(model), Some(heard_at)) => Some((
				outcome == "heard",
				Transcript {
					text: if outcome == "heard" { row.text.ok_or(VoiceError::Unreadable)? } else { String::new() },
					language: row.language,
					model,
					heard_at,
				},
			)),
			_ => None,
		};
		let (words, heard) = heard.map_or((false, None), |(words, heard)| (words, Some(heard)));
		// An idea's answer and a take let go of are not on the conveyor, as
		// `conveyor` holds them.
		let on_conveyor = crate::delete::deletable(studio, &row.of_ref, row.answers) == Some(crate::delete::Deletable::StudioTake) && row.set_aside_at.is_none();
		let (station, station_at, station_seq) = on_the_conveyor(on_conveyor, row.to_station.as_deref(), row.moved_at, row.seq)?;
		all.push((
			words,
			Take {
				id: row.id,
				of_ref: row.of_ref,
				recorded_at: row.recorded_at,
				stored_at: row.stored_at,
				size: row.size,
				set_aside_at: row.set_aside_at,
				heard,
				station,
				station_at,
				station_seq,
			},
		));
	}
	let Some(search) = search else {
		all.truncate(NEWEST);
		return Ok(Library {
			takes: all.into_iter().map(|(_, take)| take).collect(),
			total,
			matched: None,
			by: By::Newest,
		});
	};
	if let Some(ranked) = by_meaning(conn, subject, search, &all).await? {
		let matched = ranked.len();
		let mut takes: Vec<Option<Take>> = all.into_iter().map(|(_, take)| Some(take)).collect();
		let takes = ranked.into_iter().take(NEWEST).filter_map(|at| takes.get_mut(at).and_then(Option::take)).collect();
		return Ok(Library {
			takes,
			total,
			matched: Some(matched),
			by: By::Meaning,
		});
	}
	let found: Vec<Take> = all
		.into_iter()
		.filter(|(_, take)| take.heard.as_ref().is_some_and(|heard| search.finds(&heard.text)))
		.map(|(_, take)| take)
		.collect();
	let matched = found.len();
	Ok(Library {
		takes: found.into_iter().take(NEWEST).collect(),
		total,
		matched: Some(matched),
		by: By::Words,
	})
}

/// A take's station, when its newest move put it there and that move's
/// number, each `None` off the conveyor.
type OnConveyor = (Option<Station>, Option<String>, Option<i64>);

/// A take's station, when its newest move put it there and that move's
/// number, if it is `on` the conveyor: in the oven, at none and 0 with no
/// move (`to` the newest move's station).
fn on_the_conveyor(on: bool, to: Option<&str>, at: Option<String>, seq: Option<i64>) -> Result<OnConveyor, VoiceError> {
	if !on {
		return Ok((None, None, None));
	}
	let station = to.map_or(Ok(Station::InTheOven), |to| Station::parse(to).ok_or(VoiceError::Unreadable))?;
	Ok((Some(station), at, Some(seq.unwrap_or(0))))
}

/// Where in `all` (each take, and whether a model heard words in it) the
/// takes `search` finds by meaning are, best first, ties newest first; or
/// `None` when it cannot search by meaning: no asked vector, one with no
/// direction (the model knew none of the asked words), no take heard yet,
/// or a take heard and not indexed under the asked model yet.
async fn by_meaning(conn: &mut SqliteConnection, subject: &str, search: &Search, all: &[(bool, Take)]) -> Result<Option<Vec<usize>>, VoiceError> {
	let Some(asked) = search.meaning.as_ref() else {
		return Ok(None);
	};
	if meaning::norm(&asked.vector) <= 0.0 || !all.iter().any(|(words, _)| *words) {
		return Ok(None);
	}
	let vectors: HashMap<String, Vec<f32>> = meaning::vectors(conn, subject, &asked.model).await?.into_iter().collect();
	if all.iter().any(|(words, take)| *words && !vectors.contains_key(&take.id)) {
		return Ok(None);
	}
	let mut found: Vec<(usize, f32)> = all
		.iter()
		.enumerate()
		.filter_map(|(at, (_, take))| vectors.get(&take.id).map(|vector| (at, meaning::cosine(&asked.vector, vector))))
		.filter(|(_, cosine)| *cosine > 0.0)
		.collect();
	// Stable, so takes as near as each other keep the newest first.
	found.sort_by(|(_, a), (_, b)| b.total_cmp(a));
	Ok(Some(found.into_iter().map(|(at, _)| at).collect()))
}

#[cfg(test)]
mod tests {
	use super::{library, By, Search, MAX_SEARCH_CHARS, NEWEST};
	use crate::transcript::Transcript;
	use crate::{conveyor::Station, set_aside::set_aside};
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
				terms: vec!["the".into(), "brand".into(), "intro".into()],
				meaning: None,
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
		let stations: Vec<_> = all.takes.iter().map(|take| (take.station, take.station_at.as_deref(), take.station_seq)).collect();
		assert_eq!(
			stations,
			[(Some(Station::InTheOven), None, Some(0)), (None, None, None), (Some(Station::InTheOven), None, Some(0))],
			"an idea's answer is not on the conveyor"
		);
		sqlx::query(
			"INSERT INTO studio_take_move (subject_id, note_id, seq, from_station, to_station, moved_at, labels, comment, issue, stored_at)
			 VALUES (?1, 'scene', 1, 'in-the-oven', 'cooking', '2026-11-03T00:00:00.000Z', NULL, NULL, NULL, 'now'),
			        (?1, 'scene', 2, 'cooking', 'canned', '2026-11-04T00:00:00.000Z', NULL, NULL, NULL, 'now')",
		)
		.bind(A)
		.execute(&mut *conn)
		.await
		.unwrap();
		let scene = library(&mut conn, A, None).await.unwrap().takes.remove(2);
		assert_eq!(
			(scene.station, scene.station_at.as_deref(), scene.station_seq),
			(Some(Station::Canned), Some("2026-11-04T00:00:00.000Z"), Some(2)),
			"its newest move"
		);

		// A scene's take that moved, then was named as an idea's answer, and
		// one that moved, then was let go of, are off the conveyor too.
		sqlx::query(
			"INSERT INTO idea_revision (subject_id, idea_id, revision, created_at, sent_at, title, topic, format, keywords, questions, stored_at)
			 VALUES (?1, 'i', 1, 'then', 'then', 't', 'Studio', 'short', '[]', 1, 'now')",
		)
		.bind(A)
		.execute(&mut *conn)
		.await
		.unwrap();
		sqlx::query("INSERT INTO idea_answer (subject_id, idea_id, revision, question, note_id, typed) VALUES (?1, 'i', 1, 'what', 'scene', NULL)")
			.bind(A)
			.execute(&mut *conn)
			.await
			.unwrap();
		sqlx::query(
			"INSERT INTO studio_take_move (subject_id, note_id, seq, from_station, to_station, moved_at, labels, comment, issue, stored_at)
			 VALUES (?1, 'loose', 1, 'in-the-oven', 'cooking', '2026-11-04T00:00:00.000Z', NULL, NULL, NULL, 'now')",
		)
		.bind(A)
		.execute(&mut *conn)
		.await
		.unwrap();
		set_aside(&mut conn, A, "loose", "2026-11-05T00:00:00.000Z", "2026-11-05T00:00:01.000Z").await.unwrap();
		let off: Vec<_> = library(&mut conn, A, None)
			.await
			.unwrap()
			.takes
			.into_iter()
			.map(|take| (take.station, take.station_at, take.station_seq))
			.collect();
		assert_eq!(
			off,
			[(None, None, None), (None, None, None), (None, None, None)],
			"named as an answer, or let go of, after it moved"
		);
		assert_eq!(
			all.takes[2].heard,
			Some(Transcript {
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

	/// ST32's search by meaning: with every heard take indexed, the asked
	/// words' vector ranks the takes best first and finds those that point
	/// its way, whatever words they used; a take not heard, or heard as
	/// silence, is never found.
	#[tokio::test]
	async fn a_search_by_meaning_ranks_the_indexed_takes_best_first() {
		use crate::meaning::{index, tests::Words, Embed as _};
		let pool = pool().await;
		note(&pool, A, "logo", "studio", "ep:s:1", "2026-11-01T10:00:00Z").await;
		note(&pool, A, "both", "studio", "ep:s:2", "2026-11-02T10:00:00Z").await;
		note(&pool, A, "korean", "studio", "ep:s:3", "2026-11-03T10:00:00Z").await;
		note(&pool, A, "brand", "studio", "ep:s:4", "2026-11-04T10:00:00Z").await;
		note(&pool, A, "quiet", "studio", "ep:s:5", "2026-11-05T10:00:00Z").await;
		note(&pool, A, "waiting", "studio", "ep:s:6", "2026-11-06T10:00:00Z").await;
		note(&pool, B, "theirs", "studio", "ep:s:1", "2026-11-07T10:00:00Z").await;
		hearing(&pool, A, "logo", "small", Some("our new logo"), "2026-11-08T00:00:00Z").await;
		hearing(&pool, A, "both", "small", Some("the brand in 한국어"), "2026-11-08T00:00:00Z").await;
		hearing(&pool, A, "korean", "small", Some("korean practice"), "2026-11-08T00:00:00Z").await;
		hearing(&pool, A, "brand", "small", Some("the brand"), "2026-11-08T00:00:00Z").await;
		hearing(&pool, A, "quiet", "small", None, "2026-11-08T00:00:00Z").await;
		hearing(&pool, B, "theirs", "small", Some("the brand"), "2026-11-08T00:00:00Z").await;
		let mut conn = pool.acquire().await.unwrap();
		for (subject, id) in [(A, "logo"), (A, "both"), (A, "korean"), (A, "brand"), (A, "quiet"), (B, "theirs")] {
			index(&mut conn, subject, id, &Words).await.unwrap();
		}

		let asked = |words: &str| Search::parse(words).unwrap().with_meaning(Words.model(), Words.embed(words));
		let found = library(&mut conn, A, Some(&asked("branding"))).await;
		let found = found.unwrap();
		assert_eq!(found.by, By::Words, "the model knows no asked word: words answer");

		let found = library(&mut conn, A, Some(&asked("brand"))).await.unwrap();
		let ids: Vec<&str> = found.takes.iter().map(|take| take.id.as_str()).collect();
		assert_eq!(ids, ["brand", "logo", "both"], "best first, the nearest newest first, and never the Korean take");
		assert_eq!((found.total, found.matched, found.by), (6, Some(3), By::Meaning));
		assert_eq!(
			found.takes[1].heard.as_ref().map(|heard| heard.text.as_str()),
			Some("our new logo"),
			"found without the word"
		);

		let other_model = Search::parse("brand").unwrap().with_meaning("another-model", Words.embed("brand"));
		assert_eq!(library(&mut conn, A, Some(&other_model)).await.unwrap().by, By::Words, "no index under that model");
		assert_eq!(library(&mut conn, A, Search::parse("brand").as_ref()).await.unwrap().by, By::Words, "meaning off");
		assert_eq!(library(&mut conn, B, Some(&asked("korean"))).await.unwrap().matched, Some(0), "B's own takes only");
	}

	/// A take heard and not indexed yet would be missed by meaning, so the
	/// whole search is by words until the catch-up has indexed it.
	#[tokio::test]
	async fn a_heard_take_not_indexed_yet_makes_the_search_one_by_words() {
		use crate::meaning::{index, tests::Words, Embed as _};
		let pool = pool().await;
		note(&pool, A, "indexed", "studio", "ep:s:1", "2026-11-01T10:00:00Z").await;
		note(&pool, A, "later", "studio", "ep:s:2", "2026-11-02T10:00:00Z").await;
		hearing(&pool, A, "indexed", "small", Some("our logo"), "2026-11-08T00:00:00Z").await;
		hearing(&pool, A, "later", "small", Some("the brand"), "2026-11-08T00:00:00Z").await;
		let mut conn = pool.acquire().await.unwrap();
		index(&mut conn, A, "indexed", &Words).await.unwrap();

		let asked = Search::parse("brand").unwrap().with_meaning(Words.model(), Words.embed("brand"));
		let found = library(&mut conn, A, Some(&asked)).await.unwrap();
		assert_eq!((found.by, found.matched, found.takes[0].id.as_str()), (By::Words, Some(1), "later"));

		index(&mut conn, A, "later", &Words).await.unwrap();
		let found = library(&mut conn, A, Some(&asked)).await.unwrap();
		assert_eq!((found.by, found.matched), (By::Meaning, Some(2)));
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

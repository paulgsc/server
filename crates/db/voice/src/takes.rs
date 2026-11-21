//! Home's take library: every take the owner keeps, of every kind, with its
//! words, in one shape.
//!
//! The story is paulgsc/some-ui#2590 (ST32), on the owner's decision 8 of
//! 2026-11-04 on paulgsc/some-ui#2248: takes are a library on home, each
//! with an ID, transcribed, kept and played from home, searchable, and
//! attached to a scene by its ID. paulgsc/some-ui#3219 (TAKES20) widened it
//! to every kind on the owner's words of 2026-11-18: "takes have one common
//! home that is an ergonomic place to filter through, sort through, and
//! find".
//!
//! A take is a voice note of any [`Of`]. A read that asks no kind ([`Ask`])
//! is the read the phone before TAKES20 makes: the [`Of::Studio`] notes alone
//! (a scene's take, `<project>:<shot>:<take>`, an idea's spoken answer,
//! `idea:<idea>:<question>`, or a take with no scene), and no take deleted
//! for good, since that phone needs every take's `ref`. A read that asks
//! kinds lists those, and the takes of them deleted for good by their
//! receipts (`voice_note_deletion`, which keeps no `ref` and nothing said).
//! Each take says what it is about by its kind ([`crate::about`]).
//!
//! - **Reads only, per subject.** One statement over `voice_note`,
//!   `voice_note_set_aside`, `voice_transcript`, `voice_note_deletion`,
//!   `studio_take_on_drive`, and `studio_take_move` and `idea_answer` for
//!   where each take is on the conveyor ([`crate::conveyor`]), filtered on
//!   the subject; a second over `voice_note_deletion` when kinds are asked.
//!   For the page answered only, a few reads by key per take: its verdict,
//!   a game bite's row and newest match, and an interview week's send. It
//!   reads no audio: playing a take is `GET /voice-notes/:id/audio`'s
//!   (`crate::audio_head`, `crate::audio_body`). It writes nothing.
//! - **The words are the ones `GET /voice-notes/:id/heard` answers**
//!   ([`crate::transcript::heard`]): the newest transcript that heard
//!   something, by `heard_at` then model, or else the newest silent ending as
//!   empty text, so the library and a take's own read never disagree.
//! - **A search by meaning, or by words.** With the asked words' vector
//!   ([`Search::with_meaning`]), a search ranks the takes by the cosine of
//!   that vector with each take's in the meaning index ([`crate::meaning`]),
//!   best first, and finds those it points the same way as (a cosine above
//!   zero). It does so only when every take listed that home heard words in
//!   has a vector under that model: off, with a take not indexed yet, with a
//!   kind the index does not hold (it holds Studio takes alone), or with
//!   asked words the model knows nothing of, [`Search`] keeps a take whose
//!   words hold every term asked, compared in Unicode lowercase, so Korean
//!   and English both match. [`By`] says which search answered, so the phone
//!   never says meaning when it searched words. No vector is answered.
//! - **Filtered, ordered and paged in memory.** The kinds and the dates
//!   bound the statement; the `about` key, the order and the page are applied
//!   to its rows. A page starts after the take `after` names in the same
//!   read, so a take recorded meanwhile never shifts it.
//! - **Cost.** One statement walks the subject's note descriptions of the
//!   kinds asked (no audio) and reads each one's chosen words by the
//!   transcripts' key; a search by meaning reads the subject's vectors under
//!   one model too, 1 KiB a take; then at most [`NEWEST`] are answered, and
//!   only the owner's own phone asks it (docs/take-library.md, "Cost").
//! - **Its one caller** is `file_host`'s `handlers::voice`
//!   (`GET /api/v1/voice-notes/takes`).

use crate::{
	about::{self, About},
	conveyor::Station,
	meaning,
	transcript::Transcript,
	Of, Problem, VoiceError,
};
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

/// The order the library lists takes in, when no search ranks them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sort {
	/// Newest recorded first, then newest to reach home, then by id.
	Newest,
	/// The reverse.
	Oldest,
}

/// What a read of the library asks, as [`Ask::parse`] reads it.
#[derive(Debug, Clone, PartialEq)]
pub struct Ask {
	/// The kinds asked; `None` for today's read, Studio takes alone and no
	/// deleted take (the phone before TAKES20 asks no kind).
	kinds: Option<Vec<Of>>,
	/// Recorded at or after, as ISO-8601 UTC to the millisecond.
	from: Option<String>,
	/// Recorded before.
	to: Option<String>,
	/// An [`About::key`] the takes must have.
	about: Option<String>,
	/// The words asked, as said.
	asked: Option<String>,
	search: Option<Search>,
	sort: Sort,
	/// The take the page starts after, as [`Cursor::read`] reads `after`.
	after: Option<Cursor>,
}

/// Where a page starts: after the take a page ended on, named by when it was
/// recorded, when it reached home and its id, so the next page continues in
/// order even if that take has since left the list (deleted, or no longer
/// found). Said as `<recordedAt>|<storedAt>|<id>`, which [`Library::next`]
/// answers; a client passes it back as it is.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Cursor {
	recorded_at: String,
	stored_at: String,
	id: String,
}

impl Cursor {
	/// The cursor of the page that ends on `take`.
	fn of(take: &Take) -> String {
		take.recorded_at.clone() + "|" + &take.stored_at + "|" + &take.id
	}

	/// `after` as a cursor, if it is one: three parts, the last a note's id.
	fn read(after: &str) -> Option<Self> {
		let mut parts = after.splitn(3, '|');
		let (Some(recorded_at), Some(stored_at), Some(id)) = (parts.next(), parts.next(), parts.next()) else {
			return None;
		};
		(!recorded_at.is_empty() && !stored_at.is_empty() && crate::is_note_id(id)).then(|| Self {
			recorded_at: recorded_at.to_owned(),
			stored_at: stored_at.to_owned(),
			id: id.to_owned(),
		})
	}

	/// The order key [`order`] compares.
	const fn key(&self) -> (&str, &str, &str) {
		(self.recorded_at.as_str(), self.stored_at.as_str(), self.id.as_str())
	}
}

/// The longest `about` key asked, in characters.
const MAX_ABOUT_CHARS: usize = 256;

/// The longest cursor asked, in characters: two instants and a note's id.
const MAX_CURSOR_CHARS: usize = 256;

impl Default for Ask {
	fn default() -> Self {
		Self {
			kinds: None,
			from: None,
			to: None,
			about: None,
			asked: None,
			search: None,
			sort: Sort::Newest,
			after: None,
		}
	}
}

impl Ask {
	/// A read of the query's `pairs`: `kind` (an [`Of`]'s name, or `all`, as
	/// often as asked), and at most once each `from` and `to` (RFC 3339
	/// instants), `about` (a key), `q` (words, [`Search::parse`]), `sort`
	/// (`newest` or `oldest`) and `after` (a take's id). Other keys are
	/// ignored.
	///
	/// # Errors
	/// Each field refused, and why.
	pub fn parse(pairs: &[(String, String)]) -> Result<Self, Vec<Problem>> {
		let said = |name: &str| pairs.iter().filter(|(key, _)| key == name).map(|(_, value)| value.as_str()).collect::<Vec<_>>();
		let once = |name: &str| match said(name).as_slice() {
			[] => Ok(None),
			[value] => Ok(Some((*value).to_owned())),
			_ => Err(()),
		};
		let mut problems: Vec<Problem> = Vec::new();
		let mut ask = Self::default();

		let kinds = said("kind");
		if !kinds.is_empty() {
			let mut chosen: Vec<Of> = Vec::new();
			for kind in kinds {
				match (kind, Of::named(kind)) {
					("all", _) => chosen.extend(Of::ALL),
					(_, Some(of)) => chosen.push(of),
					(_, None) => {
						problems.push(("kind", "a kind of voice note, or all"));
						break;
					}
				}
			}
			chosen.sort_by_key(|of| of.as_str());
			chosen.dedup();
			ask.kinds = Some(chosen);
		}
		for (name, slot) in [("from", &mut ask.from), ("to", &mut ask.to)] {
			match once(name).map(|at| at.map(|at| crate::instant(&at))) {
				Ok(None) => {}
				Ok(Some(Some(at))) => *slot = Some(at),
				_ => problems.push((name, "an RFC 3339 instant, said once")),
			}
		}
		match once("about") {
			Ok(None) => {}
			Ok(Some(key)) if !key.is_empty() && key.chars().count() <= MAX_ABOUT_CHARS => ask.about = Some(key),
			_ => problems.push(("about", "a take's about key, said once")),
		}
		match once("q") {
			Ok(None) => {}
			Ok(Some(words)) => match Search::parse(&words) {
				Some(search) => {
					ask.search = Some(search);
					ask.asked = Some(words);
				}
				None => problems.push(("q", "words to search for, at most 200 characters, said once")),
			},
			Err(()) => problems.push(("q", "words to search for, at most 200 characters, said once")),
		}
		match once("sort").as_ref().map(Option::as_deref) {
			Ok(None | Some("newest")) => {}
			Ok(Some("oldest")) => ask.sort = Sort::Oldest,
			_ => problems.push(("sort", "newest or oldest, said once")),
		}
		match once("after") {
			Ok(None) => {}
			Ok(Some(after)) if after.chars().count() <= MAX_CURSOR_CHARS && Cursor::read(&after).is_some() => ask.after = Cursor::read(&after),
			_ => problems.push(("after", "a page's `next`, said once")),
		}
		if problems.is_empty() {
			Ok(ask)
		} else {
			Err(problems)
		}
	}

	/// The words asked, if any, as said: what a search by meaning embeds.
	#[must_use]
	pub fn asked(&self) -> Option<&str> {
		self.asked.as_deref()
	}

	/// The same read, its search by meaning where it can be
	/// ([`Search::with_meaning`]).
	#[must_use]
	pub fn with_meaning(self, model: &str, vector: Vec<f32>) -> Self {
		Self {
			search: self.search.map(|search| search.with_meaning(model, vector)),
			..self
		}
	}

	/// The same read, with `search`.
	#[must_use]
	pub fn searching(self, search: Option<Search>) -> Self {
		Self { search, ..self }
	}

	/// The kinds asked, comma-separated, as the statement filters on them.
	fn kinds_listed(&self) -> String {
		self.kinds.as_deref().unwrap_or(&[Of::Studio]).iter().map(|of| of.as_str()).collect::<Vec<_>>().join(",")
	}
}

/// What became of a take.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Status {
	/// Home holds it, recording and all.
	Kept,
	/// The owner let it go on the phone; home keeps it.
	SetAside,
	/// Its recording was let go, with a receipt; the note and its words stay.
	RecordingLetGo,
	/// Deleted for good from My data; only its receipt is left.
	Deleted,
}

/// What a delete left: when, and what went and stayed, in fixed words.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Gone {
	pub deleted_at: String,
	pub went: Vec<String>,
	pub stayed: Vec<String>,
}

/// Where a take is kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Kept {
	/// Home holds its recording.
	pub home: bool,
	/// The build-journal session listed it in Drive.
	pub drive: bool,
}

/// One take, as the library lists it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Take {
	/// The phone's id for the take, the one a scene names it by.
	pub id: String,
	/// What kind of note it is.
	pub kind: Of,
	/// What the take was said for, as the phone named it; null only for a
	/// take deleted for good, whose receipt keeps no `ref`, and listed only
	/// when the read asks a kind.
	#[serde(rename = "ref")]
	pub of_ref: Option<String>,
	/// When the phone recorded it, ISO-8601 UTC.
	pub recorded_at: String,
	/// When it reached home, ISO-8601 UTC.
	pub stored_at: String,
	/// Its audio's bytes.
	pub size: i64,
	/// How long it runs, in milliseconds, as home heard it; null until then.
	pub duration_ms: Option<i64>,
	/// When the owner set it aside, if they did; it is kept either way.
	pub set_aside_at: Option<String>,
	/// What home heard in it; null while home has not heard it yet.
	pub heard: Option<Transcript>,
	/// What home made of what it heard (HEARD1,
	/// [`crate::verdict::of_note`]); null while home has not judged it.
	pub verdict: Option<crate::verdict::Verdict>,
	/// What became of it.
	pub status: Status,
	/// Its receipt, when its recording or the whole of it was deleted.
	pub receipt: Option<Gone>,
	/// Where it is kept.
	pub kept: Kept,
	/// Whether it is on the conveyor ([`crate::conveyor`]): a Studio take of
	/// a scene or none, never an idea's answer, not let go of. Every other
	/// kind is not on it.
	pub on_conveyor: bool,
	/// Where the take is on the conveyor; null off it.
	pub station: Option<Station>,
	/// When its newest move put it there; null in the oven, or off it.
	pub station_at: Option<String>,
	/// Its newest move's number, 0 in the oven, what a move of it names;
	/// null off the conveyor.
	pub station_seq: Option<i64>,
	/// What it is about, by its kind ([`crate::about`]); null for a take
	/// deleted for good.
	pub about: Option<About>,
}

/// The library's answer.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Library {
	/// The takes chosen, at most [`NEWEST`].
	pub takes: Vec<Take>,
	/// How many takes the read's kinds, dates and `about` hold.
	pub total: usize,
	/// With a search, how many takes it found, which may be more than it
	/// lists; null without one.
	pub matched: Option<usize>,
	/// How the takes were chosen.
	pub by: By,
	/// The cursor to ask `after` for the next page, passed back as it is;
	/// null on the last.
	pub next: Option<String>,
}

/// `subject`'s take library, or the part of it `ask` asks for; `None` when
/// a search ranked by meaning no longer finds the take `ask`'s `after` ended
/// on.
///
/// In `ask`'s order (newest recorded first by default), or with a search by
/// meaning, best first and then in that order; at most [`NEWEST`].
///
/// # Errors
/// [`VoiceError::Unreadable`] for a heard transcript with no text, a stored
/// vector of the wrong size or another stored row this crate never writes,
/// and [`VoiceError::Storage`] for any `sqlx` failure.
pub async fn library(conn: &mut SqliteConnection, subject: &str, ask: &Ask) -> Result<Option<Library>, VoiceError> {
	let Some(mut library) = chosen(conn, subject, ask).await? else {
		return Ok(None);
	};
	// Each take answered carries its verdict (HEARD1) and what only another
	// table says of it: a few reads by key each, of at most [`NEWEST`].
	let mut labels = std::collections::BTreeMap::new();
	let mut sends = about::Sends::new();
	for take in &mut library.takes {
		if take.status == Status::Deleted {
			continue;
		}
		take.verdict = crate::verdict::of_note(conn, subject, &take.id).await?;
		if let Some(about) = take.about.as_mut() {
			about::with_game(conn, subject, &take.id, about, &mut labels).await?;
			about::with_rung(conn, subject, &take.id, about, &mut sends).await?;
		}
	}
	Ok(Some(library))
}

/// The order key of a take: when recorded, then reached home, then its id.
const fn order(take: &Take) -> (&str, &str, &str) {
	(take.recorded_at.as_str(), take.stored_at.as_str(), take.id.as_str())
}

/// [`library`]'s takes, without their verdicts or what other tables say.
#[expect(clippy::too_many_lines, reason = "one read, its rows, and its page, in order")]
async fn chosen(conn: &mut SqliteConnection, subject: &str, ask: &Ask) -> Result<Option<Library>, VoiceError> {
	let kinds = ask.kinds_listed();
	let (from, to) = (ask.from.as_deref(), ask.to.as_deref());
	// `transcript::heard`'s ORDER BY, kept equal by
	// `the_words_are_the_ones_the_takes_own_read_answers`.
	let rows = sqlx::query!(
		r#"SELECT note.note_id AS "id!: String", note.of_kind AS "of_kind!: String", note.of_ref AS "of_ref!: String",
		          note.recorded_at AS "recorded_at!: String", note.stored_at AS "stored_at!: String",
		          note.size AS "size!: i64", aside.set_aside_at AS "set_aside_at?: String",
		          heard.outcome AS "outcome?: String", heard.text AS "text?: String",
		          heard.language AS "language?: String", heard.model AS "model?: String",
		          heard.heard_at AS "heard_at?: String", heard.duration_ms AS "duration_ms?: i64",
		          moved.to_station AS "to_station?: String", moved.moved_at AS "moved_at?: String", moved.seq AS "seq?: i64",
		          answer.idea_id AS "idea?: String", answer.question AS "question?: String",
		          gone.deleted_at AS "let_go_at?: String", gone.went AS "went?: String", gone.stayed AS "stayed?: String",
		          EXISTS (SELECT 1 FROM studio_take_on_drive AS drive WHERE drive.subject_id = note.subject_id AND drive.note_id = note.note_id) AS "on_drive!: bool"
		   FROM voice_note AS note
		   LEFT JOIN voice_note_set_aside AS aside ON aside.subject_id = note.subject_id AND aside.note_id = note.note_id
		   LEFT JOIN studio_take_move AS moved ON moved.subject_id = note.subject_id AND moved.note_id = note.note_id
		     AND moved.seq = (SELECT MAX(later.seq) FROM studio_take_move AS later WHERE later.subject_id = note.subject_id AND later.note_id = note.note_id)
		   LEFT JOIN idea_answer AS answer ON answer.rowid = (
		     SELECT first.rowid FROM idea_answer AS first WHERE first.subject_id = note.subject_id AND first.note_id = note.note_id
		     ORDER BY first.revision, first.rowid LIMIT 1
		   )
		   LEFT JOIN voice_note_deletion AS gone ON gone.subject_id = note.subject_id AND gone.note_id = note.note_id
		   LEFT JOIN voice_transcript AS heard ON heard.rowid = (
		     SELECT hearing.rowid FROM voice_transcript AS hearing
		     WHERE hearing.subject_id = note.subject_id AND hearing.note_id = note.note_id
		       AND hearing.outcome IN ('heard', 'silent')
		     ORDER BY hearing.outcome = 'heard' DESC, hearing.heard_at DESC, hearing.model DESC
		     LIMIT 1
		   )
		   WHERE note.subject_id = ?1
		     AND instr(',' || ?2 || ',', ',' || note.of_kind || ',') > 0
		     AND (?3 IS NULL OR note.recorded_at >= ?3)
		     AND (?4 IS NULL OR note.recorded_at < ?4)"#,
		subject,
		kinds,
		from,
		to
	)
	.fetch_all(&mut *conn)
	.await?;
	let list = |text: &str| serde_json::from_str::<Vec<String>>(text).map_err(|_| VoiceError::Unreadable);
	let mut all: Vec<(bool, Take)> = Vec::with_capacity(rows.len());
	for row in rows {
		let of = Of::named(&row.of_kind).ok_or(VoiceError::Unreadable)?;
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
		let answers = row.idea.is_some();
		let idea = row.idea.zip(row.question);
		// An idea's answer and a take let go of are not on the conveyor, as
		// `conveyor` holds them; no other kind is ever on it.
		let on_conveyor = crate::delete::deletable(of.as_str(), &row.of_ref, answers) == Some(crate::delete::Deletable::StudioTake) && row.set_aside_at.is_none();
		let (station, station_at, station_seq) = on_the_conveyor(on_conveyor, row.to_station.as_deref(), row.moved_at, row.seq)?;
		let receipt = match (row.let_go_at, row.went, row.stayed) {
			(Some(deleted_at), Some(went), Some(stayed)) => Some(Gone {
				deleted_at,
				went: list(&went)?,
				stayed: list(&stayed)?,
			}),
			_ => None,
		};
		let status = match (&receipt, &row.set_aside_at) {
			(Some(_), _) => Status::RecordingLetGo,
			(None, Some(_)) => Status::SetAside,
			(None, None) => Status::Kept,
		};
		all.push((
			words,
			Take {
				about: Some(about::about(of, &row.of_ref, idea)),
				id: row.id,
				kind: of,
				of_ref: Some(row.of_ref),
				recorded_at: row.recorded_at,
				stored_at: row.stored_at,
				size: row.size,
				duration_ms: row.duration_ms,
				set_aside_at: row.set_aside_at,
				heard,
				verdict: None,
				kept: Kept {
					home: receipt.is_none(),
					drive: row.on_drive,
				},
				status,
				receipt,
				on_conveyor,
				station,
				station_at,
				station_seq,
			},
		));
	}
	// A read that asks a kind lists the takes deleted for good too, by their
	// receipts; today's read never does, as its phone needs every `ref`.
	if ask.kinds.is_some() {
		let gone = sqlx::query!(
			r#"SELECT gone.note_id AS "id!: String", gone.of_kind AS "of_kind!: String", gone.recorded_at AS "recorded_at!: String",
			          gone.size AS "size!: i64", gone.stored_at AS "stored_at!: String", gone.deleted_at AS "deleted_at!: String",
			          gone.went AS "went!: String", gone.stayed AS "stayed!: String",
			          EXISTS (SELECT 1 FROM studio_take_on_drive AS drive WHERE drive.subject_id = gone.subject_id AND drive.note_id = gone.note_id) AS "on_drive!: bool"
			   FROM voice_note_deletion AS gone
			   WHERE gone.subject_id = ?1
			     AND NOT EXISTS (SELECT 1 FROM voice_note AS note WHERE note.subject_id = gone.subject_id AND note.note_id = gone.note_id)
			     AND instr(',' || ?2 || ',', ',' || gone.of_kind || ',') > 0
			     AND (?3 IS NULL OR gone.recorded_at >= ?3)
			     AND (?4 IS NULL OR gone.recorded_at < ?4)"#,
			subject,
			kinds,
			from,
			to
		)
		.fetch_all(&mut *conn)
		.await?;
		for row in gone {
			all.push((
				false,
				Take {
					id: row.id,
					kind: Of::named(&row.of_kind).ok_or(VoiceError::Unreadable)?,
					of_ref: None,
					recorded_at: row.recorded_at,
					stored_at: row.stored_at,
					size: row.size,
					duration_ms: None,
					set_aside_at: None,
					heard: None,
					verdict: None,
					status: Status::Deleted,
					receipt: Some(Gone {
						deleted_at: row.deleted_at,
						went: list(&row.went)?,
						stayed: list(&row.stayed)?,
					}),
					// A delete keeps the record that Drive holds the take
					// (`delete::remove`, its receipt's `drive copy`).
					kept: Kept { home: false, drive: row.on_drive },
					on_conveyor: false,
					station: None,
					station_at: None,
					station_seq: None,
					about: None,
				},
			));
		}
	}
	if let Some(key) = ask.about.as_deref() {
		all.retain(|(_, take)| take.about.as_ref().is_some_and(|about| about.key == key));
	}
	match ask.sort {
		Sort::Newest => all.sort_by(|(_, a), (_, b)| order(b).cmp(&order(a))),
		Sort::Oldest => all.sort_by(|(_, a), (_, b)| order(a).cmp(&order(b))),
	}
	let total = all.len();
	let (listed, matched, by) = match ask.search.as_ref() {
		None => (all.into_iter().map(|(_, take)| take).collect::<Vec<_>>(), None, By::Newest),
		Some(search) => {
			if let Some(ranked) = by_meaning(conn, subject, search, &all).await? {
				let mut takes: Vec<Option<Take>> = all.into_iter().map(|(_, take)| Some(take)).collect();
				let found: Vec<Take> = ranked.into_iter().filter_map(|at| takes.get_mut(at).and_then(Option::take)).collect();
				let matched = found.len();
				(found, Some(matched), By::Meaning)
			} else {
				let found: Vec<Take> = all
					.into_iter()
					.filter(|(_, take)| take.heard.as_ref().is_some_and(|heard| search.finds(&heard.text)))
					.map(|(_, take)| take)
					.collect();
				let matched = found.len();
				(found, Some(matched), By::Words)
			}
		}
	};
	// Sorted, a page starts at the first take past the cursor's place, whether
	// or not that take is still listed. Ranked by meaning, there is no place
	// to compare: it starts after the cursor's take, which must still be found.
	let start = match (ask.after.as_ref(), by) {
		(None, _) => 0,
		(Some(after), By::Meaning) => match listed.iter().position(|take| take.id == after.id) {
			Some(at) => at + 1,
			None => return Ok(None),
		},
		(Some(after), By::Newest | By::Words) => listed
			.iter()
			.position(|take| match ask.sort {
				Sort::Newest => order(take) < after.key(),
				Sort::Oldest => order(take) > after.key(),
			})
			.unwrap_or(listed.len()),
	};
	let rest = listed.len().saturating_sub(start);
	let takes: Vec<Take> = listed.into_iter().skip(start).take(NEWEST).collect();
	let next = (rest > NEWEST).then(|| takes.last().map(Cursor::of)).flatten();
	Ok(Some(Library { takes, total, matched, by, next }))
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
	use super::{library, Ask, By, Search, MAX_SEARCH_CHARS, NEWEST};
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

		let all = library(&mut conn, A, &Ask::default()).await.unwrap().unwrap();
		let listed: Vec<(&str, &str)> = all.takes.iter().map(|take| (take.id.as_str(), take.of_ref.as_deref().unwrap())).collect();
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
		let scene = library(&mut conn, A, &Ask::default()).await.unwrap().unwrap().takes.remove(2);
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
		let off: Vec<_> = library(&mut conn, A, &Ask::default())
			.await
			.unwrap()
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
		assert!(library(&mut conn, "subject-c", &Ask::default()).await.unwrap().unwrap().takes.is_empty());
	}

	#[tokio::test]
	async fn the_words_are_the_ones_the_takes_own_read_answers() {
		let pool = pool().await;
		note(&pool, A, "t", "studio", "ep04:s1:1", "2026-11-01T10:00:00Z").await;
		hearing(&pool, A, "t", "small", Some("an older hearing"), "2026-11-01T10:05:00Z").await;
		hearing(&pool, A, "t", "medium", Some("the newer hearing"), "2026-11-01T11:00:00Z").await;
		hearing(&pool, A, "t", "large", None, "2026-11-01T12:00:00Z").await;
		let mut conn = pool.acquire().await.unwrap();

		let words = library(&mut conn, A, &Ask::default()).await.unwrap().unwrap().takes[0].heard.clone().unwrap();
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

		let found = library(&mut conn, A, &Ask::default().searching(Search::parse("brand TALK"))).await.unwrap().unwrap();
		assert_eq!(found.by, By::Words);
		assert_eq!(found.total, NEWEST + 3, "every take home holds");
		assert_eq!(found.matched, Some(NEWEST + 1), "every take found, past the ones listed");
		assert_eq!(found.takes.len(), NEWEST);
		assert_eq!(found.takes[0].id, "t50", "the newest found first");
		assert!(
			found.takes.iter().all(|take| take.id.starts_with('t')),
			"neither the take without the terms nor the one not yet heard"
		);

		let none = library(&mut conn, A, &Ask::default().searching(Search::parse("nowhere"))).await.unwrap().unwrap();
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
		let found = library(&mut conn, A, &Ask::default().searching(Some(asked("branding")))).await.unwrap().unwrap();
		assert_eq!(found.by, By::Words, "the model knows no asked word: words answer");

		let found = library(&mut conn, A, &Ask::default().searching(Some(asked("brand")))).await.unwrap().unwrap();
		let ids: Vec<&str> = found.takes.iter().map(|take| take.id.as_str()).collect();
		assert_eq!(ids, ["brand", "logo", "both"], "best first, the nearest newest first, and never the Korean take");
		assert_eq!((found.total, found.matched, found.by), (6, Some(3), By::Meaning));
		assert_eq!(
			found.takes[1].heard.as_ref().map(|heard| heard.text.as_str()),
			Some("our new logo"),
			"found without the word"
		);

		let other_model = Search::parse("brand").unwrap().with_meaning("another-model", Words.embed("brand"));
		assert_eq!(
			library(&mut conn, A, &Ask::default().searching(Some(other_model))).await.unwrap().unwrap().by,
			By::Words,
			"no index under that model"
		);
		assert_eq!(
			library(&mut conn, A, &Ask::default().searching(Search::parse("brand"))).await.unwrap().unwrap().by,
			By::Words,
			"meaning off"
		);
		assert_eq!(
			library(&mut conn, B, &Ask::default().searching(Some(asked("korean")))).await.unwrap().unwrap().matched,
			Some(0),
			"B's own takes only"
		);
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
		let found = library(&mut conn, A, &Ask::default().searching(Some(asked.clone()))).await.unwrap().unwrap();
		assert_eq!((found.by, found.matched, found.takes[0].id.as_str()), (By::Words, Some(1), "later"));

		index(&mut conn, A, "later", &Words).await.unwrap();
		let found = library(&mut conn, A, &Ask::default().searching(Some(asked))).await.unwrap().unwrap();
		assert_eq!((found.by, found.matched), (By::Meaning, Some(2)));
	}

	/// What a read with `pairs` answers for A.
	async fn read(conn: &mut sqlx::SqliteConnection, pairs: &[(&str, &str)]) -> Option<super::Library> {
		let pairs: Vec<(String, String)> = pairs.iter().map(|(key, value)| ((*key).to_owned(), (*value).to_owned())).collect();
		library(conn, A, &Ask::parse(&pairs).unwrap()).await.unwrap()
	}

	/// TAKES20: every kind of take in one list, each saying what it is about,
	/// what became of it and where it is kept; the read before TAKES20 still
	/// answers Studio takes alone.
	#[tokio::test]
	#[expect(clippy::too_many_lines, reason = "one fixture of every kind, then each thing a read of it says")]
	async fn every_kind_is_listed_once_a_kind_is_asked() {
		use super::Status;
		use crate::about::Detail;
		let pool = pool().await;
		let kinds = [
			("reflection", "session-7f2a"),
			("jobs", "pick-1"),
			("scene", "stamp-1"),
			("game", "401772"),
			("studio", "ep04:s1:1"),
			("soundbite", "direct"),
			("lens", "line-3"),
			("leetype", "f1:s2"),
			("rust-round", "2026-w47:ENG-12"),
			("interview", "2026-w47:about-you"),
		];
		for (at, (of, of_ref)) in kinds.iter().enumerate() {
			let when = String::from("2026-11-10T10:") + &two(at) + ":00.000Z";
			note(&pool, A, of, of, of_ref, &when).await;
		}
		note(&pool, B, "theirs", "game", "401772", "2026-11-10T11:00:00.000Z").await;
		hearing(&pool, A, "game", "small", Some("what a catch on third and long"), "2026-11-10T12:00:00Z").await;
		sqlx::query(
			"INSERT INTO game_bite (subject_id, note_id, scenario_id, how, offer_seq, live_strip, context_line, watching, started_at, clock_offset_ms, mode, catalog_version, unit, unit_side)
			 VALUES (?1, 'game', 'in.third-long', 'offered', 3, 'Q3 4:12 3rd & 9', NULL, 1, '2026-11-10T10:02:58.000Z', NULL, 'standard', 1, 'defense', 'CHI')",
		)
		.bind(A)
		.execute(&pool)
		.await
		.unwrap();
		sqlx::query(
			"INSERT INTO game_feed_body (sha256, body) VALUES (?1, CAST(?2 AS BLOB));
			 INSERT INTO game_feed_response (kind, game_id, url, fetched_at, status, sha256, size)
			 VALUES ('summary', '401772', 'u', 'then', 200, ?1, length(CAST(?2 AS BLOB)))",
		)
		.bind("0".repeat(64))
		.bind(r#"{"header":{"competitions":[{"competitors":[{"homeAway":"home","team":{"abbreviation":"CHI"}},{"homeAway":"away","team":{"abbreviation":"NE"}}]}]}}"#)
		.execute(&pool)
		.await
		.unwrap();
		sqlx::query(
			"INSERT INTO game_feed_play (game_id, espn_play_id, period, clock, play_type, text, response_id, supersedes, seen_at)
			 VALUES ('401772', 'p9', 3, '4:20', 'Pass Reception', 'an early text', 1, NULL, 'then')",
		)
		.execute(&pool)
		.await
		.unwrap();
		sqlx::query(
			"INSERT INTO game_feed_play (game_id, espn_play_id, period, clock, play_type, text, response_id, supersedes, seen_at)
			 VALUES ('401772', 'p9', 3, '4:12', 'Pass Reception', 'a catch for 12 yards', 1, 1, 'later')",
		)
		.execute(&pool)
		.await
		.unwrap();
		sqlx::query(
			"INSERT INTO bite_match (subject_id, note_id, scenario_id, play_id, method, gap_seconds, engine_version, matched_at)
			 VALUES (?1, 'game', 'in.third-long', 'p9', 'nearest', 8, 1, 'then')",
		)
		.bind(A)
		.execute(&pool)
		.await
		.unwrap();
		sqlx::query(
			"INSERT INTO interview_week_draft (subject_id, week_id, sent_at, body, received_at)
			 VALUES (?1, '2026-w47', 'then', ?2, 'then')",
		)
		.bind(A)
		.bind(r#"{"week":"2026-w47","at":"2026-11-17T10:00:00.000Z","answers":[{"question":"about-you","take":"interview","recordedAt":"2026-11-10T10:09:00.000Z","rung":"cues"}],"reads":[],"passed":[],"passedReads":[]}"#)
		.execute(&pool)
		.await
		.unwrap();
		// One deleted for good, one whose recording was let go: each a receipt.
		sqlx::query(
			"INSERT INTO voice_note_deletion (subject_id, note_id, of_kind, recorded_at, size, stored_at, deleted_at, went, stayed)
			 VALUES (?1, 'gone', 'studio', '2026-11-09T10:00:00.000Z', 10, 'then', '2026-11-12T00:00:00.000Z', '[\"recording\",\"words\"]', '[]'),
			        (?1, 'reflection', 'reflection', '2026-11-10T10:00:00.000Z', 10, 'then', '2026-11-12T00:00:00.000Z', '[\"recording\"]', '[\"note\"]')",
		)
		.bind(A)
		.execute(&pool)
		.await
		.unwrap();
		sqlx::query(
			"INSERT INTO studio_take_on_drive (subject_id, note_id, listed_at, list_name, kept_at)
			 VALUES (?1, 'gone', 'then', 'drive-takes-20261101T000000Z.json', 'now')",
		)
		.bind(A)
		.execute(&pool)
		.await
		.unwrap();
		let mut conn = pool.acquire().await.unwrap();

		let today = read(&mut conn, &[]).await.unwrap();
		let ids: Vec<&str> = today.takes.iter().map(|take| take.id.as_str()).collect();
		assert_eq!((ids.as_slice(), today.total), (["studio"].as_slice(), 1), "today's read: Studio alone, no receipt");

		let all = read(&mut conn, &[("kind", "all")]).await.unwrap();
		let listed: Vec<(&str, &str)> = all.takes.iter().map(|take| (take.id.as_str(), take.kind.as_str())).collect();
		let mut expected: Vec<(&str, &str)> = kinds.iter().rev().map(|(of, _)| (*of, *of)).collect();
		expected.push(("gone", "studio"));
		assert_eq!(listed, expected, "every kind, newest first, the deleted take last");
		assert_eq!((all.total, all.next.as_deref(), all.by), (11, None, By::Newest));
		assert!(
			all.takes.iter().all(|take| take.id == "gone" || take.about.is_some()),
			"every kept take says what it is about"
		);

		let gone = all.takes.last().unwrap();
		assert_eq!(
			(gone.status, gone.of_ref.as_deref(), gone.about.is_none(), gone.kept.home, gone.kept.drive),
			(Status::Deleted, None, true, false, true),
			"deleted from home, and still on Drive"
		);
		let let_go = all.takes.iter().find(|take| take.id == "reflection").unwrap();
		assert_eq!(
			(let_go.status, let_go.kept.home, let_go.of_ref.as_deref()),
			(Status::RecordingLetGo, false, Some("session-7f2a"))
		);
		assert_eq!(let_go.receipt.as_ref().unwrap().stayed, ["note"]);
		let studio = all.takes.iter().find(|take| take.id == "studio").unwrap();
		assert!(studio.on_conveyor && studio.kept.home && studio.status == Status::Kept);
		assert!(
			all.takes.iter().filter(|take| take.id != "studio").all(|take| !take.on_conveyor && take.station.is_none()),
			"only a Studio take is on the conveyor"
		);

		let game = all.takes.iter().find(|take| take.id == "game").unwrap();
		let Detail::Game(about) = &game.about.as_ref().unwrap().detail else {
			panic!("a game's about");
		};
		let bite = about.bite.as_ref().unwrap();
		assert_eq!(
			(about.game.as_str(), bite.scenario.as_deref(), bite.strip.as_str()),
			("401772", Some("in.third-long"), "Q3 4:12 3rd & 9")
		);
		let matched = about.matched.as_ref().unwrap();
		let play = matched.play.as_ref().unwrap();
		assert_eq!(
			(matched.gap_seconds, play.clock.as_deref(), play.text.as_deref()),
			(Some(8), Some("4:12"), Some("a catch for 12 yards")),
			"the play's latest version"
		);
		assert_eq!(about.half, Some(game_scenarios::Half::Second), "its play is in the 3rd");
		assert_eq!(
			(bite.unit, bite.unit_side.as_deref()),
			(Some(game_scenarios::Unit::Defense), Some("CHI")),
			"what it was about, as said"
		);
		let wire = serde_json::to_value(bite).unwrap();
		assert_eq!((&wire["unit"], &wire["unitSide"]), (&serde_json::json!("defense"), &serde_json::json!("CHI")));
		assert_eq!(game.duration_ms, Some(1000));

		let interview = all.takes.iter().find(|take| take.id == "interview").unwrap();
		let Detail::Interview { rung, .. } = &interview.about.as_ref().unwrap().detail else {
			panic!("an interview's about");
		};
		assert_eq!(*rung, Some(rust_round_repo::interview::Rung::Cues), "the rung the week's send gave it");

		let games = read(&mut conn, &[("kind", "game"), ("about", "game:401772")]).await.unwrap();
		assert_eq!((games.takes.len(), games.total), (1, 1), "one game's takes, A's alone");
		let week = read(&mut conn, &[("kind", "all"), ("about", "interview:2026-w47")]).await.unwrap();
		assert_eq!(week.takes.iter().map(|take| take.id.as_str()).collect::<Vec<_>>(), ["interview"]);
		let between = read(&mut conn, &[("kind", "all"), ("from", "2026-11-10T10:02:00Z"), ("to", "2026-11-10T10:04:00Z")])
			.await
			.unwrap();
		assert_eq!(
			between.takes.iter().map(|take| take.id.as_str()).collect::<Vec<_>>(),
			["game", "scene"],
			"from inclusive, to exclusive"
		);
		let oldest = read(&mut conn, &[("kind", "all"), ("sort", "oldest")]).await.unwrap();
		assert_eq!(oldest.takes.first().map(|take| take.id.as_str()), Some("gone"));
		let heard = read(&mut conn, &[("kind", "all"), ("q", "catch")]).await.unwrap();
		assert_eq!((heard.takes.len(), heard.matched, heard.by), (1, Some(1), By::Words), "what home heard, in any kind");
	}

	#[tokio::test]
	async fn a_read_pages_after_the_take_it_ended_on() {
		let pool = pool().await;
		for i in 0..=NEWEST {
			let id = String::from("t") + &two(i);
			let at = String::from("2026-10-01T10:") + &two(i) + ":00.000Z";
			note(&pool, A, &id, "jobs", "pick", &at).await;
		}
		let mut conn = pool.acquire().await.unwrap();
		let first = read(&mut conn, &[("kind", "jobs")]).await.unwrap();
		let next = first.next.clone().unwrap();
		assert_eq!(
			(first.takes.len(), first.total, next.as_str()),
			(NEWEST, NEWEST + 1, "2026-10-01T10:01:00.000Z|2026-11-01T00:00:00.000Z|t01")
		);
		// A take recorded since, and the page's last take gone, change nothing.
		drop(conn);
		note(&pool, A, "later", "jobs", "pick", "2026-12-01T10:00:00.000Z").await;
		sqlx::query("DELETE FROM voice_note WHERE subject_id = ?1 AND note_id = 't01'")
			.bind(A)
			.execute(&pool)
			.await
			.unwrap();
		let mut conn = pool.acquire().await.unwrap();
		let second = read(&mut conn, &[("kind", "jobs"), ("after", &next)]).await.unwrap();
		assert_eq!((second.takes.len(), second.takes[0].id.as_str(), second.next), (1, "t00", None));
		let oldest = read(&mut conn, &[("kind", "jobs"), ("sort", "oldest"), ("after", &next)]).await.unwrap();
		assert_eq!(oldest.takes.first().map(|take| take.id.as_str()), Some("t02"), "past the place, the other way");
	}

	#[test]
	fn a_read_refuses_each_field_it_cannot_read() {
		let refused = |pairs: &[(&str, &str)]| {
			let pairs: Vec<(String, String)> = pairs.iter().map(|(key, value)| ((*key).to_owned(), (*value).to_owned())).collect();
			Ask::parse(&pairs).unwrap_err().into_iter().map(|(field, _)| field).collect::<Vec<_>>()
		};
		assert_eq!(refused(&[("kind", "diary")]), ["kind"]);
		assert_eq!(refused(&[("from", "yesterday"), ("to", "soon")]), ["from", "to"]);
		assert_eq!(refused(&[("sort", "best")]), ["sort"]);
		assert_eq!(refused(&[("sort", "newest"), ("sort", "oldest")]), ["sort"]);
		assert_eq!(refused(&[("after", "a b")]), ["after"]);
		assert_eq!(refused(&[("after", "t1")]), ["after"], "a bare id is no cursor");
		assert_eq!(refused(&[("about", "")]), ["about"]);
		assert_eq!(refused(&[("q", " ")]), ["q"]);
		assert!(Ask::parse(&[("kind".into(), "all".into()), ("kind".into(), "game".into()), ("other".into(), "x".into())]).is_ok());
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

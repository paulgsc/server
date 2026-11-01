//! Home's bookkeeping for the reflections routine, put in the mailbox's
//! `index` folder after each pass (paulgsc/server#509, HF7).
//!
//! The morning routine used to work these out itself, from everything Drive
//! and the mailbox hold (paulgsc/some-ui `packages/ui/topik/ROUTINE.md`):
//! which reflections and drama prompts no scene or drama answers yet, every
//! word id the routine's decks already use (archive included), the genres
//! and levels of its latest scenes, how many of its lessons wait unopened
//! (Part D), and which voice notes it still waits on, for how long. Home
//! holds every scene, drama, written reflection, drama prompt, list of
//! opened lessons and reflection or scene voice note the routine reads, so it
//! computes them here and the routine reads one small file. The model fills
//! values; it does not do the bookkeeping (paulgsc/some-ui#2162).
//!
//! **What it is computed from** ([`mailbox_repo::reflections_inputs`]):
//! every scene and drama home kept from the mailbox (`mailbox_item`); every
//! written reflection, drama prompt and list of opened lessons the mailbox
//! took from home as home filed it and home has not archived (the copy in
//! `mailbox_outgoing` of what `written_reflection` and `drama_prompt` keep,
//! which is what the routine can read); every reflection or scene voice note
//! home holds (`voice_note`); and each of those notes' transcripts the
//! mailbox took. No new table.
//!
//! **What it leaves to the routine**, since home holds none of it: Drive's
//! own files. A reflection or prompt shared to Drive before the phone sent
//! it home, a Drive scene folder the mailbox never got (one retired to
//! `Read aloud/archive/` before the mailbox, say), and which mailbox
//! reflection a Drive reflection file already holds. The routine lists Drive
//! as before and joins those itself; the index names what home holds.
//!
//! **A view, not a record**, as the Jobs index is (the parent module, owner,
//! 2026-10-31): no table keeps it, and what home remembers is a
//! [`super::LastIndex`] in memory.
//!
//! **Deterministic, and moved by the clock only at moments it names.** The
//! index is a pure function of those rows and the moment it is computed. Only
//! three rules read the clock, each at a moment the rows fix:
//!
//! - a group of written reflections settles [`SETTLE_HOURS`] after its
//!   newest was written (`settled`);
//! - a reflection or prompt waiting on a voice note home has not heard
//!   stops waiting [`VOICE_WAIT_DAYS`] after the mailbox took it (`voice`
//!   turns from `waiting` to `missed`);
//! - a lesson stops counting as unopened [`UNOPENED_DAYS`] after home took it
//!   in, or [`UNOPENED_DAYS_BEFORE_OPENED`] while the phone has filed no list
//!   of opened lessons.
//!
//! `validUntil` is the first of those moments still to come, or `null` when
//! none is. Between two such moments the same rows give the same bytes.
//!
//! **Put only when it moves.** A pass first reads when home last took in
//! anything the index counts ([`mailbox_repo::reflections_as_of`]). When that
//! is what the last index the mailbox answered was computed from and the
//! pass is before that index's `validUntil`, nothing is computed or put.
//! Otherwise it computes and puts `index-reflections-<stamp>.json`: once each
//! time something it counts arrives, and once each time a moment it carries
//! passes. An idle pass puts none, and the index adds no listing.
//!
//! **What it carries, and what it never does.** File names, reflection and
//! session ids, bases, stamps, scene titles, word ids with their dictionary
//! forms, note ids, and transcripts' names and hashes: what the routine needs
//! to know what to read. Never a reflection's or a prompt's words (a prompt
//! is read only for the line that names its notes by id), nor a transcript's,
//! and nothing that is or points at audio (docs/identity.md invariant 19).
//!
//! **The rules are `packages/ui/topik/ROUTINE.md`'s.** Step 3 (grouping and
//! naming written reflections, and their two-hour wait), step 4 (word-id
//! reuse, genre and level), step 7 (which prompts are answered), "Voice
//! notes" (the two-day wait) and Part D (unopened lessons); a change to one
//! there is a change here. The tests port them.

use chrono::{DateTime, Duration, SecondsFormat, Utc};
use mailbox_kit::{Mailbox, MailboxTransport};
use mailbox_repo::{HeldLesson, HeldNote, HeldTranscript, ReflectionsInputs, TakenFile};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use std::collections::{BTreeMap, BTreeSet, HashMap};

use super::{index_name, newest_transcripts, put_index, stamp_of, Computed, LastIndex, INDEX_SCHEMA};
use crate::mailbox::pass::{Failure, Outcome};

/// The routine this index is for, as its name and `routine` say.
pub const REFLECTIONS: &str = "reflections";

/// The zone the routine's days are in: a written reflection's group is "the
/// day of their `at` in `America/Los_Angeles`" (`ROUTINE.md` step 3).
pub const REFLECTIONS_ZONE: chrono_tz::Tz = chrono_tz::America::Los_Angeles;

/// How long after its newest reflection was written a group waits, "since
/// more of that session may be on its way" (step 3).
pub const SETTLE_HOURS: i64 = 2;

/// How long a reflection or prompt waits for home to hear a voice note,
/// from when the mailbox took it ("Voice notes").
pub const VOICE_WAIT_DAYS: i64 = 2;

/// A lesson counts as unopened when it was put in this many days (D2).
pub const UNOPENED_DAYS: i64 = 30;

/// The same, while the phone has filed no list of opened lessons yet (D2).
pub const UNOPENED_DAYS_BEFORE_OPENED: i64 = 7;

/// How many of the latest scenes the index names: "a genre not used by the
/// last three scenes" (D3), and the newest one's level (steps 4 and D3).
pub const RECENT_SCENES: usize = 3;

// ---------------------------------------------------------------------------
// The files, as far as the index reads them. One that does not read as its
// schema is `skipped` and counts for nothing.
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct Word {
	id: String,
	lemma: String,
}

#[derive(Deserialize)]
struct Deck {
	words: Vec<Word>,
}

#[derive(Deserialize)]
struct SceneFile {
	schema: String,
	answers: String,
	title: String,
	decks: Vec<Deck>,
}

#[derive(Deserialize)]
struct DramaFile {
	schema: String,
	answers: String,
}

/// A written reflection, as far as grouping it goes: its words (`text`)
/// are never read.
#[derive(Deserialize)]
struct ReflectionFile {
	schema: String,
	session: String,
	at: String,
}

#[derive(Deserialize)]
struct PromptFile {
	schema: String,
	prompt: String,
	text: String,
}

#[derive(Deserialize)]
struct OpenedFile {
	schema: String,
	opened: Vec<String>,
}

fn scene_of(lesson: &HeldLesson) -> Option<SceneFile> {
	let scene: SceneFile = serde_json::from_slice(&lesson.body).ok()?;
	(scene.schema == "some-ui.routine-scene/1").then_some(scene)
}

fn drama_of(lesson: &HeldLesson) -> Option<DramaFile> {
	let drama: DramaFile = serde_json::from_slice(&lesson.body).ok()?;
	(drama.schema == "some-ui.routine-drama/1").then_some(drama)
}

fn reflection_of(file: &TakenFile) -> Option<(ReflectionFile, DateTime<Utc>)> {
	let reflection: ReflectionFile = serde_json::from_slice(&file.body).ok()?;
	let at = instant(&reflection.at)?;
	(reflection.schema == "some-ui.reflection/1").then_some((reflection, at))
}

fn prompt_of(file: &TakenFile) -> Option<(PromptFile, Vec<(String, String)>)> {
	let prompt: PromptFile = serde_json::from_slice(&file.body).ok()?;
	let notes = notes_named(&prompt.text)?;
	(prompt.schema == "some-ui.drama-prompt/1").then_some((prompt, notes))
}

fn opened_of(file: &TakenFile) -> Option<OpenedFile> {
	let opened: OpenedFile = serde_json::from_slice(&file.body).ok()?;
	(opened.schema == "some-ui.routine-opened/1").then_some(opened)
}

/// The notes a prompt names, `(voice file name, id)`, in the order they
/// were said: the JSON of its `<!-- notes: {...} -->` line (paulgsc/some-ui
/// topik's `nextSceneFile`), ordered by the `<n>` of each
/// `drama-<stamp>.voice-<n>.<ext>`. Empty for a prompt with no such line
/// (one with no voice notes); `None` for a line that does not read.
fn notes_named(text: &str) -> Option<Vec<(String, String)>> {
	let Some(line) = text.lines().map(str::trim).find(|line| line.starts_with("<!-- notes: ")) else {
		return Some(Vec::new());
	};
	let json = line.strip_prefix("<!-- notes: ")?.strip_suffix(" -->")?;
	let named: BTreeMap<String, String> = serde_json::from_str(json).ok()?;
	let said = |file: &str| -> u64 {
		file
			.rsplit_once(".voice-")
			.and_then(|(_, rest)| rest.split('.').next())
			.and_then(|n| n.parse().ok())
			.unwrap_or(u64::MAX)
	};
	let mut notes: Vec<(String, String)> = named.into_iter().collect();
	notes.sort_by(|(a, _), (b, _)| said(a).cmp(&said(b)).then_with(|| a.cmp(b)));
	Some(notes)
}

/// An ISO-8601 instant; `None` when it does not read.
fn instant(text: &str) -> Option<DateTime<Utc>> {
	Some(DateTime::parse_from_rfc3339(text).ok()?.with_timezone(&Utc))
}

/// `at` in the mailbox's own format, so two compare as text.
fn written(at: DateTime<Utc>) -> String {
	at.to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// The day `at` falls on in [`REFLECTIONS_ZONE`], as `YYYY-MM-DD`.
fn day_of(at: DateTime<Utc>) -> String {
	at.with_timezone(&REFLECTIONS_ZONE).format("%Y-%m-%d").to_string()
}

/// A group's base: `<day>_reflection_<session8>`, `<session8>` being the
/// session's first 8 letters, digits or `-`, as written (step 3).
fn base_of(day: &str, session: &str) -> String {
	let session8: String = session.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').take(8).collect();
	String::from(day) + "_reflection_" + &session8
}

// ---------------------------------------------------------------------------
// The index, as the mailbox takes it. Field order is the bytes' order.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct Answered {
	/// Every base a scene home holds answers (`answers`), a stock scene's
	/// included, sorted.
	scenes: Vec<String>,
	/// Every prompt a drama home holds answers (`drama-<stamp>`), sorted.
	dramas: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
struct WordUsed {
	id: String,
	lemma: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct Recent {
	name: String,
	answers: String,
	/// The scene's folder name, `TOPIK <level> · <genre> · <title>`.
	title: String,
	/// `<level>` and `<genre>` read from `title`, or `None` when it does
	/// not read so.
	level: Option<u8>,
	genre: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct Unopened {
	count: usize,
	/// Each one, as `<folder>/<name>`, sorted.
	lessons: Vec<String>,
	/// The list of opened lessons it went by, or `None` while there is none.
	opened: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct Transcript {
	name: String,
	sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct GroupNote {
	note: String,
	transcript: Option<Transcript>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct PromptNote {
	file: String,
	note: String,
	transcript: Option<Transcript>,
}

/// Whether a reflection or prompt still waits on a voice note, and until when.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct Voice {
	/// `heard` when each of its notes has a transcript (or it has none),
	/// `waiting` while one has none and its wait is on, `missed` once that
	/// wait is over: answer it from what it says in writing.
	state: &'static str,
	/// When its wait counts from: when the mailbox took it (a group's
	/// newest).
	since: String,
	/// When `waiting` turns to `missed`; `None` unless `waiting`.
	until: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct Group {
	base: String,
	session: String,
	day: String,
	/// Its reflections' names in the mailbox, in the order they were written.
	reflections: Vec<String>,
	#[serde(rename = "newestAt")]
	newest_at: String,
	/// Whether [`SETTLE_HOURS`] have passed since `newestAt`.
	settled: bool,
	notes: Vec<GroupNote>,
	voice: Voice,
}

/// A drama prompt as the index names it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct Asked {
	name: String,
	prompt: String,
	notes: Vec<PromptNote>,
	voice: Voice,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct ReflectionsIndex<'a> {
	schema: &'static str,
	routine: &'static str,
	#[serde(rename = "asOf")]
	as_of: Option<&'a str>,
	/// The first moment still to come at which this index would say
	/// something else with nothing new arriving, or `None`.
	#[serde(rename = "validUntil")]
	valid_until: Option<String>,
	skipped: Vec<String>,
	answered: Answered,
	words: Vec<WordUsed>,
	recent: Vec<Recent>,
	unopened: Unopened,
	/// The groups of written reflections no scene answers, newest first.
	reflections: Vec<Group>,
	/// The drama prompts no drama answers, oldest first.
	prompts: Vec<Asked>,
}

/// The moments still to come at which the index changes; the first is its
/// `validUntil`.
struct Moments {
	now: DateTime<Utc>,
	first: Option<DateTime<Utc>>,
}

impl Moments {
	/// Whether `at` has come; one that has not is remembered.
	fn passed(&mut self, at: DateTime<Utc>) -> bool {
		if at <= self.now {
			return true;
		}
		if self.first.is_none_or(|first| at < first) {
			self.first = Some(at);
		}
		false
	}
}

/// How a reflection or prompt stands on its notes: each one's newest
/// transcript, and its wait.
fn voice_of<'a>(mut notes: impl Iterator<Item = Option<&'a HeldTranscript>>, since: DateTime<Utc>, moments: &mut Moments) -> Voice {
	let unheard = notes.any(|transcript| transcript.is_none());
	let ends = since + Duration::days(VOICE_WAIT_DAYS);
	let state = if !unheard {
		"heard"
	} else if moments.passed(ends) {
		"missed"
	} else {
		"waiting"
	};
	Voice {
		state,
		since: written(since),
		until: (state == "waiting").then(|| written(ends)),
	}
}

fn transcript_of(held: &HeldTranscript) -> Transcript {
	Transcript {
		name: held.name.clone(),
		sha256: held.sha256.clone(),
	}
}

/// One group of written reflections as it is read: by session and day.
struct Gathering<'a> {
	session: String,
	day: String,
	/// `(at, name)` of each.
	written: Vec<(DateTime<Utc>, &'a str)>,
	/// When the mailbox took its newest.
	since: DateTime<Utc>,
}

/// The written reflections grouped by session and day (step 3), by base, and
/// the names of those that do not read.
fn gather<'a>(reflections: &'a [TakenFile], skipped: &mut Vec<String>) -> BTreeMap<String, Gathering<'a>> {
	let mut groups: BTreeMap<(String, String), Gathering<'a>> = BTreeMap::new();
	for file in reflections {
		let (Some((reflection, at)), Some(taken)) = (reflection_of(file), instant(&file.handed_at)) else {
			skipped.push(file.name.clone());
			continue;
		};
		let day = day_of(at);
		let group = groups.entry((reflection.session.clone(), day.clone())).or_insert_with(|| Gathering {
			session: reflection.session,
			day,
			written: Vec::new(),
			since: taken,
		});
		group.written.push((at, file.name.as_str()));
		group.since = group.since.max(taken);
	}
	groups
		.into_values()
		.map(|mut group| {
			group.written.sort_unstable();
			(base_of(&group.day, &group.session), group)
		})
		.collect()
}

/// The groups no scene answers, newest first, each with its notes: the
/// reflection notes of its session recorded on its day ("Voice notes").
fn waiting_groups(
	gathered: BTreeMap<String, Gathering<'_>>,
	answered: &BTreeSet<String>,
	(notes, newest): (&[HeldNote], &HashMap<&str, (u64, &HeldTranscript)>),
	moments: &mut Moments,
) -> Vec<Group> {
	let mut groups: Vec<Group> = gathered
		.into_iter()
		.filter(|(base, _)| !answered.contains(base))
		.map(|(base, group)| {
			let said: Vec<&HeldNote> = notes
				.iter()
				.filter(|note| note.of_kind == "reflection" && note.of_ref == group.session)
				.filter(|note| instant(&note.recorded_at).is_some_and(|at| day_of(at) == group.day))
				.collect();
			let heard = |note: &HeldNote| newest.get(note.note_id.as_str()).map(|(_, held)| *held);
			let newest_at = group.written.last().map_or(group.since, |(at, _)| *at);
			let settled = moments.passed(newest_at + Duration::hours(SETTLE_HOURS));
			Group {
				voice: voice_of(said.iter().map(|note| heard(note)), group.since, moments),
				notes: said
					.iter()
					.map(|note| GroupNote {
						note: note.note_id.clone(),
						transcript: heard(note).map(transcript_of),
					})
					.collect(),
				reflections: group.written.iter().map(|(_, name)| String::from(*name)).collect(),
				newest_at: written(newest_at),
				settled,
				base,
				session: group.session,
				day: group.day,
			}
		})
		.collect();
	groups.sort_by(|a, b| b.newest_at.cmp(&a.newest_at).then_with(|| a.base.cmp(&b.base)));
	groups
}

/// The prompts no drama answers, oldest first, each with the notes it names.
fn waiting_prompts(
	prompts: &[TakenFile],
	answered: &BTreeSet<String>,
	newest: &HashMap<&str, (u64, &HeldTranscript)>,
	moments: &mut Moments,
	skipped: &mut Vec<String>,
) -> Vec<Asked> {
	let mut waiting = Vec::new();
	for file in prompts {
		let (Some((prompt, named)), Some(taken)) = (prompt_of(file), instant(&file.handed_at)) else {
			skipped.push(file.name.clone());
			continue;
		};
		if answered.contains(&prompt.prompt) {
			continue;
		}
		let heard = |id: &str| newest.get(id).map(|(_, held)| *held);
		waiting.push(Asked {
			voice: voice_of(named.iter().map(|(_, id)| heard(id)), taken, moments),
			notes: named
				.iter()
				.map(|(file, id)| PromptNote {
					file: file.clone(),
					note: id.clone(),
					transcript: heard(id).map(transcript_of),
				})
				.collect(),
			name: file.name.clone(),
			prompt: prompt.prompt,
		});
	}
	waiting.sort_by(|a, b| a.name.cmp(&b.name));
	waiting
}

/// The lessons put in the last [`UNOPENED_DAYS`] (or
/// [`UNOPENED_DAYS_BEFORE_OPENED`] with no list yet) that the newest list of
/// opened lessons does not name (D1, D2).
fn unopened(inputs: &ReflectionsInputs, moments: &mut Moments, skipped: &mut Vec<String>) -> Unopened {
	let opened = inputs.opened.as_ref().and_then(|file| {
		let read = opened_of(file);
		if read.is_none() {
			skipped.push(file.name.clone());
		}
		read.map(|opened| (file.name.clone(), opened.opened))
	});
	let window = Duration::days(if opened.is_some() { UNOPENED_DAYS } else { UNOPENED_DAYS_BEFORE_OPENED });
	let named: BTreeSet<&str> = opened.iter().flat_map(|(_, opened)| opened).map(String::as_str).collect();
	let lessons = inputs
		.scenes
		.iter()
		.map(|lesson| ("scene/", lesson))
		.chain(inputs.dramas.iter().map(|lesson| ("drama/", lesson)));
	let mut unopened: Vec<String> = Vec::new();
	for (folder, lesson) in lessons {
		let key = String::from(folder) + &lesson.name;
		let Some(put) = instant(&lesson.pulled_at) else {
			continue;
		};
		if !named.contains(key.as_str()) && !moments.passed(put + window) {
			unopened.push(key);
		}
	}
	unopened.sort();
	Unopened {
		count: unopened.len(),
		lessons: unopened,
		opened: opened.map(|(name, _)| name),
	}
}

/// The latest [`RECENT_SCENES`] scenes, newest first, as their folder names
/// say: `TOPIK <level> · <genre> · <title>`.
fn recent_of(scenes: &[(&HeldLesson, SceneFile)]) -> Vec<Recent> {
	scenes
		.iter()
		.rev()
		.take(RECENT_SCENES)
		.map(|(lesson, scene)| {
			let mut parts = scene.title.split(" · ");
			let level = parts.next().and_then(|topik| topik.strip_prefix("TOPIK ")).and_then(|level| level.parse().ok());
			let genre = parts.next().filter(|genre| !genre.is_empty()).map(String::from);
			Recent {
				name: lesson.name.clone(),
				answers: scene.answers.clone(),
				title: scene.title.clone(),
				level: level.filter(|_| genre.is_some()),
				genre: genre.filter(|_| level.is_some()),
			}
		})
		.collect()
}

/// The reflections index for `inputs` at `now`, stamped `as_of`: its bytes.
///
/// Pure: the same inputs give the same bytes at any moment before the
/// `validUntil` they give (the module doc).
#[must_use]
pub fn reflections_index(inputs: &ReflectionsInputs, now: DateTime<Utc>, as_of: Option<&str>) -> Vec<u8> {
	computed_index(inputs, now, as_of).0
}

/// [`reflections_index`]'s bytes, and its `validUntil`.
fn computed_index(inputs: &ReflectionsInputs, now: DateTime<Utc>, as_of: Option<&str>) -> (Vec<u8>, Option<String>) {
	let mut skipped = Vec::new();
	let mut moments = Moments { now, first: None };
	let mut scenes: Vec<(&HeldLesson, SceneFile)> = Vec::new();
	for lesson in &inputs.scenes {
		match scene_of(lesson) {
			Some(scene) => scenes.push((lesson, scene)),
			None => skipped.push(lesson.name.clone()),
		}
	}
	scenes.sort_by_key(|(lesson, _)| lesson.seq);
	let mut dramas: BTreeSet<String> = BTreeSet::new();
	for lesson in &inputs.dramas {
		match drama_of(lesson) {
			Some(drama) => {
				dramas.insert(drama.answers);
			}
			None => skipped.push(lesson.name.clone()),
		}
	}
	let answered: BTreeSet<String> = scenes.iter().map(|(_, scene)| scene.answers.clone()).collect();
	let words: BTreeSet<WordUsed> = scenes
		.iter()
		.flat_map(|(_, scene)| scene.decks.iter().flat_map(|deck| &deck.words))
		.map(|word| WordUsed {
			id: word.id.clone(),
			lemma: word.lemma.clone(),
		})
		.collect();
	let newest = newest_transcripts(&inputs.transcripts);
	let gathered = gather(&inputs.reflections, &mut skipped);
	let reflections = waiting_groups(gathered, &answered, (&inputs.notes, &newest), &mut moments);
	let prompts = waiting_prompts(&inputs.prompts, &dramas, &newest, &mut moments, &mut skipped);
	let unopened = unopened(inputs, &mut moments, &mut skipped);
	skipped.sort();
	let valid_until = moments.first.map(written);
	let index = ReflectionsIndex {
		schema: INDEX_SCHEMA,
		routine: REFLECTIONS,
		as_of,
		valid_until: valid_until.clone(),
		skipped,
		answered: Answered {
			scenes: answered.into_iter().collect(),
			dramas: dramas.into_iter().collect(),
		},
		words: words.into_iter().collect(),
		recent: recent_of(&scenes),
		unopened,
		reflections,
		prompts,
	};
	let mut body = Vec::new();
	// Writing these structs to a `Vec` cannot fail: every key is a string.
	let _ = serde_json::to_writer(&mut body, &index);
	(body, valid_until)
}

/// Computes `subject`'s reflections index and puts it, when it has moved.
///
/// That is when what it counts has moved, or a moment it carries has passed,
/// since the last one the mailbox answered (the module doc). It writes
/// nothing to the database, so it takes no deletion hold.
///
/// # Errors
/// A [`Failure`] when the database fails or the mailbox cannot be reached;
/// the next pass computes and puts it again.
pub async fn put_reflections_index<T: MailboxTransport>(
	db: &SqlitePool,
	mailbox: &Mailbox<T>,
	subject: &str,
	last: &LastIndex,
	now: &(impl Fn() -> String + Sync),
	outcome: &mut Outcome,
) -> Result<(), Failure> {
	let at = now();
	let (Some(moment), Some(stamp)) = (instant(&at), stamp_of(&at)) else {
		return Ok(());
	};
	let (as_of, inputs) = {
		let mut tx = db.begin().await.map_err(Failure::Storage)?;
		let as_of = mailbox_repo::reflections_as_of(&mut tx, subject).await.map_err(crate::mailbox::pass::storage)?;
		// Nothing to count yet: no index, and the routine reads as before.
		if as_of.is_none() {
			return Ok(());
		}
		// Nothing arrived, and no moment the last index carries has come.
		if last.holds(subject, as_of.as_deref(), |until| instant(until).is_some_and(|until| moment < until)) {
			return Ok(());
		}
		let inputs = mailbox_repo::reflections_inputs(&mut tx, subject).await.map_err(crate::mailbox::pass::storage)?;
		tx.commit().await.map_err(Failure::Storage)?;
		(as_of, inputs)
	};
	let (body, valid_until) = computed_index(&inputs, moment, as_of.as_deref());
	let computed = Computed {
		subject: subject.to_owned(),
		as_of,
		valid: valid_until,
	};
	put_index(mailbox, &index_name(REFLECTIONS, &stamp), body, (last, computed), outcome).await
}

#[cfg(test)]
mod tests {
	//! The computation from fixture rows, each rule ported from paulgsc/some-ui
	//! `packages/ui/topik/ROUTINE.md`.

	use super::{reflections_index, REFLECTIONS};
	use crate::mailbox::index::index_name;
	use chrono::{DateTime, Utc};
	use mailbox_repo::{HeldLesson, HeldNote, HeldTranscript, ReflectionsInputs, TakenFile};
	use serde_json::{json, Value};

	fn at(text: &str) -> DateTime<Utc> {
		DateTime::parse_from_rfc3339(text).unwrap().with_timezone(&Utc)
	}

	fn index(inputs: &ReflectionsInputs, now: &str) -> Value {
		serde_json::from_slice(&reflections_index(inputs, at(now), Some("2026-10-30T15:00:00.000Z"))).unwrap()
	}

	fn deck(words: &[(&str, &str)]) -> Value {
		json!({ "schemaVersion": 1, "id": "d", "title": "t", "words": words.iter().map(|(id, lemma)| json!({ "id": id, "lemma": lemma, "gloss": "g" })).collect::<Vec<_>>(), "lines": [] })
	}

	/// A scene home kept: `seq`, answering `answers`, titled `title`.
	fn scene(seq: i64, answers: &str, title: &str, words: &[(&str, &str)]) -> HeldLesson {
		let body = json!({ "schema": "some-ui.routine-scene/1", "answers": answers, "title": title, "decks": [deck(words), deck(&[])] });
		HeldLesson {
			name: String::from("scene-") + answers + ".json",
			seq,
			pulled_at: String::from("2026-10-30T15:00:00.000Z"),
			body: body.to_string().into_bytes(),
		}
	}

	fn drama(seq: i64, stamp: &str) -> HeldLesson {
		HeldLesson {
			name: String::from(stamp) + ".json",
			seq,
			pulled_at: String::from("2026-10-30T15:00:00.000Z"),
			body: json!({ "schema": "some-ui.routine-drama/1", "answers": stamp, "tree": {} }).to_string().into_bytes(),
		}
	}

	/// A written reflection the mailbox took at `taken`.
	fn reflection(id: &str, session: &str, written_at: &str, taken: &str) -> TakenFile {
		let body = json!({ "schema": "some-ui.reflection/1", "reflection": id, "session": session, "at": written_at, "text": "My own words." });
		TakenFile {
			name: String::from("reflection-") + id + ".json",
			body: body.to_string().into_bytes(),
			handed_at: String::from(taken),
		}
	}

	/// A drama prompt the mailbox took at `taken`, naming `notes` (file, id).
	fn prompt(stamp: &str, notes: &[(&str, &str)], taken: &str) -> TakenFile {
		let mut text = String::from("> **For an agent:** follow the prompt below.\n\n");
		if !notes.is_empty() {
			let named: serde_json::Map<String, Value> = notes.iter().map(|(file, id)| (String::from(*file), json!(id))).collect();
			text += &(String::from("<!-- notes: ") + &Value::Object(named).to_string() + " -->\n\n");
		}
		text += "The scene I want next.";
		TakenFile {
			name: String::from(stamp) + ".prompt.json",
			body: json!({ "schema": "some-ui.drama-prompt/1", "prompt": stamp, "text": text }).to_string().into_bytes(),
			handed_at: String::from(taken),
		}
	}

	fn note(id: &str, of_kind: &str, of_ref: &str, recorded_at: &str) -> HeldNote {
		HeldNote {
			note_id: String::from(id),
			of_kind: String::from(of_kind),
			of_ref: String::from(of_ref),
			recorded_at: String::from(recorded_at),
		}
	}

	fn transcript(name: &str) -> HeldTranscript {
		HeldTranscript {
			name: String::from(name),
			sha256: String::from("sha-of-") + name,
		}
	}

	const SESSION: &str = "7f3a91c2-0000-4000-8000-000000000001";

	#[test]
	fn an_empty_home_has_an_empty_index() {
		let value: Value = serde_json::from_slice(&reflections_index(&ReflectionsInputs::default(), at("2026-10-31T15:00:00.000Z"), None)).unwrap();
		assert_eq!(
			value,
			json!({
				"schema": "some-ui.routine-index/1", "routine": "reflections", "asOf": null, "validUntil": null, "skipped": [],
				"answered": { "scenes": [], "dramas": [] }, "words": [], "recent": [],
				"unopened": { "count": 0, "lessons": [], "opened": null }, "reflections": [], "prompts": []
			})
		);
		assert_eq!(index_name(REFLECTIONS, "20261031T150000123Z"), "index-reflections-20261031T150000123Z.json");
	}

	/// Step 3: one group per session and day of `at` in Los Angeles, named
	/// `<day>_reflection_<session8>`, its reflections in the order written.
	#[test]
	fn written_reflections_group_by_session_and_los_angeles_day() {
		let inputs = ReflectionsInputs {
			reflections: vec![
				reflection("r2", SESSION, "2026-10-29T18:00:00.000Z", "2026-10-29T18:01:00.000Z"),
				reflection("r1", SESSION, "2026-10-29T16:00:00.000Z", "2026-10-29T16:01:00.000Z"),
				// 06:30 UTC on the 30th is the 29th in Los Angeles.
				reflection("r3", SESSION, "2026-10-30T06:30:00.000Z", "2026-10-30T06:31:00.000Z"),
				reflection("r4", SESSION, "2026-10-30T08:00:00.000Z", "2026-10-30T08:01:00.000Z"),
				reflection("r5", "s_7:x", "2026-10-29T17:00:00.000Z", "2026-10-29T17:01:00.000Z"),
			],
			..ReflectionsInputs::default()
		};
		let value = index(&inputs, "2026-10-31T15:00:00.000Z");
		let groups: Vec<(&str, Vec<&str>)> = value["reflections"]
			.as_array()
			.unwrap()
			.iter()
			.map(|group| {
				let names = group["reflections"].as_array().unwrap().iter().map(|name| name.as_str().unwrap()).collect();
				(group["base"].as_str().unwrap(), names)
			})
			.collect();
		assert_eq!(
			groups,
			[
				("2026-10-30_reflection_7f3a91c2", vec!["reflection-r4.json"]),
				("2026-10-29_reflection_7f3a91c2", vec!["reflection-r1.json", "reflection-r2.json", "reflection-r3.json"]),
				("2026-10-29_reflection_s7x", vec!["reflection-r5.json"]),
			],
			"newest first; the session's first 8 letters, digits or -"
		);
		let first = &value["reflections"][1];
		assert_eq!((first["session"].as_str(), first["day"].as_str()), (Some(SESSION), Some("2026-10-29")));
		assert_eq!(first["newestAt"], "2026-10-30T06:30:00.000Z");
		assert!(
			!String::from_utf8(reflections_index(&inputs, at("2026-10-31T15:00:00.000Z"), None))
				.unwrap()
				.contains("My own words"),
			"never a reflection's words"
		);
	}

	/// Steps 3 and 7: a scene folder answering a base (a stock one too) and a
	/// drama answering a stamp mark them answered, and they are not listed.
	#[test]
	fn what_a_scene_or_a_drama_answers_is_answered_and_not_listed() {
		let inputs = ReflectionsInputs {
			scenes: vec![
				scene(2, "2026-10-29_reflection_7f3a91c2", "TOPIK 2 · romance · At the café", &[]),
				scene(5, "2026-10-30_stock_1", "TOPIK 2 · thriller · The last train", &[]),
			],
			dramas: vec![drama(3, "drama-20261029T071503120Z")],
			reflections: vec![
				reflection("r1", SESSION, "2026-10-29T16:00:00.000Z", "2026-10-29T16:01:00.000Z"),
				reflection("r2", SESSION, "2026-10-30T16:00:00.000Z", "2026-10-30T16:01:00.000Z"),
			],
			prompts: vec![
				prompt("drama-20261029T071503120Z", &[], "2026-10-29T07:16:00.000Z"),
				prompt("drama-20261030T071503120Z", &[], "2026-10-30T07:16:00.000Z"),
			],
			..ReflectionsInputs::default()
		};
		let value = index(&inputs, "2026-10-31T15:00:00.000Z");
		assert_eq!(
			value["answered"],
			json!({ "scenes": ["2026-10-29_reflection_7f3a91c2", "2026-10-30_stock_1"], "dramas": ["drama-20261029T071503120Z"] })
		);
		assert_eq!(value["reflections"].as_array().unwrap().len(), 1);
		assert_eq!(value["reflections"][0]["base"], "2026-10-30_reflection_7f3a91c2");
		assert_eq!(value["prompts"].as_array().unwrap().len(), 1);
		assert_eq!(value["prompts"][0]["prompt"], "drama-20261030T071503120Z");
		assert_eq!(value["prompts"][0]["name"], "drama-20261030T071503120Z.prompt.json");
	}

	/// Step 3: a group whose newest reflection is under two hours old waits,
	/// and the index holds only until it settles.
	#[test]
	fn a_group_settles_two_hours_after_its_newest_reflection() {
		let inputs = ReflectionsInputs {
			reflections: vec![
				reflection("r1", SESSION, "2026-10-31T13:00:00.000Z", "2026-10-31T13:01:00.000Z"),
				reflection("r2", SESSION, "2026-10-31T14:00:00.000Z", "2026-10-31T14:01:00.000Z"),
			],
			..ReflectionsInputs::default()
		};
		let young = index(&inputs, "2026-10-31T15:59:59.999Z");
		assert_eq!(young["reflections"][0]["settled"], false);
		assert_eq!(young["validUntil"], "2026-10-31T16:00:00.000Z");
		let settled = index(&inputs, "2026-10-31T16:00:00.000Z");
		assert_eq!(settled["reflections"][0]["settled"], true);
		assert_eq!(settled["validUntil"], Value::Null);
	}

	/// "Voice notes": a group's notes are its session's reflection notes
	/// recorded on its day; one home has not heard keeps it waiting for two
	/// days from when the mailbox took its newest reflection, then it is
	/// answered without it.
	#[test]
	fn a_groups_unheard_note_waits_two_days_from_its_newest_put_then_is_missed() {
		let mut inputs = ReflectionsInputs {
			reflections: vec![
				reflection("r1", SESSION, "2026-10-29T16:00:00.000Z", "2026-10-29T16:01:00.000Z"),
				reflection("r2", SESSION, "2026-10-29T17:00:00.000Z", "2026-10-29T17:05:00.000Z"),
			],
			notes: vec![
				note("v1", "reflection", SESSION, "2026-10-29T16:30:00.000Z"),
				note("v2", "reflection", SESSION, "2026-10-29T16:40:00.000Z"),
				note("other-day", "reflection", SESSION, "2026-10-28T16:40:00.000Z"),
				note("other-session", "reflection", "s2", "2026-10-29T16:40:00.000Z"),
				note("a-scene", "scene", SESSION, "2026-10-29T16:40:00.000Z"),
			],
			transcripts: vec![transcript("transcript-v1-1.json"), transcript("transcript-v1-2.json")],
			..ReflectionsInputs::default()
		};
		let waiting = index(&inputs, "2026-10-30T15:00:00.000Z");
		let group = &waiting["reflections"][0];
		assert_eq!(
			group["notes"],
			json!([
				{ "note": "v1", "transcript": { "name": "transcript-v1-2.json", "sha256": "sha-of-transcript-v1-2.json" } },
				{ "note": "v2", "transcript": null }
			]),
			"the newest transcript; not another day's, session's or a scene note"
		);
		assert_eq!(
			group["voice"],
			json!({ "state": "waiting", "since": "2026-10-29T17:05:00.000Z", "until": "2026-10-31T17:05:00.000Z" })
		);
		assert_eq!(waiting["validUntil"], "2026-10-31T17:05:00.000Z");
		let missed = index(&inputs, "2026-10-31T17:05:00.000Z");
		assert_eq!(
			missed["reflections"][0]["voice"],
			json!({ "state": "missed", "since": "2026-10-29T17:05:00.000Z", "until": null })
		);
		assert_eq!(missed["validUntil"], Value::Null);
		inputs.transcripts.push(transcript("transcript-v2-1.json"));
		assert_eq!(index(&inputs, "2026-10-30T15:00:00.000Z")["reflections"][0]["voice"]["state"], "heard");
	}

	/// Step 7 and "Voice notes": a prompt's notes are those its `notes` line
	/// names, in the order said, each waited on as a reflection's is.
	#[test]
	fn a_prompts_notes_are_the_ones_it_names_in_the_order_said() {
		let named = [
			("drama-20261030T071503120Z.voice-10.webm", "n10"),
			("drama-20261030T071503120Z.voice-2.webm", "n2"),
			("drama-20261030T071503120Z.voice-1.webm", "n1"),
		];
		let inputs = ReflectionsInputs {
			prompts: vec![
				prompt("drama-20261030T071503120Z", &named, "2026-10-30T07:16:00.000Z"),
				prompt("drama-20261029T071503120Z", &[], "2026-10-29T07:16:00.000Z"),
			],
			transcripts: vec![transcript("transcript-n1-1.json"), transcript("transcript-n2-1.json")],
			..ReflectionsInputs::default()
		};
		let value = index(&inputs, "2026-10-31T15:00:00.000Z");
		let prompts = value["prompts"].as_array().unwrap();
		assert_eq!(prompts[0]["prompt"], "drama-20261029T071503120Z", "oldest first");
		assert_eq!(prompts[0]["voice"]["state"], "heard", "nothing said aloud: nothing to wait on");
		let order: Vec<&str> = prompts[1]["notes"].as_array().unwrap().iter().map(|note| note["note"].as_str().unwrap()).collect();
		assert_eq!(order, ["n1", "n2", "n10"]);
		assert_eq!(prompts[1]["notes"][0]["file"], "drama-20261030T071503120Z.voice-1.webm");
		assert_eq!(
			prompts[1]["voice"],
			json!({ "state": "waiting", "since": "2026-10-30T07:16:00.000Z", "until": "2026-11-01T07:16:00.000Z" })
		);
		assert_eq!(index(&inputs, "2026-11-01T07:16:00.000Z")["prompts"][1]["voice"]["state"], "missed");
		assert!(
			!String::from_utf8(reflections_index(&inputs, at("2026-10-31T15:00:00.000Z"), None))
				.unwrap()
				.contains("The scene I want"),
			"never a prompt's words"
		);
	}

	/// Step 4: every word id with its dictionary form, across every deck of
	/// every scene, archive or not, once each; an id written with two forms
	/// is named with both.
	#[test]
	fn words_are_every_id_and_form_any_deck_uses_once_each() {
		let inputs = ReflectionsInputs {
			scenes: vec![
				scene(1, "a", "TOPIK 1 · everyday · One", &[("coffee", "커피"), ("person", "사람")]),
				scene(2, "b", "TOPIK 1 · everyday · Two", &[("person", "사람"), ("give", "드리다"), ("give", "주다")]),
			],
			..ReflectionsInputs::default()
		};
		assert_eq!(
			index(&inputs, "2026-10-31T15:00:00.000Z")["words"],
			json!([
				{ "id": "coffee", "lemma": "커피" }, { "id": "give", "lemma": "드리다" }, { "id": "give", "lemma": "주다" }, { "id": "person", "lemma": "사람" }
			])
		);
	}

	/// Steps 4 and D3: the latest three scenes, newest first, with the level
	/// and genre their folder names carry.
	#[test]
	fn recent_is_the_latest_three_scenes_with_their_level_and_genre() {
		let inputs = ReflectionsInputs {
			scenes: vec![
				scene(1, "a", "TOPIK 1 · everyday · One", &[]),
				scene(4, "d", "TOPIK 3 · thriller · Four", &[]),
				scene(2, "b", "TOPIK 2 · romance · Two", &[]),
				scene(3, "c", "Not a folder name", &[]),
			],
			..ReflectionsInputs::default()
		};
		assert_eq!(
			index(&inputs, "2026-10-31T15:00:00.000Z")["recent"],
			json!([
				{ "name": "scene-d.json", "answers": "d", "title": "TOPIK 3 · thriller · Four", "level": 3, "genre": "thriller" },
				{ "name": "scene-c.json", "answers": "c", "title": "Not a folder name", "level": null, "genre": null },
				{ "name": "scene-b.json", "answers": "b", "title": "TOPIK 2 · romance · Two", "level": 2, "genre": "romance" }
			])
		);
	}

	fn lesson_put(mut lesson: HeldLesson, pulled_at: &str) -> HeldLesson {
		lesson.pulled_at = String::from(pulled_at);
		lesson
	}

	/// D1 and D2: a lesson put in the last 30 days that the newest opened
	/// list does not name is unopened; with no list yet, only the last 7.
	#[test]
	fn unopened_counts_the_last_thirty_days_less_what_was_opened_or_seven_with_no_list() {
		let mut inputs = ReflectionsInputs {
			scenes: vec![
				lesson_put(scene(1, "old", "TOPIK 1 · everyday · Old", &[]), "2026-10-01T15:00:00.000Z"),
				lesson_put(scene(2, "mid", "TOPIK 1 · everyday · Mid", &[]), "2026-10-20T15:00:00.000Z"),
				lesson_put(scene(3, "new", "TOPIK 1 · everyday · New", &[]), "2026-10-30T15:00:00.000Z"),
			],
			dramas: vec![lesson_put(drama(4, "drama-20261029T071503120Z"), "2026-10-29T08:00:00.000Z")],
			..ReflectionsInputs::default()
		};
		let none_yet = index(&inputs, "2026-10-31T15:00:00.000Z");
		assert_eq!(
			none_yet["unopened"],
			json!({ "count": 2, "lessons": ["drama/drama-20261029T071503120Z.json", "scene/scene-new.json"], "opened": null })
		);
		assert_eq!(none_yet["validUntil"], "2026-11-05T08:00:00.000Z", "the drama leaves the seven days first");
		inputs.opened = Some(TakenFile {
			name: String::from("opened-20261031T000000000Z.json"),
			body: json!({ "schema": "some-ui.routine-opened/1", "opened": ["scene/scene-new.json"], "at": "2026-10-31T00:00:00.000Z" })
				.to_string()
				.into_bytes(),
			handed_at: String::from("2026-10-31T00:00:01.000Z"),
		});
		let listed = index(&inputs, "2026-10-31T15:00:00.000Z");
		assert_eq!(
			listed["unopened"],
			json!({ "count": 2, "lessons": ["drama/drama-20261029T071503120Z.json", "scene/scene-mid.json"], "opened": "opened-20261031T000000000Z.json" })
		);
		assert_eq!(listed["validUntil"], "2026-11-19T15:00:00.000Z", "mid leaves the thirty days first");
	}

	/// The index moves with the clock only at the moments it carries: any two
	/// moments before its `validUntil` give the same bytes.
	#[test]
	fn the_same_rows_give_the_same_bytes_until_valid_until() {
		let inputs = ReflectionsInputs {
			scenes: vec![lesson_put(scene(1, "a", "TOPIK 1 · everyday · One", &[("coffee", "커피")]), "2026-10-30T15:00:00.000Z")],
			reflections: vec![reflection("r1", SESSION, "2026-10-31T14:00:00.000Z", "2026-10-31T14:01:00.000Z")],
			notes: vec![note("v1", "reflection", SESSION, "2026-10-31T13:59:00.000Z")],
			..ReflectionsInputs::default()
		};
		let bytes = |now: &str| reflections_index(&inputs, at(now), Some("2026-10-31T14:01:00.000Z"));
		let first = index(&inputs, "2026-10-31T14:30:00.000Z");
		assert_eq!(first["validUntil"], "2026-10-31T16:00:00.000Z", "the group settles first");
		assert_eq!(bytes("2026-10-31T14:30:00.000Z"), bytes("2026-10-31T15:59:59.999Z"));
		assert_ne!(bytes("2026-10-31T15:59:59.999Z"), bytes("2026-10-31T16:00:00.000Z"));
		assert_eq!(
			index(&inputs, "2026-10-31T16:00:00.000Z")["validUntil"],
			"2026-11-02T14:01:00.000Z",
			"then the note's wait ends"
		);
		let text = String::from_utf8(bytes("2026-10-31T14:30:00.000Z")).unwrap();
		let keys = [
			"{\"schema\":",
			",\"routine\":",
			",\"asOf\":",
			",\"validUntil\":",
			",\"skipped\":",
			",\"answered\":",
			",\"words\":",
			",\"recent\":",
			",\"unopened\":",
			",\"reflections\":",
			",\"prompts\":",
		];
		let positions: Vec<usize> = keys.iter().map(|key| text.find(key).unwrap()).collect();
		assert!(positions.windows(2).all(|pair| pair[0] < pair[1]), "keys in their fixed order: {text}");
	}

	#[test]
	fn a_file_that_does_not_read_as_its_schema_is_skipped_and_counts_for_nothing() {
		let broken = |name: &str| HeldLesson {
			name: String::from(name),
			seq: 9,
			pulled_at: String::from("2026-10-30T15:00:00.000Z"),
			body: br#"{"schema":"some-ui.routine-scene/1","answers":"x"}"#.to_vec(),
		};
		let mut bad_line = prompt("drama-20261030T071503120Z", &[], "2026-10-30T07:16:00.000Z");
		bad_line.body = json!({ "schema": "some-ui.drama-prompt/1", "prompt": "drama-20261030T071503120Z", "text": "<!-- notes: {not json} -->" })
			.to_string()
			.into_bytes();
		let late = reflection("r1", SESSION, "yesterday", "2026-10-30T16:01:00.000Z");
		let inputs = ReflectionsInputs {
			scenes: vec![broken("scene-x.json")],
			dramas: vec![broken("drama-x.json")],
			reflections: vec![late],
			prompts: vec![bad_line],
			..ReflectionsInputs::default()
		};
		let value = index(&inputs, "2026-10-31T15:00:00.000Z");
		assert_eq!(
			value["skipped"],
			json!(["drama-20261030T071503120Z.prompt.json", "drama-x.json", "reflection-r1.json", "scene-x.json"])
		);
		assert_eq!(value["answered"], json!({ "scenes": [], "dramas": [] }));
		assert_eq!((value["reflections"].as_array().unwrap().len(), value["prompts"].as_array().unwrap().len()), (0, 0));
		assert_eq!(value["unopened"]["count"], 2, "a lesson is a lesson on the phone, read or not");
	}
}

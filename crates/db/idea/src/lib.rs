//! The owner's ideas, kept on home as one record each (STUDIO ST26:
//! paulgsc/server#645, the phone's half paulgsc/some-ui#2453): the title,
//! topic, format and keywords, the question set's version and its answers,
//! sent together; whether the owner is still furnishing it; whether it was set
//! aside; and the topics and keywords lists the owner edits.
//!
//! `20261101002600_create_idea.up.sql` records the design; in short:
//!
//! - **Append-only.** Every write is a new row or nothing: [`put_idea`] adds a
//!   revision only when the send changes the idea, [`mark_furnishing`] and
//!   [`mark_entry`] a mark only when it changes what stands, and [`set_aside`]
//!   one mark per idea. No row is updated (each table's trigger aborts an
//!   `UPDATE`), and nothing here deletes. Account deletion is the one delete:
//!   its generic sweep of `SUBJECT_SCOPED_TABLES`, and the migration's
//!   triggers on `account`.
//! - **An answer's audio is a voice note's.** A send names `studio` notes by
//!   id, and only ones the subject already holds; their audio went home
//!   through `PUT /voice-notes/:id` and only there (docs/identity.md
//!   invariant 23). This crate reads `voice_note`'s kind and, for a read, what
//!   home heard in a note (`voice_repo::transcript::heard`), and writes
//!   neither.
//! - **The lists are seeds and marks.** A list is its seeds
//!   ([`SEED_TOPICS`], [`SEED_KEYWORDS`]) with each entry's latest mark
//!   applied. A send must use entries on the lists as they stand; taking one
//!   off leaves every revision that used it as it is.
//! - **Per subject, and only per subject.** Every query takes the subject and
//!   filters on it; nothing reads across subjects.
//! - **No background writer.** The one caller is `file_host`'s
//!   `handlers::ideas`. A `git grep -n -e idea_revision -e idea_answer -e
//!   idea_furnishing -e idea_set_aside -e idea_list_mark -- '*.rs'` shows no
//!   other writer.

mod model;

pub use model::{
	instant, is_entry, is_id, parse_bundle, question_set, Answer, Bundle, Content, List, Problem, Said, BODY_CEILING, FORMATS, MAX_ENTRY_CHARS, MAX_KEYWORDS, MAX_LIST_ENTRIES,
	MAX_TITLE_CHARS, MAX_TYPED_CHARS, QUESTION_SETS, SEED_KEYWORDS, SEED_TOPICS,
};
use serde::Serialize;
use sqlx::SqliteConnection;
use voice_repo::transcript::Hearing;

/// The most ideas [`ideas`] lists: the newest begun.
pub const IDEAS_LISTED: i64 = 1_000;

/// What a write did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
	/// It changed what home holds: a row is added.
	Added,
	/// Home already holds it as sent: nothing written.
	Unchanged,
}

impl Change {
	/// The wire name.
	#[must_use]
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::Added => "added",
			Self::Unchanged => "unchanged",
		}
	}
}

/// What a set-aside mark did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetAside {
	/// The idea is set aside now.
	Marked,
	/// It already was: nothing written, the first mark stands.
	AlreadyMarked,
	/// The subject holds no such idea: nothing written.
	NotHeld,
}

/// Why a write wrote nothing.
#[derive(Debug)]
pub enum IdeaError {
	/// The write breaks a rule; every problem, by field.
	Invalid(Vec<Problem>),
	/// A send already held under its `sentAt` carried a different bundle.
	Rewritten,
	/// The idea is set aside for good, and takes no new revision.
	SetAside,
	/// These named voice notes are not the subject's own `studio` notes on
	/// home; by the phone's ids, in the order the answers name them.
	NotesNotHeld(Vec<String>),
	/// A stored row this crate wrote does not read back.
	Unreadable,
	Storage(sqlx::Error),
}

impl std::fmt::Display for IdeaError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::Invalid(problems) => {
				f.write_str("not a write the ideas record takes:")?;
				for (field, why) in problems {
					write!(f, " {field} {why};")?;
				}
				Ok(())
			}
			Self::Rewritten => f.write_str(REWRITTEN),
			Self::SetAside => f.write_str(SET_ASIDE),
			Self::NotesNotHeld(_) => f.write_str(NOTES_NOT_HELD),
			Self::Unreadable => f.write_str(UNREADABLE),
			Self::Storage(err) => write!(f, "database error: {err}"),
		}
	}
}

impl std::error::Error for IdeaError {}

impl From<sqlx::Error> for IdeaError {
	fn from(err: sqlx::Error) -> Self {
		Self::Storage(err)
	}
}

impl From<voice_repo::VoiceError> for IdeaError {
	fn from(err: voice_repo::VoiceError) -> Self {
		match err {
			voice_repo::VoiceError::Storage(err) => Self::Storage(err),
			_ => Self::Unreadable,
		}
	}
}

/// What [`IdeaError::Rewritten`] says.
pub const REWRITTEN: &str = "this send already holds a different bundle: an idea's revisions are append-only";

/// What [`IdeaError::SetAside`] says.
pub const SET_ASIDE: &str = "this idea is set aside for good and takes no new revision";

/// What [`IdeaError::NotesNotHeld`] says.
pub const NOTES_NOT_HELD: &str = "a named voice note is not one of this subject's studio notes on home: send it first";

/// What [`IdeaError::Unreadable`] says.
pub const UNREADABLE: &str = "a stored idea does not read back";

/// What home heard in an answer's voice note.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Heard {
	/// What was heard; empty when home found the note silent.
	pub text: String,
	/// Whisper's code for the language detected; none for a silent note.
	pub language: Option<String>,
	/// The model file home heard it with.
	pub model: String,
	/// When home heard it: ISO-8601 UTC.
	pub heard_at: String,
}

/// One answer as home serves it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnswerView {
	pub question: String,
	/// The `studio` voice note that answers it, by the phone's id.
	pub note: Option<String>,
	/// The words typed when voice failed.
	pub typed: Option<String>,
	/// What home heard in the note; none while it has not, and for a typed
	/// answer.
	pub heard: Option<Heard>,
}

/// An idea as home serves it: its latest revision and its marks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Idea {
	pub id: String,
	/// When the phone began it: ISO-8601 UTC to the millisecond.
	pub created_at: String,
	/// The latest revision's number, 1 for the first send.
	pub revision: i64,
	/// When the latest revision was sent.
	pub sent_at: String,
	pub title: String,
	pub topic: String,
	/// `short` or `video`.
	pub format: String,
	pub keywords: Vec<String>,
	/// The question set's version.
	pub questions: i64,
	/// In the question set's order.
	pub answers: Vec<AnswerView>,
	/// The latest "still furnishing" mark; false until one is made.
	pub furnishing: bool,
	/// Whether the owner set it aside. The list leaves such an idea out.
	pub set_aside: bool,
}

/// The ideas home lists.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Ideas {
	/// Newest begun first, set-aside ideas left out, at most [`IDEAS_LISTED`].
	pub ideas: Vec<Idea>,
}

/// The lists as they stand.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Lists {
	/// The seeds still on, in seed order, then the entries added, in the
	/// order they were last added.
	pub topics: Vec<String>,
	pub keywords: Vec<String>,
}

impl Lists {
	fn of(&self, list: List) -> &[String] {
		match list {
			List::Topic => &self.topics,
			List::Keyword => &self.keywords,
		}
	}
}

struct RevisionRow {
	revision: i64,
	created_at: String,
	sent_at: String,
	title: String,
	topic: String,
	format: String,
	keywords: String,
	questions: i64,
}

async fn latest_revision(conn: &mut SqliteConnection, subject: &str, id: &str) -> Result<Option<RevisionRow>, IdeaError> {
	Ok(
		sqlx::query_as!(
			RevisionRow,
			"SELECT revision, created_at, sent_at, title, topic, format, keywords, questions FROM idea_revision
		 WHERE subject_id = ?1 AND idea_id = ?2 ORDER BY revision DESC LIMIT 1",
			subject,
			id
		)
		.fetch_optional(&mut *conn)
		.await?,
	)
}

async fn revision_sent_at(conn: &mut SqliteConnection, subject: &str, id: &str, sent_at: &str) -> Result<Option<RevisionRow>, IdeaError> {
	Ok(
		sqlx::query_as!(
			RevisionRow,
			"SELECT revision, created_at, sent_at, title, topic, format, keywords, questions FROM idea_revision
		 WHERE subject_id = ?1 AND idea_id = ?2 AND sent_at = ?3 ORDER BY revision LIMIT 1",
			subject,
			id,
			sent_at
		)
		.fetch_optional(&mut *conn)
		.await?,
	)
}

async fn held_answers(conn: &mut SqliteConnection, subject: &str, id: &str, row: &RevisionRow) -> Result<Vec<Answer>, IdeaError> {
	let rows = sqlx::query!(
		"SELECT question, note_id, typed FROM idea_answer WHERE subject_id = ?1 AND idea_id = ?2 AND revision = ?3",
		subject,
		id,
		row.revision
	)
	.fetch_all(&mut *conn)
	.await?;
	let set = question_set(row.questions).ok_or(IdeaError::Unreadable)?;
	let mut answers = rows
		.into_iter()
		.map(|answer| {
			let said = match (answer.note_id, answer.typed) {
				(Some(note), None) => Said::Note(note),
				(None, Some(typed)) => Said::Typed(typed),
				_ => return Err(IdeaError::Unreadable),
			};
			Ok(Answer { question: answer.question, said })
		})
		.collect::<Result<Vec<Answer>, IdeaError>>()?;
	answers.sort_by_key(|answer| set.iter().position(|question| *question == answer.question));
	Ok(answers)
}

async fn held_content(conn: &mut SqliteConnection, subject: &str, id: &str, row: &RevisionRow) -> Result<Content, IdeaError> {
	let answers = held_answers(conn, subject, id, row).await?;
	Ok(Content {
		created_at: row.created_at.clone(),
		title: row.title.clone(),
		topic: row.topic.clone(),
		format: row.format.clone(),
		keywords: serde_json::from_str(&row.keywords).map_err(|_| IdeaError::Unreadable)?,
		questions: row.questions,
		answers,
	})
}

async fn furnishing(conn: &mut SqliteConnection, subject: &str, id: &str) -> Result<bool, IdeaError> {
	let latest = sqlx::query_scalar!(
		"SELECT furnishing FROM idea_furnishing WHERE subject_id = ?1 AND idea_id = ?2 ORDER BY seq DESC LIMIT 1",
		subject,
		id
	)
	.fetch_optional(&mut *conn)
	.await?;
	Ok(latest == Some(1))
}

async fn is_set_aside(conn: &mut SqliteConnection, subject: &str, id: &str) -> Result<bool, IdeaError> {
	Ok(
		sqlx::query_scalar!(
			r#"SELECT EXISTS (SELECT 1 FROM idea_set_aside WHERE subject_id = ?1 AND idea_id = ?2) AS "held!: bool""#,
			subject,
			id
		)
		.fetch_one(&mut *conn)
		.await?,
	)
}

async fn heard(conn: &mut SqliteConnection, subject: &str, note: &str) -> Result<Option<Heard>, IdeaError> {
	Ok(match voice_repo::transcript::heard(conn, subject, note).await? {
		Hearing::NotHeld | Hearing::Waiting => None,
		Hearing::Heard(transcript) => Some(Heard {
			text: transcript.text,
			language: transcript.language,
			model: transcript.model,
			heard_at: transcript.heard_at,
		}),
	})
}

async fn view(conn: &mut SqliteConnection, subject: &str, id: &str, row: RevisionRow) -> Result<Idea, IdeaError> {
	let content = held_content(conn, subject, id, &row).await?;
	let mut answers = Vec::with_capacity(content.answers.len());
	for answer in content.answers {
		answers.push(match answer.said {
			Said::Note(note) => AnswerView {
				question: answer.question,
				heard: heard(conn, subject, &note).await?,
				note: Some(note),
				typed: None,
			},
			Said::Typed(typed) => AnswerView {
				question: answer.question,
				note: None,
				typed: Some(typed),
				heard: None,
			},
		});
	}
	Ok(Idea {
		id: id.to_owned(),
		created_at: content.created_at,
		revision: row.revision,
		sent_at: row.sent_at,
		title: content.title,
		topic: content.topic,
		format: content.format,
		keywords: content.keywords,
		questions: content.questions,
		answers,
		furnishing: furnishing(conn, subject, id).await?,
		set_aside: is_set_aside(conn, subject, id).await?,
	})
}

/// `subject`'s idea `id` as it stands, set aside or not; none for an idea home
/// does not hold.
///
/// # Errors
/// [`IdeaError::Unreadable`] for a stored row that does not read back, and
/// [`IdeaError::Storage`] for any `sqlx` failure.
pub async fn idea(conn: &mut SqliteConnection, subject: &str, id: &str) -> Result<Option<Idea>, IdeaError> {
	match latest_revision(conn, subject, id).await? {
		Some(row) => view(conn, subject, id, row).await.map(Some),
		None => Ok(None),
	}
}

/// The newest [`IDEAS_LISTED`] of `subject`'s ideas not set aside, newest
/// begun first, each as it stands.
///
/// # Errors
/// As [`idea`].
pub async fn ideas(conn: &mut SqliteConnection, subject: &str) -> Result<Ideas, IdeaError> {
	let rows = sqlx::query!(
		r#"SELECT r.idea_id AS "idea_id!: String", r.revision AS "revision!: i64", r.created_at AS "created_at!: String",
		          r.sent_at AS "sent_at!: String", r.title AS "title!: String", r.topic AS "topic!: String",
		          r.format AS "format!: String", r.keywords AS "keywords!: String", r.questions AS "questions!: i64"
		   FROM idea_revision r
		   WHERE r.subject_id = ?1
		     AND r.revision = (SELECT MAX(m.revision) FROM idea_revision m WHERE m.subject_id = r.subject_id AND m.idea_id = r.idea_id)
		     AND NOT EXISTS (SELECT 1 FROM idea_set_aside s WHERE s.subject_id = r.subject_id AND s.idea_id = r.idea_id)
		   ORDER BY r.created_at DESC, r.idea_id
		   LIMIT ?2"#,
		subject,
		IDEAS_LISTED
	)
	.fetch_all(&mut *conn)
	.await?;
	let mut ideas = Vec::with_capacity(rows.len());
	for row in rows {
		let revision = RevisionRow {
			revision: row.revision,
			created_at: row.created_at,
			sent_at: row.sent_at,
			title: row.title,
			topic: row.topic,
			format: row.format,
			keywords: row.keywords,
			questions: row.questions,
		};
		ideas.push(view(conn, subject, &row.idea_id, revision).await?);
	}
	Ok(Ideas { ideas })
}

/// `subject`'s lists as they stand.
///
/// # Errors
/// [`IdeaError::Storage`] for any `sqlx` failure.
pub async fn lists(conn: &mut SqliteConnection, subject: &str) -> Result<Lists, IdeaError> {
	let marks = sqlx::query!(
		r#"SELECT list AS "list!: String", entry AS "entry!: String", MAX(seq) AS "seq!: i64", listed AS "listed!: i64", stored_at AS "stored_at!: String"
		   FROM idea_list_mark WHERE subject_id = ?1 GROUP BY list, entry
		   ORDER BY stored_at, entry"#,
		subject
	)
	.fetch_all(&mut *conn)
	.await?;
	let build = |list: List| {
		let latest = |entry: &str| marks.iter().find(|mark| mark.list == list.as_str() && mark.entry == entry).map(|mark| mark.listed == 1);
		let mut entries: Vec<String> = list.seeds().iter().filter(|seed| latest(seed) != Some(false)).map(|seed| (*seed).to_owned()).collect();
		entries.extend(
			marks
				.iter()
				.filter(|mark| mark.list == list.as_str() && mark.listed == 1 && !list.seeds().contains(&mark.entry.as_str()))
				.map(|mark| mark.entry.clone()),
		);
		entries
	};
	Ok(Lists {
		topics: build(List::Topic),
		keywords: build(List::Keyword),
	})
}

/// Adds `entry` to `subject`'s `list` (`listed`) or takes it off, as of `now`,
/// and answers what it did with the lists as they now stand. An entry already
/// where the mark would put it is unchanged.
///
/// Run it in a `BEGIN IMMEDIATE` transaction, so the read that decides and the
/// write see one snapshot.
///
/// # Errors
/// [`IdeaError::Invalid`] for an entry that is not one ([`is_entry`]) or an
/// addition past [`MAX_LIST_ENTRIES`], and [`IdeaError::Storage`] for any
/// `sqlx` failure. A refusal writes nothing.
pub async fn mark_entry(conn: &mut SqliteConnection, subject: &str, list: List, entry: &str, listed: bool, now: &str) -> Result<(Change, Lists), IdeaError> {
	if !is_entry(entry) {
		return Err(IdeaError::Invalid(vec![(
			"name",
			"must be 1 to 40 characters, no space at either end, no control character or `/`",
		)]));
	}
	let standing = lists(conn, subject).await?;
	let on = standing.of(list).iter().any(|held| held == entry);
	if on == listed {
		return Ok((Change::Unchanged, standing));
	}
	if listed && standing.of(list).len() >= MAX_LIST_ENTRIES {
		return Err(IdeaError::Invalid(vec![("name", "the list already holds 100 entries: take one off first")]));
	}
	let kind = list.as_str();
	let flag = i64::from(listed);
	sqlx::query!(
		"INSERT INTO idea_list_mark (subject_id, list, entry, seq, listed, stored_at)
		 SELECT ?1, ?2, ?3, COALESCE(MAX(seq), 0) + 1, ?4, ?5 FROM idea_list_mark WHERE subject_id = ?1 AND list = ?2 AND entry = ?3",
		subject,
		kind,
		entry,
		flag,
		now
	)
	.execute(&mut *conn)
	.await?;
	Ok((Change::Added, lists(conn, subject).await?))
}

async fn notes_not_held(conn: &mut SqliteConnection, subject: &str, content: &Content) -> Result<Vec<String>, IdeaError> {
	let mut missing = Vec::new();
	for answer in &content.answers {
		if let Said::Note(note) = &answer.said {
			let kind = sqlx::query_scalar!("SELECT of_kind FROM voice_note WHERE subject_id = ?1 AND note_id = ?2", subject, note)
				.fetch_optional(&mut *conn)
				.await?;
			if kind.as_deref() != Some("studio") {
				missing.push(note.clone());
			}
		}
	}
	Ok(missing)
}

/// Stores the send in `body` as a revision of `subject`'s idea `id`, as of
/// `now`, and answers what it did with the idea as it now stands.
///
/// A send held already (its `sentAt` and bundle a revision's) and a send
/// whose bundle is the latest revision's write nothing. Any other is the next
/// revision, once it holds every rule.
///
/// Run it in a `BEGIN IMMEDIATE` transaction, so the reads that decide and the
/// write see one snapshot.
///
/// # Errors
/// [`IdeaError::Invalid`] naming every problem [`parse_bundle`] finds, or a
/// `createdAt` other than the idea's, a topic or keyword not on its list as
/// it stands; [`IdeaError::Rewritten`] for a different bundle under a held
/// `sentAt`; [`IdeaError::SetAside`] for an idea set aside;
/// [`IdeaError::NotesNotHeld`] naming each voice note that is not the
/// subject's own `studio` note on home; and [`IdeaError::Storage`] for any
/// `sqlx` failure. A refusal is decided before anything is written.
pub async fn put_idea(conn: &mut SqliteConnection, subject: &str, id: &str, body: &[u8], now: &str) -> Result<(Change, Idea), IdeaError> {
	let bundle = parse_bundle(id, body).map_err(IdeaError::Invalid)?;
	if let Some(held) = revision_sent_at(conn, subject, id, &bundle.sent_at).await? {
		if held_content(conn, subject, id, &held).await? != bundle.content {
			return Err(IdeaError::Rewritten);
		}
		let latest = latest_revision(conn, subject, id).await?.ok_or(IdeaError::Unreadable)?;
		return Ok((Change::Unchanged, view(conn, subject, id, latest).await?));
	}
	if is_set_aside(conn, subject, id).await? {
		return Err(IdeaError::SetAside);
	}
	let mut latest = latest_revision(conn, subject, id).await?;
	let same = match latest.as_ref() {
		Some(row) => held_content(conn, subject, id, row).await? == bundle.content,
		None => false,
	};
	if same {
		if let Some(row) = latest.take() {
			return Ok((Change::Unchanged, view(conn, subject, id, row).await?));
		}
	}
	let content = bundle.content;
	let mut problems = Vec::new();
	if latest.as_ref().is_some_and(|latest| latest.created_at != content.created_at) {
		problems.push(("createdAt", "must be the idea's, as its first send carried it"));
	}
	let standing = lists(conn, subject).await?;
	if !standing.topics.contains(&content.topic) {
		problems.push(("topic", "must be on the topics list as it stands"));
	}
	if !content.keywords.iter().all(|keyword| standing.keywords.contains(keyword)) {
		problems.push(("keywords", "each must be on the keywords list as it stands"));
	}
	if !problems.is_empty() {
		return Err(IdeaError::Invalid(problems));
	}
	let missing = notes_not_held(conn, subject, &content).await?;
	if !missing.is_empty() {
		return Err(IdeaError::NotesNotHeld(missing));
	}
	let revision = latest.map_or(1, |latest| latest.revision + 1);
	let keywords = serde_json::Value::from(content.keywords.clone()).to_string();
	sqlx::query!(
		"INSERT INTO idea_revision (subject_id, idea_id, revision, created_at, sent_at, title, topic, format, keywords, questions, stored_at)
		 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
		subject,
		id,
		revision,
		content.created_at,
		bundle.sent_at,
		content.title,
		content.topic,
		content.format,
		keywords,
		content.questions,
		now
	)
	.execute(&mut *conn)
	.await?;
	for answer in &content.answers {
		let (note, typed) = match &answer.said {
			Said::Note(note) => (Some(note.as_str()), None),
			Said::Typed(typed) => (None, Some(typed.as_str())),
		};
		sqlx::query!(
			"INSERT INTO idea_answer (subject_id, idea_id, revision, question, note_id, typed) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
			subject,
			id,
			revision,
			answer.question,
			note,
			typed
		)
		.execute(&mut *conn)
		.await?;
	}
	let latest = latest_revision(conn, subject, id).await?.ok_or(IdeaError::Unreadable)?;
	Ok((Change::Added, view(conn, subject, id, latest).await?))
}

/// Marks `subject`'s idea `id` as still being furnished or not, as of `now`.
/// A mark that is already the latest is unchanged.
///
/// Run it in a `BEGIN IMMEDIATE` transaction, as [`mark_entry`].
///
/// # Errors
/// [`IdeaError::Storage`] for any `sqlx` failure. `None` for an idea the
/// subject does not hold, which writes nothing.
pub async fn mark_furnishing(conn: &mut SqliteConnection, subject: &str, id: &str, furnishing_now: bool, now: &str) -> Result<Option<Change>, IdeaError> {
	if latest_revision(conn, subject, id).await?.is_none() {
		return Ok(None);
	}
	if furnishing(conn, subject, id).await? == furnishing_now {
		return Ok(Some(Change::Unchanged));
	}
	let flag = i64::from(furnishing_now);
	sqlx::query!(
		"INSERT INTO idea_furnishing (subject_id, idea_id, seq, furnishing, stored_at)
		 SELECT ?1, ?2, COALESCE(MAX(seq), 0) + 1, ?3, ?4 FROM idea_furnishing WHERE subject_id = ?1 AND idea_id = ?2",
		subject,
		id,
		flag,
		now
	)
	.execute(&mut *conn)
	.await?;
	Ok(Some(Change::Added))
}

/// Sets `subject`'s idea `id` aside for good, recorded by home at `now`.
///
/// Run it in a `BEGIN IMMEDIATE` transaction, so the write and the read that
/// tells an earlier mark from no idea see one snapshot.
///
/// # Errors
/// [`IdeaError::Storage`] for any `sqlx` failure; nothing is written then.
pub async fn set_aside(conn: &mut SqliteConnection, subject: &str, id: &str, now: &str) -> Result<SetAside, IdeaError> {
	let marked = sqlx::query!(
		"INSERT INTO idea_set_aside (subject_id, idea_id, stored_at)
		 SELECT subject_id, idea_id, ?3 FROM idea_revision WHERE subject_id = ?1 AND idea_id = ?2 AND revision = 1
		 ON CONFLICT (subject_id, idea_id) DO NOTHING",
		subject,
		id,
		now
	)
	.execute(&mut *conn)
	.await?
	.rows_affected();
	if marked > 0 {
		return Ok(SetAside::Marked);
	}
	Ok(if latest_revision(conn, subject, id).await?.is_some() {
		SetAside::AlreadyMarked
	} else {
		SetAside::NotHeld
	})
}

#[cfg(test)]
mod tests;

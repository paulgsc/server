//! The owner's voice notes (paulgsc/server#460), audio included: what the
//! phone sends home so home can keep each note and transcribe it.
//!
//! `20261025000100_create_voice_note.up.sql` records the design; in short:
//!
//! - **The one place home holds audio** (docs/identity.md invariant 23). A
//!   note is described in `voice_note` and its bytes sit beside it in
//!   `voice_note_audio`, written together in one transaction, so a read of
//!   what home holds never pulls audio.
//! - **Append-only, and kept for good.** [`put_note`] stores a note under its
//!   id, or nothing when that id already holds the same note; other bytes or
//!   another description under a held id are refused
//!   ([`VoiceError::Rewritten`]), never stored. No row is ever updated (the
//!   triggers abort an `UPDATE`), and nothing here deletes, a transcribed
//!   note included. Account deletion is the one delete: its generic sweep of
//!   `SUBJECT_SCOPED_TABLES`, and the migration's trigger on `account`.
//! - **Checked before it is kept.** A write passes [`describe`]: an id, what
//!   the note was said for, when, a container the transcriber reads, and a
//!   size within [`NOTE_CEILING`].
//! - **A game bite is a voice note** (paulgsc/some-ui#2074, "Decided" 1,
//!   2026-10-26). Said for [`Of::Game`], its `ref` is the game's id, and what
//!   it was said about and what the screen showed ([`Bite`]) is `game_bite`'s
//!   row beside it, written in the same transaction as the note and its
//!   audio, compared on a replay as the note is, and never updated. Only a
//!   game note carries a bite, and every game note carries one.
//! - **Per subject, and only per subject.** Every query takes the subject and
//!   filters on it; nothing reads across subjects.
//! - **No background writer of a note.** The one caller of [`put_note`] is
//!   `file_host`'s `handlers::voice` (`PUT /api/v1/voice-notes/:id`). A
//!   `git grep -n 'voice_note\b\|voice_note_audio\|game_bite' -- '*.rs'`
//!   shows no other writer.
//!
//! What home heard in each note is [`transcript`]'s: `voice_transcript`,
//! written only by `file_host`'s `transcribe` worker, which reads a note's
//! audio and never writes it.

pub mod transcript;

use chrono::{DateTime, SecondsFormat, Utc};
use serde::Serialize;
use sha2::{Digest as _, Sha256};
use sqlx::SqliteConnection;

/// A field and why it was refused.
pub type Problem = (&'static str, &'static str);

/// The most bytes one note may hold.
///
/// About four minutes at the phone's 32 kbps: four times its own one-minute
/// cap (`SOUNDBITE_TAKE_MAX_BYTES` in paulgsc/some-ui, 240,000 bytes), and
/// under axum's default body limit.
pub const NOTE_CEILING: usize = 1024 * 1024;

const MAX_ID_CHARS: usize = 64;
const MAX_REF_CHARS: usize = 200;

/// What a note was said for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Of {
	/// A study session's reflection.
	Reflection,
	/// A Jobs pick or scout.
	Jobs,
	/// A drama prompt's next scene, said aloud.
	Scene,
	/// A game bite: said while watching an NFL game, about one scenario
	/// (paulgsc/some-ui#2074). Its `ref` is the game's id, and the bite's own
	/// description is [`Bite`].
	Game,
}

impl Of {
	const ALL: [Self; 4] = [Self::Reflection, Self::Jobs, Self::Scene, Self::Game];

	/// The wire and stored name.
	#[must_use]
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::Reflection => "reflection",
			Self::Jobs => "jobs",
			Self::Scene => "scene",
			Self::Game => "game",
		}
	}

	/// The kind `name` names, if any.
	#[must_use]
	pub fn named(name: &str) -> Option<Self> {
		Self::ALL.into_iter().find(|of| of.as_str() == name)
	}
}

/// How a bite's scenario came to be said.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum How {
	/// The screen offered the scenario.
	Offered,
	/// The owner said what just happened.
	JustHappened,
	/// Said before any scenario was chosen: the bite names none.
	SaidFirst,
	/// Chosen from the whole set.
	EveryScenario,
}

impl How {
	const ALL: [Self; 4] = [Self::Offered, Self::JustHappened, Self::SaidFirst, Self::EveryScenario];

	/// The wire and stored name.
	#[must_use]
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::Offered => "offered",
			Self::JustHappened => "just-happened",
			Self::SaidFirst => "said-first",
			Self::EveryScenario => "every-scenario",
		}
	}

	/// The way `name` names, if any.
	#[must_use]
	pub fn named(name: &str) -> Option<Self> {
		Self::ALL.into_iter().find(|how| how.as_str() == name)
	}
}

/// How much the screen offered while a bite was said.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
	Light,
	Standard,
	Full,
}

impl Mode {
	const ALL: [Self; 3] = [Self::Light, Self::Standard, Self::Full];

	/// The wire and stored name.
	#[must_use]
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::Light => "light",
			Self::Standard => "standard",
			Self::Full => "full",
		}
	}

	/// The mode `name` names, if any.
	#[must_use]
	pub fn named(name: &str) -> Option<Self> {
		Self::ALL.into_iter().find(|mode| mode.as_str() == name)
	}
}

/// What a game bite was said about, and what the screen showed: `game_bite`'s
/// row beside the note, whose `ref` is the game's id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Bite {
	/// The scenario's id as the phone's copy of the set names it
	/// (`in.third-long`); `None` exactly when the bite was said first. See
	/// [`SaidBite::scenario`].
	pub scenario: Option<String>,
	/// How the scenario came to be said.
	pub how: How,
	/// Which offer the screen was showing, when it showed one.
	pub seq: Option<i64>,
	/// The live strip as the screen showed it.
	pub strip: String,
	/// The line of context the screen showed, if any.
	pub context: Option<String>,
	/// Whether the owner said they were watching the game.
	pub watching: bool,
	/// When the owner started saying it, on the phone's clock, as ISO-8601
	/// UTC to the millisecond.
	pub started: String,
	/// Home's clock minus the phone's, in milliseconds, when known. Not
	/// compared on a replay: see [`SaidBite::offset`].
	pub offset: Option<i64>,
	/// How much the screen offered.
	pub mode: Mode,
	/// The version of the scenario set it was said under.
	pub catalog: i64,
}

/// The containers the phone records in, and the transcriber reads.
pub const CONTENT_TYPES: [&str; 3] = ["audio/webm", "audio/ogg", "audio/mp4"];

/// A note as home describes it: everything but its bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Note {
	/// The phone's id for the note.
	pub id: String,
	/// What it was said for.
	pub of: Of,
	/// Which one: the session's id, the pick's or scout's id, or the drama
	/// prompt's stamp, as the phone names it.
	#[serde(rename = "ref")]
	pub of_ref: String,
	/// When the phone recorded it, as ISO-8601 UTC to the millisecond.
	pub at: String,
	/// Its container, one of [`CONTENT_TYPES`].
	pub content_type: &'static str,
	/// Its length in bytes.
	pub size: i64,
	/// The lowercase hex SHA-256 of its bytes.
	pub sha256: String,
	/// For a note said for [`Of::Game`], the bite; for any other, `None`,
	/// and absent from the wire.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub bite: Option<Bite>,
}

/// What the phone says about a note it sends, before home checks it.
#[derive(Debug, Clone, Copy)]
pub struct Said<'a> {
	pub of: Option<&'a str>,
	pub of_ref: Option<&'a str>,
	pub at: Option<&'a str>,
	/// The request's `Content-Type`, parameters and all.
	pub content_type: Option<&'a str>,
	/// What the phone says about a game bite: required for [`Of::Game`]
	/// (all but the fields a bite may lack) and refused for any other.
	pub bite: SaidBite<'a>,
}

/// What the phone says about a game bite, before home checks it: the query's
/// bite fields, each named as here. This is the one full list; the handler,
/// the route and docs/identity.md point to it.
///
/// `how`, `strip`, `watching`, `started`, `mode` and `catalog` are required;
/// `scenario`, `seq`, `context` and `offset` are said when the bite has them.
/// `strip` and `context` are the game situation the screen showed, not what
/// was said, and travel in the request's URL (docs/identity.md invariant 23).
#[derive(Debug, Clone, Copy, Default)]
pub struct SaidBite<'a> {
	/// The scenario's id as the phone's copy of the scenario set names it
	/// (`in.third-long`), absent exactly when said first. Checked for shape
	/// only, never against `game_scenarios`' set: a bite said under a newer
	/// set than home's is still kept.
	pub scenario: Option<&'a str>,
	/// `offered`, `just-happened`, `said-first` or `every-scenario`.
	pub how: Option<&'a str>,
	/// Which offer the screen was showing, a whole number from 0.
	pub seq: Option<&'a str>,
	/// The live strip as the screen showed it, empty included.
	pub strip: Option<&'a str>,
	/// The line of context the screen showed; empty is the same as absent.
	pub context: Option<&'a str>,
	/// `1` (watching) or `0`.
	pub watching: Option<&'a str>,
	/// When the owner started saying it, on the phone's clock (RFC 3339).
	pub started: Option<&'a str>,
	/// Home's clock minus the phone's, in whole milliseconds, once known.
	///
	/// **Not part of a bite's identity.** A retry may learn the offset after
	/// the first try (absent, then said) or learn a better one, and a bite
	/// refused for that would be stranded on the phone. So a replay is
	/// compared without it: the first stored value stays, and a replay that
	/// differs only here is [`Change::Unchanged`].
	pub offset: Option<&'a str>,
	/// `light`, `standard` or `full`.
	pub mode: Option<&'a str>,
	/// The scenario set's version, a whole number from 1.
	pub catalog: Option<&'a str>,
}

impl SaidBite<'_> {
	/// The fields said, by their query names.
	fn said(&self) -> impl Iterator<Item = &'static str> + '_ {
		[
			("scenario", self.scenario),
			("how", self.how),
			("seq", self.seq),
			("strip", self.strip),
			("context", self.context),
			("watching", self.watching),
			("started", self.started),
			("offset", self.offset),
			("mode", self.mode),
			("catalog", self.catalog),
		]
		.into_iter()
		.filter_map(|(field, value)| value.map(|_| field))
	}
}

/// The most characters a bite's strip or context line may hold.
pub const MAX_LINE_CHARS: usize = 500;
/// The most a phone's clock may be off home's, either way: a day.
const MAX_OFFSET_MS: i64 = 86_400_000;
const MAX_SCENARIO_CHARS: usize = 64;

/// `at` as ISO-8601 UTC to the millisecond, if it is an RFC 3339 instant.
fn instant(at: &str) -> Option<String> {
	DateTime::parse_from_rfc3339(at)
		.ok()
		.map(|at| at.with_timezone(&Utc).to_rfc3339_opts(SecondsFormat::Millis, true))
}

/// A content type's container without its parameters (`audio/webm;codecs=opus`
/// is `audio/webm`), if it is one the transcriber reads.
fn container(content_type: &str) -> Option<&'static str> {
	let base = content_type.split(';').next().unwrap_or_default().trim();
	CONTENT_TYPES.into_iter().find(|known| known.eq_ignore_ascii_case(base))
}

/// Whether `text` is a non-empty name of at most `max` characters, each a
/// letter, digit, `-`, `_`, `.` or `:`, as the phone's ids and stamps are.
fn is_name(text: &str, max: usize) -> bool {
	!text.is_empty() && text.chars().count() <= max && text.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':'))
}

/// `text` as a whole number of at most `max`, if it is one: ASCII digits
/// only, with a leading `-` when `signed`.
fn whole(text: &str, signed: bool, max: i64) -> Option<i64> {
	let digits = if signed { text.strip_prefix('-').unwrap_or(text) } else { text };
	if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
		return None;
	}
	let value: i64 = text.parse().ok()?;
	(value.abs() <= max).then_some(value)
}

/// Whether `text` is a line of at most [`MAX_LINE_CHARS`] characters, none a
/// control character.
fn is_line(text: &str) -> bool {
	text.chars().count() <= MAX_LINE_CHARS && !text.chars().any(char::is_control)
}

/// The bite the phone `said`, or `None` with its problems pushed.
fn describe_bite(said: SaidBite<'_>, problems: &mut Vec<Problem>) -> Option<Bite> {
	let before = problems.len();
	let how = said.how.and_then(How::named);
	if how.is_none() {
		problems.push(("how", "not how a bite was said: offered, just-happened, said-first or every-scenario"));
	}
	let scenario = match (how, said.scenario) {
		(Some(How::SaidFirst), Some(_)) => {
			problems.push(("scenario", "a bite said first names no scenario"));
			None
		}
		(Some(How::SaidFirst) | None, None) => None,
		(_, Some(scenario)) if is_name(scenario, MAX_SCENARIO_CHARS) => Some(scenario.to_owned()),
		(_, Some(_)) => {
			problems.push(("scenario", "not a scenario id: 1 to 64 letters, digits, '-', '_', '.' or ':'"));
			None
		}
		(Some(_), None) => {
			problems.push(("scenario", "missing: a bite not said first names its scenario"));
			None
		}
	};
	let seq = said.seq.and_then(|seq| {
		let parsed = whole(seq, false, i64::from(u32::MAX));
		if parsed.is_none() {
			problems.push(("seq", "not an offer's number: a whole number from 0"));
		}
		parsed
	});
	let strip = said.strip.filter(|strip| is_line(strip));
	if strip.is_none() {
		problems.push(("strip", "not the live strip: at most 500 characters, no control characters"));
	}
	// An empty context is no context: a bite is never refused for it.
	let context = match said.context.filter(|context| !context.is_empty()) {
		None => None,
		Some(context) if is_line(context) => Some(context.to_owned()),
		Some(_) => {
			problems.push(("context", "not a context line: at most 500 characters, no control characters"));
			None
		}
	};
	let watching = match said.watching {
		Some("1") => Some(true),
		Some("0") => Some(false),
		_ => None,
	};
	if watching.is_none() {
		problems.push(("watching", "not 1 (watching) or 0 (not)"));
	}
	let started = said.started.and_then(instant);
	if started.is_none() {
		problems.push(("started", "not an RFC 3339 instant"));
	}
	let offset = said.offset.and_then(|offset| {
		let parsed = whole(offset, true, MAX_OFFSET_MS);
		if parsed.is_none() {
			problems.push(("offset", "not a clock offset: whole milliseconds, at most a day either way"));
		}
		parsed
	});
	let mode = said.mode.and_then(Mode::named);
	if mode.is_none() {
		problems.push(("mode", "not a mode: light, standard or full"));
	}
	let catalog = said.catalog.and_then(|catalog| whole(catalog, false, i64::from(u32::MAX))).filter(|catalog| *catalog >= 1);
	if catalog.is_none() {
		problems.push(("catalog", "not a catalogue version: a whole number from 1"));
	}
	match (how, strip, watching, started, mode, catalog) {
		(Some(how), Some(strip), Some(watching), Some(started), Some(mode), Some(catalog)) if problems.len() == before => Some(Bite {
			scenario,
			how,
			seq,
			strip: strip.to_owned(),
			context,
			watching,
			started,
			offset,
			mode,
			catalog,
		}),
		_ => None,
	}
}

/// The note `body` is, as the phone `said`, under id `id`; or every problem
/// with it, by field.
///
/// # Errors
/// Every refused field: `id`, `of`, `ref`, `at`, `contentType` or `body`,
/// then, for a game bite, any of its fields that is missing or wrong, or, for
/// any other note, each bite field said at all (only a game bite carries one).
pub fn describe(id: &str, said: Said<'_>, body: &[u8]) -> Result<Note, Vec<Problem>> {
	let mut problems: Vec<Problem> = Vec::new();
	if !is_name(id, MAX_ID_CHARS) {
		problems.push(("id", "not a note id: 1 to 64 letters, digits, '-', '_', '.' or ':'"));
	}
	let of = said.of.and_then(Of::named);
	if of.is_none() {
		problems.push(("of", "not what a note is said for: reflection, jobs, scene or game"));
	}
	let of_ref = said.of_ref.filter(|of_ref| is_name(of_ref, MAX_REF_CHARS));
	if of_ref.is_none() {
		problems.push(("ref", "not which one: 1 to 200 letters, digits, '-', '_', '.' or ':'"));
	}
	let at = said.at.and_then(instant);
	if at.is_none() {
		problems.push(("at", "not an RFC 3339 instant"));
	}
	let content_type = said.content_type.and_then(container);
	if content_type.is_none() {
		problems.push(("contentType", "not audio/webm, audio/ogg or audio/mp4"));
	}
	if body.is_empty() {
		problems.push(("body", "empty: a note holds audio"));
	} else if body.len() > NOTE_CEILING {
		problems.push(("body", "larger than a note may be (1 MiB)"));
	}
	let size = i64::try_from(body.len()).ok();
	let bite = match of {
		Some(Of::Game) => describe_bite(said.bite, &mut problems),
		Some(_) => {
			for field in said.bite.said() {
				problems.push((field, "not said for a game: only a game bite carries it"));
			}
			None
		}
		None => None,
	};
	match (of, of_ref, at, content_type, size) {
		(Some(of), Some(of_ref), Some(at), Some(content_type), Some(size)) if problems.is_empty() => Ok(Note {
			id: id.to_owned(),
			of,
			of_ref: of_ref.to_owned(),
			at,
			content_type,
			size,
			sha256: hex::encode(Sha256::digest(body)),
			bite,
		}),
		_ => Err(problems),
	}
}

/// What a put did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
	/// Home did not hold it: it is stored now.
	Added,
	/// Home already holds this very note: nothing written.
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

/// Why a put wrote nothing.
#[derive(Debug)]
pub enum VoiceError {
	/// The id already holds a different note, which is never replaced.
	Rewritten,
	/// A stored description this crate wrote does not read back.
	Unreadable,
	Storage(sqlx::Error),
}

impl std::fmt::Display for VoiceError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::Rewritten => f.write_str(REWRITTEN),
			Self::Unreadable => f.write_str("a stored voice note does not read back"),
			Self::Storage(err) => write!(f, "database error: {err}"),
		}
	}
}

impl std::error::Error for VoiceError {}

impl From<sqlx::Error> for VoiceError {
	fn from(err: sqlx::Error) -> Self {
		Self::Storage(err)
	}
}

/// What [`VoiceError::Rewritten`] says.
pub const REWRITTEN: &str = "this note id already holds a different note: voice notes are append-only";

/// Stores `note`, whose bytes are `body`, for `subject` as of `now`: its
/// description and its audio together.
///
/// Run it in a `BEGIN IMMEDIATE` transaction, so the read that decides and
/// both writes see one snapshot, and land together or not at all.
///
/// # Errors
/// [`VoiceError::Rewritten`] when the note's id already holds other bytes or
/// another description (a bite's offset aside: see [`SaidBite::offset`]),
/// [`VoiceError::Unreadable`] for a held description
/// that does not read back, and [`VoiceError::Storage`] for any `sqlx`
/// failure. A refusal is decided before anything is written.
pub async fn put_note(conn: &mut SqliteConnection, subject: &str, note: &Note, body: &[u8], now: &str) -> Result<Change, VoiceError> {
	let standing = sqlx::query!(
		r#"SELECT of_kind, of_ref, recorded_at, content_type, size, sha256 FROM voice_note WHERE subject_id = ?1 AND note_id = ?2"#,
		subject,
		note.id
	)
	.fetch_optional(&mut *conn)
	.await?;
	if let Some(standing) = standing {
		let of = Of::named(&standing.of_kind).ok_or(VoiceError::Unreadable)?;
		let bite = if of == Of::Game {
			let mut held = held_bite(conn, subject, &note.id).await?;
			// The offset is no part of a bite's identity (`SaidBite::offset`):
			// the stored one stays, and a replay is compared without it.
			if let Some(said) = &note.bite {
				held.offset = said.offset;
			}
			Some(held)
		} else {
			None
		};
		let held = Note {
			id: note.id.clone(),
			of,
			of_ref: standing.of_ref,
			at: standing.recorded_at,
			content_type: container(&standing.content_type).ok_or(VoiceError::Unreadable)?,
			size: standing.size,
			sha256: standing.sha256,
			bite,
		};
		return if held == *note { Ok(Change::Unchanged) } else { Err(VoiceError::Rewritten) };
	}
	let of = note.of.as_str();
	sqlx::query!(
		"INSERT INTO voice_note (subject_id, note_id, of_kind, of_ref, recorded_at, content_type, size, sha256, stored_at)
		 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
		subject,
		note.id,
		of,
		note.of_ref,
		note.at,
		note.content_type,
		note.size,
		note.sha256,
		now
	)
	.execute(&mut *conn)
	.await?;
	sqlx::query!("INSERT INTO voice_note_audio (subject_id, note_id, body) VALUES (?1, ?2, ?3)", subject, note.id, body)
		.execute(&mut *conn)
		.await?;
	if let Some(bite) = &note.bite {
		let (how, mode, watching) = (bite.how.as_str(), bite.mode.as_str(), i64::from(bite.watching));
		sqlx::query!(
			"INSERT INTO game_bite (subject_id, note_id, scenario_id, how, offer_seq, live_strip, context_line, watching,
			                        started_at, clock_offset_ms, mode, catalog_version)
			 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
			subject,
			note.id,
			bite.scenario,
			how,
			bite.seq,
			bite.strip,
			bite.context,
			watching,
			bite.started,
			bite.offset,
			mode,
			bite.catalog
		)
		.execute(&mut *conn)
		.await?;
	}
	Ok(Change::Added)
}

/// The bite held beside `subject`'s game note `note_id`.
///
/// # Errors
/// [`VoiceError::Unreadable`] when the note has no bite, or one that does not
/// read back as this crate wrote it, and [`VoiceError::Storage`] for any
/// `sqlx` failure.
async fn held_bite(conn: &mut SqliteConnection, subject: &str, note_id: &str) -> Result<Bite, VoiceError> {
	let row = sqlx::query!(
		r#"SELECT scenario_id, how, offer_seq, live_strip, context_line, watching, started_at, clock_offset_ms, mode, catalog_version
		   FROM game_bite WHERE subject_id = ?1 AND note_id = ?2"#,
		subject,
		note_id
	)
	.fetch_optional(&mut *conn)
	.await?
	.ok_or(VoiceError::Unreadable)?;
	Ok(Bite {
		scenario: row.scenario_id,
		how: How::named(&row.how).ok_or(VoiceError::Unreadable)?,
		seq: row.offer_seq,
		strip: row.live_strip,
		context: row.context_line,
		watching: row.watching != 0,
		started: row.started_at,
		offset: row.clock_offset_ms,
		mode: Mode::named(&row.mode).ok_or(VoiceError::Unreadable)?,
		catalog: row.catalog_version,
	})
}

#[cfg(test)]
mod tests {
	use super::{describe, put_note, Change, Note, Of, Said, SaidBite, VoiceError, MAX_LINE_CHARS, NOTE_CEILING};
	use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};

	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");
	const PREVIOUS_MIGRATION: i64 = 20_261_023_000_200;
	/// The migration before `game` was a note's kind.
	const BEFORE_GAME: i64 = 20_261_026_000_300;
	const NOW: &str = "2026-10-25T03:00:00.000Z";
	const A: &str = "subject-a";
	const B: &str = "subject-b";

	async fn pool() -> SqlitePool {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();
		pool
	}

	fn said(at: &str) -> Said<'_> {
		Said {
			of: Some("reflection"),
			of_ref: Some("session-7f2a"),
			at: Some(at),
			content_type: Some("audio/webm;codecs=opus"),
			bite: SaidBite::default(),
		}
	}

	/// What the phone says about a bite on game `401772981`, every field said.
	fn said_bite() -> SaidBite<'static> {
		SaidBite {
			scenario: Some("in.third-long"),
			how: Some("offered"),
			seq: Some("12"),
			strip: Some("KC 17 – BUF 14 · Q3 4:12 · 3rd & 8 at BUF 41 ✓"),
			context: Some("Mahomes 9/14 on third down"),
			watching: Some("1"),
			started: Some("2026-10-26T13:41:07.250-07:00"),
			offset: Some("-1350"),
			mode: Some("standard"),
			catalog: Some("1"),
		}
	}

	fn said_game(bite: SaidBite<'_>) -> Said<'_> {
		Said {
			of: Some("game"),
			of_ref: Some("401772981"),
			at: Some("2026-10-26T13:41:09-07:00"),
			content_type: Some("audio/mp4"),
			bite,
		}
	}

	fn game(id: &str, body: &[u8]) -> Note {
		describe(id, said_game(said_bite()), body).unwrap()
	}

	fn fields(problems: Vec<(&'static str, &'static str)>) -> Vec<&'static str> {
		problems.into_iter().map(|(field, _)| field).collect()
	}

	fn note(id: &str, body: &[u8]) -> Note {
		describe(id, said("2026-10-24T20:10:00-07:00"), body).unwrap()
	}

	async fn put(pool: &SqlitePool, subject: &str, note: &Note, body: &[u8]) -> Result<Change, VoiceError> {
		let mut tx = pool.begin().await.unwrap();
		let change = put_note(&mut tx, subject, note, body, NOW).await?;
		tx.commit().await.unwrap();
		Ok(change)
	}

	async fn rows(pool: &SqlitePool, table: &str) -> i64 {
		sqlx::query_scalar(&(String::from("SELECT COUNT(*) FROM ") + table)).fetch_one(pool).await.unwrap()
	}

	#[tokio::test]
	async fn the_migration_round_trips() {
		let pool = pool().await;
		let tables = || sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name IN ('voice_note', 'voice_note_audio', 'game_bite')");
		assert_eq!(tables().fetch_one(&pool).await.unwrap(), 3);
		MIGRATOR.undo(&pool, PREVIOUS_MIGRATION).await.unwrap();
		assert_eq!(tables().fetch_one(&pool).await.unwrap(), 0, "down drops them");
		MIGRATOR.run(&pool).await.unwrap();
		assert_eq!(tables().fetch_one(&pool).await.unwrap(), 3, "up recreates them");
	}

	/// The columns the migration describes and no others: a new one is a
	/// decision about what the server learns from a note.
	#[tokio::test]
	async fn the_tables_hold_exactly_their_described_columns() {
		let pool = pool().await;
		for (table, expected) in [
			(
				"voice_note",
				&["subject_id", "note_id", "of_kind", "of_ref", "recorded_at", "content_type", "size", "sha256", "stored_at"][..],
			),
			("voice_note_audio", &["subject_id", "note_id", "body"][..]),
			(
				"game_bite",
				&[
					"subject_id",
					"note_id",
					"scenario_id",
					"how",
					"offer_seq",
					"live_strip",
					"context_line",
					"watching",
					"started_at",
					"clock_offset_ms",
					"mode",
					"catalog_version",
				][..],
			),
		] {
			let columns: Vec<String> = sqlx::query_scalar("SELECT name FROM pragma_table_info(?1) ORDER BY cid")
				.bind(table)
				.fetch_all(&pool)
				.await
				.unwrap();
			assert_eq!(columns, expected, "{table}");
		}
	}

	#[test]
	fn a_note_is_described_as_home_keeps_it() {
		let body = b"opus frames";
		let held = note("n1", body);
		assert_eq!(held.of, Of::Reflection);
		assert_eq!(held.at, "2026-10-25T03:10:00.000Z", "the instant in UTC, to the millisecond");
		assert_eq!(held.content_type, "audio/webm", "the container, without its parameters");
		assert_eq!(held.size, 11);
		assert_eq!(held.sha256.len(), 64);
		assert!(held.sha256.bytes().all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()));
	}

	#[test]
	fn a_refused_note_names_every_field() {
		let nothing = Said {
			of: None,
			of_ref: None,
			at: None,
			content_type: None,
			bite: SaidBite::default(),
		};
		let fields: Vec<&str> = describe("", nothing, b"").unwrap_err().into_iter().map(|(field, _)| field).collect();
		assert_eq!(fields, ["id", "of", "ref", "at", "contentType", "body"]);

		let wrong = Said {
			of: Some("diary"),
			of_ref: Some("has a space"),
			at: Some("yesterday"),
			content_type: Some("video/webm"),
			bite: SaidBite::default(),
		};
		let fields: Vec<&str> = describe("a/b", wrong, &vec![0; NOTE_CEILING + 1])
			.unwrap_err()
			.into_iter()
			.map(|(field, _)| field)
			.collect();
		assert_eq!(fields, ["id", "of", "ref", "at", "contentType", "body"]);

		for content_type in ["audio/webm", "AUDIO/OGG; codecs=opus", "audio/mp4"] {
			let ok = Said {
				content_type: Some(content_type),
				..said("2026-10-25T03:10:00Z")
			};
			assert!(describe("n1", ok, b"x").is_ok(), "{content_type}");
		}
		assert!(
			describe("n1", said("2026-10-25T03:10:00Z"), &vec![0; NOTE_CEILING]).is_ok(),
			"the ceiling itself is allowed"
		);
	}

	#[tokio::test]
	async fn a_note_is_stored_once_and_never_replaced() {
		let pool = pool().await;
		let first = note("n1", b"take one");
		assert_eq!(put(&pool, A, &first, b"take one").await.unwrap(), Change::Added);
		assert_eq!(put(&pool, A, &first, b"take one").await.unwrap(), Change::Unchanged, "the same note writes nothing");

		let other_bytes = note("n1", b"take two");
		assert!(
			matches!(put(&pool, A, &other_bytes, b"take two").await, Err(VoiceError::Rewritten)),
			"other bytes under a held id"
		);
		let mut other_ref = first.clone();
		other_ref.of_ref = String::from("session-other");
		assert!(
			matches!(put(&pool, A, &other_ref, b"take one").await, Err(VoiceError::Rewritten)),
			"another description under a held id"
		);
		assert_eq!((rows(&pool, "voice_note").await, rows(&pool, "voice_note_audio").await), (1, 1));

		assert_eq!(put(&pool, B, &other_bytes, b"take two").await.unwrap(), Change::Added, "B's ids are B's");

		for update in ["UPDATE voice_note SET of_ref = 'x'", "UPDATE voice_note_audio SET body = x'00'"] {
			let refused = sqlx::query(update).execute(&pool).await;
			assert!(refused.unwrap_err().to_string().contains("append-only"), "no row is ever updated: {update}");
		}
	}

	#[tokio::test]
	async fn the_audio_is_kept_byte_for_byte_beside_its_note() {
		let pool = pool().await;
		let body: Vec<u8> = (0..=255).collect();
		put(&pool, A, &note("n1", &body), &body).await.unwrap();
		let held: Vec<u8> = sqlx::query_scalar("SELECT body FROM voice_note_audio WHERE subject_id = ?1 AND note_id = 'n1'")
			.bind(A)
			.fetch_one(&pool)
			.await
			.unwrap();
		assert_eq!(held, body);
		let orphans: i64 = sqlx::query_scalar(
			"SELECT COUNT(*) FROM voice_note_audio a WHERE NOT EXISTS
			 (SELECT 1 FROM voice_note n WHERE n.subject_id = a.subject_id AND n.note_id = a.note_id)",
		)
		.fetch_one(&pool)
		.await
		.unwrap();
		assert_eq!(orphans, 0);
	}

	#[tokio::test]
	async fn a_note_whose_audio_fails_leaves_no_description() {
		let pool = pool().await;
		// Audio already held under the id with no description: the second
		// insert fails, and the transaction keeps neither half.
		sqlx::query("INSERT INTO voice_note_audio (subject_id, note_id, body) VALUES (?1, 'n1', x'00')")
			.bind(A)
			.execute(&pool)
			.await
			.unwrap();
		assert!(matches!(put(&pool, A, &note("n1", b"take"), b"take").await, Err(VoiceError::Storage(_))));
		assert_eq!(rows(&pool, "voice_note").await, 0);
	}

	#[tokio::test]
	async fn the_notes_leave_with_the_account() {
		let pool = pool().await;
		for subject in [A, B] {
			sqlx::query("INSERT INTO account (subject_id, user_handle) VALUES (?1, ?2)")
				.bind(subject)
				.bind(subject.as_bytes())
				.execute(&pool)
				.await
				.unwrap();
			put(&pool, subject, &note("n1", b"take"), b"take").await.unwrap();
			put(&pool, subject, &game("b1", b"take"), b"take").await.unwrap();
		}
		sqlx::query("DELETE FROM account WHERE subject_id = ?1").bind(A).execute(&pool).await.unwrap();
		for table in ["voice_note", "voice_note_audio", "game_bite"] {
			let left: Vec<String> = sqlx::query_scalar(&(String::from("SELECT DISTINCT subject_id FROM ") + table))
				.fetch_all(&pool)
				.await
				.unwrap();
			assert_eq!(left, [B], "{table}: A's notes went with A's account, B's stayed");
		}
	}

	/// Every field of a whole bite is checked as stored, in
	/// `a_bite_is_stored_once_with_every_field_and_never_replaced`; this is
	/// the bite that leaves out all it may.
	#[test]
	fn a_bite_is_described_as_home_keeps_it() {
		let held = game("b1", b"take");
		assert_eq!((held.of, held.of_ref.as_str(), held.content_type), (Of::Game, "401772981", "audio/mp4"));
		assert_eq!(note("n1", b"take").bite, None, "a note said for anything else has none");

		let first = SaidBite {
			scenario: None,
			how: Some("said-first"),
			seq: None,
			context: None,
			offset: None,
			strip: Some(""),
			watching: Some("0"),
			..said_bite()
		};
		let said_first = describe("b2", said_game(first), b"take").unwrap().bite.unwrap();
		assert_eq!(
			(said_first.scenario, said_first.seq, said_first.context, said_first.offset, said_first.watching),
			(None, None, None, None, false),
			"said first: no scenario, and every nullable field may be left out"
		);
		assert_eq!(said_first.strip, "", "an empty strip is what the screen showed");

		let empty_context = SaidBite { context: Some(""), ..said_bite() };
		let bite = describe("b3", said_game(empty_context), b"take").unwrap().bite.unwrap();
		assert_eq!(bite.context, None, "an empty context is no context, never a refusal");
	}

	#[test]
	fn a_game_note_missing_or_misstating_its_bite_is_refused() {
		let missing = describe("b1", said_game(SaidBite::default()), b"take").unwrap_err();
		assert_eq!(fields(missing), ["how", "strip", "watching", "started", "mode", "catalog"]);

		let wrong = SaidBite {
			scenario: Some("in third"),
			how: Some("guessed"),
			seq: Some("-1"),
			strip: Some("line\nbreak"),
			context: Some("tab\there"),
			watching: Some("true"),
			started: Some("at kickoff"),
			offset: Some("86400001"),
			mode: Some("loud"),
			catalog: Some("0"),
		};
		assert_eq!(
			fields(describe("b1", said_game(wrong), b"take").unwrap_err()),
			["how", "scenario", "seq", "strip", "context", "watching", "started", "offset", "mode", "catalog"]
		);

		let long = "x".repeat(MAX_LINE_CHARS + 1);
		let too_long = SaidBite {
			strip: Some(&long),
			context: Some(&long),
			..said_bite()
		};
		assert_eq!(fields(describe("b1", said_game(too_long), b"take").unwrap_err()), ["strip", "context"]);

		let mut scenario_for_said_first = said_bite();
		scenario_for_said_first.how = Some("said-first");
		assert_eq!(
			fields(describe("b1", said_game(scenario_for_said_first), b"take").unwrap_err()),
			["scenario"],
			"a bite said first names no scenario"
		);
		let mut no_scenario = said_bite();
		no_scenario.scenario = None;
		assert_eq!(
			fields(describe("b1", said_game(no_scenario), b"take").unwrap_err()),
			["scenario"],
			"any other bite names one"
		);
		for number in ["+1", "1.0", "1e3", " 1", "0x1", "99999999999999999999"] {
			let mut odd = said_bite();
			odd.catalog = Some(number);
			assert_eq!(fields(describe("b1", said_game(odd), b"take").unwrap_err()), ["catalog"], "{number}");
		}
	}

	#[test]
	fn a_note_said_for_anything_else_carries_no_bite() {
		for of in ["reflection", "jobs", "scene"] {
			let mut with_bite = said("2026-10-25T03:10:00Z");
			with_bite.of = Some(of);
			with_bite.bite = said_bite();
			assert_eq!(
				fields(describe("n1", with_bite, b"take").unwrap_err()),
				["scenario", "how", "seq", "strip", "context", "watching", "started", "offset", "mode", "catalog"],
				"{of}"
			);
			with_bite.bite = SaidBite {
				context: Some("one"),
				..SaidBite::default()
			};
			assert_eq!(fields(describe("n1", with_bite, b"take").unwrap_err()), ["context"], "{of}: even one");
		}
	}

	#[tokio::test]
	async fn a_bite_is_stored_once_with_every_field_and_never_replaced() {
		type Row = (Option<String>, String, Option<i64>, String, Option<String>, i64, String, Option<i64>, String, i64);
		let pool = pool().await;
		let first = game("b1", b"take one");
		assert_eq!(put(&pool, A, &first, b"take one").await.unwrap(), Change::Added);
		assert_eq!(put(&pool, A, &first, b"take one").await.unwrap(), Change::Unchanged, "the same bite writes nothing");

		let row: Row = sqlx::query_as(
			"SELECT scenario_id, how, offer_seq, live_strip, context_line, watching, started_at, clock_offset_ms, mode, catalog_version
			 FROM game_bite WHERE subject_id = ?1 AND note_id = 'b1'",
		)
		.bind(A)
		.fetch_one(&pool)
		.await
		.unwrap();
		assert_eq!(
			row,
			(
				Some(String::from("in.third-long")),
				String::from("offered"),
				Some(12),
				String::from("KC 17 – BUF 14 · Q3 4:12 · 3rd & 8 at BUF 41 ✓"),
				Some(String::from("Mahomes 9/14 on third down")),
				1,
				String::from("2026-10-26T20:41:07.250Z"),
				Some(-1350),
				String::from("standard"),
				1
			),
			"every field, the strip and context byte for byte"
		);
		let of_kind: String = sqlx::query_scalar("SELECT of_kind FROM voice_note WHERE note_id = 'b1'").fetch_one(&pool).await.unwrap();
		assert_eq!(of_kind, "game");

		let mut other_strip = first.clone();
		if let Some(bite) = other_strip.bite.as_mut() {
			bite.strip.push('!');
		}
		let mut other_game = first.clone();
		other_game.of_ref = String::from("401772982");
		for (changed, what) in [(other_strip, "another strip"), (other_game, "another game")] {
			assert!(matches!(put(&pool, A, &changed, b"take one").await, Err(VoiceError::Rewritten)), "{what} under a held id");
		}
		assert!(
			matches!(put(&pool, A, &game("b1", b"take two"), b"take two").await, Err(VoiceError::Rewritten)),
			"other bytes under a held id"
		);
		assert!(
			matches!(put(&pool, A, &note("b1", b"take one"), b"take one").await, Err(VoiceError::Rewritten)),
			"the same audio said for something else"
		);
		assert_eq!(
			(rows(&pool, "voice_note").await, rows(&pool, "voice_note_audio").await, rows(&pool, "game_bite").await),
			(1, 1, 1)
		);
		assert!(
			matches!(put(&pool, A, &game("n1", b"take"), b"take").await, Ok(Change::Added)),
			"a game note beside the rest"
		);

		let refused = sqlx::query("UPDATE game_bite SET live_strip = 'x'").execute(&pool).await;
		assert!(refused.unwrap_err().to_string().contains("append-only"), "no bite is ever updated");
	}

	/// A retry whose clock offset became known, or changed, is the same bite:
	/// the first stored offset stays, and nothing is refused or stranded.
	#[tokio::test]
	async fn a_bite_replayed_with_another_offset_is_unchanged() {
		let pool = pool().await;
		let offset_of = |id: &'static str| {
			let pool = pool.clone();
			async move {
				sqlx::query_scalar::<_, Option<i64>>("SELECT clock_offset_ms FROM game_bite WHERE note_id = ?1")
					.bind(id)
					.fetch_one(&pool)
					.await
					.unwrap()
			}
		};
		let with_offset = |id: &str, offset: Option<&'static str>| {
			let mut bite = said_bite();
			bite.offset = offset;
			describe(id, said_game(bite), b"take").unwrap()
		};

		assert_eq!(put(&pool, A, &with_offset("b1", None), b"take").await.unwrap(), Change::Added);
		for later in [Some("-1350"), Some("20"), None] {
			assert_eq!(put(&pool, A, &with_offset("b1", later), b"take").await.unwrap(), Change::Unchanged, "{later:?}");
		}
		assert_eq!(offset_of("b1").await, None, "the first stored offset stays: unknown");

		assert_eq!(put(&pool, A, &with_offset("b2", Some("-1350")), b"take").await.unwrap(), Change::Added);
		for later in [Some("20"), None] {
			assert_eq!(put(&pool, A, &with_offset("b2", later), b"take").await.unwrap(), Change::Unchanged, "{later:?}");
		}
		assert_eq!(offset_of("b2").await, Some(-1350), "the first stored offset stays");
		assert_eq!(rows(&pool, "game_bite").await, 2);
	}

	#[tokio::test]
	async fn a_bite_whose_write_fails_leaves_no_note() {
		let pool = pool().await;
		// A bite already held under the id with no note: its insert fails, and
		// the transaction keeps neither the note nor its audio.
		sqlx::query(
			"INSERT INTO game_bite (subject_id, note_id, how, live_strip, watching, started_at, mode, catalog_version)
			 VALUES (?1, 'b1', 'said-first', '', 0, 'then', 'light', 1)",
		)
		.bind(A)
		.execute(&pool)
		.await
		.unwrap();
		assert!(matches!(put(&pool, A, &game("b1", b"take"), b"take").await, Err(VoiceError::Storage(_))));
		assert_eq!((rows(&pool, "voice_note").await, rows(&pool, "voice_note_audio").await), (0, 0));
	}

	/// The rule between two columns, which the schema holds on its own; each
	/// column's own values are `describe`'s, tested above.
	#[tokio::test]
	async fn the_schema_refuses_a_bite_that_breaks_its_rules() {
		let pool = pool().await;
		for (bad, why) in [("NULL, 'offered'", "a scenario unless said first"), ("'in.x', 'said-first'", "no scenario when said first")] {
			let insert = String::from(
				"INSERT INTO game_bite (subject_id, note_id, scenario_id, how, watching, mode, catalog_version, live_strip, started_at)
				 VALUES ('s', 'b', ",
			) + bad
				+ ", 0, 'light', 1, '', 'then')";
			assert!(sqlx::query(&insert).execute(&pool).await.is_err(), "{why}");
		}
	}

	/// The rebuild that widened `voice_note` for `game` keeps every note held
	/// before it, still append-only, still indexed for the transcriber, and
	/// still leaving with the account.
	#[tokio::test]
	async fn the_rebuilt_notes_keep_what_was_held_before() {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();
		MIGRATOR.undo(&pool, BEFORE_GAME).await.unwrap();
		sqlx::query("INSERT INTO account (subject_id, user_handle) VALUES (?1, ?2)")
			.bind(A)
			.bind(A.as_bytes())
			.execute(&pool)
			.await
			.unwrap();
		sqlx::query(
			"INSERT INTO voice_note (subject_id, note_id, of_kind, of_ref, recorded_at, content_type, size, sha256, stored_at)
			 VALUES (?1, 'old', 'scene', 'stamp-1', 'then', 'audio/ogg', 4, 'abc', 'now')",
		)
		.bind(A)
		.execute(&pool)
		.await
		.unwrap();
		sqlx::query("INSERT INTO voice_note_audio (subject_id, note_id, body) VALUES (?1, 'old', x'01020304')")
			.bind(A)
			.execute(&pool)
			.await
			.unwrap();
		MIGRATOR.run(&pool).await.unwrap();

		let held: (String, String, String, String, String, i64, String, String) =
			sqlx::query_as("SELECT note_id, of_kind, of_ref, recorded_at, content_type, size, sha256, stored_at FROM voice_note")
				.fetch_one(&pool)
				.await
				.unwrap();
		assert_eq!(
			held,
			(
				"old".into(),
				"scene".into(),
				"stamp-1".into(),
				"then".into(),
				"audio/ogg".into(),
				4,
				"abc".into(),
				"now".into()
			)
		);
		let refused = sqlx::query("UPDATE voice_note SET of_ref = 'x'").execute(&pool).await;
		assert!(refused.unwrap_err().to_string().contains("append-only"), "still append-only");
		let index: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name = 'voice_note_by_stored_at'")
			.fetch_one(&pool)
			.await
			.unwrap();
		assert_eq!(index, 1, "the transcriber's index is back");

		// Going back refuses rather than discards once a bite is held.
		put(&pool, A, &game("b1", b"take"), b"take").await.unwrap();
		assert!(MIGRATOR.undo(&pool, BEFORE_GAME).await.is_err(), "a held bite refuses the rollback");
		assert_eq!((rows(&pool, "voice_note").await, rows(&pool, "game_bite").await), (2, 1), "and nothing went");

		sqlx::query("DELETE FROM account WHERE subject_id = ?1").bind(A).execute(&pool).await.unwrap();
		assert_eq!(rows(&pool, "voice_note").await + rows(&pool, "voice_note_audio").await, 0, "still leaves with the account");
	}
}

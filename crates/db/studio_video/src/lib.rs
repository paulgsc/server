//! A finished Studio video's DNA, queued on home (STUDIO ST33, slice 1:
//! paulgsc/some-ui#2591; the owner's decision 9 on paulgsc/some-ui#2248):
//! the template it goes through, the shape built in Studio, the takes it uses
//! by voice note id, and the data that fills it, as one record per revision
//! the phone queues.
//!
//! `20261105000200_create_studio_video_queued.up.sql` records the design; in
//! short:
//!
//! - **Append-only.** [`queue`] adds a row for a revision not yet queued, and
//!   nothing for the same DNA again; different bytes under a queued revision
//!   are refused ([`DnaError::Rewritten`]). No row is updated (the table's
//!   trigger aborts an `UPDATE`), and nothing here deletes. Account deletion
//!   is the one delete: its generic sweep of `SUBJECT_SCOPED_TABLES`, and the
//!   migration's trigger on `account`.
//! - **Kept as sent.** The DNA's bytes are stored as the phone sent them,
//!   with their SHA-256. Home checks only what [`model`] lists and reads
//!   nothing inside `shape` or `data`.
//! - **A take is a voice note, named by id.** Each take must be one of the
//!   subject's own `studio` notes home holds and not one the owner set aside,
//!   the rule an idea's answer is held to
//!   (`voice_repo::set_aside::studio_take_held`, read here and in
//!   `idea_repo`). The DNA carries no audio and no words of a take.
//! - **Queued is the only stage in this slice.** Slice 2 adds the routine
//!   that fills a DNA (`filling`, `filled`, `problem`) and slice 3 the render
//!   (`made`). Until then nothing reads the table for a routine or the
//!   mailbox.
//! - **Per subject, and only per subject.** Every query takes the subject and
//!   filters on it; nothing reads across subjects.

mod model;

use model::parse_dna;
pub use model::{Problem, DNA_CEILING, SCHEMA};
use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::SqliteConnection;

/// The most queued videos [`videos`] lists: the newest queued.
pub const VIDEOS_LISTED: i64 = 200;

/// What a write did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
	/// The revision is queued now: a row is added.
	Added,
	/// Home already holds this DNA under its revision: nothing written.
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

/// Where a queued video stands. Slice 1 knows one stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
	/// Home holds its DNA, and nothing has worked on it yet.
	Queued,
}

impl Stage {
	/// The wire name.
	#[must_use]
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::Queued => "queued",
		}
	}
}

/// Why a write wrote nothing.
#[derive(Debug)]
pub enum DnaError {
	/// The DNA breaks a rule; every problem, by field.
	Invalid(Vec<Problem>),
	/// The revision is queued already, with other bytes.
	Rewritten,
	/// These voice notes the takes name are not the subject's own `studio`
	/// notes on home, or are ones the owner set aside; by the phone's ids, in
	/// the order the takes first name them.
	TakesNotHeld(Vec<String>),
	Storage(sqlx::Error),
}

impl std::fmt::Display for DnaError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::Invalid(problems) => {
				f.write_str("not a DNA home queues:")?;
				for (field, why) in problems {
					write!(f, " {field} {why};")?;
				}
				Ok(())
			}
			Self::Rewritten => f.write_str(REWRITTEN),
			Self::TakesNotHeld(_) => f.write_str(TAKES_NOT_HELD),
			Self::Storage(err) => write!(f, "database error: {err}"),
		}
	}
}

impl std::error::Error for DnaError {}

impl From<sqlx::Error> for DnaError {
	fn from(err: sqlx::Error) -> Self {
		Self::Storage(err)
	}
}

/// What [`DnaError::Rewritten`] says.
pub const REWRITTEN: &str = "this revision is already queued with a different DNA: a queued video is append-only, so queue the next revision";

/// What [`DnaError::TakesNotHeld`] says.
pub const TAKES_NOT_HELD: &str = "a take names a voice note that is not one of this subject's studio notes on home, or was set aside: send it first";

/// A queued video as home lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Queued {
	/// The phone's id for the video.
	pub video: String,
	pub rev: i64,
	pub title: String,
	/// When the owner finished it: ISO-8601 UTC to the millisecond.
	pub queued_at: String,
	/// When it reached home.
	pub stored_at: String,
	/// [`Stage`]'s wire name.
	pub stage: &'static str,
}

/// The queued videos home lists.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Videos {
	/// Newest queued first, at most [`VIDEOS_LISTED`].
	pub videos: Vec<Queued>,
}

struct Row {
	video_id: String,
	rev: i64,
	title: String,
	queued_at: String,
	stored_at: String,
}

impl From<Row> for Queued {
	fn from(row: Row) -> Self {
		Self {
			video: row.video_id,
			rev: row.rev,
			title: row.title,
			queued_at: row.queued_at,
			stored_at: row.stored_at,
			stage: Stage::Queued.as_str(),
		}
	}
}

/// Stores the DNA in `body` as revision `rev` of `subject`'s video `video`
/// (each as the path writes it), as of `now`, and answers what it did with
/// the video as home now lists it.
///
/// The same bytes under a queued revision write nothing. A revision not yet
/// queued is stored once it holds every rule.
///
/// Run it in a `BEGIN IMMEDIATE` transaction, so the reads that decide and the
/// write see one snapshot.
///
/// # Errors
/// [`DnaError::Invalid`] naming every problem the DNA has;
/// [`DnaError::Rewritten`] for other bytes under a queued revision;
/// [`DnaError::TakesNotHeld`] naming each voice note a take names that is not
/// the subject's own `studio` note on home, or that the owner set aside; and
/// [`DnaError::Storage`] for any `sqlx` failure. A refusal is decided before
/// anything is written.
pub async fn queue(conn: &mut SqliteConnection, subject: &str, video: &str, rev: &str, body: &[u8], now: &str) -> Result<(Change, Queued), DnaError> {
	let dna = parse_dna(video, rev, body).map_err(DnaError::Invalid)?;
	let held = sqlx::query!(
		r#"SELECT dna AS "dna!: String", title AS "title!: String", queued_at AS "queued_at!: String", stored_at AS "stored_at!: String"
		   FROM studio_video_queued WHERE subject_id = ?1 AND video_id = ?2 AND rev = ?3"#,
		subject,
		dna.video,
		dna.rev
	)
	.fetch_optional(&mut *conn)
	.await?;
	if let Some(held) = held {
		if held.dna != dna.text {
			return Err(DnaError::Rewritten);
		}
		let row = Row {
			video_id: dna.video,
			rev: dna.rev,
			title: held.title,
			queued_at: held.queued_at,
			stored_at: held.stored_at,
		};
		return Ok((Change::Unchanged, row.into()));
	}
	let mut missing = Vec::new();
	for note in &dna.notes {
		if !voice_repo::set_aside::studio_take_held(conn, subject, note).await? {
			missing.push(note.clone());
		}
	}
	if !missing.is_empty() {
		return Err(DnaError::TakesNotHeld(missing));
	}
	let sha256 = hex::encode(Sha256::digest(body));
	sqlx::query!(
		"INSERT INTO studio_video_queued (subject_id, video_id, rev, title, queued_at, template_kind, template_id, template_version, dna, dna_sha256, stored_at)
		 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
		subject,
		dna.video,
		dna.rev,
		dna.title,
		dna.queued_at,
		dna.template.kind,
		dna.template.id,
		dna.template.version,
		dna.text,
		sha256,
		now
	)
	.execute(&mut *conn)
	.await?;
	let row = Row {
		video_id: dna.video,
		rev: dna.rev,
		title: dna.title,
		queued_at: dna.queued_at,
		stored_at: now.to_owned(),
	};
	Ok((Change::Added, row.into()))
}

/// The newest [`VIDEOS_LISTED`] of `subject`'s queued videos, each revision
/// queued its own entry, newest queued first.
///
/// # Errors
/// [`DnaError::Storage`] for any `sqlx` failure.
pub async fn videos(conn: &mut SqliteConnection, subject: &str) -> Result<Videos, DnaError> {
	let rows = sqlx::query_as!(
		Row,
		"SELECT video_id, rev, title, queued_at, stored_at FROM studio_video_queued
		 WHERE subject_id = ?1 ORDER BY queued_at DESC, stored_at DESC, video_id, rev DESC LIMIT ?2",
		subject,
		VIDEOS_LISTED
	)
	.fetch_all(&mut *conn)
	.await?;
	Ok(Videos {
		videos: rows.into_iter().map(Queued::from).collect(),
	})
}

#[cfg(test)]
mod tests;

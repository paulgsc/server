//! Home's bookkeeping for the interview essay routine: each final interview
//! week's record, put in the mailbox's `index` folder after each pass
//! (paulgsc/some-ui#2935, IW4), as [`super::rust`] puts Rust Rounds' weeks.
//!
//! `{"schema":"some-ui.routine-index/1","routine":"interview","asOf","weeks":[…]}`:
//! `weeks` is every final week among the [`NEWEST_WEEKS`] newest interview
//! weeks home holds, newest first, each exactly as
//! `GET /interview-weeks/:id/record` answers it
//! (`handlers::interview_weeks::InterviewRecord`, the same struct): which
//! take counted for each question and each card at the deadline, or a
//! silence. `asOf` is the latest of the listed weeks' deadlines and of when
//! home took their finals, or `null` when none is listed.
//!
//! **Final, never a draft,** by Rust Rounds' rules: a week is listed once
//! home's clock is past its `due`, and the pass makes its standing draft
//! final first (`rust_round_repo::draft::finalize`, under the deletion hold,
//! in a `BEGIN IMMEDIATE` transaction), so a week no one reads still
//! finalizes, with no clock or loop of its own: the mailbox pass that runs
//! anyway does it, at most one interval after `due`. A closed week whose send
//! still reads as a draft (one sent after `due`, which no route takes) is
//! left out and logged.
//!
//! **A view, not a record**: no table keeps it, and what home remembers is a
//! [`LastIndex`] in memory, the SHA-256 of the bytes, so a pass puts only
//! when a record changes, and once after a restart.
//!
//! **Bounded per pass:** one read of the newest [`NEWEST_WEEKS`] weeks off the
//! key's range, one of the subject's drafts, and per closed week its send; a
//! write only for a week with a draft past its deadline, once a week.
//!
//! **What it carries:** the subject's own weeks, and nothing beyond them: a
//! take is named by its voice note's id, never audio (docs/identity.md
//! invariant 23). Nothing of a record reaches a log line. A body over the
//! mailbox's ceiling drops its oldest weeks until it fits, as Rust Rounds'
//! does.

use mailbox_kit::{Mailbox, MailboxTransport};
use rust_round_repo::draft::{self, Kind};
use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::{Connection, SqlitePool};
use std::collections::HashSet;

use super::{index_name, put_index, stamp_of, Computed, LastIndex, INDEX_SCHEMA};
use crate::auth::DeletionLock;
use crate::handlers::interview_weeks::{is_closed, reads_as_named, week_record, InterviewRecord};
use crate::mailbox::pass::{storage, Failure, Outcome, MAILBOX_BODY_CEILING};

/// The routine this index is for, as its name and `routine` say.
pub const INTERVIEW: &str = "interview";

/// How many of the newest weeks a pass looks at, as for Rust Rounds
/// ([`super::rust::NEWEST_WEEKS`]).
pub const NEWEST_WEEKS: i64 = super::rust::NEWEST_WEEKS;

#[derive(Serialize)]
struct InterviewIndex<'a> {
	schema: &'static str,
	routine: &'static str,
	#[serde(rename = "asOf")]
	as_of: Option<String>,
	weeks: &'a [InterviewRecord],
}

/// The latest of each week's deadline and of when home took its final, as
/// UTC to the millisecond; `None` for no weeks.
fn as_of(weeks: &[InterviewRecord]) -> Option<String> {
	weeks
		.iter()
		.flat_map(|week| std::iter::once(week.due.as_str()).chain(week.sent.iter().map(|sent| sent.received_at.as_str())))
		.filter_map(|at| chrono::DateTime::parse_from_rfc3339(at).ok())
		.map(|at| at.with_timezone(&chrono::Utc))
		.max()
		.map(|at| at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
}

/// The index's bytes for `weeks` (newest first).
#[must_use]
pub fn interview_index(weeks: &[InterviewRecord]) -> Vec<u8> {
	let index = InterviewIndex {
		schema: INDEX_SCHEMA,
		routine: INTERVIEW,
		as_of: as_of(weeks),
		weeks,
	};
	let mut body = Vec::new();
	// Structs of strings and booleans always serialise.
	let _ = serde_json::to_writer(&mut body, &index);
	body
}

/// The bytes to put for `weeks`: every week while they fit under the
/// mailbox's ceiling, else without the oldest until they do or one is left.
fn fitted(mut weeks: Vec<InterviewRecord>) -> Vec<u8> {
	let mut body = interview_index(&weeks);
	while body.len() > MAILBOX_BODY_CEILING && weeks.len() > 1 {
		weeks.pop();
		body = interview_index(&weeks);
	}
	body
}

/// Makes each of `subject`'s newest interview weeks final once its deadline
/// has passed, and puts their records when they changed since the last index
/// the mailbox answered (the module doc). Before any week home holds is past
/// its deadline, it puts none.
///
/// # Errors
/// A [`Failure`] when the database fails or the mailbox cannot be reached;
/// nothing is remembered, so the next pass finalizes, computes and puts it
/// again.
pub(in crate::mailbox) async fn put_interview_index<T: MailboxTransport>(
	db: &SqlitePool,
	mailbox: &Mailbox<T>,
	subject: &str,
	deletion: &DeletionLock,
	last: &LastIndex,
	now: &(impl Fn() -> String + Sync),
	outcome: &mut Outcome,
) -> Result<(), Failure> {
	let at = now();
	let Some(stamp) = stamp_of(&at) else {
		return Ok(());
	};
	let mut conn = db.acquire().await.map_err(Failure::Storage)?;
	let held = mailbox_repo::weeks_named(&mut conn, subject, mailbox_repo::INTERVIEW_WEEK, NEWEST_WEEKS)
		.await
		.map_err(storage)?;
	let closed: Vec<_> = held
		.iter()
		.filter_map(|file| reads_as_named(&file.id, &file.body))
		.filter(|week| is_closed(week, &at))
		.collect();
	// No week is past its deadline yet: no index, and the routine writes none.
	if closed.is_empty() {
		return Ok(());
	}
	let drafted: HashSet<String> = draft::drafted_weeks(&mut conn, Kind::Interview, subject)
		.await
		.map_err(Failure::Storage)?
		.into_iter()
		.collect();
	let mut weeks = Vec::with_capacity(closed.len());
	for week in closed {
		if let (true, Some(window)) = (drafted.contains(&week.id), &week.window) {
			let _hold = deletion.hold().await;
			let mut tx = conn.begin_with("BEGIN IMMEDIATE").await.map_err(Failure::Storage)?;
			draft::finalize(&mut tx, Kind::Interview, subject, &week.id, &window.due, &at)
				.await
				.map_err(Failure::Storage)?;
			tx.commit().await.map_err(Failure::Storage)?;
		}
		let Some(record) = week_record(&mut conn, subject, week, &at).await.map_err(Failure::Storage)? else {
			continue;
		};
		if record.sent.as_ref().is_some_and(|sent| sent.draft) {
			tracing::error!(
				kind = "draft_after_due",
				"an interview week past its deadline still holds only a draft; it is left out of the interview index"
			);
			continue;
		}
		weeks.push(record);
	}
	drop(conn);
	let body = fitted(weeks);
	let marker = hex::encode(Sha256::digest(&body));
	if last.holds(subject, Some(&marker), |_| true) {
		return Ok(());
	}
	let computed = Computed {
		subject: subject.to_owned(),
		as_of: Some(marker),
		valid: None,
	};
	put_index(mailbox, &index_name(INTERVIEW, &stamp), body, (last, computed), outcome).await
}

#[cfg(test)]
mod tests {
	use super::{fitted, interview_index};
	use crate::handlers::interview_weeks::{CardRecord, InterviewRecord, QuestionRecord};
	use crate::handlers::rust_week_record::SentAt;
	use crate::mailbox::pass::MAILBOX_BODY_CEILING;
	use serde_json::{json, Value};

	fn week(id: &str, due: &str, question: &str) -> InterviewRecord {
		InterviewRecord {
			week: id.to_owned(),
			opens: String::from("2026-11-16T00:00:00-08:00"),
			due: due.to_owned(),
			closed: true,
			sent: Some(SentAt {
				at: String::from("2026-11-18T18:00:00.000Z"),
				received_at: String::from("2026-11-18T18:00:01.000Z"),
				draft: false,
			}),
			questions: vec![
				QuestionRecord {
					question: question.to_owned(),
					answered: true,
					take: Some(String::from("n1")),
					recorded_at: Some(String::from("2026-11-18T17:00:00.000Z")),
				},
				QuestionRecord {
					question: String::from("a-conflict"),
					answered: false,
					take: None,
					recorded_at: None,
				},
			],
			cards: vec![CardRecord {
				card: String::from("home-first"),
				read: false,
				take: None,
				recorded_at: None,
			}],
			unanswered: vec![String::from("a-conflict")],
			unread: vec![String::from("home-first")],
		}
	}

	#[test]
	fn the_index_carries_each_final_week_in_the_records_shape() {
		let weeks = [week("2026-w47", "2026-11-22T23:59:00-08:00", "about-you")];
		let index: Value = serde_json::from_slice(&interview_index(&weeks)).unwrap();
		assert_eq!(
			index,
			json!({
				"schema": "some-ui.routine-index/1",
				"routine": "interview",
				"asOf": "2026-11-23T07:59:00.000Z",
				"weeks": [{
					"week": "2026-w47",
					"opens": "2026-11-16T00:00:00-08:00",
					"due": "2026-11-22T23:59:00-08:00",
					"closed": true,
					"sent": { "at": "2026-11-18T18:00:00.000Z", "receivedAt": "2026-11-18T18:00:01.000Z", "draft": false },
					"questions": [
						{ "question": "about-you", "answered": true, "take": "n1", "recordedAt": "2026-11-18T17:00:00.000Z" },
						{ "question": "a-conflict", "answered": false, "take": null, "recordedAt": null },
					],
					"cards": [{ "card": "home-first", "read": false, "take": null, "recordedAt": null }],
					"unanswered": ["a-conflict"],
					"unread": ["home-first"],
				}],
			})
		);
		let empty: Value = serde_json::from_slice(&interview_index(&[])).unwrap();
		assert_eq!((&empty["asOf"], &empty["weeks"]), (&json!(null), &json!([])));
	}

	#[test]
	fn a_body_over_the_ceiling_drops_its_oldest_weeks_until_it_fits() {
		let big = "q".repeat(MAILBOX_BODY_CEILING / 2);
		let weeks = vec![week("2026-w47", "2026-11-22T23:59:00-08:00", &big), week("2026-w46", "2026-11-15T23:59:00-08:00", &big)];
		let index: Value = serde_json::from_slice(&fitted(weeks)).unwrap();
		let kept: Vec<&str> = index["weeks"].as_array().unwrap().iter().map(|week| week["week"].as_str().unwrap()).collect();
		assert_eq!(kept, ["2026-w47"], "the newest week is kept");
	}
}

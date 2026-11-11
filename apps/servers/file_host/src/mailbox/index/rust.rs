//! Home's bookkeeping for the Rust Rounds essay routine: each final week's
//! record, put in the mailbox's `index` folder after each pass (RR18,
//! paulgsc/some-ui#2693; the owner's "A for RR18", 2026-11-11).
//!
//! The Monday routine writes each week's essay (`rust-essay-<week>.json` in
//! `brief`), and home refuses one whose take or silence per ticket disagrees
//! with the week's final ([`rust_round_repo::essay::against`]). The routine
//! reads the final here: which take counted for each ticket at the deadline,
//! and the errors pasted in the week, which the next week is seeded from
//! (paulgsc/some-ui#2685).
//!
//! `{"schema":"some-ui.routine-index/1","routine":"rust","asOf","weeks":[…]}`:
//! `weeks` is every final week among the [`NEWEST_WEEKS`] newest weeks home
//! holds, newest first, each exactly as `GET /rust-weeks/:id/record` answers
//! it (`handlers::rust_week_record::WeekRecord`, the same struct); `asOf` is
//! the latest of the listed weeks' deadlines and of when home took their
//! finals and pastes, or `null` when none is listed.
//!
//! **Final, never a draft.** A week is listed once home's clock is past its
//! `due`. The pass makes its standing draft final first
//! ([`rust_round_repo::sent::finalize`], under the deletion hold, in a
//! `BEGIN IMMEDIATE` transaction), as the first read after the deadline
//! does, so a week no one reads still finalizes: no new clock or loop, the
//! mailbox pass that runs anyway does it, at most one interval after `due`.
//! A closed week whose send still reads as a draft after that (one sent after
//! `due`, which no route takes) is left out and logged, never put.
//!
//! **A view, not a record**, as the Jobs index is (the parent module): no
//! table keeps it, and what home remembers is a [`LastIndex`] in memory. The
//! marker is the SHA-256 of the bytes: [`rust_index`] is a pure function of
//! the records, so a pass puts only when they change (a week finalizes, a
//! paste or an open of a closed week reaches home late, a week leaves the
//! newest [`NEWEST_WEEKS`]), and once after a restart. What a final says of
//! each ticket, a take or a silence, never changes once it is final, so
//! every index that lists a week agrees with `against` on it.
//!
//! **Bounded per pass.** One read of the newest [`NEWEST_WEEKS`] weeks off the
//! key's range, one of the subject's drafts, and per closed week its send,
//! pastes and opens, each a keyed read; a write only for a week with a draft
//! past its deadline, which happens once a week.
//!
//! **What it carries:** the subject's own weeks, and nothing beyond them: a
//! take is named by its voice note's id, never audio (docs/identity.md
//! invariant 23), and a paste is the owner's own error text, which the
//! routine reads. No peer address or client id is in any record. Nothing of a
//! record reaches a log line.
//!
//! **Too big:** a body over the mailbox's ceiling drops its oldest weeks until
//! it fits; one whose newest week alone is over it is not put, and is counted
//! as refused (`too_big`), as every index is.

use mailbox_kit::{Mailbox, MailboxTransport};
use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::{Connection, SqlitePool};
use std::collections::HashSet;

use super::{index_name, put_index, stamp_of, Computed, LastIndex, INDEX_SCHEMA};
use crate::auth::DeletionLock;
use crate::handlers::rust_rounds::reads_as_named;
use crate::handlers::rust_week_record::{is_closed, week_record, WeekRecord};
use crate::mailbox::pass::{storage, Failure, Outcome, MAILBOX_BODY_CEILING};

/// The routine this index is for, as its name and `routine` say.
pub const RUST: &str = "rust";

/// How many of the newest weeks home holds a pass looks at: the week just
/// past its deadline, and three before it, so a routine that missed a Monday
/// or three still finds the week it owes.
pub const NEWEST_WEEKS: i64 = 4;

#[derive(Serialize)]
struct RustIndex<'a> {
	schema: &'static str,
	routine: &'static str,
	#[serde(rename = "asOf")]
	as_of: Option<String>,
	weeks: &'a [WeekRecord],
}

/// An instant as home writes its own: UTC, to the millisecond.
fn utc(at: &str) -> Option<chrono::DateTime<chrono::Utc>> {
	chrono::DateTime::parse_from_rfc3339(at).ok().map(|at| at.with_timezone(&chrono::Utc))
}

/// The latest of each week's deadline and of when home took its final and
/// its pastes, as UTC to the millisecond; `None` for no weeks.
fn as_of(weeks: &[WeekRecord]) -> Option<String> {
	weeks
		.iter()
		.flat_map(|week| {
			std::iter::once(week.due.as_str())
				.chain(week.sent.iter().map(|sent| sent.received_at.as_str()))
				.chain(week.pastes.iter().map(|paste| paste.received_at.as_str()))
		})
		.filter_map(utc)
		.max()
		.map(|at| at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
}

/// The index's bytes for `weeks` (newest first).
#[must_use]
pub fn rust_index(weeks: &[WeekRecord]) -> Vec<u8> {
	let index = RustIndex {
		schema: INDEX_SCHEMA,
		routine: RUST,
		as_of: as_of(weeks),
		weeks,
	};
	let mut body = Vec::new();
	// Structs of strings, numbers and booleans always serialise.
	let _ = serde_json::to_writer(&mut body, &index);
	body
}

/// The bytes to put for `weeks`: every week while they fit under the
/// mailbox's ceiling, else without the oldest until they do or one is left.
fn fitted(mut weeks: Vec<WeekRecord>) -> Vec<u8> {
	let mut body = rust_index(&weeks);
	while body.len() > MAILBOX_BODY_CEILING && weeks.len() > 1 {
		weeks.pop();
		body = rust_index(&weeks);
	}
	body
}

/// Makes each of `subject`'s newest weeks final once its deadline has passed,
/// and puts their records when they changed since the last index the mailbox
/// answered (the module doc). Before any week home holds is past its
/// deadline, it puts none.
///
/// # Errors
/// A [`Failure`] when the database fails or the mailbox cannot be reached;
/// nothing is remembered, so the next pass finalizes, computes and puts it
/// again.
pub(in crate::mailbox) async fn put_rust_index<T: MailboxTransport>(
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
	let held = mailbox_repo::newest_rust_weeks(&mut conn, subject, NEWEST_WEEKS).await.map_err(storage)?;
	let closed: Vec<_> = held
		.iter()
		.filter_map(|file| reads_as_named(&file.id, &file.body))
		.filter(|week| is_closed(week, &at))
		.collect();
	// No week is past its deadline yet: no index, and the routine writes none.
	if closed.is_empty() {
		return Ok(());
	}
	let drafted: HashSet<String> = rust_round_repo::sent::drafted_weeks(&mut conn, subject)
		.await
		.map_err(Failure::Storage)?
		.into_iter()
		.collect();
	let mut weeks = Vec::with_capacity(closed.len());
	for week in closed {
		if let (true, Some(window)) = (drafted.contains(&week.id), &week.window) {
			let _hold = deletion.hold().await;
			let mut tx = conn.begin_with("BEGIN IMMEDIATE").await.map_err(Failure::Storage)?;
			rust_round_repo::sent::finalize(&mut tx, subject, &week.id, &window.due, &at)
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
				"a week past its deadline still holds only a draft; it is left out of the rust index"
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
	put_index(mailbox, &index_name(RUST, &stamp), body, (last, computed), outcome).await
}

#[cfg(test)]
mod tests {
	use super::{fitted, rust_index};
	use crate::handlers::rust_week_record::{SentAt, TicketRecord, WeekRecord};
	use crate::mailbox::pass::MAILBOX_BODY_CEILING;
	use rust_round_repo::sent::StoredPaste;
	use serde_json::{json, Value};

	fn week(id: &str, due: &str, paste: Option<&str>) -> WeekRecord {
		WeekRecord {
			week: id.to_owned(),
			opens: String::from("2026-11-09T00:00:00-08:00"),
			due: due.to_owned(),
			closed: true,
			sent: Some(SentAt {
				at: String::from("2026-11-12T18:00:00.000Z"),
				received_at: String::from("2026-11-12T18:00:01.000Z"),
				draft: false,
			}),
			tickets: vec![
				TicketRecord {
					ticket: String::from("QM-1"),
					answered: true,
					take: Some(String::from("2026-w46:QM-1")),
					recorded_at: Some(String::from("2026-11-12T17:00:00.000Z")),
					opens: 2,
				},
				TicketRecord {
					ticket: String::from("QM-2"),
					answered: false,
					take: None,
					recorded_at: None,
					opens: 0,
				},
			],
			unanswered: vec![String::from("QM-2")],
			pastes: paste
				.map(|text| StoredPaste {
					at: String::from("2026-11-13T09:00:00.000Z"),
					text: text.to_owned(),
					note: None,
					received_at: String::from("2026-11-17T09:00:00.000Z"),
				})
				.into_iter()
				.collect(),
		}
	}

	#[test]
	fn the_index_carries_each_final_week_in_the_records_shape() {
		let weeks = [week("2026-w46", "2026-11-15T23:59:00-08:00", Some("error[E0502]: cannot borrow"))];
		let index: Value = serde_json::from_slice(&rust_index(&weeks)).unwrap();
		assert_eq!(
			index,
			json!({
				"schema": "some-ui.routine-index/1",
				"routine": "rust",
				"asOf": "2026-11-17T09:00:00.000Z",
				"weeks": [{
					"week": "2026-w46",
					"opens": "2026-11-09T00:00:00-08:00",
					"due": "2026-11-15T23:59:00-08:00",
					"closed": true,
					"sent": { "at": "2026-11-12T18:00:00.000Z", "receivedAt": "2026-11-12T18:00:01.000Z", "draft": false },
					"tickets": [
						{ "ticket": "QM-1", "answered": true, "take": "2026-w46:QM-1", "recordedAt": "2026-11-12T17:00:00.000Z", "opens": 2 },
						{ "ticket": "QM-2", "answered": false, "take": null, "recordedAt": null, "opens": 0 },
					],
					"unanswered": ["QM-2"],
					"pastes": [{ "at": "2026-11-13T09:00:00.000Z", "text": "error[E0502]: cannot borrow", "note": null, "receivedAt": "2026-11-17T09:00:00.000Z" }],
				}],
			})
		);
		assert_eq!(rust_index(&weeks), rust_index(&weeks), "the same records give the same bytes");
	}

	#[test]
	fn as_of_is_the_latest_deadline_or_arrival_and_null_for_none() {
		let weeks = [week("2026-w46", "2026-11-15T23:59:00-08:00", None)];
		let index: Value = serde_json::from_slice(&rust_index(&weeks)).unwrap();
		assert_eq!(index["asOf"], json!("2026-11-16T07:59:00.000Z"), "the deadline, in UTC, is later than the send");
		let empty: Value = serde_json::from_slice(&rust_index(&[])).unwrap();
		assert_eq!((&empty["asOf"], &empty["weeks"]), (&json!(null), &json!([])));
	}

	#[test]
	fn a_body_over_the_ceiling_drops_its_oldest_weeks_until_it_fits() {
		let big = "e".repeat(MAILBOX_BODY_CEILING / 2);
		let weeks = vec![
			week("2026-w46", "2026-11-15T23:59:00-08:00", Some(&big)),
			week("2026-w45", "2026-11-08T23:59:00-08:00", Some(&big)),
		];
		let index: Value = serde_json::from_slice(&fitted(weeks)).unwrap();
		let kept: Vec<&str> = index["weeks"].as_array().unwrap().iter().map(|week| week["week"].as_str().unwrap()).collect();
		assert_eq!(kept, ["2026-w46"], "the newest week is kept");
		let alone = vec![week("2026-w46", "2026-11-15T23:59:00-08:00", Some(&"e".repeat(MAILBOX_BODY_CEILING)))];
		assert!(fitted(alone).len() > MAILBOX_BODY_CEILING, "one week over the ceiling is left for put_index to refuse");
	}
}

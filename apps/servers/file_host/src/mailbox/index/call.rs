//! Home's bookkeeping for the weekly call's routine, put in the mailbox's
//! `index` folder after each pass (paulgsc/some-ui#2635, ST36's second slice).
//!
//! The routine writes each week's call to `paulgsc/testing`, which keeps only
//! the last few, and lets a week leave that repository only once home holds
//! it (paulgsc/some-ui `packages/video-render/CALL-ROUTINE.md`, step 6). This
//! index is how the routine knows: every week home holds (`call_week`, kept by
//! [`crate::call_weeks`]), newest first, with the SHA-256 of the bytes home
//! holds, so a week the repository has since edited reads as not the one home
//! keeps.
//!
//! `{"schema":"some-ui.routine-index/1","routine":"call","asOf","held":[{"from","to","sha256"}]}`:
//! `asOf` is when home last took a week; `held` is every week it holds.
//!
//! **A view, not a record**, as the Jobs index is (the parent module): no
//! table keeps it, and what home remembers is a [`LastIndex`] in memory.
//! Home holds weeks for nobody in particular (`call_week` is not
//! subject-scoped), so the index is the same whoever the pass is for.
//!
//! **Put only when it moves.** `call_week` only grows, so how many weeks it
//! holds says whether anything arrived. A pass that finds the count the last
//! index the mailbox answered was computed from puts nothing; one that finds
//! a new week puts `index-call-<stamp>.json`. Before home holds a week it
//! puts none, and the routine prunes nothing.
//!
//! **What it carries:** days and hashes. Never a week's words.

use mailbox_kit::{Mailbox, MailboxTransport};
use serde::Serialize;
use sqlx::SqlitePool;

use super::{index_name, put_index, stamp_of, Computed, LastIndex, INDEX_SCHEMA};
use crate::mailbox::pass::{Failure, Outcome};

/// The routine this index is for, as its name and `routine` say.
pub const CALL: &str = "call";

#[derive(Serialize)]
struct HeldWeek<'a> {
	from: &'a str,
	to: &'a str,
	sha256: &'a str,
}

#[derive(Serialize)]
struct CallIndex<'a> {
	schema: &'static str,
	routine: &'static str,
	#[serde(rename = "asOf")]
	as_of: &'a str,
	held: Vec<HeldWeek<'a>>,
}

/// The index's bytes for `held` (newest first), taken in at `as_of`.
#[must_use]
pub fn call_index(held: &[call_week_repo::HeldWeek], as_of: &str) -> Vec<u8> {
	let index = CallIndex {
		schema: INDEX_SCHEMA,
		routine: CALL,
		as_of,
		held: held
			.iter()
			.map(|week| HeldWeek {
				from: &week.from,
				to: &week.to,
				sha256: &week.sha256,
			})
			.collect(),
	};
	let mut body = Vec::new();
	// A struct of strings always serialises.
	let _ = serde_json::to_writer(&mut body, &index);
	body
}

/// Computes the call's index and puts it, when home has taken a week since
/// the last one the mailbox answered (the module doc). It writes nothing to
/// the database, so it takes no deletion hold.
///
/// # Errors
/// A [`Failure`] when the database fails or the mailbox cannot be reached;
/// the next pass computes and puts it again.
pub(in crate::mailbox) async fn put_call_index<T: MailboxTransport>(
	db: &SqlitePool,
	mailbox: &Mailbox<T>,
	subject: &str,
	last: &LastIndex,
	now: &(impl Fn() -> String + Sync),
	outcome: &mut Outcome,
) -> Result<(), Failure> {
	let Some(stamp) = stamp_of(&now()) else {
		return Ok(());
	};
	let (marker, as_of, held) = {
		let mut conn = db.acquire().await.map_err(Failure::Storage)?;
		// Nothing held yet: no index, and the routine prunes nothing.
		let Some((count, as_of)) = call_week_repo::as_of(&mut conn).await.map_err(Failure::Storage)? else {
			return Ok(());
		};
		let marker = count.to_string();
		if last.holds(subject, Some(&marker), |_| true) {
			return Ok(());
		}
		let held = call_week_repo::held(&mut conn).await.map_err(Failure::Storage)?;
		(marker, as_of, held)
	};
	let computed = Computed {
		subject: subject.to_owned(),
		as_of: Some(marker),
		valid: None,
	};
	put_index(mailbox, &index_name(CALL, &stamp), call_index(&held, &as_of), (last, computed), outcome).await
}

#[cfg(test)]
mod tests {
	use super::{call_index, CALL};
	use crate::mailbox::index::index_name;
	use call_week_repo::HeldWeek;
	use serde_json::{json, Value};

	#[test]
	fn the_index_lists_every_week_held_with_its_hash() {
		let held = [
			HeldWeek {
				from: String::from("2026-11-06"),
				to: String::from("2026-11-12"),
				sha256: "b".repeat(64),
			},
			HeldWeek {
				from: String::from("2026-10-30"),
				to: String::from("2026-11-05"),
				sha256: "a".repeat(64),
			},
		];
		let body = call_index(&held, "2026-11-13T09:00:00.000Z");
		let text = std::str::from_utf8(&body).unwrap();
		assert!(text.starts_with(r#"{"schema":"some-ui.routine-index/1","routine":"call","asOf":"#), "{text}");
		let index: Value = serde_json::from_slice(&body).unwrap();
		assert_eq!(
			index,
			json!({
				"schema": "some-ui.routine-index/1",
				"routine": "call",
				"asOf": "2026-11-13T09:00:00.000Z",
				"held": [
					{ "from": "2026-11-06", "to": "2026-11-12", "sha256": "b".repeat(64) },
					{ "from": "2026-10-30", "to": "2026-11-05", "sha256": "a".repeat(64) },
				],
			})
		);
		assert_eq!(index_name(CALL, "20261113T090000000Z"), "index-call-20261113T090000000Z.json");
	}
}

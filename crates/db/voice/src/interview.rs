//! A week of mock interview's first takes (paulgsc/some-ui#3175, IW8).
//!
//! The phone sends a question's first take home on its own, as an
//! `interview` note whose `ref` is `<week>:<question>`
//! ([`crate::is_interview_ref`]), before any send names it, and may record
//! the question again later. A note no send names is no error: it is held
//! as every note is, heard as every note is, and the week's record names
//! each question's **first** take, the earliest note said for it
//! ([`firsts`]), beside the one the send stands on.
//!
//! "Earliest" is by when the phone recorded it, compared as instants
//! (`rust_round_repo::time_order_key`, so offsets never misorder it), then by
//! note id for a tie. A note deleted from My data is no longer held, so the
//! next earliest is the first.
//!
//! **Per subject.** The one query takes the subject and filters on it, and
//! reads the week's refs as one range of `voice_note_by_ref`.

use crate::{is_interview_ref, Of};
use rust_round_repo::time_order_key;
use sqlx::SqliteConnection;

/// A question's first take: the earliest `interview` note said for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct First {
	/// The question's id.
	pub question: String,
	/// The note's id.
	pub take: String,
	/// When the phone recorded it.
	pub recorded_at: String,
}

/// The first take of each of `week`'s questions, in question-id order.
///
/// Only questions `subject` holds an `interview` note for are named. A
/// teleprompter card's read (`<week>:read:<card>`) is no question's, and a
/// note whose `ref` does not read as an interview ref is passed over.
///
/// # Errors
/// Any `sqlx` failure.
pub async fn firsts(conn: &mut SqliteConnection, subject: &str, week: &str) -> Result<Vec<First>, sqlx::Error> {
	let of = Of::Interview.as_str();
	// Every ref of the week, and no other: `:` sorts just before `;`.
	let from = String::from(week) + ":";
	let to = String::from(week) + ";";
	let notes = sqlx::query!(
		r#"SELECT note_id AS "note_id!", of_ref AS "of_ref!", recorded_at AS "recorded_at!"
		   FROM voice_note
		   WHERE of_kind = ?1 AND of_ref >= ?2 AND of_ref < ?3 AND subject_id = ?4"#,
		of,
		from,
		to,
		subject
	)
	.fetch_all(&mut *conn)
	.await?;
	let mut firsts: Vec<First> = Vec::new();
	for note in notes {
		if !is_interview_ref(&note.of_ref) {
			continue;
		}
		let Some((_, question)) = note.of_ref.split_once(':') else {
			continue;
		};
		if question.contains(':') {
			continue;
		}
		let earlier = |held: &First| (time_order_key(&note.recorded_at), note.note_id.as_str()) < (time_order_key(&held.recorded_at), held.take.as_str());
		match firsts.iter_mut().find(|held| held.question == question) {
			Some(held) if earlier(held) => {
				held.take.clone_from(&note.note_id);
				held.recorded_at.clone_from(&note.recorded_at);
			}
			Some(_) => {}
			None => firsts.push(First {
				question: question.to_owned(),
				take: note.note_id.clone(),
				recorded_at: note.recorded_at.clone(),
			}),
		}
	}
	firsts.sort_by(|a, b| a.question.cmp(&b.question));
	Ok(firsts)
}

#[cfg(test)]
mod tests {
	use super::{firsts, First};
	use sqlx::sqlite::SqlitePoolOptions;
	use sqlx::SqliteConnection;

	const A: &str = "subject-a";
	const B: &str = "subject-b";

	async fn note(conn: &mut SqliteConnection, subject: &str, id: &str, of_ref: &str, recorded_at: &str) {
		sqlx::query(
			"INSERT INTO voice_note (subject_id, note_id, of_kind, of_ref, recorded_at, content_type, size, sha256, stored_at)
			 VALUES (?1, ?2, 'interview', ?3, ?4, 'audio/webm', 1, 's', 'now')",
		)
		.bind(subject)
		.bind(id)
		.bind(of_ref)
		.bind(recorded_at)
		.execute(&mut *conn)
		.await
		.unwrap();
	}

	fn first(question: &str, take: &str, recorded_at: &str) -> First {
		First {
			question: question.to_owned(),
			take: take.to_owned(),
			recorded_at: recorded_at.to_owned(),
		}
	}

	/// Each question's earliest note by instant, offsets included, a tie
	/// going to the lower id; a read, another week and another subject's
	/// notes are none of it.
	#[tokio::test]
	async fn each_questions_first_is_its_earliest_note() {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		sqlx::migrate!("../../../migrations").run(&pool).await.unwrap();
		let mut conn = pool.acquire().await.unwrap();
		assert!(firsts(&mut conn, A, "2026-w47").await.unwrap().is_empty(), "none held");
		note(&mut conn, A, "late", "2026-w47:about-you", "2026-11-18T09:00:00.000Z").await;
		// 08:00 UTC, which a string compare puts after 09:00Z.
		note(&mut conn, A, "early", "2026-w47:about-you", "2026-11-18T10:00:00.000+02:00").await;
		note(&mut conn, A, "tie-b", "2026-w47:a-conflict", "2026-11-18T07:00:00.000Z").await;
		note(&mut conn, A, "tie-a", "2026-w47:a-conflict", "2026-11-18T07:00:00.000Z").await;
		note(&mut conn, A, "read", "2026-w47:read:home-first", "2026-11-17T07:00:00.000Z").await;
		note(&mut conn, A, "older", "2026-w46:about-you", "2026-11-10T07:00:00.000Z").await;
		note(&mut conn, A, "longer", "2026-w470:about-you", "2026-11-10T07:00:00.000Z").await;
		note(&mut conn, B, "theirs", "2026-w47:about-you", "2026-11-01T07:00:00.000Z").await;
		assert_eq!(
			firsts(&mut conn, A, "2026-w47").await.unwrap(),
			[
				first("a-conflict", "tie-a", "2026-11-18T07:00:00.000Z"),
				first("about-you", "early", "2026-11-18T10:00:00.000+02:00"),
			]
		);
	}
}

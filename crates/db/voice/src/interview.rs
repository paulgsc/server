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
//! **Week-close let-go** ([`let_go_retakes`]): when the week becomes final,
//! each question keeps the recordings of its earliest note, its newest and
//! the one the final names; every other note's recording is let go through
//! the one delete path, with a receipt, and everything else of it stays.
//!
//! **Per subject.** The one query takes the subject and filters on it, and
//! reads the week's refs as one range of `voice_note_by_ref`.

use crate::{
	attention,
	delete::{let_go_audio, Deletable, Deleted, Receipt},
	is_interview_ref, verdict, Of, VoiceError,
};
use chrono::{DateTime, FixedOffset};
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

/// One `interview` note said for a week's question, as [`held`] reads it.
struct Held {
	note_id: String,
	question: String,
	recorded_at: String,
	/// Some model ended it: heard it, or found it silent.
	ended: bool,
}

impl Held {
	/// The key notes are ordered by: when recorded, as an instant, then id.
	fn order(&self) -> ((bool, Option<DateTime<FixedOffset>>), &str) {
		(time_order_key(&self.recorded_at), self.note_id.as_str())
	}
}

/// Every `interview` note `subject` holds said for one of `week`'s
/// questions (a card's read is none), as one range of `voice_note_by_ref`.
async fn held(conn: &mut SqliteConnection, subject: &str, week: &str) -> Result<Vec<Held>, sqlx::Error> {
	let of = Of::Interview.as_str();
	// Every ref of the week, and no other: `:` sorts just before `;`.
	let from = String::from(week) + ":";
	let to = String::from(week) + ";";
	let notes = sqlx::query!(
		r#"SELECT note.note_id AS "note_id!", note.of_ref AS "of_ref!", note.recorded_at AS "recorded_at!",
		          EXISTS (SELECT 1 FROM voice_transcript AS heard
		                  WHERE heard.subject_id = note.subject_id AND heard.note_id = note.note_id
		                    AND heard.outcome IN ('heard', 'silent')
		                    AND NOT EXISTS (SELECT 1 FROM voice_note_superseding sup WHERE sup.subject_id = heard.subject_id AND sup.note_id = heard.note_id AND sup.hears_under <> heard.model)) AS "ended!: bool"
		   FROM voice_note AS note
		   WHERE note.of_kind = ?1 AND note.of_ref >= ?2 AND note.of_ref < ?3 AND note.subject_id = ?4"#,
		of,
		from,
		to,
		subject
	)
	.fetch_all(&mut *conn)
	.await?;
	Ok(
		notes
			.into_iter()
			.filter(|note| is_interview_ref(&note.of_ref))
			.filter_map(|note| {
				let (_, question) = note.of_ref.split_once(':')?;
				(!question.contains(':')).then(|| Held {
					question: question.to_owned(),
					note_id: note.note_id,
					recorded_at: note.recorded_at,
					ended: note.ended,
				})
			})
			.collect(),
	)
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
	let mut firsts: Vec<First> = Vec::new();
	for note in held(conn, subject, week).await? {
		let earlier = |first: &First| note.order() < (time_order_key(&first.recorded_at), first.take.as_str());
		match firsts.iter_mut().find(|first| first.question == note.question) {
			Some(first) if earlier(first) => {
				first.take.clone_from(&note.note_id);
				first.recorded_at.clone_from(&note.recorded_at);
			}
			Some(_) => {}
			None => firsts.push(First {
				question: note.question.clone(),
				take: note.note_id.clone(),
				recorded_at: note.recorded_at.clone(),
			}),
		}
	}
	firsts.sort_by(|a, b| a.question.cmp(&b.question));
	Ok(firsts)
}

/// Lets go of the recordings of `week`'s middle takes for `subject` at
/// `now`, as the week becomes final, and answers each receipt kept.
///
/// The owner's ruling (paulgsc/some-ui#3175, 2026-11-17, "keep first and
/// latest, go with your proposal"), with the keep rule a call of Claude's
/// records. For each question, home keeps the recording of:
///
/// - its **earliest** note, where the owner started cold;
/// - its **newest** note, where they got to;
/// - the take the week's final names for it (`sent`), when that is another:
///   a send is the owner's own choice, and nothing it names is let go.
///
/// So a question with two notes or fewer loses nothing. Every other note's
/// recording goes through [`crate::delete::let_go_audio`], the one delete
/// path's audio-only act, each with its receipt; its description, words,
/// verdicts and counts stay. Only a note home has heard and judged `heard`
/// ([`crate::verdict::newest`]) is let go: one no model has yet ended, or
/// that has no verdict yet, keeps its recording, so what home hears of it is
/// never lost, and so does one whose newest verdict asks the owner to act
/// ([`crate::attention::asks`]), which is theirs to reconcile. None of those
/// is let go later: this runs once, when the week becomes final.
///
/// Run it in the `BEGIN IMMEDIATE` transaction that makes the week final.
///
/// # Errors
/// [`VoiceError::Storage`] for any `sqlx` failure, and
/// [`VoiceError::Unreadable`] for a receipt that does not read back.
pub async fn let_go_retakes(conn: &mut SqliteConnection, subject: &str, week: &str, sent: &[(&str, &str)], on: &[Deletable], now: &str) -> Result<Vec<Receipt>, VoiceError> {
	let notes = held(conn, subject, week).await?;
	let mut receipts = Vec::new();
	let mut questions: Vec<&str> = notes.iter().map(|note| note.question.as_str()).collect();
	questions.sort_unstable();
	questions.dedup();
	for question in questions {
		let mut said: Vec<&Held> = notes.iter().filter(|note| note.question == question).collect();
		if said.len() <= 2 {
			continue;
		}
		said.sort_by(|a, b| a.order().cmp(&b.order()));
		let named = sent.iter().find(|(answered, _)| *answered == question).map(|(_, take)| *take);
		let last = said.len() - 1;
		for (index, note) in said.into_iter().enumerate() {
			if index == 0 || index == last || named == Some(note.note_id.as_str()) || !note.ended {
				continue;
			}
			// Only a take home heard and judged heard: one not yet judged may
			// still be heard again, and one whose verdict asks the owner to act
			// is theirs to reconcile.
			let heard = verdict::newest(conn, subject, &note.note_id)
				.await?
				.is_some_and(|newest| !attention::asks(newest.verdict.kind));
			if !heard {
				continue;
			}
			if let Deleted::Gone(receipt) = let_go_audio(conn, subject, &note.note_id, on, now).await? {
				receipts.push(receipt);
			}
		}
	}
	Ok(receipts)
}

#[cfg(test)]
mod tests {
	use super::{firsts, let_go_retakes, First};
	use crate::delete::{Deletable, SWITCHED_ON};
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
		sqlx::query("INSERT INTO voice_note_audio (subject_id, note_id, body) VALUES (?1, ?2, x'00')")
			.bind(subject)
			.bind(id)
			.execute(&mut *conn)
			.await
			.unwrap();
	}

	/// Home heard `id` (`outcome` `heard` or `silent`, or `failed`), its
	/// verdict `verdict`.
	async fn heard(conn: &mut SqliteConnection, id: &str, outcome: &str, verdict: &str) {
		sqlx::query(
			"INSERT INTO voice_transcript (subject_id, note_id, model, engine, attempt, outcome, language, language_p, duration_ms, text, took_ms, heard_at)
			 VALUES (?1, ?2, 'm', 'e', 1, ?3, CASE WHEN ?3 = 'heard' THEN 'en' END, CASE WHEN ?3 = 'heard' THEN 0.9 END, 1,
			         CASE WHEN ?3 = 'heard' THEN 'w' END, 1, 'now')",
		)
		.bind(A)
		.bind(id)
		.bind(outcome)
		.execute(&mut *conn)
		.await
		.unwrap();
		sqlx::query("INSERT INTO voice_transcript_verdict (subject_id, note_id, model, attempt, rule, verdict, reasons, speech_ms, judged_at) VALUES (?1, ?2, 'm', 1, 'r', ?3, '', CASE WHEN ?3 IN ('heard', 'garbled') THEN 1 END, 'now')")
			.bind(A)
			.bind(id)
			.bind(verdict)
			.execute(&mut *conn)
			.await
			.unwrap();
	}

	async fn audio(conn: &mut SqliteConnection, id: &str) -> bool {
		sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM voice_note_audio WHERE subject_id = ?1 AND note_id = ?2)")
			.bind(A)
			.bind(id)
			.fetch_one(&mut *conn)
			.await
			.unwrap()
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

	/// IW8's keep rule: each question keeps the recordings of its earliest
	/// note, its newest and the one the final names; a question of two notes
	/// or fewer loses none; a note not yet ended, or whose verdict asks the
	/// owner to act, keeps its recording; every other one is let go with its
	/// receipt, the note and its words staying. Switched off, none goes.
	#[tokio::test]
	async fn a_closing_week_keeps_first_newest_and_sent_and_lets_go_the_rest() {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		sqlx::migrate!("../../../migrations").run(&pool).await.unwrap();
		let mut conn = pool.acquire().await.unwrap();
		// about-you: six takes, hourly; the final names the third.
		for (id, at) in [
			("a1", "2026-11-18T01:00:00.000Z"),
			("a2", "2026-11-18T02:00:00.000Z"),
			("a3", "2026-11-18T03:00:00.000Z"),
			("a4", "2026-11-18T04:00:00.000Z"),
			("a5", "2026-11-18T05:00:00.000Z"),
			("a6", "2026-11-18T06:00:00.000Z"),
			("a7", "2026-11-18T07:00:00.000Z"),
		] {
			note(&mut conn, A, id, "2026-w47:about-you", at).await;
		}
		for id in ["a1", "a2", "a3", "a6", "a7"] {
			heard(&mut conn, id, "heard", "heard").await;
		}
		heard(&mut conn, "a4", "silent", "nothing_said").await;
		heard(&mut conn, "a5", "failed", "failed").await;
		// a-conflict: two takes; a read of a card: three.
		for (id, at) in [("c1", "2026-11-18T01:00:00.000Z"), ("c2", "2026-11-18T02:00:00.000Z")] {
			note(&mut conn, A, id, "2026-w47:a-conflict", at).await;
			heard(&mut conn, id, "heard", "heard").await;
		}
		for (id, at) in [("r1", "2026-11-18T01:00:00.000Z"), ("r2", "2026-11-18T02:00:00.000Z"), ("r3", "2026-11-18T03:00:00.000Z")] {
			note(&mut conn, A, id, "2026-w47:read:home-first", at).await;
			heard(&mut conn, id, "heard", "heard").await;
		}
		let sent = [("about-you", "a3"), ("a-conflict", "c1")];
		assert!(
			let_go_retakes(&mut conn, A, "2026-w47", &sent, &[Deletable::Unheard], "then").await.unwrap().is_empty(),
			"switched off"
		);
		let gone = let_go_retakes(&mut conn, A, "2026-w47", &sent, SWITCHED_ON, "then").await.unwrap();
		let ids: Vec<&str> = gone.iter().map(|receipt| receipt.id.as_str()).collect();
		assert_eq!(ids, ["a2", "a6"], "a4's verdict asks the owner, and a5 was never ended");
		for (id, kept) in [
			("a1", true),
			("a2", false),
			("a3", true),
			("a4", true),
			("a5", true),
			("a6", false),
			("a7", true),
			("c1", true),
			("c2", true),
			("r2", true),
		] {
			assert_eq!(audio(&mut conn, id).await, kept, "{id}");
		}
		let words: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM voice_transcript WHERE note_id = 'a2'")
			.fetch_one(&mut *conn)
			.await
			.unwrap();
		assert_eq!(words, 1, "its words stay");
		assert_eq!(gone[0].went, ["recording"]);
		assert!(
			let_go_retakes(&mut conn, A, "2026-w47", &sent, SWITCHED_ON, "later")
				.await
				.unwrap()
				.iter()
				.all(|receipt| receipt.deleted_at == "then"),
			"again: the same receipts"
		);
	}
}

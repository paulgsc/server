//! The take library's meaning index (paulgsc/some-ui#2590, ST32;
//! paulgsc/server#675): `voice_note_meaning`, one vector per Studio take and
//! embedding model.
//!
//! `20261105000100_create_voice_note_meaning.up.sql` and
//! docs/take-library.md, "Where the meaning index lives", record the design;
//! in short:
//!
//! - **Derived from the chosen words.** [`index`] embeds the words the
//!   library lists for a take (the newest hearing that heard something, as
//!   [`crate::transcript::heard`] chooses it) with an [`Embed`], and keeps
//!   the vector under the embedder's model name, replacing the one made
//!   from words a better transcript has since replaced. Nothing else writes
//!   the table.
//! - **Written where the words are.** `file_host`'s transcriber calls
//!   [`index`] in the transaction that keeps a hearing, and catches up on
//!   the takes heard before the index existed ([`unindexed`]), a bounded
//!   batch a pass.
//! - **Takes only, per subject.** A row is written only for a `studio` note
//!   the subject holds, in the statement that writes it.
//! - **What was said, kept as such** (docs/identity.md invariant 35). A
//!   vector is read only by [`crate::takes`]' search, inside this crate
//!   ([`vectors`] is not `pub`), which answers takes, never vectors. Nothing
//!   here logs.
//! - **It leaves with the take**: [`crate::delete`] removes a take's vectors
//!   in the transaction that deletes it, and the account sweep and the
//!   table's trigger remove them with the account.

use crate::{Of, VoiceError};
use sqlx::SqliteConnection;

/// How many numbers a vector holds.
pub const DIMENSIONS: usize = 256;

/// A vector's size as stored: [`DIMENSIONS`] little-endian `f32`s.
pub const BYTES: usize = DIMENSIONS * 4;

/// What makes a vector from words: `file_host::meaning`'s static model, or a
/// test's fixture.
pub trait Embed: Send + Sync {
	/// The model's name, stored beside each vector it makes, so a better
	/// model's vectors sit beside the old under a name of their own.
	fn model(&self) -> &str;

	/// `text`'s vector: [`DIMENSIONS`] numbers, unit length, or all zeros
	/// when the model knows none of its tokens. Never fails, and never logs.
	fn embed(&self, text: &str) -> Vec<f32>;
}

/// What [`index`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Indexed {
	/// The take's vector is kept, made from its chosen words.
	Kept,
	/// The note is a take no model has heard words in: nothing to embed.
	NoWords,
	/// The subject holds no `studio` note under that id: nothing written.
	NotATake,
}

/// Embeds the chosen words of `subject`'s take `note_id` with `embed`, and
/// keeps the vector under its model, replacing the one kept before.
///
/// Run it in the transaction that keeps a hearing, or in a `BEGIN
/// IMMEDIATE` transaction of its own under the deletion hold
/// (docs/identity.md invariant 9), so the take it reads is the one it
/// writes for.
///
/// # Errors
/// Any `sqlx` failure; nothing is written then.
pub async fn index(conn: &mut SqliteConnection, subject: &str, note_id: &str, embed: &dyn Embed) -> Result<Indexed, VoiceError> {
	let studio = Of::Studio.as_str();
	let take = sqlx::query_scalar!(
		r#"SELECT EXISTS (SELECT 1 FROM voice_note WHERE subject_id = ?1 AND note_id = ?2 AND of_kind = ?3) AS "take!: bool""#,
		subject,
		note_id,
		studio
	)
	.fetch_one(&mut *conn)
	.await?;
	if !take {
		return Ok(Indexed::NotATake);
	}
	// `transcript::heard`'s choice when anything was heard: the newest
	// hearing by `heard_at`, then model. Kept equal by
	// `a_takes_vector_is_made_from_the_words_the_library_lists`.
	let words = sqlx::query_scalar!(
		r#"SELECT text AS "text!: String" FROM voice_transcript
		   WHERE subject_id = ?1 AND note_id = ?2 AND outcome = 'heard'
		   ORDER BY heard_at DESC, model DESC
		   LIMIT 1"#,
		subject,
		note_id
	)
	.fetch_optional(&mut *conn)
	.await?;
	let Some(words) = words else {
		return Ok(Indexed::NoWords);
	};
	let vector = to_blob(&embed.embed(&words));
	let model = embed.model();
	sqlx::query!(
		"INSERT INTO voice_note_meaning (subject_id, note_id, model, vector)
		 SELECT ?1, ?2, ?3, ?4
		 WHERE EXISTS (SELECT 1 FROM voice_note WHERE subject_id = ?1 AND note_id = ?2 AND of_kind = ?5)
		 ON CONFLICT (subject_id, note_id, model) DO UPDATE SET vector = excluded.vector",
		subject,
		note_id,
		model,
		vector,
		studio
	)
	.execute(&mut *conn)
	.await?;
	Ok(Indexed::Kept)
}

/// A take some model heard words in that has no vector under `model`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unindexed {
	pub subject: String,
	pub note_id: String,
}

/// The takes heard before `model`'s index existed: every `studio` note a
/// model heard words in with no vector under `model`, oldest to reach home
/// first, at most `limit`.
///
/// # Errors
/// Any `sqlx` failure.
pub async fn unindexed(conn: &mut SqliteConnection, model: &str, limit: i64) -> Result<Vec<Unindexed>, VoiceError> {
	let studio = Of::Studio.as_str();
	let rows = sqlx::query!(
		r#"SELECT n.subject_id AS "subject_id!", n.note_id AS "note_id!" FROM voice_note n
		 WHERE n.of_kind = ?1
		   AND EXISTS (SELECT 1 FROM voice_transcript t
		               WHERE t.subject_id = n.subject_id AND t.note_id = n.note_id AND t.outcome = 'heard')
		   AND NOT EXISTS (SELECT 1 FROM voice_note_meaning m
		                   WHERE m.subject_id = n.subject_id AND m.note_id = n.note_id AND m.model = ?2)
		 ORDER BY n.stored_at, n.subject_id, n.note_id
		 LIMIT ?3"#,
		studio,
		model,
		limit
	)
	.fetch_all(&mut *conn)
	.await?;
	Ok(
		rows
			.into_iter()
			.map(|row| Unindexed {
				subject: row.subject_id,
				note_id: row.note_id,
			})
			.collect(),
	)
}

/// Every vector `subject` holds under `model`, by take.
///
/// Read only by [`crate::takes`]' search: a vector is never answered.
///
/// # Errors
/// [`VoiceError::Unreadable`] for a stored vector that is not
/// [`BYTES`] long, which the schema refuses, and any `sqlx` failure.
pub(crate) async fn vectors(conn: &mut SqliteConnection, subject: &str, model: &str) -> Result<Vec<(String, Vec<f32>)>, VoiceError> {
	let rows = sqlx::query!(
		r#"SELECT note_id AS "note_id!: String", vector AS "vector!: Vec<u8>" FROM voice_note_meaning
		   WHERE subject_id = ?1 AND model = ?2"#,
		subject,
		model
	)
	.fetch_all(&mut *conn)
	.await?;
	rows
		.into_iter()
		.map(|row| from_blob(&row.vector).map(|vector| (row.note_id, vector)).ok_or(VoiceError::Unreadable))
		.collect()
}

/// The cosine of the angle between `a` and `b`: 1 for the same meaning, 0
/// for unrelated or for a zero vector, which has no direction.
#[must_use]
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
	let dot = a.iter().zip(b).fold(0.0_f32, |sum, (x, y)| x.mul_add(*y, sum));
	let norms = norm(a) * norm(b);
	if norms > 0.0 {
		dot / norms
	} else {
		0.0
	}
}

/// `v`'s length.
#[must_use]
pub fn norm(v: &[f32]) -> f32 {
	v.iter().fold(0.0_f32, |sum, x| x.mul_add(*x, sum)).sqrt()
}

/// `vector` as stored: each number's four little-endian bytes, in order.
fn to_blob(vector: &[f32]) -> Vec<u8> {
	vector.iter().flat_map(|x| x.to_le_bytes()).collect()
}

/// A stored vector read back, or `None` when it is not [`BYTES`] long.
fn from_blob(blob: &[u8]) -> Option<Vec<f32>> {
	(blob.len() == BYTES).then(|| blob.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect())
}

#[cfg(test)]
pub(crate) mod tests {
	use super::{cosine, from_blob, index, to_blob, unindexed, vectors, Embed, Indexed, BYTES, DIMENSIONS};
	use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};

	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");
	const A: &str = "subject-a";
	const B: &str = "subject-b";

	/// A vector per word it knows, summed and made unit length: `brand`
	/// points one way, `korean` another, and any other word nowhere.
	pub(crate) struct Words;

	impl Embed for Words {
		fn model(&self) -> &'static str {
			"fixture"
		}

		fn embed(&self, text: &str) -> Vec<f32> {
			let mut v = vec![0.0_f32; DIMENSIONS];
			for word in text.split_whitespace() {
				match word.to_lowercase().as_str() {
					"brand" | "logo" => v[0] += 1.0,
					"korean" | "한국어" => v[1] += 1.0,
					_ => {}
				}
			}
			let n = super::norm(&v);
			if n > 0.0 {
				for x in &mut v {
					*x /= n;
				}
			}
			v
		}
	}

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

	async fn note(pool: &SqlitePool, subject: &str, id: &str, of: &str, stored_at: &str) {
		sqlx::query(
			"INSERT INTO voice_note (subject_id, note_id, of_kind, of_ref, recorded_at, content_type, size, sha256, stored_at)
			 VALUES (?1, ?2, ?3, 'ep:s:1', '2026-11-01T00:00:00Z', 'audio/webm', 10, 's', ?4)",
		)
		.bind(subject)
		.bind(id)
		.bind(of)
		.bind(stored_at)
		.execute(pool)
		.await
		.unwrap();
	}

	async fn hearing(pool: &SqlitePool, subject: &str, id: &str, model: &str, text: Option<&str>, at: &str) {
		let outcome = if text.is_some() { "heard" } else { "silent" };
		sqlx::query(
			"INSERT INTO voice_transcript
			   (subject_id, note_id, model, engine, attempt, outcome, language, language_p, duration_ms, text, took_ms, heard_at)
			 VALUES (?1, ?2, ?3, 'whisper.cpp', 1, ?4, ?5, ?6, 1000, ?7, 5, ?8)",
		)
		.bind(subject)
		.bind(id)
		.bind(model)
		.bind(outcome)
		.bind(text.map(|_| "en"))
		.bind(text.map(|_| 0.9))
		.bind(text)
		.bind(at)
		.execute(pool)
		.await
		.unwrap();
	}

	async fn stored(conn: &mut sqlx::SqliteConnection) -> Vec<(String, String, String, Vec<u8>)> {
		sqlx::query_as("SELECT subject_id, note_id, model, vector FROM voice_note_meaning ORDER BY subject_id, note_id, model")
			.fetch_all(conn)
			.await
			.unwrap()
	}

	#[tokio::test]
	async fn the_table_holds_exactly_its_described_columns() {
		let pool = pool().await;
		let columns: Vec<String> = sqlx::query_scalar("SELECT name FROM pragma_table_info('voice_note_meaning') ORDER BY cid")
			.fetch_all(&pool)
			.await
			.unwrap();
		assert_eq!(columns, ["subject_id", "note_id", "model", "vector"], "no ref, no text, no time");
		let short = sqlx::query("INSERT INTO voice_note_meaning (subject_id, note_id, model, vector) VALUES (?1, 't', 'm', x'00')")
			.bind(A)
			.execute(&pool)
			.await;
		assert!(short.is_err(), "a vector is 1,024 bytes");
	}

	#[test]
	fn a_vector_is_stored_as_little_endian_floats_and_read_back_whole() {
		let v: Vec<f32> = (0..DIMENSIONS).map(|i| f32::from(u8::try_from(i % 7).unwrap()) - 3.5).collect();
		let blob = to_blob(&v);
		assert_eq!(blob.len(), BYTES);
		assert_eq!(&blob[..4], &(-3.5_f32).to_le_bytes());
		assert_eq!(from_blob(&blob), Some(v));
		assert_eq!(from_blob(&blob[1..]), None);
	}

	#[test]
	fn the_cosine_is_one_for_the_same_direction_and_zero_for_none() {
		let brand = Words.embed("the brand");
		assert!((cosine(&brand, &Words.embed("BRAND logo")) - 1.0).abs() < 1e-6);
		assert!(cosine(&brand, &Words.embed("korean")).abs() < 1e-6);
		assert!(cosine(&brand, &Words.embed("nothing known")).abs() < f32::EPSILON, "a zero vector has no direction");
	}

	/// The vector is made from the words the library lists: the newest
	/// hearing that heard something, replaced when a better one arrives.
	#[tokio::test]
	async fn a_takes_vector_is_made_from_the_words_the_library_lists() {
		let pool = pool().await;
		note(&pool, A, "t", "studio", "2026-11-01T00:00:00Z").await;
		hearing(&pool, A, "t", "small", Some("the brand"), "2026-11-01T10:00:00Z").await;
		hearing(&pool, A, "t", "large", None, "2026-11-01T12:00:00Z").await;
		let mut conn = pool.acquire().await.unwrap();

		assert_eq!(index(&mut conn, A, "t", &Words).await.unwrap(), Indexed::Kept);
		let crate::transcript::Hearing::Heard(listed) = crate::transcript::heard(&mut conn, A, "t").await.unwrap() else {
			panic!("heard");
		};
		let kept = stored(&mut conn).await;
		assert_eq!(kept.len(), 1);
		assert_eq!((kept[0].0.as_str(), kept[0].1.as_str(), kept[0].2.as_str()), (A, "t", "fixture"));
		assert_eq!(kept[0].3, to_blob(&Words.embed(&listed.text)), "a later silence does not change the words");

		drop(conn);
		hearing(&pool, A, "t", "medium", Some("한국어 korean"), "2026-11-01T13:00:00Z").await;
		let mut conn = pool.acquire().await.unwrap();
		assert_eq!(index(&mut conn, A, "t", &Words).await.unwrap(), Indexed::Kept);
		let kept = stored(&mut conn).await;
		assert_eq!(kept.len(), 1, "one vector per take and model");
		assert_eq!(kept[0].3, to_blob(&Words.embed("한국어 korean")), "replaced by the newer words");
	}

	#[tokio::test]
	async fn only_a_heard_take_the_subject_holds_gets_a_vector() {
		let pool = pool().await;
		note(&pool, A, "reflection", "reflection", "2026-11-01T00:00:00Z").await;
		hearing(&pool, A, "reflection", "small", Some("the brand"), "2026-11-01T10:00:00Z").await;
		note(&pool, A, "quiet", "studio", "2026-11-01T00:00:00Z").await;
		hearing(&pool, A, "quiet", "small", None, "2026-11-01T10:00:00Z").await;
		note(&pool, A, "waiting", "studio", "2026-11-01T00:00:00Z").await;
		let mut conn = pool.acquire().await.unwrap();

		assert_eq!(index(&mut conn, A, "reflection", &Words).await.unwrap(), Indexed::NotATake);
		assert_eq!(index(&mut conn, B, "quiet", &Words).await.unwrap(), Indexed::NotATake, "another subject's");
		assert_eq!(index(&mut conn, A, "quiet", &Words).await.unwrap(), Indexed::NoWords, "silent");
		assert_eq!(index(&mut conn, A, "waiting", &Words).await.unwrap(), Indexed::NoWords, "not heard yet");
		assert_eq!(index(&mut conn, A, "gone", &Words).await.unwrap(), Indexed::NotATake);
		assert!(stored(&mut conn).await.is_empty());
	}

	#[tokio::test]
	async fn the_takes_heard_before_the_index_wait_oldest_first_until_indexed() {
		let pool = pool().await;
		note(&pool, A, "newer", "studio", "2026-11-03T00:00:00Z").await;
		note(&pool, B, "older", "studio", "2026-11-01T00:00:00Z").await;
		note(&pool, A, "quiet", "studio", "2026-11-01T00:00:00Z").await;
		note(&pool, A, "reflection", "reflection", "2026-11-01T00:00:00Z").await;
		for (subject, id) in [(A, "newer"), (B, "older"), (A, "reflection")] {
			hearing(&pool, subject, id, "small", Some("the brand"), "2026-11-04T00:00:00Z").await;
		}
		hearing(&pool, A, "quiet", "small", None, "2026-11-04T00:00:00Z").await;
		let mut conn = pool.acquire().await.unwrap();

		let waiting = |rows: Vec<super::Unindexed>| rows.into_iter().map(|row| (row.subject, row.note_id)).collect::<Vec<_>>();
		assert_eq!(
			waiting(unindexed(&mut conn, "fixture", 10).await.unwrap()),
			[(B.to_owned(), "older".to_owned()), (A.to_owned(), "newer".to_owned())],
			"heard takes only, oldest first"
		);
		assert_eq!(unindexed(&mut conn, "fixture", 1).await.unwrap().len(), 1, "bounded");
		index(&mut conn, B, "older", &Words).await.unwrap();
		assert_eq!(waiting(unindexed(&mut conn, "fixture", 10).await.unwrap()), [(A.to_owned(), "newer".to_owned())]);
		assert_eq!(unindexed(&mut conn, "a-better-model", 10).await.unwrap().len(), 2, "a new model starts its own index");
	}

	#[tokio::test]
	async fn a_subjects_vectors_are_its_own_and_leave_with_the_account() {
		let pool = pool().await;
		for subject in [A, B] {
			note(&pool, subject, "t", "studio", "2026-11-01T00:00:00Z").await;
			hearing(&pool, subject, "t", "small", Some("the brand"), "2026-11-01T10:00:00Z").await;
			let mut conn = pool.acquire().await.unwrap();
			index(&mut conn, subject, "t", &Words).await.unwrap();
		}
		let mut conn = pool.acquire().await.unwrap();
		let mine = vectors(&mut conn, A, "fixture").await.unwrap();
		assert_eq!(mine, [(String::from("t"), Words.embed("the brand"))]);
		assert!(vectors(&mut conn, A, "another").await.unwrap().is_empty());
		drop(conn);

		sqlx::query("DELETE FROM account WHERE subject_id = ?1").bind(A).execute(&pool).await.unwrap();
		let left: Vec<String> = sqlx::query_scalar("SELECT subject_id FROM voice_note_meaning").fetch_all(&pool).await.unwrap();
		assert_eq!(left, [B], "the trigger takes the deleted account's vectors only");
	}
}

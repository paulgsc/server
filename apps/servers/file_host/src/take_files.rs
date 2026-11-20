//! One hand-off of the owner's takes to home's render worker.
//!
//! For every essay that plays them (paulgsc/server#1052; recon
//! paulgsc/some-ui#3317): a Rust Rounds or interview essay's (`rust_essays`)
//! and a cards essay's spoken cards (`cards_essays::pass`).
//!
//! [`hand_off`] reads one take, by its note id, into the files the
//! recorder takes beside its essay (`@some-ui/video-render`'s `take-files`):
//!
//! - `<name>.<webm|ogg|m4a>`: its audio, as the phone sent it;
//! - `<name>.words.json`: its word times ([`TimedWord`], one wire type),
//!   only where home kept them;
//! - `<name>.transcript.json`, when asked: the `some-ui.voice-transcript/1`
//!   body the mailbox files (`mailbox::pass::transcript_body`), its verdict
//!   included.
//!
//! What a caller does with a take that is not ready is its own rule: a Rust
//! Rounds essay waits for a take not yet home and is withdrawn for one
//! deleted; a cards essay refuses both. This module only says which.
//! Nothing here logs: a take's words never reach a log line.

use serde::Serialize;
use sqlx::SqliteConnection;
use voice_repo::{Of, VoiceError};

/// A word of a take with its times, as `<name>.words.json` holds it.
///
/// Its fields are in name order, the order `serde_json` gives a map, so the file
/// is byte for byte what home wrote before this type (RR16).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimedWord<'a> {
	pub end_ms: i64,
	pub start_ms: i64,
	pub text: &'a str,
}

/// One take a caller asks for.
#[derive(Debug, Clone, Copy)]
pub struct Ask<'a> {
	/// The take's note id.
	pub take: &'a str,
	/// Its files' stem: the item it answers (`QM-311`), or the take's id.
	pub name: &'a str,
	/// What it must have been said for; `None` for any kind.
	pub of: Option<Of>,
	/// Whether to hand its transcript too, and so ask that home heard it.
	pub transcript: bool,
}

/// What home holds of an asked take.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Take {
	/// Its files, by name, and their bytes.
	Files(Vec<(String, Vec<u8>)>),
	/// Its audio is not on home: `deleted` from My data (a receipt is
	/// kept), or never sent, said for another kind, or its recording let go.
	Absent { deleted: bool },
	/// Asked with its transcript, and home has not heard it yet.
	NotHeardYet,
	/// Asked with its transcript, and home couldn't make it out, heard
	/// nothing said in it, or every attempt failed (HEARD1).
	NotMadeOut,
}

/// `subject`'s take `ask.take`, as the module doc says. A few indexed reads.
///
/// # Errors
/// [`VoiceError::Storage`] for any `sqlx` failure, and
/// [`VoiceError::Unreadable`] for a held note that does not read back.
pub async fn hand_off(conn: &mut SqliteConnection, subject: &str, ask: Ask<'_>) -> Result<Take, VoiceError> {
	let note = if ask.transcript || ask.of.is_none() {
		voice_repo::note_head(conn, subject, ask.take)
			.await?
			.filter(|note| ask.of.is_none_or(|of| note.of_kind == of.as_str()))
	} else {
		None
	};
	let content_type = match (ask.of, &note) {
		(_, Some(note)) => Some(note.content_type),
		(Some(of), None) if !ask.transcript => voice_repo::audio_head(conn, subject, ask.take, of).await?.map(|head| head.content_type),
		_ => None,
	};
	let Some(content_type) = content_type else {
		let deleted = voice_repo::delete::receipt(conn, subject, ask.take).await?.is_some();
		return Ok(Take::Absent { deleted });
	};
	// Whether home heard it before its audio is read, so a take waited for
	// or refused costs no read of its bytes.
	let transcript = if ask.transcript {
		let heard = match voice_repo::transcript::heard(conn, subject, ask.take).await? {
			voice_repo::transcript::Hearing::Heard(heard) => heard,
			voice_repo::transcript::Hearing::NotHeld => {
				let deleted = voice_repo::delete::receipt(conn, subject, ask.take).await?.is_some();
				return Ok(Take::Absent { deleted });
			}
			voice_repo::transcript::Hearing::Waiting => {
				// Every attempt failed for good: never to be heard.
				let verdict = voice_repo::verdict::of_note(conn, subject, ask.take).await?;
				return Ok(if verdict.is_some_and(|verdict| verdict.kind != voice_repo::verdict::Kind::Heard) {
					Take::NotMadeOut
				} else {
					Take::NotHeardYet
				});
			}
		};
		let verdict = voice_repo::verdict::of_ending(conn, subject, ask.take, &heard.model).await?;
		// A transcript kept before verdicts were reads as heard, as the
		// recorder reads it (`hearingOf`); an empty one heard nothing said.
		if verdict.as_ref().is_some_and(|verdict| verdict.kind != voice_repo::verdict::Kind::Heard) || heard.text.trim_matches(mailbox_repo::BLANK).is_empty() {
			return Ok(Take::NotMadeOut);
		}
		let Some(note) = note else {
			return Ok(Take::Absent { deleted: false });
		};
		let said = voice_repo::transcript::words_heard_by(conn, subject, ask.take, &heard.model).await?;
		let filed = mailbox_repo::Heard {
			note_id: ask.take.to_owned(),
			model: heard.model,
			of_kind: note.of_kind,
			of_ref: note.of_ref,
			recorded_at: note.recorded_at,
			text: heard.text,
			language: heard.language,
			heard_at: heard.heard_at,
		};
		Some(crate::mailbox::pass::transcript_body(&filed, &said, verdict.as_ref()))
	} else {
		None
	};
	let body = match ask.of {
		Some(of) => voice_repo::audio_body(conn, subject, ask.take, of).await?,
		None => voice_repo::transcript::audio(conn, subject, ask.take).await?,
	};
	let Some(audio) = body else {
		let deleted = voice_repo::delete::receipt(conn, subject, ask.take).await?.is_some();
		return Ok(Take::Absent { deleted });
	};
	let extension = match content_type {
		"audio/ogg" => "ogg",
		"audio/mp4" => "m4a",
		_ => "webm",
	};
	let mut files = vec![(String::from(ask.name) + "." + extension, audio)];
	let words = voice_repo::transcript::words(conn, subject, ask.take).await?;
	if !words.is_empty() {
		files.push((String::from(ask.name) + ".words.json", words_file(&words)));
	}
	if let Some(transcript) = transcript {
		files.push((String::from(ask.name) + ".transcript.json", transcript));
	}
	Ok(Take::Files(files))
}

/// `<name>.words.json`'s bytes.
#[must_use]
pub fn words_file(words: &[voice_repo::transcript::Word]) -> Vec<u8> {
	let timed: Vec<TimedWord<'_>> = words
		.iter()
		.map(|word| TimedWord {
			end_ms: word.end_ms,
			start_ms: word.start_ms,
			text: &word.text,
		})
		.collect();
	// Disallowed for tracing; these are a file's bytes, never logged.
	#[allow(clippy::disallowed_methods)]
	serde_json::to_vec(&timed).unwrap_or_default()
}

#[cfg(test)]
mod tests {
	use super::*;

	/// The word times a Rust Rounds essay was given before this type, byte for
	/// byte: the shared type changes nothing it hands over.
	#[test]
	fn word_times_are_the_bytes_home_wrote_before() {
		let words = [
			voice_repo::transcript::Word {
				text: String::from("every"),
				start_ms: 0,
				end_ms: 400,
				p: Some(0.9),
				language: Some(String::from("en")),
			},
			voice_repo::transcript::Word {
				text: String::from("\"field\","),
				start_ms: 450,
				end_ms: 900,
				p: None,
				language: None,
			},
		];
		let before: Vec<serde_json::Value> = words
			.iter()
			.map(|word| serde_json::json!({ "text": word.text, "startMs": word.start_ms, "endMs": word.end_ms }))
			.collect();
		assert_eq!(words_file(&words), serde_json::Value::Array(before).to_string().into_bytes());
	}
}

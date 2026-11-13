//! Interview takes are heard verbatim, fillers and all (paulgsc/some-ui#2933,
//! IW2).
//!
//! The interview essay counts "um"s and "uh"s (paulgsc/some-ui#2931), and
//! Whisper leaves them out by default: it was trained on transcripts that
//! tidy speech up. What it was trained on also teaches it to continue in the
//! style of the text before it, so an initial prompt written the way a
//! verbatim transcript reads, fillers and false starts included, keeps them
//! as words. The owner chose this over a second model (2026-11-13): it is the
//! same model, so nothing new runs on home, and only the decoder's first
//! window of each run reads [`PROMPT`]'s few dozen tokens more.
//!
//! - **Interview takes only.** A note of `voice_repo::transcript::VERBATIM`'s
//!   kind is decoded with the prompt, and every other kind exactly as
//!   before ([`super::engine::Decode`]).
//! - **Kept under its own key.** A verbatim transcription is a different
//!   transcription, so `voice_transcript` keeps it under the model's file
//!   name and [`NAME`] ([`key`]): the two never end each other's notes, and
//!   whoever reads one (the mailbox body's `model`) knows which it has. A
//!   changed prompt takes a new [`NAME`], and its notes are heard again beside
//!   the old rows, never mixed with them.
//! - **[`FILLERS`] is the closed list the essay counts.** It is written
//!   here, where the prompt is built to keep its words, and copied by the
//!   interview week's measured faults in paulgsc/some-ui (IW3,
//!   `packages/ui/interview-week`), which names this file. A change to it is
//!   a change to both.
//! - **English runs only.** Each run of speech is heard in its own language
//!   (ST23); the prompt is English, and handed to a Korean run it would pull
//!   the run toward English. A Korean run in an interview take is heard as
//!   any note's is, and keeps its own language.

/// The prompt's name, the part of the key after the model's file name. A
/// changed [`PROMPT`] or [`FILLERS`] takes the next number.
pub const NAME: &str = "verbatim-1";

/// The fillers the interview essay counts, lowercase: a word is one when,
/// lowercased and with the punctuation at either end trimmed off, it is one
/// of these ([`is_filler`]).
///
/// Only sounds that are never anything but a filler. "Like", "you know",
/// "I mean" and "so" are often fillers too, and [`PROMPT`] has them so their
/// filler uses are kept, but each is also an ordinary word, and telling which
/// is a judgment, not a count, so the essay leaves it to the model's half.
pub const FILLERS: [&str; 8] = ["um", "uh", "er", "erm", "ah", "hmm", "mm", "uhm"];

/// What the decoder reads before an interview take's English speech: a
/// verbatim transcript's style, with every one of [`FILLERS`] in it.
pub const PROMPT: &str = "Um, so, uh, I was... I mean, like, you know, I was, um, working on the, uh, the server. Hmm. Er, erm, \
                          mm, ah, well, uhm, so yeah, I, I think, uh, that was it.";

/// The language [`PROMPT`] is written in, and the only one it is given to.
pub const LANGUAGE: &str = "en";

/// What a verbatim transcription by `model` is kept under:
/// `ggml-small-q5_1.bin+verbatim-1`.
#[must_use]
pub fn key(model: &str) -> String {
	[model, "+", NAME].concat()
}

/// Whether `word`, as Whisper wrote it, is one of [`FILLERS`].
#[must_use]
pub fn is_filler(word: &str) -> bool {
	FILLERS.contains(&bare(word).as_str())
}

/// `word` lowercased, the punctuation at either end trimmed off.
fn bare(word: &str) -> String {
	word.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase()
}

#[cfg(test)]
mod tests {
	use super::{bare, is_filler, key, FILLERS, NAME, PROMPT};

	#[test]
	fn a_verbatim_transcription_is_kept_under_the_model_and_the_prompt() {
		assert_eq!(key("ggml-small-q5_1.bin"), "ggml-small-q5_1.bin+verbatim-1");
		assert_ne!(key("ggml-small-q5_1.bin"), "ggml-small-q5_1.bin", "never the plain key");
		// The mailbox takes a model of at most 100 characters
		// (paulgsc/some-ui `apps/mailbox/src/core.ts`).
		assert!(key("ggml-large-v3-turbo-q5_0.bin").len() <= 100);
		assert!(!NAME.contains(char::is_whitespace));
	}

	/// The prompt shows the decoder each filler, and is something whisper.cpp
	/// can be handed (a C string: no NUL).
	#[test]
	fn the_prompt_has_every_filler_it_keeps() {
		let said: Vec<&str> = PROMPT.split_whitespace().collect();
		for filler in FILLERS {
			assert!(said.iter().any(|word| bare(word) == filler), "{filler}");
		}
		assert!(!PROMPT.contains('\0'));
	}

	#[test]
	fn a_filler_is_one_whatever_its_case_and_punctuation() {
		for word in ["um", "Um,", "uh...", "Hmm.", "\"mm\"", "Erm?", "UHM"] {
			assert!(is_filler(word), "{word}");
		}
		for word in ["umbrella", "like", "so", "I", "", "...", "humm", "ahead"] {
			assert!(!is_filler(word), "{word}");
		}
	}

	/// The list is closed, lowercase, and has no word twice: paulgsc/some-ui's
	/// copy compares equal to it.
	#[test]
	fn the_fillers_are_a_closed_lowercase_list() {
		let mut sorted = FILLERS.to_vec();
		sorted.sort_unstable();
		sorted.dedup();
		assert_eq!(sorted.len(), FILLERS.len());
		assert!(FILLERS.iter().all(|filler| filler.chars().all(|c| c.is_ascii_lowercase())));
	}
}

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
//! - **[`FILLERS`] is the closed list the essay counts**, owned by the
//!   interview week's measured faults in paulgsc/some-ui (IW3, call C265)
//!   and copied here, where [`PROMPT`] is written to show the decoder every
//!   one of them. A change to it is made there first, then here, with a new
//!   [`NAME`].
//! - **English runs only.** Each run of speech is heard in its own language
//!   (ST23); the prompt is English, and handed to a Korean run it would pull
//!   the run toward English. A Korean run in an interview take is heard as
//!   any note's is, and keeps its own language.

/// The prompt's name, the part of the key after the model's file name. A
/// changed [`PROMPT`] or [`FILLERS`] takes the next number.
pub const NAME: &str = "verbatim-1";

// Copy of paulgsc/some-ui packages/ui/interview-week/src/lib/faults/fillers.json
// (C265, #2934), which owns the list; change it there first.
/// The fillers the interview essay counts: vocalised hesitations only,
/// matched as some-ui's measures match them ([`fillers_in`]).
///
/// "Like", "you know", "I mean" and "so" are often fillers too, and
/// [`PROMPT`] has them so their filler uses are kept, but each is also an
/// ordinary word, and telling which is a judgment, not a count, so the essay
/// leaves it to the model's half.
pub const FILLERS: [&str; 13] = ["um", "umm", "uh", "uhh", "uhm", "er", "erm", "ah", "ahh", "hm", "hmm", "mm", "mhm"];

/// What the decoder reads before an interview take's English speech: a
/// verbatim transcript's style, with every one of [`FILLERS`] in it.
pub const PROMPT: &str = "Um, so, uh, I was... I mean, like, you know, I was, umm, working on the, uhh, the server. Hmm. Er, erm, \
                          mm, ah, well, uhm, so yeah, I, I think, ahh, hm, mhm, uh, that was it.";

/// The language [`PROMPT`] is written in, and the only one it is given to.
pub const LANGUAGE: &str = "en";

/// What a verbatim transcription by `model` is kept under:
/// `ggml-small-q5_1.bin+verbatim-1`.
#[must_use]
pub fn key(model: &str) -> String {
	[model, "+", NAME].concat()
}

/// How many of [`FILLERS`] `word`, as Whisper wrote it, holds.
///
/// Matched as paulgsc/some-ui's measures match: lowercased, apostrophes dropped, then
/// each run of letters and digits matched whole. "Um," and "uh..." are one
/// each, and "uh-huh" is one, its "uh".
#[must_use]
pub fn fillers_in(word: &str) -> usize {
	runs(word).iter().filter(|run| FILLERS.contains(&run.as_str())).count()
}

/// `word`'s runs of letters and digits, lowercased, apostrophes dropped.
fn runs(word: &str) -> Vec<String> {
	let kept: String = word.chars().filter(|c| !matches!(c, '\'' | '\u{2019}')).flat_map(char::to_lowercase).collect();
	kept.split(|c: char| !c.is_alphanumeric()).filter(|run| !run.is_empty()).map(str::to_owned).collect()
}

#[cfg(test)]
mod tests {
	use super::{fillers_in, key, runs, FILLERS, NAME, PROMPT};

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
			assert!(said.iter().any(|word| runs(word).iter().any(|run| run == filler)), "{filler}");
		}
		assert!(!PROMPT.contains('\0'));
	}

	#[test]
	fn a_filler_is_counted_whatever_its_case_and_punctuation() {
		for word in ["um", "Um,", "uh...", "Hmm.", "\"mm\"", "Erm?", "UHM", "Mhm.", "uh-huh", "\u{2019}umm"] {
			assert_eq!(fillers_in(word), 1, "{word}");
		}
		assert_eq!(fillers_in("um-uh"), 2, "each run on its own");
		assert_eq!(fillers_in("Mm-hmm"), 2, "as some-ui counts it: its mm, then its hmm");
		for word in ["umbrella", "like", "so", "I", "", "...", "humm", "ahead"] {
			assert_eq!(fillers_in(word), 0, "{word}");
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

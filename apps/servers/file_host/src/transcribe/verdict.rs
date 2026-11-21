//! Home judges every transcript it makes (paulgsc/server#908, HEARD1).
//!
//! Heard, garbled, nothing said, or failed, from the transcriber's own
//! signals, so discovery happens here and not in a routine that reads the
//! take later.
//!
//! **What it judges on** ([`Measures`], from `engine`): per segment Whisper
//! decoded, its mean token log-probability (`avg_logprob`), how repetitive its
//! text is (`compression_ratio`) and its probability of holding no speech
//! (`no_speech_prob`); per word, how sure the model was of it; and for the
//! take, the language detected for longest against the one its kind is said
//! in, the words heard per second of speech, and the speech's loudness.
//!
//! **The rule is [`RULE`]** and its thresholds are named below, each with its
//! reason. Whisper's own defaults where Whisper has one; the word share was
//! read off the transcripts the mailbox held on 2026-11-16 (23 with five
//! words or more): every take the owner heard as fine had at most 0.24 of its
//! words below [`WORD_P_FLOOR`], and the two w46 Rust Rounds takes a routine
//! found "mostly garbled" had 0.47 and 0.61. A change to any threshold is a
//! new rule, kept beside the old verdicts rather than over them.
//!
//! **Only final attempts are judged**: an ending, and a failure the worker
//! will not retry ([`judge`]'s `last`). Nothing heard reaches a log line: a
//! verdict carries kinds and numbers, never words.

use super::engine::Measures;
use voice_repo::transcript::Outcome;
use voice_repo::verdict::{Kind, Signals, Verdict};

/// The thresholds below, as one name kept beside each verdict.
pub const RULE: &str = "heard-1";

/// A segment Whisper heard with a mean token log-probability below this is
/// one it was unsure of: Whisper's own `log_prob_threshold`, past which it
/// decodes the window again.
pub const LOGPROB_FLOOR: f64 = -1.0;

/// A segment whose text is more repetitive than this (its length over its
/// zlib length) is a decoding loop, not speech: Whisper's own
/// `compression_ratio_threshold`.
pub const COMPRESSION_CEILING: f64 = 2.4;

/// The no-speech probability past which an unsure segment is silence.
///
/// A segment more likely than this to hold no speech, heard with a log-prob
/// below [`LOGPROB_FLOOR`] too, is words made of silence: Whisper's own
/// `no_speech_threshold`, which it pairs with the log-prob the same way.
pub const NO_SPEECH_CEILING: f64 = 0.6;

/// A word the model gave a probability below this is one it was unsure of.
/// Half: below it, the model thought another word likelier than not.
pub const WORD_P_FLOOR: f64 = 0.5;

/// A take with at least this share of its words below [`WORD_P_FLOOR`] is
/// garbled: fine takes held at most 0.24 on 2026-11-16, the garbled w46 ones
/// 0.47 and 0.61, and this sits between them.
pub const UNSURE_WORD_SHARE: f64 = 0.4;

/// A take with this much speech or more...
pub const SPARSE_MIN_SPEECH_MS: i64 = 10_000;

/// ...and fewer words a second of it than this was not heard: speech runs
/// at two to three words a second (fine takes on 2026-11-16: 1.6 to 3.1),
/// so under one, most of what was said is missing.
pub const SPARSE_WORDS_PER_SECOND: f64 = 1.0;

/// What home made of `outcome`.
///
/// `measures` are what the engine said of the attempt, `expected` the
/// language the note's kind is said in
/// (`voice_repo::verdict::expected_language`), and `last` whether the
/// worker will not try the note again if it failed.
///
/// None for a failure that will be retried: it says nothing final.
#[must_use]
pub fn judge(outcome: &Outcome, measures: Option<&Measures>, expected: Option<&str>, last: bool) -> Option<Verdict> {
	let verdict = |kind, reasons: Vec<&str>, signals| Verdict {
		kind,
		reasons: reasons.into_iter().map(str::to_owned).collect(),
		rule: String::from(RULE),
		signals,
	};
	let failed = |reason| last.then(|| verdict(Kind::Failed, vec![reason], None));
	match outcome {
		Outcome::Silent => Some(verdict(Kind::NothingSaid, Vec::new(), None)),
		// Words that are only white space are nothing said, as silence is
		// (paulgsc/server#913): the take is filed and read as a silence.
		Outcome::Heard { text, .. } if text.trim().is_empty() => Some(verdict(Kind::NothingSaid, Vec::new(), None)),
		Outcome::Unreadable => failed("unreadable"),
		Outcome::TooLong => failed("too_long"),
		Outcome::Deadline => failed("deadline"),
		Outcome::Failed => failed("engine"),
		Outcome::Heard { language, language_p, words, .. } => {
			let signals = signals(measures, language, *language_p, words, expected);
			let reasons = reasons(&signals);
			let kind = if reasons.is_empty() { Kind::Heard } else { Kind::Garbled };
			Some(verdict(kind, reasons, Some(signals)))
		}
	}
}

/// The take's signals: the segments' read together, by each one's length.
fn signals(measures: Option<&Measures>, language: &str, language_p: f64, words: &[voice_repo::transcript::Word], expected: Option<&str>) -> Signals {
	let segments = measures.map_or(&[][..], |measures| measures.segments.as_slice());
	let total_ms: f64 = segments.iter().map(|segment| weight(segment.ms)).sum();
	let avg_logprob = if total_ms > 0.0 {
		segments.iter().map(|segment| segment.avg_logprob * weight(segment.ms)).sum::<f64>() / total_ms
	} else {
		0.0
	};
	let most = |of: fn(&super::engine::Segment) -> f64| segments.iter().map(of).fold(0.0, f64::max);
	let sure: Vec<f64> = words.iter().filter_map(|word| word.p).filter(|p| p.is_finite()).collect();
	let unsure_share = if sure.is_empty() {
		0.0
	} else {
		#[allow(clippy::cast_precision_loss)] // counts of words, far below 2^52
		let share = sure.iter().filter(|&&p| p < WORD_P_FLOOR).count() as f64 / sure.len() as f64;
		share
	};
	let speech_ms = measures.map_or(0, |measures| measures.speech_ms);
	#[allow(clippy::cast_precision_loss)] // a ten-minute ceiling of speech, and its words
	let words_per_second = if speech_ms > 0 { words.len() as f64 * 1000.0 / speech_ms as f64 } else { 0.0 };
	let (detected, detected_p) = measures.and_then(|measures| measures.detected.clone()).unwrap_or_else(|| (language.to_owned(), language_p));
	Signals {
		avg_logprob,
		compression_ratio: most(|segment| segment.compression_ratio),
		no_speech_prob: segments
			.iter()
			.filter(|segment| segment.avg_logprob < LOGPROB_FLOOR)
			.map(|segment| segment.no_speech_prob)
			.fold(0.0, f64::max),
		unsure_share,
		language: detected,
		language_p: detected_p,
		expected_language: expected.map(str::to_owned),
		words_per_second,
		speech_ms,
		peak_dbfs: measures.map_or(f64::NEG_INFINITY, |measures| measures.peak_dbfs),
		rms_dbfs: measures.map_or(f64::NEG_INFINITY, |measures| measures.rms_dbfs),
	}
}

/// A segment's weight: its length, and at least a millisecond, so a segment
/// whisper.cpp timed as nothing still counts.
#[allow(clippy::cast_precision_loss)] // milliseconds of a ten-minute note
fn weight(ms: i64) -> f64 {
	ms.max(1) as f64
}

/// Why `signals` say the take is garbled; empty when they do not.
fn reasons(signals: &Signals) -> Vec<&'static str> {
	let mut reasons = Vec::new();
	if signals.avg_logprob < LOGPROB_FLOOR || signals.unsure_share >= UNSURE_WORD_SHARE {
		reasons.push("low_confidence");
	}
	if signals.compression_ratio > COMPRESSION_CEILING {
		reasons.push("repetitive");
	}
	// `signals` keeps the no-speech probability of unsure segments only, as
	// Whisper pairs the two.
	if signals.no_speech_prob > NO_SPEECH_CEILING {
		reasons.push("no_speech");
	}
	if signals.expected_language.as_deref().is_some_and(|expected| expected != signals.language) {
		reasons.push("off_language");
	}
	if signals.speech_ms >= SPARSE_MIN_SPEECH_MS && signals.words_per_second < SPARSE_WORDS_PER_SECOND {
		reasons.push("sparse");
	}
	reasons
}

#[cfg(test)]
mod tests {
	use super::{judge, Measures, RULE};
	use crate::transcribe::engine::Segment;
	use voice_repo::transcript::{Outcome, Word};
	use voice_repo::verdict::Kind;

	fn word(p: f64) -> Word {
		Word {
			text: String::from("w"),
			start_ms: 0,
			end_ms: 1,
			p: Some(p),
			language: Some(String::from("en")),
		}
	}

	fn heard(ps: &[f64]) -> Outcome {
		Outcome::Heard {
			language: String::from("en"),
			language_p: 0.97,
			text: String::from("w"),
			words: ps.iter().copied().map(word).collect(),
		}
	}

	fn measures(avg_logprob: f64, compression_ratio: f64, no_speech_prob: f64, speech_ms: i64, detected: &str) -> Measures {
		Measures {
			segments: vec![Segment {
				ms: speech_ms,
				avg_logprob,
				compression_ratio,
				no_speech_prob,
			}],
			speech_ms,
			peak_dbfs: -6.0,
			rms_dbfs: -28.0,
			detected: Some((detected.to_owned(), 0.9)),
		}
	}

	fn kind_and_reasons(outcome: &Outcome, measures: Option<&Measures>, expected: Option<&str>) -> (Kind, Vec<String>) {
		let verdict = judge(outcome, measures, expected, true).unwrap();
		assert_eq!(verdict.rule, RULE);
		(verdict.kind, verdict.reasons)
	}

	#[test]
	fn a_clear_take_is_heard() {
		let fine = heard(&[0.9; 20]);
		let (kind, reasons) = kind_and_reasons(&fine, Some(&measures(-0.3, 1.4, 0.01, 8_000, "en")), Some("en"));
		assert_eq!((kind, reasons.len()), (Kind::Heard, 0));
	}

	/// QM-311's shape: most words below half, the model's own log-prob not
	/// yet past its floor.
	#[test]
	fn most_words_unsure_is_garbled() {
		let mut ps = vec![0.3; 23];
		ps.extend([0.9; 15]);
		let (kind, reasons) = kind_and_reasons(&heard(&ps), Some(&measures(-0.7, 1.5, 0.1, 25_000, "en")), Some("en"));
		assert_eq!((kind, reasons), (Kind::Garbled, vec![String::from("low_confidence")]));
		let mut fine = vec![0.3; 5];
		fine.extend([0.9; 15]);
		assert_eq!(kind_and_reasons(&heard(&fine), None, None).0, Kind::Heard, "a quarter unsure is a fine take");
	}

	#[test]
	fn whispers_own_thresholds_garble_a_take() {
		let fine = heard(&[0.9; 30]);
		let reasons = |m: Measures| kind_and_reasons(&fine, Some(&m), None).1;
		assert_eq!(reasons(measures(-1.2, 1.4, 0.1, 12_000, "en")), ["low_confidence"]);
		assert_eq!(reasons(measures(-0.2, 3.0, 0.1, 12_000, "en")), ["repetitive"]);
		assert_eq!(reasons(measures(-1.2, 1.4, 0.8, 12_000, "en")), ["low_confidence", "no_speech"]);
		assert!(
			reasons(measures(-0.2, 1.4, 0.8, 12_000, "en")).is_empty(),
			"no speech counts only with a low log-prob, as Whisper pairs them"
		);
	}

	/// QM-312's shape: a take of an English kind heard as another language.
	#[test]
	fn another_language_than_the_kinds_is_garbled() {
		let fine = heard(&[0.9; 30]);
		assert_eq!(kind_and_reasons(&fine, Some(&measures(-0.2, 1.4, 0.0, 12_000, "pt")), Some("en")).1, ["off_language"]);
		assert_eq!(
			kind_and_reasons(&fine, Some(&measures(-0.2, 1.4, 0.0, 12_000, "ko")), None).0,
			Kind::Heard,
			"a reflection may be Korean"
		);
	}

	#[test]
	fn a_long_take_with_few_words_is_sparse() {
		let few = heard(&[0.9; 8]);
		assert_eq!(kind_and_reasons(&few, Some(&measures(-0.2, 1.4, 0.0, 20_000, "en")), None).1, ["sparse"]);
		assert_eq!(
			kind_and_reasons(&few, Some(&measures(-0.2, 1.4, 0.0, 9_000, "en")), None).0,
			Kind::Heard,
			"too short to say"
		);
	}

	#[test]
	fn silence_is_nothing_said_and_only_a_last_failure_is_failed() {
		assert_eq!(kind_and_reasons(&Outcome::Silent, None, Some("en")), (Kind::NothingSaid, Vec::new()));
		assert_eq!(kind_and_reasons(&Outcome::Deadline, None, None), (Kind::Failed, vec![String::from("deadline")]));
		assert_eq!(kind_and_reasons(&Outcome::Failed, None, None).1, ["engine"]);
		assert_eq!(judge(&Outcome::Unreadable, None, None, false), None, "retried: nothing final");
	}

	/// paulgsc/server#913: an ending heard as white space alone is nothing
	/// said, whatever the engine measured of it, so it is filed as a silence.
	#[test]
	fn words_of_white_space_alone_are_nothing_said() {
		for blank in ["", " ", "\n\t", "\u{a0}\u{3000}"] {
			let outcome = Outcome::Heard {
				language: String::from("en"),
				language_p: 0.97,
				text: blank.to_owned(),
				words: Vec::new(),
			};
			let measured = measures(-0.3, 1.4, 0.01, 8_000, "en");
			assert_eq!(kind_and_reasons(&outcome, Some(&measured), Some("en")), (Kind::NothingSaid, Vec::new()), "{blank:?}");
		}
	}
}
